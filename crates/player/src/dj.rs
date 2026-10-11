//! E5b: DJ Livi en el motor propio de Velo (sin el celular ni la app oficial).
//!
//! Como la app oficial: el contexto DJ (`spotify:playlist:37i9dQZF1EYkqdzj48dyYq`) trae
//! `lexicon_context_url`; la sesión `lexicon-session-provider` (servicio interno de Spotify, no
//! documentado) entrega bloques de unas cinco canciones cuyos metadatos llevan narraciones SSML
//! (`narration.intro.*` antes de la canción, `narration.outro.*` después) y `next_page_url` para
//! el bloque siguiente. La voz se pide al TTS de Spotify (`client-tts/v1/fulfill`, protobuf
//! `TtsRequest` codificado a mano; responde 303 hacia un MP3 firmado de `tts.spotifycdn.com`),
//! se decodifica en memoria (symphonia) y suena por la misma salida que las canciones (volumen,
//! fades y visualizador, `pcm.rs`). Nunca se escribe audio en disco.
//!
//! Referencias: spotlightishere/spotdj (MIT) y `client-tts.proto` / `tts-resolve.proto` de
//! librespot-protocol 0.8. Los registros nunca incluyen tokens, URLs firmadas ni SSML.

use std::{
    collections::HashMap,
    future::Future,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use http::{Method, Request};
use librespot_core::Session;
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

use crate::{
    EngineState, TrackEndTimerState,
    extras::DJ_LIVI_URI,
    pcm::{VoiceClip, VoiceProgress},
};

const TIMEOUT: Duration = Duration::from_secs(15);
/// Lo que se espera a una voz precargada antes de seguir solo con música.
const VOICE_WAIT: Duration = Duration::from_secs(6);
/// La primera voz de un bloque se pide en el momento: margen para TTS + descarga.
const FIRST_VOICE_WAIT: Duration = Duration::from_secs(10);
/// Tras cargar un bloque, lo que tarda Spirc en mostrar su primera canción.
const LOAD_WAIT: Duration = Duration::from_secs(10);
/// Tras el límite de una canción, lo que tarda en aparecer la siguiente (ya precargada).
const NEXT_TRACK_WAIT: Duration = Duration::from_secs(3);
/// Margen sobre la duración de la voz antes de darla por colgada (salida que no consume).
const VOICE_STALL: Duration = Duration::from_secs(3);
const MAX_VOICE_BYTES: usize = 8 * 1024 * 1024;
const MAX_VOICE_SECONDS: usize = 120;
const MAX_SSML_BYTES: usize = 16 * 1024;
const MAX_BLOCK_TRACKS: usize = 100;
/// Volumen de referencia de la normalización de Spotify (LUFS) al que se lleva la voz.
const TARGET_LUFS: f32 = -14.0;
const OUTPUT_RATE: u32 = librespot_playback::SAMPLE_RATE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DjError {
    #[error("Spotify no entregó la sesión de DJ")]
    Unavailable,
    #[error("La sesión de DJ no trae canciones")]
    Empty,
    #[error("El TTS de Spotify respondió {0}")]
    Tts(u16),
    #[error("La voz de DJ no se pudo decodificar")]
    Decode,
    #[error("Spotify tardó demasiado")]
    Timeout,
    #[error("El motor de audio se detuvo")]
    Stopped,
}

/// Una narración de Livi tal como la manda la sesión lexicon.
#[derive(Debug, Clone, PartialEq)]
pub struct Narration {
    pub ssml: String,
    /// `TtsVoice` (VOICE35 → 35).
    pub voice: u64,
    /// `TtsProvider` (SONANTIC_FAST → 6).
    pub provider: u64,
    pub sample_rate: u32,
    pub loudness_db: Option<f32>,
    pub true_peak_db: Option<f32>,
    /// Imagen de Livi (https), la carátula mientras habla.
    pub image: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DjTrack {
    pub uri: String,
    pub intro: Option<Narration>,
    pub outro: Option<Narration>,
    /// `dj.jump_target`: se conserva para depurar; Velo «cambia de onda» pidiendo el bloque
    /// siguiente (ver docs/E5-E6.md).
    pub jump_target: Option<String>,
    pub segment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DjBlock {
    pub tracks: Vec<DjTrack>,
    /// `hm://lexicon-session-provider/...` del bloque siguiente.
    pub next_page_url: Option<String>,
    /// Texto localizado del botón «Elige tú, Livi» que manda Spotify.
    pub jump_label: Option<String>,
}

impl DjBlock {
    fn index_of(&self, uri: &str) -> Option<usize> {
        self.tracks.iter().position(|t| t.uri == uri)
    }
    fn uris(&self) -> Vec<String> {
        self.tracks.iter().map(|t| t.uri.clone()).collect()
    }
    /// Huella del bloque (primera canción y su presentación): dos sesiones con la misma
    /// huella son el mismo set que Spotify devolvió de nuevo.
    pub(crate) fn fingerprint(&self) -> Option<String> {
        let first = self.tracks.first()?;
        let intro = first.intro.as_ref().map_or("", |n| n.ssml.as_str());
        Some(format!("{}|{intro}", first.uri))
    }
}

/// Lo que la app muestra del DJ (parte de [`EngineState`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DjState {
    pub active: bool,
    /// Livi está hablando: sustituye a la canción en la UI y en el panel multimedia.
    pub speech: Option<DjSpeech>,
    pub jump_label: Option<String>,
    /// Cuántos bloques (ondas) empezaron en esta sesión; sube con cada «Cambiar de onda».
    pub section: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DjSpeech {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub image_url: Option<String>,
    /// La narración como texto plano (SSML sin etiquetas).
    pub line: String,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub position_at: Instant,
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DjCommand {
    /// «Siguiente» con el DJ: salta la voz, o la canción (al final del bloque, al bloque nuevo).
    Next,
    /// «Elige tú, Livi»: el bloque siguiente con su presentación.
    Jump,
}

// ------------------------------------------------------------------------------ lexicon

fn text(meta: &Value, key: &str) -> Option<String> {
    meta.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn voice_number(name: &str) -> Option<u64> {
    let n: u64 = name.trim().trim_start_matches("VOICE").parse().ok()?;
    (1..=40).contains(&n).then_some(n)
}

fn provider_number(name: &str) -> Option<u64> {
    Some(match name.trim() {
        "CLOUD_TTS" => 1,
        "READSPEAKER" => 2,
        "POLLY" => 3,
        "WELL_SAID" => 4,
        "SONANTIC_DEPRECATED" => 5,
        "SONANTIC_FAST" => 6,
        other => other.parse().ok().filter(|n| (1..=6).contains(n))?,
    })
}

fn narration(meta: &Value, kind: &str) -> Option<Narration> {
    let key = |k: &str| text(meta, &format!("narration.{kind}.{k}"));
    let ssml = key("ssml").filter(|s| s.len() <= MAX_SSML_BYTES)?;
    let number = |k: &str| {
        key(k)
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite())
    };
    Some(Narration {
        ssml,
        voice: voice_number(&key("voice")?)?,
        provider: provider_number(&key("tts_provider").unwrap_or_else(|| "SONANTIC_FAST".into()))?,
        sample_rate: key("sample_rate")
            .and_then(|v| v.parse().ok())
            .filter(|r| (8_000..=48_000).contains(r))
            .unwrap_or(OUTPUT_RATE),
        loudness_db: number("loudness"),
        true_peak_db: number("true_peak"),
        image: key("image").filter(|u| u.starts_with("https://")),
        title: key("title").unwrap_or_else(|| "DJ".into()),
        artist: key("artist").unwrap_or_else(|| "DJ Livi".into()),
        album: key("album_title").unwrap_or_else(|| "DJ".into()),
    })
}

/// Las páginas siguientes son rutas internas `hm://` (spclient les quita el prefijo).
fn valid_page_url(url: &str) -> bool {
    url.strip_prefix("hm://").is_some_and(|rest| {
        !rest.is_empty()
            && !rest.contains("..")
            && !rest.contains("://")
            && rest.bytes().all(|b| b.is_ascii_graphic())
    })
}

fn parse_value(value: &Value) -> Result<DjBlock, DjError> {
    let mut tracks = Vec::new();
    let mut next_page_url = None;
    for page in value["pages"].as_array().into_iter().flatten() {
        for track in page["tracks"].as_array().into_iter().flatten() {
            let Some(uri) = track["uri"]
                .as_str()
                .filter(|u| crate::parse_track_uri(u).is_ok())
            else {
                continue; // episodios, anuncios o entradas sin URI: el DJ solo pone canciones
            };
            if tracks.len() == MAX_BLOCK_TRACKS {
                break;
            }
            let meta = &track["metadata"];
            tracks.push(DjTrack {
                uri: uri.to_owned(),
                intro: narration(meta, "intro"),
                outro: narration(meta, "outro"),
                jump_target: text(meta, "dj.jump_target"),
                segment: text(meta, "segment"),
            });
        }
        let next = page
            .get("next_page_url")
            .or_else(|| page.get("nextPageUrl"))
            .and_then(Value::as_str);
        if let Some(url) = next.filter(|u| valid_page_url(u)) {
            next_page_url = Some(url.to_owned());
        }
    }
    if tracks.is_empty() {
        return Err(DjError::Empty);
    }
    Ok(DjBlock {
        tracks,
        next_page_url,
        jump_label: text(&value["metadata"], "dj.interactivity.localized_jump_button"),
    })
}

/// Un bloque de la sesión lexicon (JSON de `context-resolve`).
pub fn parse_block(json: &[u8]) -> Result<DjBlock, DjError> {
    let value: Value = serde_json::from_slice(json).map_err(|_| DjError::Unavailable)?;
    parse_value(&value)
}

/// Sin `lexicon_context_url`, las canciones del propio contexto (sin narraciones).
fn block_from_context(context: &librespot_protocol::context::Context) -> Result<DjBlock, DjError> {
    let pages: Vec<Value> = context
        .pages
        .iter()
        .map(|page| {
            let tracks: Vec<Value> = page
                .tracks
                .iter()
                .map(|t| json!({"uri": t.uri.clone().unwrap_or_default(), "metadata": t.metadata}))
                .collect();
            json!({"tracks": tracks, "next_page_url": page.next_page_url.clone()})
        })
        .collect();
    parse_value(&json!({"metadata": context.metadata, "pages": pages}))
}

/// Marca única por petición: Spotify puede devolver la misma sesión cacheada para la misma
/// URL, y con esta marca cada inicio pide un set nuevo.
fn nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos:x}{:x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn with_nonce(url: &str, nonce: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}nonce={nonce}")
}

/// La sesión DJ: resuelve el contexto y pide el primer bloque (con `nonce`, marcado como
/// petición nueva).
async fn fetch_session_block(session: &Session, nonce: Option<&str>) -> Result<DjBlock, DjError> {
    let spclient = session.spclient();
    let context = tokio::time::timeout(TIMEOUT, spclient.get_context(DJ_LIVI_URI))
        .await
        .map_err(|_| DjError::Timeout)?
        .map_err(|_| DjError::Unavailable)?;
    let url = match context.metadata.get("lexicon_context_url") {
        Some(url) => match nonce {
            Some(nonce) => with_nonce(url, nonce),
            None => url.clone(),
        },
        None => return block_from_context(&context),
    };
    fetch_page(session, &url).await
}

async fn fetch_page(session: &Session, url: &str) -> Result<DjBlock, DjError> {
    if !valid_page_url(url) {
        return Err(DjError::Unavailable);
    }
    let body = tokio::time::timeout(TIMEOUT, session.spclient().get_next_page(url))
        .await
        .map_err(|_| DjError::Timeout)?
        .map_err(|_| DjError::Unavailable)?;
    parse_block(&body)
}

/// El primer bloque (`None`: resuelve la sesión desde el contexto DJ) o el de `next_page_url`.
pub async fn fetch_block(session: &Session, url: Option<&str>) -> Result<DjBlock, DjError> {
    match url {
        Some(url) => fetch_page(session, url).await,
        None => fetch_session_block(session, None).await,
    }
}

/// Una sesión DJ NUEVA: cada petición lleva una marca única y, si aun así Spotify devuelve el
/// set de `avoid` (huella del anterior), se reintenta con otra marca. Si Spotify rechaza la
/// marca, cae a la petición simple. Nunca falla por repetir: devuelve lo último que llegó.
pub(crate) async fn fresh_session<F, Fut>(
    mut fetch: F,
    avoid: Option<&str>,
) -> Result<DjBlock, DjError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<DjBlock, DjError>>,
{
    let mut last = None;
    for _ in 0..3 {
        let block = match fetch(Some(nonce())).await {
            Ok(block) => block,
            Err(_) => return fetch(None).await,
        };
        if avoid.is_none_or(|a| block.fingerprint().as_deref() != Some(a)) {
            return Ok(block);
        }
        last = Some(block);
    }
    last.ok_or(DjError::Unavailable)
}

/// [`fresh_session`] contra Spotify.
pub async fn fetch_fresh_block(session: &Session, avoid: Option<&str>) -> Result<DjBlock, DjError> {
    fresh_session(
        |nonce| async move { fetch_session_block(session, nonce.as_deref()).await },
        avoid,
    )
    .await
}

// ------------------------------------------------------------------------------ voz (TTS)

fn varint(buf: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

/// `spotify.narration.proto.TtsRequest`: ssml = 2, audio_format = 3 (MP3 = 5), tts_voice = 5,
/// tts_provider = 6, sample_rate_hz = 7. librespot-protocol trae el .proto pero no lo compila.
pub fn tts_request(narration: &Narration) -> Vec<u8> {
    let mut body = vec![0x12];
    varint(&mut body, narration.ssml.len() as u64);
    body.extend_from_slice(narration.ssml.as_bytes());
    for (tag, value) in [
        (0x18, 5),
        (0x28, narration.voice),
        (0x30, narration.provider),
        (0x38, u64::from(narration.sample_rate)),
    ] {
        body.push(tag);
        varint(&mut body, value);
    }
    body
}

/// `TtsResponse { string url = 1 }` (por si el TTS contesta 200 en vez de redirigir).
fn tts_response_url(body: &[u8]) -> Option<String> {
    let mut at = 0;
    let read_varint = |at: &mut usize| -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *body.get(*at)?;
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    };
    while at < body.len() {
        let key = read_varint(&mut at)?;
        match key & 7 {
            0 => {
                read_varint(&mut at)?;
            }
            2 => {
                let len = usize::try_from(read_varint(&mut at)?).ok()?;
                let bytes = body.get(at..at.checked_add(len)?)?;
                at += len;
                if key >> 3 == 1 {
                    return String::from_utf8(bytes.to_vec()).ok();
                }
            }
            _ => return None,
        }
    }
    None
}

fn looks_like_mp3(body: &[u8]) -> bool {
    body.starts_with(b"ID3") || (body.len() > 1 && body[0] == 0xFF && body[1] & 0xE0 == 0xE0)
}

/// La URL firmada del audio: https (o loopback, para el servidor de pruebas).
fn valid_audio_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://127.0.0.1:")
}

/// Respuesta HTTP mínima que necesita el flujo del TTS.
pub(crate) struct HttpReply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Bytes,
}

/// POST del `TtsRequest`; 3xx → GET (sin credenciales) del MP3 de `Location`. Inyectable para
/// probar con un servidor local; nunca registra la URL firmada.
pub(crate) async fn fulfill<S, Fut>(
    url: &str,
    authorization: &str,
    client_token: Option<&str>,
    request: Vec<u8>,
    send: S,
) -> Result<Bytes, DjError>
where
    S: Fn(Request<Bytes>) -> Fut,
    Fut: Future<Output = Result<HttpReply, DjError>>,
{
    let mut post = Request::builder()
        .method(Method::POST)
        .uri(url)
        .header("authorization", authorization)
        .header("content-type", "application/x-protobuf")
        .header("accept", "*/*");
    if let Some(token) = client_token {
        post = post.header("client-token", token);
    }
    let reply = send(
        post.body(Bytes::from(request))
            .map_err(|_| DjError::Unavailable)?,
    )
    .await?;
    let audio_url = match reply.status {
        300..=399 => reply.location.ok_or(DjError::Tts(reply.status))?,
        200..=299 if looks_like_mp3(&reply.body) => return Ok(reply.body),
        200..=299 => tts_response_url(&reply.body).ok_or(DjError::Tts(reply.status))?,
        status => return Err(DjError::Tts(status)),
    };
    if !valid_audio_url(&audio_url) {
        return Err(DjError::Unavailable);
    }
    let get = Request::builder()
        .method(Method::GET)
        .uri(audio_url)
        .body(Bytes::new())
        .map_err(|_| DjError::Unavailable)?;
    let reply = send(get).await?;
    if !(200..300).contains(&reply.status) {
        return Err(DjError::Tts(reply.status));
    }
    if reply.body.is_empty() {
        return Err(DjError::Decode);
    }
    Ok(reply.body)
}

async fn session_send(
    http: &librespot_core::http_client::HttpClient,
    request: Request<Bytes>,
) -> Result<HttpReply, DjError> {
    use http_body_util::BodyExt;
    // request_fut: las redirecciones llegan como respuesta (request() las trata como error).
    let response = tokio::time::timeout(
        TIMEOUT,
        http.request_fut(request)
            .map_err(|_| DjError::Unavailable)?,
    )
    .await
    .map_err(|_| DjError::Timeout)?
    .map_err(|_| DjError::Unavailable)?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = if (300..400).contains(&status) {
        Bytes::new()
    } else {
        let limited = http_body_util::Limited::new(response.into_body(), MAX_VOICE_BYTES);
        tokio::time::timeout(TIMEOUT, limited.collect())
            .await
            .map_err(|_| DjError::Timeout)?
            .map_err(|_| DjError::Unavailable)?
            .to_bytes()
    };
    Ok(HttpReply {
        status,
        location,
        body,
    })
}

/// La voz de una narración, decodificada a PCM en memoria (sin tocar el disco).
pub async fn fetch_voice(session: &Session, narration: &Narration) -> Result<VoiceClip, DjError> {
    let spclient = session.spclient();
    let base = spclient
        .base_url()
        .await
        .map_err(|_| DjError::Unavailable)?;
    let token = session
        .login5()
        .auth_token()
        .await
        .map_err(|_| DjError::Unavailable)?;
    let authorization = format!("{} {}", token.token_type, token.access_token);
    let client_token = spclient.client_token().await.ok();
    let http = session.http_client();
    let mp3 = fulfill(
        &format!("{base}/client-tts/v1/fulfill"),
        &authorization,
        client_token.as_deref(),
        tts_request(narration),
        |request| session_send(http, request),
    )
    .await?;
    let gain = voice_gain(narration.loudness_db, narration.true_peak_db);
    tokio::task::spawn_blocking(move || decode_voice(&mp3, gain))
        .await
        .map_err(|_| DjError::Decode)?
}

/// Ganancia lineal que lleva la voz a la referencia de las canciones sin pasar de -1 dBTP.
pub fn voice_gain(loudness_db: Option<f32>, true_peak_db: Option<f32>) -> f32 {
    let Some(loudness) = loudness_db else {
        return 1.0;
    };
    let mut db = (TARGET_LUFS - loudness).clamp(-12.0, 6.0);
    match true_peak_db {
        Some(peak) => db = db.min(-1.0 - peak),
        None => db = db.min(0.0),
    }
    10f32.powf(db / 20.0)
}

/// Interpolación lineal (la voz de Spotify ya llega a 44,1 kHz; esto es la red de seguridad).
fn resample(samples: Vec<f32>, channels: usize, from: u32, to: u32) -> Vec<f32> {
    let frames = samples.len() / channels;
    if from == to || from == 0 || frames < 2 {
        return samples;
    }
    let out_frames = (frames as u64 * u64::from(to) / u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(to);
    let mut out = Vec::with_capacity(out_frames * channels);
    for i in 0..out_frames {
        let pos = i as f64 * step;
        let i0 = (pos as usize).min(frames - 1);
        let i1 = (i0 + 1).min(frames - 1);
        let t = (pos - i0 as f64) as f32;
        for c in 0..channels {
            let a = samples[i0 * channels + c];
            let b = samples[i1 * channels + c];
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// MP3 → PCM f32 a 44,1 kHz (mono o estéreo) con la ganancia de la narración.
pub fn decode_voice(mp3: &[u8], gain: f32) -> Result<VoiceClip, DjError> {
    use symphonia::core::{
        audio::SampleBuffer, codecs::CODEC_TYPE_NULL, codecs::DecoderOptions, errors::Error,
        formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
    };
    let source = MediaSourceStream::new(Box::new(Cursor::new(mp3.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| DjError::Decode)?
        .format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DjError::Decode)?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| DjError::Decode)?;
    let (mut samples, mut rate, mut channels) = (Vec::new(), 0u32, 0usize);
    // Fin del stream (o basura al final): lo ya decodificado vale.
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        if channels == 0 {
            (rate, channels) = (spec.rate, spec.channels.count());
        }
        if spec.channels.count() != channels || channels == 0 {
            continue;
        }
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
        if samples.len() > MAX_VOICE_SECONDS * 48_000 * channels {
            break;
        }
    }
    if samples.is_empty() || channels == 0 || rate == 0 {
        return Err(DjError::Decode);
    }
    let (samples, channels) = if channels <= 2 {
        (samples, channels)
    } else {
        let stereo = samples
            .chunks_exact(channels)
            .flat_map(|frame| [frame[0], frame[1]])
            .collect();
        (stereo, 2)
    };
    let samples = resample(samples, channels, rate, OUTPUT_RATE)
        .into_iter()
        .map(|s| s * gain)
        .collect();
    VoiceClip::new(samples, channels).ok_or(DjError::Decode)
}

/// El texto que dice Livi: SSML sin etiquetas ni entidades, con espacios normalizados.
pub fn plain_text(ssml: &str) -> String {
    let mut out = String::with_capacity(ssml.len());
    let mut tag = String::new();
    let mut in_tag = false;
    let mut quote = None;
    for c in ssml.chars() {
        match c {
            '<' if !in_tag => {
                in_tag = true;
                tag.clear();
                quote = None;
            }
            '\'' | '"' if in_tag => {
                if quote == Some(c) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(c);
                }
                tag.push(c);
            }
            '>' if in_tag && quote.is_none() => {
                in_tag = false;
                let name = tag.trim_start_matches('/').split([' ', '/']).next();
                if matches!(name, Some("break" | "p" | "s" | "speak")) {
                    out.push(' ');
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let mut decoded = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(at) = rest.find('&') {
        decoded.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity = rest.find(';').filter(|end| *end <= 16);
        let character = entity.and_then(|end| match &rest[1..end] {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "amp" => Some('&'),
            numeric => numeric
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| numeric.strip_prefix('#').and_then(|n| n.parse().ok()))
                .and_then(char::from_u32),
        });
        if let (Some(end), Some(character)) = (entity, character) {
            decoded.push(character);
            rest = &rest[end + 1..];
        } else {
            decoded.push('&');
            rest = &rest[1..];
        }
    }
    decoded.push_str(rest);
    let mut line = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    for mark in [",", ".", "!", "?", ";", ":"] {
        line = line.replace(&format!(" {mark}"), mark);
    }
    line
}

// ------------------------------------------------------------------------------ controlador

/// Lo que el controlador necesita del motor (inyectable en las pruebas).
pub(crate) trait DjHost: Send + Sync + 'static {
    fn fetch_block(
        &self,
        url: Option<String>,
    ) -> impl Future<Output = Result<DjBlock, DjError>> + Send;
    fn fetch_voice(
        &self,
        narration: Narration,
    ) -> impl Future<Output = Result<VoiceClip, DjError>> + Send;
    /// Carga el bloque como lista explícita; `play: false` lo deja en pausa en la primera.
    fn load(&self, uris: Vec<String>, play: bool) -> Result<(), DjError>;
    fn resume(&self) -> Result<(), DjError>;
    fn pause(&self);
    /// «Siguiente» normal de Spirc.
    fn skip(&self) -> Result<(), DjError>;
    /// Pausa en el límite audible de `uri` (mecanismo de «al terminar esta canción»).
    fn arm(&self, uri: &str) -> Option<watch::Receiver<TrackEndTimerState>>;
    fn speak(&self, clip: Arc<VoiceClip>) -> Option<watch::Receiver<VoiceProgress>>;
    fn stop_voice(&self);
    fn release_output(&self);
    fn state(&self) -> watch::Receiver<EngineState>;
    fn publish(&self, dj: DjState);
    fn error(&self, error: DjError);
}

/// Aborta la tarea al soltarse (precargas que ya no hacen falta).
struct Pending<T>(JoinHandle<T>);
impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

type VoiceTask = Pending<Result<Arc<VoiceClip>, DjError>>;

/// El DJ terminó (otra carga, otro dispositivo, motor cerrado o sin más bloques).
struct End;

enum Step {
    /// Empezar el bloque actual; puede llevar el cierre de la última canción del anterior.
    Begin(Option<Narration>),
    Jump,
}

struct Dj<H: DjHost> {
    host: Arc<H>,
    block: DjBlock,
    next: Option<Pending<Result<DjBlock, DjError>>>,
    voices: HashMap<String, VoiceTask>,
    current: Option<String>,
    armed: Option<(String, watch::Receiver<TrackEndTimerState>)>,
    cmds: mpsc::UnboundedReceiver<DjCommand>,
    state: watch::Receiver<EngineState>,
    was_active: bool,
    jump_pending: bool,
    view: DjState,
}

enum Event {
    Cmd(Option<DjCommand>),
    State(bool),
    Boundary(Option<TrackEndTimerState>),
}

fn key(uri: &str, kind: &str) -> String {
    format!("{uri}|{kind}")
}

/// Corre el DJ hasta que termina; siempre deja el estado inactivo y la salida libre.
pub(crate) async fn run<H: DjHost>(
    host: Arc<H>,
    first: DjBlock,
    cmds: mpsc::UnboundedReceiver<DjCommand>,
) {
    let state = host.state();
    let mut dj = Dj {
        view: DjState {
            active: true,
            speech: None,
            jump_label: first.jump_label.clone(),
            section: 0,
        },
        host: host.clone(),
        block: first,
        next: None,
        voices: HashMap::new(),
        current: None,
        armed: None,
        cmds,
        state,
        was_active: false,
        jump_pending: false,
    };
    let _ = dj.main().await;
    drop(dj);
    host.stop_voice();
    host.release_output();
    host.publish(DjState::default());
}

impl<H: DjHost> Dj<H> {
    fn failure(&self, error: DjError) -> End {
        self.host.error(error);
        End
    }
    fn publish(&self) {
        self.host.publish(self.view.clone());
    }

    async fn main(&mut self) -> Result<(), End> {
        self.publish();
        let mut step = Some(Step::Begin(None));
        loop {
            if let Some(next) = step.take() {
                step = match next {
                    Step::Begin(outro) => self.begin(outro).await?,
                    Step::Jump => self.jump().await?,
                };
                continue;
            }
            let event = {
                let armed = &mut self.armed;
                tokio::select! {
                    cmd = self.cmds.recv() => Event::Cmd(cmd),
                    changed = self.state.changed() => Event::State(changed.is_ok()),
                    boundary = async {
                        match armed {
                            Some((_, rx)) => rx
                                .wait_for(|s| *s != TrackEndTimerState::Armed)
                                .await
                                .ok()
                                .map(|s| *s),
                            None => std::future::pending().await,
                        }
                    } => Event::Boundary(boundary),
                }
            };
            step = match event {
                Event::Cmd(None) | Event::State(false) => return Err(End),
                Event::Cmd(Some(DjCommand::Jump)) => Some(Step::Jump),
                Event::Cmd(Some(DjCommand::Next)) => self.next_track().await?,
                Event::State(true) => {
                    self.on_state()?;
                    None
                }
                Event::Boundary(state) => self.boundary(state).await?,
            };
        }
    }

    fn prefetch(&mut self, key: String, narration: Option<&Narration>) {
        let Some(narration) = narration.cloned() else {
            return;
        };
        if self.voices.contains_key(&key) {
            return;
        }
        let host = self.host.clone();
        self.voices.insert(
            key,
            Pending(tokio::spawn(async move {
                host.fetch_voice(narration).await.map(Arc::new)
            })),
        );
    }

    async fn take_voice(
        &mut self,
        key: &str,
        narration: Option<&Narration>,
        wait: Duration,
    ) -> Option<Arc<VoiceClip>> {
        narration?;
        self.prefetch(key.to_owned(), narration);
        let mut task = self.voices.remove(key)?;
        match tokio::time::timeout(wait, &mut task.0).await {
            Ok(Ok(Ok(clip))) => Some(clip),
            Ok(Ok(Err(error))) => {
                self.host.error(error);
                tracing::debug!(%error, "DJ: narración omitida, sigue la música");
                None
            }
            _ => {
                self.host.error(DjError::Timeout);
                tracing::debug!("DJ: la voz no llegó a tiempo, sigue la música");
                None
            }
        }
    }

    /// Espera a que la canción actual cumpla `accept` (o vence `wait`).
    async fn wait_track(
        &mut self,
        accept: impl Fn(Option<&str>) -> bool,
        wait: Duration,
    ) -> Option<Option<String>> {
        let state = &mut self.state;
        tokio::time::timeout(wait, async {
            loop {
                {
                    let st = state.borrow_and_update();
                    let uri = st.track.as_ref().map(|t| t.uri.as_str());
                    if accept(uri) {
                        return Some(uri.map(str::to_owned));
                    }
                }
                if state.changed().await.is_err() {
                    return None;
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    /// Bloque nuevo: presentación (y cierre pendiente), luego su primera canción.
    async fn begin(&mut self, pending_outro: Option<Narration>) -> Result<Option<Step>, End> {
        self.was_active |= self.state.borrow().active;
        self.armed = None;
        self.current = None;
        self.view.jump_label = self.block.jump_label.clone();
        self.view.section += 1;
        self.publish();
        let first = self.block.tracks.first().cloned().ok_or(End)?;
        self.prefetch(key(&first.uri, "intro"), first.intro.as_ref());
        let outro = self
            .take_voice("pending|outro", pending_outro.as_ref(), FIRST_VOICE_WAIT)
            .await;
        let intro = self
            .take_voice(
                &key(&first.uri, "intro"),
                first.intro.as_ref(),
                FIRST_VOICE_WAIT,
            )
            .await;
        let talk = outro.is_some() || intro.is_some();
        if self.was_active && !self.state.borrow().active {
            return Err(End);
        }
        self.host
            .load(self.block.uris(), !talk)
            .map_err(|error| self.failure(error))?;
        let uri = first.uri.clone();
        if self
            .wait_track(|u| u == Some(uri.as_str()), LOAD_WAIT)
            .await
            .is_none()
        {
            tracing::debug!("DJ: Spotify no cargó el bloque");
            self.host.error(DjError::Timeout);
            return Err(End);
        }
        self.was_active |= self.state.borrow().active;
        self.speak(outro, pending_outro.as_ref()).await?;
        if !self.jump_pending {
            self.speak(intro, first.intro.as_ref()).await?;
        }
        if std::mem::take(&mut self.jump_pending) {
            return Ok(Some(Step::Jump));
        }
        if talk {
            self.host.resume().map_err(|error| self.failure(error))?;
        }
        self.track_started(&first.uri);
        Ok(None)
    }

    /// «Cambiar de onda»: corta la canción y pasa al bloque siguiente con su presentación.
    /// Sin página siguiente pide una sesión nueva; si tampoco llega, la música sigue y el DJ
    /// también (el botón nunca apaga a Livi).
    async fn jump(&mut self) -> Result<Option<Step>, End> {
        if self.was_active && !self.state.borrow().active {
            return Err(End);
        }
        self.armed = None;
        self.host.pause();
        if self.load_next_block(true).await?.is_some() {
            return Ok(Some(Step::Begin(None)));
        }
        self.host.error(DjError::Unavailable);
        self.host.resume().map_err(|error| self.failure(error))?;
        if let Some(uri) = self.current.clone() {
            self.track_started(&uri);
        }
        Ok(None)
    }

    /// `Some(())` si hay bloque siguiente (ya es `self.block`); `None` si Spotify no da más.
    /// Con `fresh`, sin página siguiente se pide una sesión DJ nueva (`fetch_block(None)`).
    async fn load_next_block(&mut self, fresh: bool) -> Result<Option<()>, End> {
        let pending = match self.next.take() {
            Some(pending) => Some(pending),
            None => {
                let url = self.block.next_page_url.clone();
                (url.is_some() || fresh).then(|| {
                    let host = self.host.clone();
                    Pending(tokio::spawn(async move { host.fetch_block(url).await }))
                })
            }
        };
        let Some(mut pending) = pending else {
            return Ok(None);
        };
        match tokio::time::timeout(TIMEOUT, &mut pending.0).await {
            Ok(Ok(Ok(mut block))) => {
                if self.was_active && !self.state.borrow().active {
                    return Err(End);
                }
                if fresh && block.fingerprint() == self.block.fingerprint() {
                    // La «página siguiente» devolvió lo mismo que ya sonó: «Cambiar de onda»
                    // no puede quedarse igual, así que se pide una sesión nueva.
                    let again = tokio::time::timeout(TIMEOUT, self.host.fetch_block(None)).await;
                    if let Ok(Ok(other)) = again {
                        block = other;
                    }
                    if self.was_active && !self.state.borrow().active {
                        return Err(End);
                    }
                }
                self.block = block;
                self.voices.retain(|k, _| k == "pending|outro");
                Ok(Some(()))
            }
            Ok(Ok(Err(error))) => {
                if self.was_active && !self.state.borrow().active {
                    return Err(End);
                }
                self.host.error(error);
                Ok(None)
            }
            _ => {
                if self.was_active && !self.state.borrow().active {
                    return Err(End);
                }
                self.host.error(DjError::Timeout);
                tracing::debug!("DJ: no llegó el bloque siguiente");
                Ok(None)
            }
        }
    }

    /// «Siguiente canción». Si Livi tiene algo que decir en esta transición, no se descarta:
    /// la música espera en pausa, habla Livi y luego entra la canción siguiente.
    async fn next_track(&mut self) -> Result<Option<Step>, End> {
        let current = self.current.clone();
        let index = current.as_deref().and_then(|uri| self.block.index_of(uri));
        let len = self.block.tracks.len();
        let last = index.is_some_and(|i| i + 1 == len);
        let has_more = self.next.is_some() || self.block.next_page_url.is_some();
        if let (Some(ended), Some(i)) = (current, index) {
            let track = &self.block.tracks[i];
            let talk = track.outro.is_some()
                || (!last
                    && self
                        .block
                        .tracks
                        .get(i + 1)
                        .is_some_and(|n| n.intro.is_some()));
            if self.armed.is_some() && talk {
                self.armed = None;
                self.host.pause();
                if !last {
                    self.host.skip().map_err(|error| self.failure(error))?;
                }
                return self.transition(ended).await;
            }
        }
        if last && has_more {
            return Ok(Some(Step::Jump));
        }
        self.armed = None;
        self.host.skip().map_err(|error| self.failure(error))?;
        if last {
            return Err(End);
        }
        Ok(None)
    }

    fn on_state(&mut self) -> Result<(), End> {
        let (active, uri) = {
            let st = self.state.borrow_and_update();
            (st.active, st.track.as_ref().map(|t| t.uri.clone()))
        };
        if self.was_active && !active {
            tracing::debug!("DJ: otro dispositivo tomó la reproducción");
            return Err(End);
        }
        self.was_active |= active;
        self.sync(uri)
    }

    /// La canción de Spirc cambió: nueva canción de la sesión, o algo ajeno (fin del DJ).
    fn sync(&mut self, uri: Option<String>) -> Result<(), End> {
        let Some(uri) = uri else { return Ok(()) };
        if self.current.as_deref() == Some(uri.as_str()) {
            return Ok(());
        }
        if self.block.index_of(&uri).is_none() {
            tracing::debug!("DJ: suena algo fuera de la sesión, termina el DJ");
            return Err(End);
        }
        if self.armed.is_some() {
            // La siguiente ya se precargó antes del final audible: la toma el límite.
            return Ok(());
        }
        self.track_started(&uri);
        Ok(())
    }

    fn current_uri(&self) -> Option<String> {
        self.state.borrow().track.as_ref().map(|t| t.uri.clone())
    }

    /// Precarga lo que viene y arma la pausa en el límite si hay algo que decir al final.
    fn track_started(&mut self, uri: &str) {
        self.current = Some(uri.to_owned());
        self.armed = None;
        let Some(index) = self.block.index_of(uri) else {
            return;
        };
        let track = self.block.tracks[index].clone();
        let next = self.block.tracks.get(index + 1).cloned();
        self.prefetch(key(uri, "outro"), track.outro.as_ref());
        if let Some(next) = &next {
            self.prefetch(key(&next.uri, "intro"), next.intro.as_ref());
        }
        let more_blocks = next.is_none() && self.block.next_page_url.is_some();
        if more_blocks && self.next.is_none() {
            if let Some(url) = self.block.next_page_url.clone() {
                let host = self.host.clone();
                self.next = Some(Pending(tokio::spawn(async move {
                    host.fetch_block(Some(url)).await
                })));
            }
        }
        let talk_after = track.outro.is_some() || next.as_ref().is_some_and(|n| n.intro.is_some());
        if talk_after || more_blocks {
            self.armed = self.host.arm(uri).map(|rx| (uri.to_owned(), rx));
        }
    }

    /// La canción armada llegó a su final audible (el motor quedó en pausa en la siguiente).
    async fn boundary(&mut self, state: Option<TrackEndTimerState>) -> Result<Option<Step>, End> {
        let Some((ended, _)) = self.armed.take() else {
            return Ok(None);
        };
        if state != Some(TrackEndTimerState::Finished) {
            // Siguiente manual, otra carga o el temporizador del usuario: sin narración.
            let uri = self.current_uri();
            self.current = Some(ended);
            self.sync(uri)?;
            return Ok(None);
        }
        self.transition(ended).await
    }

    /// El cambio de canción con narración: cierre, presentación y la canción siguiente.
    async fn transition(&mut self, ended: String) -> Result<Option<Step>, End> {
        let Some(index) = self.block.index_of(&ended) else {
            return Err(End);
        };
        let outro = self.block.tracks[index].outro.clone();
        if index + 1 == self.block.tracks.len() {
            if outro.is_some() {
                // El cierre de la última canción suena antes de la presentación del bloque nuevo.
                if let Some(task) = self.voices.remove(&key(&ended, "outro")) {
                    self.voices.insert("pending|outro".into(), task);
                }
            }
            if self.load_next_block(false).await?.is_some() {
                return Ok(Some(Step::Begin(outro)));
            }
            let clip = self
                .take_voice("pending|outro", outro.as_ref(), VOICE_WAIT)
                .await;
            self.speak(clip, outro.as_ref()).await?;
            return Err(End);
        }
        let following = self
            .wait_track(|u| u.is_some_and(|u| u != ended), NEXT_TRACK_WAIT)
            .await
            .flatten();
        let Some(following) = following else {
            // Repetir canción: sigue la misma; se vuelve a armar.
            self.host.resume().map_err(|error| self.failure(error))?;
            self.track_started(&ended);
            return Ok(None);
        };
        let intro = self
            .block
            .index_of(&following)
            .and_then(|i| self.block.tracks[i].intro.clone());
        let outro_clip = self
            .take_voice(&key(&ended, "outro"), outro.as_ref(), VOICE_WAIT)
            .await;
        self.speak(outro_clip, outro.as_ref()).await?;
        if !self.jump_pending {
            let intro_clip = self
                .take_voice(&key(&following, "intro"), intro.as_ref(), VOICE_WAIT)
                .await;
            self.speak(intro_clip, intro.as_ref()).await?;
        }
        if std::mem::take(&mut self.jump_pending) {
            return Ok(Some(Step::Jump));
        }
        self.host.resume().map_err(|error| self.failure(error))?;
        if self.block.index_of(&following).is_some() {
            self.track_started(&following);
        }
        Ok(None)
    }

    /// Hace sonar una voz y publica sus datos hasta que termina (o se salta).
    async fn speak(
        &mut self,
        clip: Option<Arc<VoiceClip>>,
        narration: Option<&Narration>,
    ) -> Result<(), End> {
        let (Some(clip), Some(narration)) = (clip, narration) else {
            return Ok(());
        };
        if self.was_active && !self.state.borrow().active {
            return Err(End);
        }
        let duration = clip.duration_ms();
        let Some(mut progress) = self.host.speak(clip) else {
            self.host.error(DjError::Stopped);
            return Ok(());
        };
        let started = Instant::now();
        self.view.speech = Some(DjSpeech {
            title: narration.title.clone(),
            artist: "DJ Livi".into(),
            album: narration.album.clone(),
            image_url: narration.image.clone(),
            line: plain_text(&narration.ssml),
            duration_ms: duration,
            position_ms: 0,
            position_at: started,
            playing: true,
        });
        self.publish();
        let result = loop {
            let now = *progress.borrow_and_update();
            if now.finished {
                break Ok(());
            }
            if let Some(speech) = self.view.speech.as_mut() {
                if (speech.position_ms, speech.playing) != (now.position_ms, now.playing)
                    || speech.position_at != now.at
                {
                    speech.position_ms = now.position_ms;
                    speech.position_at = now.at;
                    speech.playing = now.playing;
                    self.host.publish(self.view.clone());
                }
            }
            // Vigía de una salida que no consume (sin reloj mientras la voz está en pausa).
            let remaining = Duration::from_millis(duration.saturating_sub(now.position_ms))
                .saturating_sub(now.at.elapsed())
                + VOICE_STALL;
            tokio::select! {
                changed = progress.changed() => if changed.is_err() { break Ok(()) },
                changed = self.state.changed() => {
                    let active = self.state.borrow_and_update().active;
                    if changed.is_err() || (self.was_active && !active) {
                        self.host.stop_voice();
                        break Err(End);
                    }
                    self.was_active |= active;
                },
                cmd = self.cmds.recv() => match cmd {
                    None => {
                        self.host.stop_voice();
                        break Err(End);
                    }
                    // El diálogo de Livi siempre suena: «siguiente» no lo descarta.
                    Some(DjCommand::Next) => {}
                    Some(DjCommand::Jump) => {
                        self.jump_pending = true;
                        self.host.stop_voice();
                    }
                },
                _ = tokio::time::sleep(remaining), if now.playing => {
                    tracing::debug!("DJ: la salida no avanzó con la voz");
                    self.host.error(DjError::Timeout);
                    self.host.stop_voice();
                    break Ok(());
                }
            }
        };
        self.view.speech = None;
        self.publish();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssml_text_decodes_entities_once_and_respects_boundaries_and_quoted_tags() {
        assert_eq!(
            plain_text(
                "<speak><p> Hola &#161;Livi! </p><p>A &amp; B<break time='1s'/>C &#xE1; &quot;x&quot; &apos;y&apos; &lt;3</p></speak>"
            ),
            "Hola ¡Livi! A & B C á \"x\" 'y' <3"
        );
        assert_eq!(
            plain_text(
                "<speak><prosody pitch='>0'>uno</prosody>, dos &amp;lt; &unknown; &#xD800;</speak>"
            ),
            "uno, dos &lt; &unknown; &#xD800;"
        );
    }
    use std::sync::Mutex;

    const A: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
    const B: &str = "spotify:track:7ouMYWpwJ422jRcDASZB7P";
    const C: &str = "spotify:track:0VjIjW4GlUZAMYd2vXMi3b";

    fn narration_meta(kind: &str, line: &str) -> Value {
        json!({
            format!("narration.{kind}.ssml"): format!("<speak xml:lang=\"es-MX\">{line} <entity type=\"artist\" uri=\"spotify:artist:x\">Banda Inventada</entity>, ¡dale!</speak>"),
            format!("narration.{kind}.voice"): "VOICE35",
            format!("narration.{kind}.tts_provider"): "SONANTIC_FAST",
            format!("narration.{kind}.sample_rate"): "44100",
            format!("narration.{kind}.loudness"): "-16.0",
            format!("narration.{kind}.true_peak"): "-3.5",
            format!("narration.{kind}.image"): "https://example.invalid/dj.png",
            format!("narration.{kind}.artist"): "DJ Livi",
            format!("narration.{kind}.album_title"): "DJ",
            format!("narration.{kind}.title"): "Up next",
        })
    }

    fn merge(a: Value, b: Value) -> Value {
        let mut a = a.as_object().cloned().unwrap_or_default();
        a.extend(b.as_object().cloned().unwrap_or_default());
        Value::Object(a)
    }

    /// Bloque inventado con la forma del JSON de la sesión lexicon (sin datos de ninguna cuenta).
    fn sample_block() -> Value {
        json!({
            "metadata": {
                "dj.interactivity.localized_jump_button": "Elige tú, Livi",
                "end_of_context_action": "stop",
            },
            "pages": [{
                "tracks": [
                    {"uri": A, "metadata": merge(json!({"segment": "s1", "dj.jump_target": "j1"}), narration_meta("intro", "Hola, empezamos con"))},
                    {"uri": "spotify:episode:5BsahJTQs8whK3VmJjUrvU", "metadata": {}},
                    {"uri": B, "metadata": narration_meta("outro", "Eso fue")},
                    {"uri": C, "metadata": narration_meta("intro", "Y ahora")},
                ],
                "next_page_url": "hm://lexicon-session-provider/context-resolve/v2/session/abc?contextUri=spotify:playlist:x",
            }],
        })
    }

    #[test]
    fn lexicon_block_keeps_tracks_narrations_and_next_page() {
        let block = parse_block(sample_block().to_string().as_bytes()).unwrap();
        assert_eq!(block.uris(), [A, B, C]);
        assert_eq!(block.jump_label.as_deref(), Some("Elige tú, Livi"));
        assert!(
            block
                .next_page_url
                .as_deref()
                .unwrap()
                .starts_with("hm://lexicon-session-provider/")
        );
        let intro = block.tracks[0].intro.as_ref().unwrap();
        assert_eq!(
            (intro.voice, intro.provider, intro.sample_rate),
            (35, 6, 44_100)
        );
        assert_eq!(intro.loudness_db, Some(-16.0));
        assert_eq!(intro.true_peak_db, Some(-3.5));
        assert_eq!(
            (
                intro.title.as_str(),
                intro.artist.as_str(),
                intro.album.as_str()
            ),
            ("Up next", "DJ Livi", "DJ")
        );
        assert_eq!(
            intro.image.as_deref(),
            Some("https://example.invalid/dj.png")
        );
        assert!(block.tracks[0].outro.is_none());
        assert_eq!(block.tracks[0].jump_target.as_deref(), Some("j1"));
        assert!(block.tracks[1].outro.is_some() && block.tracks[1].intro.is_none());
        assert_eq!(
            plain_text(&intro.ssml),
            "Hola, empezamos con Banda Inventada, ¡dale!"
        );
    }

    #[test]
    fn invalid_lexicon_data_is_rejected_or_ignored() {
        assert_eq!(parse_block(b"no es json"), Err(DjError::Unavailable));
        assert_eq!(
            parse_block(br#"{"pages": [{"tracks": []}]}"#),
            Err(DjError::Empty)
        );
        let mut value = sample_block();
        value["pages"][0]["next_page_url"] = json!("https://evil.invalid/next");
        value["pages"][0]["tracks"][0]["metadata"]["narration.intro.voice"] = json!("VOICE99");
        value["pages"][0]["tracks"][0]["metadata"]["narration.intro.image"] =
            json!("http://plain.invalid/x.png");
        value["pages"][0]["tracks"][2]["metadata"]["narration.outro.tts_provider"] = json!("NUEVO");
        let block = parse_value(&value).unwrap();
        assert!(block.next_page_url.is_none());
        assert!(
            block.tracks[0].intro.is_none(),
            "voz desconocida: no se puede pedir"
        );
        assert!(block.tracks[1].outro.is_none());
        for url in ["hm://", "hm://a/../b", "hm://a b", "/relative"] {
            assert!(!valid_page_url(url), "{url}");
        }
    }

    #[test]
    fn tts_request_is_encoded_byte_by_byte() {
        let narration = Narration {
            ssml: "<speak>Hola</speak>".into(),
            voice: 35,
            provider: 6,
            sample_rate: 44_100,
            loudness_db: None,
            true_peak_db: None,
            image: None,
            title: "DJ".into(),
            artist: "DJ Livi".into(),
            album: "DJ".into(),
        };
        let mut expected = vec![0x12, 19];
        expected.extend_from_slice(b"<speak>Hola</speak>");
        // audio_format=MP3, voice=35, provider=SONANTIC_FAST, 44100 = 0xAC44 → C4 D8 02.
        expected.extend_from_slice(&[0x18, 5, 0x28, 35, 0x30, 6, 0x38, 0xC4, 0xD8, 0x02]);
        assert_eq!(tts_request(&narration), expected);
        let long = Narration {
            ssml: "x".repeat(300),
            ..narration
        };
        assert_eq!(&tts_request(&long)[..3], &[0x12, 0xAC, 0x02]);
    }

    #[test]
    fn tts_response_url_and_mp3_sniffing() {
        let mut body = vec![0x0A, 18];
        body.extend_from_slice(b"https://x.invalid/");
        assert_eq!(
            tts_response_url(&body).as_deref(),
            Some("https://x.invalid/")
        );
        assert_eq!(tts_response_url(&[0x10, 0x01]), None);
        assert!(looks_like_mp3(b"ID3\x04"));
        assert!(looks_like_mp3(&[0xFF, 0xFB, 0x90]));
        assert!(!looks_like_mp3(b"<html>"));
    }

    #[test]
    fn voice_gain_targets_song_loudness_without_clipping() {
        assert_eq!(voice_gain(None, None), 1.0);
        let louder = voice_gain(Some(-16.0), Some(-3.5));
        assert!((louder - 10f32.powf(2.0 / 20.0)).abs() < 1e-4);
        let capped = voice_gain(Some(-20.0), Some(-1.5));
        assert!(
            (capped - 10f32.powf(0.5 / 20.0)).abs() < 1e-4,
            "pico real a -1 dBTP"
        );
        assert!(voice_gain(Some(-10.0), None) < 1.0);
    }

    #[test]
    fn resampler_keeps_channels_and_length() {
        let mono: Vec<f32> = (0..22_050).map(|i| i as f32 / 22_050.0).collect();
        let out = resample(mono, 1, 22_050, 44_100);
        assert_eq!(out.len(), 44_100);
        assert!(out.windows(2).all(|w| w[1] >= w[0]));
        let stereo = vec![0.5, -0.5, 0.5, -0.5];
        assert_eq!(resample(stereo.clone(), 2, 44_100, 44_100), stereo);
    }

    /// MPEG-1 Layer III, 128 kb/s, 44,1 kHz, estéreo, sin padding: 417 bytes de silencio.
    fn silent_mp3(frames: usize) -> Vec<u8> {
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        frame.repeat(frames)
    }

    #[test]
    fn mp3_decodes_to_pcm_in_memory() {
        let clip = decode_voice(&silent_mp3(40), 1.0).unwrap();
        assert_eq!(clip.channels(), 2);
        assert!(clip.frames() >= 30 * 1152, "{} frames", clip.frames());
        assert!(clip.duration_ms() > 700);
        assert_eq!(decode_voice(b"esto no es audio", 1.0), Err(DjError::Decode));
    }

    #[tokio::test]
    async fn tts_follows_303_without_credentials_and_maps_failures() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_bytes, header, method, path},
        };
        let server = MockServer::start().await;
        let narration = parse_value(&sample_block()).unwrap().tracks[0]
            .intro
            .clone()
            .unwrap();
        let audio = silent_mp3(3);
        Mock::given(method("POST"))
            .and(path("/client-tts/v1/fulfill"))
            .and(header("authorization", "Bearer simulado"))
            .and(header("client-token", "ct-simulado"))
            .and(header("content-type", "application/x-protobuf"))
            .and(body_bytes(tts_request(&narration)))
            .respond_with(ResponseTemplate::new(303).insert_header(
                "location",
                format!("{}/v2/generate/voz.mp3?firma=caduca", server.uri()).as_str(),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/generate/voz.mp3"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(audio.clone()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/caido/client-tts/v1/fulfill"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let seen_auth = Arc::new(Mutex::new(Vec::new()));
        let send = |request: Request<Bytes>| {
            let client = client.clone();
            let seen = seen_auth.clone();
            async move {
                seen.lock()
                    .unwrap()
                    .push(request.headers().contains_key("authorization"));
                let request =
                    reqwest::Request::try_from(request).map_err(|_| DjError::Unavailable)?;
                let response = client
                    .execute(request)
                    .await
                    .map_err(|_| DjError::Unavailable)?;
                Ok(HttpReply {
                    status: response.status().as_u16(),
                    location: response
                        .headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned),
                    body: response.bytes().await.map_err(|_| DjError::Unavailable)?,
                })
            }
        };
        let mp3 = fulfill(
            &format!("{}/client-tts/v1/fulfill", server.uri()),
            "Bearer simulado",
            Some("ct-simulado"),
            tts_request(&narration),
            send,
        )
        .await
        .unwrap();
        assert_eq!(mp3.as_ref(), audio.as_slice());
        assert_eq!(
            *seen_auth.lock().unwrap(),
            [true, false],
            "el CDN no recibe el token"
        );
        assert_eq!(
            fulfill(
                &format!("{}/caido/client-tts/v1/fulfill", server.uri()),
                "Bearer simulado",
                None,
                tts_request(&narration),
                send,
            )
            .await,
            Err(DjError::Tts(503))
        );
        server.verify().await;
    }

    // ---------------------------------------------------------------- controlador simulado

    struct FakeHost {
        log: Mutex<Vec<String>>,
        state: watch::Sender<EngineState>,
        published: Mutex<Vec<DjState>>,
        timers: Mutex<HashMap<String, watch::Sender<TrackEndTimerState>>>,
        voices: Mutex<Vec<watch::Sender<VoiceProgress>>>,
        tts_down: bool,
        blocks: Mutex<Vec<DjBlock>>,
    }

    impl FakeHost {
        fn new(tts_down: bool) -> Arc<Self> {
            let state = EngineState {
                active: true,
                ..EngineState::default()
            };
            Arc::new(Self {
                log: Mutex::new(Vec::new()),
                state: watch::channel(state).0,
                published: Mutex::new(Vec::new()),
                timers: Mutex::new(HashMap::new()),
                voices: Mutex::new(Vec::new()),
                tts_down,
                blocks: Mutex::new(Vec::new()),
            })
        }
        fn log(&self, entry: impl Into<String>) {
            self.log.lock().unwrap().push(entry.into());
        }
        fn entries(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
        fn set_track(&self, uri: &str) {
            self.state.send_modify(|s| {
                s.track = Some(crate::EngineTrack {
                    uri: uri.into(),
                    title: uri.into(),
                    artist: String::new(),
                    album: String::new(),
                    duration_ms: 1000,
                    cover_url: None,
                })
            });
        }
        fn finish_track(&self, uri: &str, next: Option<&str>) {
            if let Some(next) = next {
                self.set_track(next);
            }
            let timer = self.timers.lock().unwrap().remove(uri);
            timer.unwrap().send_replace(TrackEndTimerState::Finished);
        }
        fn end_voice(&self) {
            let voice = self.voices.lock().unwrap().pop().expect("voz sonando");
            voice.send_replace(VoiceProgress {
                position_ms: 10,
                at: Instant::now(),
                playing: false,
                finished: true,
            });
        }
    }

    impl DjHost for FakeHost {
        fn fetch_block(
            &self,
            url: Option<String>,
        ) -> impl Future<Output = Result<DjBlock, DjError>> + Send {
            self.log(format!("bloque {}", url.is_some()));
            let block = self.blocks.lock().unwrap().pop();
            async move { block.ok_or(DjError::Unavailable) }
        }
        fn fetch_voice(
            &self,
            narration: Narration,
        ) -> impl Future<Output = Result<VoiceClip, DjError>> + Send {
            let down = self.tts_down;
            async move {
                if down {
                    return Err(DjError::Tts(503));
                }
                let marker = if narration.ssml.contains("Hola") {
                    0.1
                } else if narration.ssml.contains("Eso fue") {
                    0.2
                } else {
                    0.3
                };
                Ok(VoiceClip::new(vec![marker; 441], 1).unwrap())
            }
        }
        fn load(&self, uris: Vec<String>, play: bool) -> Result<(), DjError> {
            self.log(format!("cargar {} play={play}", uris.len()));
            self.set_track(&uris[0]);
            Ok(())
        }
        fn resume(&self) -> Result<(), DjError> {
            self.log("reanudar");
            Ok(())
        }
        fn pause(&self) {
            self.log("pausar");
        }
        fn skip(&self) -> Result<(), DjError> {
            self.log("saltar");
            Ok(())
        }
        fn arm(&self, uri: &str) -> Option<watch::Receiver<TrackEndTimerState>> {
            self.log(format!("armar {}", &uri[14..18]));
            let (tx, rx) = watch::channel(TrackEndTimerState::Armed);
            self.timers.lock().unwrap().insert(uri.into(), tx);
            Some(rx)
        }
        fn speak(&self, clip: Arc<VoiceClip>) -> Option<watch::Receiver<VoiceProgress>> {
            let which = match (clip.frame(0)[0] * 10.0).round() as i32 {
                1 => "intro A",
                2 => "outro B",
                _ => "intro C",
            };
            self.log(format!("voz {which}"));
            let (tx, rx) = watch::channel(VoiceProgress {
                position_ms: 0,
                at: Instant::now(),
                playing: true,
                finished: false,
            });
            self.voices.lock().unwrap().push(tx);
            Some(rx)
        }
        fn stop_voice(&self) {}
        fn release_output(&self) {
            self.log("soltar salida");
        }
        fn state(&self) -> watch::Receiver<EngineState> {
            self.state.subscribe()
        }
        fn publish(&self, dj: DjState) {
            self.published.lock().unwrap().push(dj);
        }
        fn error(&self, _error: DjError) {}
    }

    async fn settle() {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    async fn until(host: &FakeHost, entry: &str) {
        for _ in 0..200 {
            if host.entries().iter().any(|e| e == entry) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("no llegó «{entry}»: {:?}", host.entries());
    }

    #[tokio::test]
    async fn voice_then_song_then_outro_and_intro_at_the_boundary() {
        let host = FakeHost::new(false);
        let block = parse_value(&sample_block()).unwrap();
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "voz intro A").await;
        {
            let published = host.published.lock().unwrap();
            let speech = published.last().unwrap().speech.clone().unwrap();
            assert_eq!(speech.artist, "DJ Livi");
            assert_eq!(
                speech.image_url.as_deref(),
                Some("https://example.invalid/dj.png")
            );
            assert_eq!(speech.line, "Hola, empezamos con Banda Inventada, ¡dale!");
            assert_eq!(speech.duration_ms, 10);
        }
        host.end_voice();
        until(&host, "reanudar").await;
        // A no tiene cierre ni B presentación: sin pausa, el paso es gapless.
        host.set_track(B);
        until(&host, "armar 7ouM").await;
        // C ya aparece antes del final audible de B (precarga), luego llega el límite.
        host.finish_track(B, Some(C));
        until(&host, "voz outro B").await;
        host.end_voice();
        until(&host, "voz intro C").await;
        assert!(
            host.published
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .speech
                .is_some()
        );
        host.end_voice();
        until(&host, "armar 0VjI").await;
        until(&host, "bloque true").await;
        assert_eq!(
            host.entries(),
            [
                "cargar 3 play=false",
                "voz intro A",
                "reanudar",
                "armar 7ouM",
                "voz outro B",
                "voz intro C",
                "reanudar",
                "armar 0VjI",
                "bloque true",
            ]
        );
        assert!(
            host.published
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .speech
                .is_none()
        );
        drop(cmds);
        task.await.unwrap();
        assert_eq!(
            host.published.lock().unwrap().last(),
            Some(&DjState::default())
        );
        assert!(host.entries().ends_with(&["soltar salida".to_string()]));
    }

    #[tokio::test]
    async fn tts_down_plays_only_music_and_the_next_block_continues() {
        let host = FakeHost::new(true);
        let mut next = parse_value(&sample_block()).unwrap();
        next.tracks.truncate(1);
        next.next_page_url = None;
        host.blocks.lock().unwrap().push(next);
        let mut block = parse_value(&sample_block()).unwrap();
        block.tracks.truncate(2);
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "cargar 2 play=true").await;
        settle().await;
        host.set_track(B);
        until(&host, "armar 7ouM").await;
        // Última del bloque: el bloque siguiente ya se pidió y entra sin voz, sonando.
        host.finish_track(B, None);
        until(&host, "cargar 1 play=true").await;
        settle().await;
        assert_eq!(
            host.entries(),
            [
                "cargar 2 play=true",
                "armar 7ouM",
                "bloque true",
                "cargar 1 play=true",
            ]
        );
        assert!(
            host.published
                .lock()
                .unwrap()
                .iter()
                .all(|s| s.speech.is_none())
        );
        drop(cmds);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn jump_during_outro_skips_the_pending_intro_and_plays_the_new_block() {
        let host = FakeHost::new(false);
        let block = parse_value(&sample_block()).unwrap();
        let mut next = block.clone();
        next.tracks.truncate(1);
        next.next_page_url = None;
        host.blocks.lock().unwrap().push(next);
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "voz intro A").await;
        host.end_voice();
        until(&host, "reanudar").await;
        host.set_track(B);
        until(&host, "armar 7ouM").await;
        host.finish_track(B, Some(C));
        until(&host, "voz outro B").await;
        cmds.send(DjCommand::Jump).unwrap();
        settle().await;
        host.end_voice();
        until(&host, "cargar 1 play=false").await;
        assert!(!host.entries().iter().any(|entry| entry == "voz intro C"));
        drop(cmds);
        task.await.unwrap();
        assert_eq!(
            host.published.lock().unwrap().last(),
            Some(&DjState::default())
        );
    }

    #[tokio::test]
    async fn jump_without_a_next_block_restores_music_and_keeps_the_dj() {
        for failed_fetch in [false, true] {
            let host = FakeHost::new(true);
            let mut block = parse_value(&sample_block()).unwrap();
            block.tracks.truncate(1);
            if !failed_fetch {
                block.next_page_url = None;
            }
            let (cmds, rx) = mpsc::unbounded_channel();
            let mut task = tokio::spawn(run(host.clone(), block, rx));
            until(&host, "cargar 1 play=true").await;
            settle().await;
            cmds.send(DjCommand::Jump).unwrap();
            until(&host, "reanudar").await;
            settle().await;
            assert!(host.entries().iter().any(|entry| entry == "pausar"));
            // El botón nunca apaga a Livi: el controlador sigue vivo y activo.
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut task)
                    .await
                    .is_err()
            );
            assert!(host.published.lock().unwrap().last().unwrap().active);
            drop(cmds);
            task.await.unwrap();
            assert_eq!(
                host.published.lock().unwrap().last(),
                Some(&DjState::default())
            );
        }
    }

    fn block_starting_with(uri: &str, line: &str) -> DjBlock {
        let mut block = parse_value(&sample_block()).unwrap();
        block.tracks.truncate(1);
        block.tracks[0].uri = uri.into();
        block.tracks[0].intro = narration(&narration_meta("intro", line), "intro");
        block
    }

    #[tokio::test]
    async fn consecutive_starts_ask_for_a_new_session_and_do_not_repeat_the_set() {
        use std::sync::Mutex as StdMutex;
        let old = block_starting_with(A, "Empecemos con");
        let new = block_starting_with(B, "Otra onda con");
        // Spotify que ignora la marca: siempre el mismo set; se reintenta y se acaba.
        let seen = StdMutex::new(Vec::new());
        let stuck = fresh_session(
            |nonce| {
                seen.lock().unwrap().push(nonce);
                let block = old.clone();
                async move { Ok(block) }
            },
            old.fingerprint().as_deref(),
        )
        .await
        .unwrap();
        assert_eq!(stuck.fingerprint(), old.fingerprint());
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(seen.iter().all(Option::is_some));
        assert_ne!(seen[0], seen[1], "cada petición lleva una marca distinta");
        // Spotify que cachea por marca: el segundo intento ya trae otro set.
        let calls = StdMutex::new(0);
        let fixed = fresh_session(
            |_| {
                let mut n = calls.lock().unwrap();
                *n += 1;
                let block = if *n == 1 { old.clone() } else { new.clone() };
                async move { Ok(block) }
            },
            old.fingerprint().as_deref(),
        )
        .await
        .unwrap();
        assert_eq!(fixed.fingerprint(), new.fingerprint());
        // Primer inicio de todos: una sola petición, con marca.
        let first = StdMutex::new(Vec::new());
        fresh_session(
            |nonce| {
                first.lock().unwrap().push(nonce);
                let block = old.clone();
                async move { Ok(block) }
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(first.into_inner().unwrap().len(), 1);
        // Si Spotify rechaza la marca, cae a la petición simple.
        let urls = StdMutex::new(Vec::new());
        let plain = fresh_session(
            |nonce| {
                urls.lock().unwrap().push(nonce.clone());
                let block = old.clone();
                async move { nonce.map_or(Ok(block), |_| Err(DjError::Unavailable)) }
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(plain.fingerprint(), old.fingerprint());
        assert_eq!(urls.into_inner().unwrap().last(), Some(&None));
        assert!(with_nonce("hm://x/s?contextUri=a", "n1").ends_with("a&nonce=n1"));
        assert!(with_nonce("hm://x/s", "n1").ends_with("s?nonce=n1"));
    }

    #[tokio::test]
    async fn jump_advances_the_section_and_never_replays_the_same_block() {
        let host = FakeHost::new(true);
        let block = parse_value(&sample_block()).unwrap();
        // La «página siguiente» devuelve el mismo set; la sesión nueva trae otro.
        let mut fresh = block_starting_with(C, "Nuevo");
        fresh.next_page_url = None;
        host.blocks.lock().unwrap().push(fresh);
        host.blocks.lock().unwrap().push(block.clone());
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "cargar 3 play=true").await;
        settle().await;
        assert_eq!(host.published.lock().unwrap().last().unwrap().section, 1);
        cmds.send(DjCommand::Jump).unwrap();
        until(&host, "cargar 1 play=true").await;
        settle().await;
        let entries = host.entries();
        assert!(entries.iter().any(|e| e == "bloque true"));
        assert!(entries.iter().any(|e| e == "bloque false"));
        let last = host.published.lock().unwrap().last().cloned().unwrap();
        assert!(last.active);
        assert_eq!(last.section, 2);
        drop(cmds);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn jump_at_the_end_of_pages_asks_for_a_fresh_session() {
        let host = FakeHost::new(true);
        let mut fresh = parse_value(&sample_block()).unwrap();
        fresh.tracks.truncate(2);
        host.blocks.lock().unwrap().push(fresh);
        let mut block = parse_value(&sample_block()).unwrap();
        block.tracks.truncate(1);
        block.next_page_url = None;
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "cargar 1 play=true").await;
        settle().await;
        cmds.send(DjCommand::Jump).unwrap();
        until(&host, "cargar 2 play=true").await;
        assert!(host.entries().iter().any(|e| e == "bloque false"));
        drop(cmds);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn taking_playback_away_ends_even_a_paused_narration_without_a_timer() {
        let host = FakeHost::new(false);
        let block = parse_value(&sample_block()).unwrap();
        let (_cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "voz intro A").await;
        host.voices
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .send_modify(|voice| voice.playing = false);
        settle().await;
        host.state.send_modify(|state| state.active = false);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            host.published.lock().unwrap().last(),
            Some(&DjState::default())
        );
        assert!(host.entries().ends_with(&["soltar salida".into()]));
    }

    #[tokio::test]
    async fn next_never_cuts_the_voice_and_jump_loads_the_next_block() {
        let host = FakeHost::new(false);
        let mut next = parse_value(&sample_block()).unwrap();
        next.tracks.remove(0);
        host.blocks.lock().unwrap().push(next);
        let block = parse_value(&sample_block()).unwrap();
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "voz intro A").await;
        cmds.send(DjCommand::Next).unwrap();
        settle().await;
        // La voz sigue sonando: «siguiente» no la descartó.
        assert!(!host.voices.lock().unwrap().is_empty());
        assert!(!host.entries().iter().any(|e| e == "reanudar"));
        host.end_voice();
        until(&host, "reanudar").await;
        cmds.send(DjCommand::Jump).unwrap();
        until(&host, "armar 7ouM").await;
        assert_eq!(
            host.entries(),
            [
                "cargar 3 play=false",
                "voz intro A",
                "reanudar",
                "pausar",
                "bloque true",
                "cargar 2 play=true",
                "armar 7ouM",
            ]
        );
        // Algo ajeno a la sesión empieza a sonar: el DJ termina solo.
        host.set_track("spotify:track:1111111111111111111111");
        task.await.unwrap();
        assert_eq!(
            host.published.lock().unwrap().last(),
            Some(&DjState::default())
        );
    }

    #[tokio::test]
    async fn next_with_a_pending_dialogue_still_plays_it_before_the_next_song() {
        let host = FakeHost::new(false);
        let block = parse_value(&sample_block()).unwrap();
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "voz intro A").await;
        host.end_voice();
        until(&host, "reanudar").await;
        host.set_track(B);
        until(&host, "armar 7ouM").await;
        // Saltar B con diálogo pendiente (cierre de B + presentación de C).
        cmds.send(DjCommand::Next).unwrap();
        until(&host, "saltar").await;
        host.set_track(C);
        until(&host, "voz outro B").await;
        // Más «siguiente» durante la voz no la cortan.
        cmds.send(DjCommand::Next).unwrap();
        cmds.send(DjCommand::Next).unwrap();
        settle().await;
        assert_eq!(host.voices.lock().unwrap().len(), 1);
        host.end_voice();
        until(&host, "voz intro C").await;
        host.end_voice();
        until(&host, "armar 0VjI").await;
        let entries = host.entries();
        let at = |name: &str| entries.iter().position(|e| e == name).unwrap();
        assert!(at("pausar") < at("saltar") && at("saltar") < at("voz outro B"));
        assert!(at("voz intro C") < entries.len() - 1);
        assert_eq!(entries.iter().filter(|e| *e == "saltar").count(), 1);
        drop(cmds);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn next_without_dialogue_just_skips() {
        let host = FakeHost::new(false);
        let block = parse_value(&sample_block()).unwrap();
        let (cmds, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(host.clone(), block, rx));
        until(&host, "voz intro A").await;
        host.end_voice();
        until(&host, "reanudar").await;
        // A no tiene cierre y el episodio siguiente no tiene presentación: salto normal.
        cmds.send(DjCommand::Next).unwrap();
        until(&host, "saltar").await;
        assert!(!host.entries().iter().any(|e| e == "pausar"));
        drop(cmds);
        task.await.unwrap();
    }
}
