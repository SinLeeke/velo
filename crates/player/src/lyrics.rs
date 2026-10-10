//! Letras de Spotify por la sesión de audio; metadatos solamente, sin reproducir.
//! Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
//! Fuentes: src/player.rs:503 y src/lyrics.rs:73 (commit 12deee4).

use std::{future::Future, time::Duration};

use bytes::Bytes;
use librespot_core::{SpotifyId, SpotifyUri, error::ErrorKind};
use serde_json::Value;
use velo_domain::{LyricLine, Lyrics};

use crate::Engine;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BYTES: usize = 512 * 1024;
const MAX_LINES: usize = 4096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LyricsError {
    #[error("URI de canción inválida")]
    InvalidUri,
    /// Lleva solo el tipo de error de librespot (p. ej. `PermissionDenied`), nunca el cuerpo.
    #[error("Spotify no entregó las letras ({0})")]
    Unavailable(String),
    #[error("La consulta de letras tardó demasiado")]
    Timeout,
    #[error("Formato de letras no compatible")]
    Format,
}

/// Un 404 es ausencia; errores y formatos desconocidos permiten al caller recurrir a LRCLIB.
/// El timeout incluye los reintentos internos de spclient. No imprime cuerpos ni credenciales.
pub async fn lyrics(engine: &Engine, uri: &str) -> Result<Option<Lyrics>, LyricsError> {
    request(uri, |id| async move {
        match engine.session().spclient().get_lyrics(&id).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(LyricsError::Unavailable(format!("{:?}", error.kind))),
        }
    })
    .await
}

async fn request<F, Fut>(uri: &str, fetch: F) -> Result<Option<Lyrics>, LyricsError>
where
    F: FnOnce(SpotifyId) -> Fut,
    Fut: Future<Output = Result<Option<Bytes>, LyricsError>>,
{
    let SpotifyUri::Track { id } =
        crate::parse_track_uri(uri).map_err(|_| LyricsError::InvalidUri)?
    else {
        return Err(LyricsError::InvalidUri);
    };
    let bytes = tokio::time::timeout(REQUEST_TIMEOUT, fetch(id))
        .await
        .map_err(|_| LyricsError::Timeout)??;
    bytes.as_deref().map(parse).transpose().map(Option::flatten)
}

/// Parseo acotado de color-lyrics JSON. Una línea sin tiempo válido degrada a texto completo,
/// en vez de publicar una sincronización parcial. Se ordena estable para Lyrics::line_at.
pub fn parse(bytes: &[u8]) -> Result<Option<Lyrics>, LyricsError> {
    if bytes.len() > MAX_BYTES {
        return Err(LyricsError::Format);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| LyricsError::Format)?;
    let lyrics = value.get("lyrics").ok_or(LyricsError::Format)?;
    let wants_synced = lyrics.get("syncType").and_then(Value::as_str) == Some("LINE_SYNCED");
    let lines = lyrics
        .get("lines")
        .and_then(Value::as_array)
        .filter(|lines| lines.len() <= MAX_LINES)
        .ok_or(LyricsError::Format)?;
    let mut text = Vec::new();
    let mut synced = Vec::new();
    for line in lines {
        let words = line
            .get("words")
            .and_then(Value::as_str)
            .ok_or(LyricsError::Format)?
            .trim();
        if words.is_empty() || words == "♪" {
            continue;
        }
        text.push(words.to_string());
        let time_ms = line.get("startTimeMs").and_then(|value| {
            value
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| value.as_u64())
                // El rol QML timeMs es int; rechazar tiempos que no puede representar.
                .filter(|time| *time <= i32::MAX as u64)
        });
        if let Some(time_ms) = time_ms.filter(|_| wants_synced) {
            synced.push(LyricLine {
                time_ms,
                text: words.to_string(),
            });
        }
    }
    if text.is_empty() {
        // ♪ no demuestra que sea instrumental: LRCLIB puede tener otra transcripción.
        return Ok(None);
    }
    if wants_synced && synced.len() == text.len() {
        synced.sort_by_key(|line| line.time_ms);
        Ok(Some(Lyrics {
            synced,
            ..Lyrics::default()
        }))
    } else {
        Ok(Some(Lyrics {
            plain: Some(text.join("\n")),
            ..Lyrics::default()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fixture_sorts_stably_and_keeps_duplicate_times() {
        let lyrics = parse(include_bytes!("../tests/fixtures/color_lyrics.json"))
            .unwrap()
            .unwrap();
        assert_eq!(lyrics.synced.len(), 3);
        assert_eq!(lyrics.synced[0].text, "Primera línea");
        assert_eq!(lyrics.synced[0].time_ms, 1000);
        assert_eq!(lyrics.line_at(0), None);
        assert_eq!(lyrics.line_at(3000), Some(2));
        assert_eq!(lyrics.synced[2].text, "Otra voz");
        assert!(lyrics.plain.is_none());
    }

    #[test]
    fn missing_invalid_or_negative_time_keeps_all_words_as_plain() {
        for time in [
            Value::Null,
            json!("bad"),
            json!(-1),
            json!("18446744073709551616"),
            json!(u64::MAX),
        ] {
            let bytes =
                serde_json::to_vec(&json!({"lyrics": {"syncType": "LINE_SYNCED", "lines": [
                    {"words": "Una", "startTimeMs": "1000"},
                    {"words": "Dos", "startTimeMs": time}
                ]}}))
                .unwrap();
            let lyrics = parse(&bytes).unwrap().unwrap();
            assert!(lyrics.synced.is_empty());
            assert_eq!(lyrics.plain.as_deref(), Some("Una\nDos"));
        }
    }

    #[test]
    fn unsynced_and_unknown_sync_modes_never_fabricate_timing() {
        for mode in ["UNSYNCED", "SYLLABLE_SYNCED", "future"] {
            let bytes = serde_json::to_vec(&json!({"lyrics": {"syncType": mode, "lines": [
                {"words": " Texto ", "startTimeMs": "0"}
            ]}}))
            .unwrap();
            let lyrics = parse(&bytes).unwrap().unwrap();
            assert_eq!(lyrics.plain.as_deref(), Some("Texto"));
            assert!(lyrics.synced.is_empty());
        }
    }

    #[test]
    fn malformed_oversized_and_empty_responses_allow_fallback() {
        for bytes in [b"bad".as_slice(), br#"{}"#, br#"{"lyrics":{"lines":[{}]}}"#] {
            assert_eq!(parse(bytes), Err(LyricsError::Format));
        }
        assert_eq!(parse(&vec![b' '; MAX_BYTES + 1]), Err(LyricsError::Format));
        let too_many = json!({"lyrics": {"lines": vec![json!({"words":"a"}); MAX_LINES + 1]}});
        assert_eq!(
            parse(&serde_json::to_vec(&too_many).unwrap()),
            Err(LyricsError::Format)
        );
        for lines in [json!([]), json!([{"words":"♪"}, {"words":" "}])] {
            let bytes = serde_json::to_vec(&json!({"lyrics": {"lines": lines}})).unwrap();
            assert_eq!(parse(&bytes), Ok(None));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn request_is_bounded_and_does_not_call_transport_for_invalid_uris() {
        let valid = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
        for uri in [
            "spotify:album:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:track:short",
            "spotify:track:ZZZZZZZZZZZZZZZZZZZZZZ",
        ] {
            assert_eq!(
                request(uri, |_| async { panic!("no debe consultar") }).await,
                Err(LyricsError::InvalidUri)
            );
        }
        assert_eq!(
            request(valid, |_| std::future::pending()).await,
            Err(LyricsError::Timeout)
        );
        assert_eq!(request(valid, |_| async { Ok(None) }).await, Ok(None));
        assert_eq!(
            request(valid, |_| async {
                Err(LyricsError::Unavailable("PermissionDenied".into()))
            })
            .await,
            Err(LyricsError::Unavailable("PermissionDenied".into()))
        );
        assert!(
            request(valid, |_| async {
                Ok(Some(Bytes::from_static(include_bytes!(
                    "../tests/fixtures/color_lyrics.json"
                ))))
            })
            .await
            .unwrap()
            .is_some()
        );
    }
}
