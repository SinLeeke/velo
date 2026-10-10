//! Queue editing and retries for Velo's own engine (task E2b+Q1).
//!
//! librespot 0.8's public Spirc has no "set queue" command (only Spotify's own clients send
//! `set_queue` through the Connect cloud) and the Web API cannot reorder or remove either. So
//! an edit reloads the playback as an explicit track list: the current song first (same track
//! means librespot only seeks, no new download), at the measured position, then the edited
//! upcoming rows, then the rest of the context that the Web API queue (≈20 rows) did not show.
//!
//! The same reload retries a song that Spotify failed to deliver (audio-key timeout): Spirc
//! has already skipped it, so it goes back to the front of an explicit list.
//!
//! Edits are positional: duplicate URIs remain distinct occurrences. Pure functions here are
//! unit-tested; the only I/O is [`context_tracks`] (the session's spclient, on a Tokio worker).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use librespot_connect::{
    LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack,
};
use librespot_core::SpotifyId;
use serde_json::Value;
use velo_domain::{MusicProvider, Playlist, RepeatMode, Track, VeloError};

use crate::{Engine, EngineState, PlayerError, TrackFailure, parse_track_uri};

/// Experimental session mutation path. Disabled until the leader validates the live probe.
pub fn session_writes_enabled() -> bool {
    std::env::var("VELO_SESSION_PLAYLIST_WRITES").is_ok_and(|v| v == "1")
}

async fn session_post_once(
    session: &librespot_core::Session,
    endpoint: &str,
    body: Vec<u8>,
    content_type: &str,
) -> velo_domain::Result<bytes::Bytes> {
    let work = async {
        let base = session
            .spclient()
            .base_url()
            .await
            .map_err(|_| VeloError::NotFound)?;
        let token = session
            .login5()
            .auth_token()
            .await
            .map_err(|_| VeloError::NotAuthenticated)?;
        let mut request = http::Request::builder()
            .method(http::Method::POST)
            .uri(format!("{base}{endpoint}"))
            .header(
                "authorization",
                format!("{} {}", token.token_type, token.access_token),
            )
            .header("content-type", content_type)
            .header("content-length", body.len());
        if let Ok(token) = session.spclient().client_token().await {
            request = request.header("client-token", token);
        }
        let request = request
            .body(bytes::Bytes::from(body))
            .map_err(|_| VeloError::NotFound)?;
        use http_body_util::BodyExt;
        let response = session
            .http_client()
            .request_fut(request)
            .map_err(|_| VeloError::Network("playlist write unavailable".into()))?
            .await
            .map_err(|_| {
                VeloError::Network("playlist write unconfirmed; verify before retry".into())
            })?;
        if !response.status().is_success() {
            return Err(VeloError::Other(format!(
                "Session playlist HTTP {}; verify before retry",
                response.status().as_u16()
            )));
        }
        response
            .into_body()
            .collect()
            .await
            .map(|b| b.to_bytes())
            .map_err(|_| {
                VeloError::Network("playlist response incomplete; verify before retry".into())
            })
    };
    tokio::time::timeout(Duration::from_secs(10), work)
        .await
        .map_err(|_| VeloError::Network("playlist write timeout; verify before retry".into()))?
}

fn playlist_changes(
    uris: &[String],
    revision: Vec<u8>,
) -> velo_domain::Result<librespot_protocol::playlist4_external::ListChanges> {
    use librespot_protocol::playlist4_external::{Add, Delta, Item, ListChanges, Op, op::Kind};
    if uris.is_empty() || uris.len() > 100 || uris.iter().any(|u| parse_track_uri(u).is_err()) {
        return Err(VeloError::Other("Invalid playlist batch".into()));
    }
    let mut add = Add::new();
    add.add_last = Some(true);
    add.items = uris
        .iter()
        .map(|uri| Item {
            uri: Some(uri.clone()),
            ..Default::default()
        })
        .collect();
    let mut op = Op::new();
    op.kind = Some(Kind::ADD.into());
    op.add = Some(add).into();
    let mut delta = Delta::new();
    delta.ops.push(op);
    let mut changes = ListChanges::new();
    changes.base_revision = Some(revision);
    changes.deltas.push(delta);
    changes.want_resulting_revisions = Some(true);
    Ok(changes)
}

/// No retry of writes: a lost response must not duplicate occurrences. Probe on a disposable playlist.
pub async fn session_add_to_playlist(
    session: &librespot_core::Session,
    id: &str,
    uris: &[String],
) -> velo_domain::Result<()> {
    use librespot_protocol::playlist4_external::SelectedListContent;
    use protobuf::Message;
    if !session_writes_enabled() {
        return Err(VeloError::Other(
            "Session playlist write probe disabled".into(),
        ));
    }
    if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(VeloError::NotFound);
    }
    // Validate the batch before any request.
    playlist_changes(uris, Vec::new())?;
    let sp = session.spclient();
    let endpoint = format!("/playlist/v2/playlist/{id}");
    let raw = tokio::time::timeout(
        Duration::from_secs(10),
        sp.request(&http::Method::GET, &endpoint, None, None),
    )
    .await
    .map_err(|_| VeloError::Network("playlist read timeout".into()))?
    .map_err(|_| VeloError::Network("playlist read failed".into()))?;
    let list = SelectedListContent::parse_from_bytes(&raw).map_err(|_| VeloError::NotFound)?;
    let revision = list.revision.ok_or(VeloError::NotFound)?;
    let changes = playlist_changes(uris, revision)?;
    session_post_once(
        session,
        &format!("{endpoint}/changes"),
        changes.write_to_bytes().map_err(|_| VeloError::NotFound)?,
        "application/x-protobuf",
    )
    .await?;
    Ok(())
}

/// Candidate create + rootlist route, intentionally probe-only until Spotify confirms its format.
/// The response URI is retained; adding songs never resolves the radio selection again.
pub async fn session_create_playlist(
    session: &librespot_core::Session,
    name: &str,
    description: &str,
) -> velo_domain::Result<Playlist> {
    if !session_writes_enabled() {
        return Err(VeloError::Other(
            "Session playlist write probe disabled".into(),
        ));
    }
    if name.trim().is_empty() {
        return Err(VeloError::NotFound);
    }
    let body = serde_json::json!({"attributes":{"name":name.trim(),"description":description},"public":false}).to_string();
    let raw = session_post_once(
        session,
        "/playlist/v2/create",
        body.into_bytes(),
        "application/json",
    )
    .await?;
    let response: Value = serde_json::from_slice(&raw).map_err(|_| VeloError::NotFound)?;
    let uri = response
        .get("uri")
        .and_then(Value::as_str)
        .ok_or(VeloError::NotFound)?;
    let id = uri
        .strip_prefix("spotify:playlist:")
        .ok_or(VeloError::NotFound)?;
    if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(VeloError::NotFound);
    }
    // Confirm rootlist visibility. The candidate endpoint may already insert it there.
    use librespot_protocol::playlist4_external::SelectedListContent;
    use protobuf::Message;
    let root = tokio::time::timeout(
        Duration::from_secs(10),
        session.spclient().get_rootlist(0, Some(10_000)),
    )
    .await
    .map_err(|_| VeloError::Other(format!("Created {uri}; rootlist confirmation timed out")))?
    .map_err(|_| VeloError::Other(format!("Created {uri}; rootlist unconfirmed")))?;
    let root = SelectedListContent::parse_from_bytes(&root).map_err(|_| VeloError::NotFound)?;
    if !root
        .contents
        .as_ref()
        .is_some_and(|c| c.items.iter().any(|i| i.uri() == uri))
    {
        return Err(VeloError::Other(format!(
            "Created {uri}; verify rootlist before retry"
        )));
    }
    Ok(Playlist {
        id: id.into(),
        uri: uri.into(),
        name: name.trim().into(),
        description: description.into(),
        owner: session.username(),
        owner_id: session.username(),
        collaborative: false,
        images: Vec::new(),
        track_count: 0,
    })
}

pub async fn session_create_playlist_probe(
    session: &librespot_core::Session,
    name: &str,
    uris: &[String],
) -> velo_domain::Result<String> {
    playlist_changes(uris, Vec::new())?;
    let playlist = session_create_playlist(session, name, "Guardada desde Velo").await?;
    if let Err(e) = session_add_to_playlist(session, &playlist.id, uris).await {
        return Err(VeloError::Other(format!(
            "Created {}; items unconfirmed: {e}",
            playlist.uri
        )));
    }
    Ok(playlist.uri)
}

/// Upper bounds for resolving the rest of a context (a 10 000-song playlist fits).
const MAX_CONTEXT_TRACKS: usize = 10_000;
const MAX_CONTEXT_PAGES: usize = 40;
const CONTEXT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueEdit {
    Remove(usize),
    /// `to` is the final row index, after the move (not an insertion boundary).
    Move {
        from: usize,
        to: usize,
    },
    /// Skips to row `.0`: it and the rows before it leave "A continuación" (the target becomes
    /// the current song; the caller plays it).
    SkipTo(usize),
}

/// Invalid/stale indices leave the list untouched. Returns whether the order changed.
pub fn edit_queue<T>(tracks: &mut Vec<T>, edit: QueueEdit) -> bool {
    match edit {
        QueueEdit::Remove(index) if index < tracks.len() => {
            tracks.remove(index);
            true
        }
        QueueEdit::Move { from, to } if from < tracks.len() && to < tracks.len() && from != to => {
            let track = tracks.remove(from);
            tracks.insert(to, track);
            true
        }
        QueueEdit::SkipTo(index) if index < tracks.len() => {
            tracks.drain(..=index);
            true
        }
        _ => false,
    }
}

/// Only songs and episodes can go into an explicit Spirc track list (no local files).
pub fn playable(uri: &str) -> bool {
    ["spotify:track:", "spotify:episode:"]
        .iter()
        .any(|p| uri.strip_prefix(p).is_some_and(|id| !id.is_empty()))
}

/// Alinea una respuesta atrasada con la pista autoritativa de Velo/Connect. Solo consume
/// el prefijo hasta la primera ocurrencia actual si el servidor aún reproduce otra pista.
/// Si ya coincide, una URI igual en «A continuación» es un duplicado legítimo.
/// None: la respuesta no permite reconstruir la cola actual; nunca publicar la cola vieja.
pub fn align_upcoming(
    current: &str,
    server_current: Option<&str>,
    mut tracks: Vec<Track>,
) -> Option<Vec<Track>> {
    if server_current == Some(current) {
        return Some(tracks);
    }
    let consumed = tracks.iter().position(|t| t.uri == current)? + 1;
    tracks.drain(..consumed);
    Some(tracks)
}

/// The part of the context (in context order) that comes after what the queue view showed.
///
/// `context` is the whole context, `current` the song playing, `visible` the upcoming rows the
/// Web API returned (user-queued songs first, then context songs). Without shuffle the visible
/// context songs are a run right after `current`: they are matched greedily, so a queued song
/// that equals the next context song does not confuse it. With shuffle the order is unknown:
/// Se resta una ocurrencia por fila; sin aleatorio se usa el índice conocido. Un current desconocido o ambiguo se rechaza.
pub fn tail_after(
    context: &[String],
    current: &str,
    visible: &[String],
    shuffle: bool,
    occurrence: Option<usize>,
) -> Result<Vec<String>, PlayerError> {
    if context.is_empty() {
        return Ok(Vec::new());
    }
    if !context.iter().any(|u| u == current) {
        return Err(PlayerError::QueueContext);
    }
    if shuffle {
        // Restar una ocurrencia por fila, nunca todas las URIs iguales.
        let mut shown: HashMap<&str, usize> = HashMap::new();
        for uri in std::iter::once(current).chain(visible.iter().map(String::as_str)) {
            *shown.entry(uri).or_default() += 1;
        }
        return Ok(context
            .iter()
            .filter(|uri| {
                if let Some(count) = shown.get_mut(uri.as_str()) {
                    if *count > 0 {
                        *count -= 1;
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect());
    }
    let at = match occurrence.filter(|at| context.get(*at).is_some_and(|u| u == current)) {
        Some(at) => at,
        None => {
            let mut matches = context
                .iter()
                .enumerate()
                .filter(|(_, uri)| uri.as_str() == current);
            let at = matches.next().ok_or(PlayerError::QueueContext)?.0;
            // Spirc 0.8 no expone el UID/índice de contextos externos: rechazar la ambigüedad.
            if matches.next().is_some() {
                return Err(PlayerError::QueueContext);
            }
            at
        }
    };
    let mut next = at + 1;
    for uri in visible {
        if context.get(next) == Some(uri) {
            next += 1;
        }
    }
    Ok(context
        .get(next..)
        .map(<[String]>::to_vec)
        .unwrap_or_default())
}
/// Fisher–Yates with a tiny xorshift: the rest of a shuffled context stays shuffled after an
/// edit (Spirc must load it unshuffled, or it would shuffle the edited rows again).
pub fn shuffle_in_place<T>(items: &mut [T], mut seed: u64) {
    seed |= 1;
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let j = (seed % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
}

/// The full list to load: `current`, then `upcoming` and `tail`, unplayable URIs left out.
pub fn rebuilt_list(current: &str, upcoming: &[String], tail: &[String]) -> Vec<String> {
    std::iter::once(current)
        .chain(upcoming.iter().map(String::as_str))
        .chain(tail.iter().map(String::as_str))
        .filter(|u| playable(u))
        .map(str::to_string)
        .collect()
}

/// Resultado de crear la playlist: si falla un lote, la playlist ya existe y debe mostrarse.
#[derive(Debug)]
pub struct SavedQueue {
    pub playlist: Playlist,
    pub items_result: velo_domain::Result<()>,
}

/// Guarda una instantánea de la canción actual y la cola visible, conservando ocurrencias.
/// No consulta el contexto ni modifica la reproducción. La playlist es privada.
/// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
/// (`App::queue_playlist_uris`/`save_queue_as_playlist`); Velo conserva los duplicados (R3b).
pub async fn save_as_playlist(
    api: &dyn MusicProvider,
    name: &str,
    current: &str,
    visible: &[String],
) -> velo_domain::Result<SavedQueue> {
    let name = name.trim();
    if name.is_empty() {
        return Err(VeloError::Other(
            "Escribe un nombre para la playlist.".into(),
        ));
    }
    let uris: Vec<String> = std::iter::once(current)
        .chain(visible.iter().map(String::as_str))
        .map(str::to_string)
        .collect();
    // Rechazar antes de crear: no omitir archivos locales ni filas desconocidas silenciosamente.
    if !uris.iter().all(|uri| {
        ["spotify:track:", "spotify:episode:"].iter().any(|prefix| {
            uri.strip_prefix(prefix)
                .is_some_and(|id| id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        })
    }) {
        return Err(VeloError::Other(
            "La cola contiene elementos que no se pueden guardar en una playlist.".into(),
        ));
    }
    let playlist = api
        .create_playlist(name, "Cola guardada desde Velo", false)
        .await?;
    let mut items_result = Ok(());
    for batch in uris.chunks(100) {
        if let Err(error) = api.add_to_playlist(&playlist.id, batch).await {
            items_result = Err(error);
            break;
        }
    }
    Ok(SavedQueue {
        playlist,
        items_result,
    })
}

/// Where the engine is in the current song now (`position_ms` was measured at `position_at`).
pub fn position_now(state: &EngineState, now: Instant) -> u64 {
    let elapsed = if state.playing {
        now.saturating_duration_since(state.position_at).as_millis() as u64
    } else {
        0
    };
    let position = state.position_ms.saturating_add(elapsed);
    match &state.track {
        Some(t) if t.duration_ms > 0 => position.min(t.duration_ms),
        _ => position,
    }
}

/// An explicit-track reload: `tracks[0]` at `position_ms`. Shuffle off on purpose (Spirc would
/// shuffle the edited order); repeat kept.
pub(crate) fn rebuild_request(
    tracks: Vec<String>,
    position_ms: u64,
    playing: bool,
    repeat: RepeatMode,
) -> Option<LoadRequest> {
    if tracks.first().is_none_or(|u| !playable(u)) {
        return None;
    }
    Some(LoadRequest::from_tracks(
        tracks,
        LoadRequestOptions {
            start_playing: playing,
            seek_to: u32::try_from(position_ms).unwrap_or(u32::MAX),
            playing_track: Some(PlayingTrack::Index(0)),
            context_options: Some(LoadContextOptions::Options(Options {
                shuffle: false,
                repeat: repeat == RepeatMode::Context,
                repeat_track: repeat == RepeatMode::Track,
            })),
        },
    ))
}

// ---------------------------------------------------------------------------- retries

/// Waits before reloading a song Spotify failed to deliver: 1 s, then 3 s, then give up.
pub const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// Failures of the same song further apart than this start counting again.
const RETRY_WINDOW: Duration = Duration::from_secs(60);

/// Retry bookkeeping for the song that failed last (pure; the app owns one).
#[derive(Debug, Default)]
pub struct RetryBook {
    uri: Option<String>,
    attempts: usize,
    since: Option<Instant>,
}

impl RetryBook {
    /// The wait before the next attempt, or `None`: not transient (the song really is not
    /// available) or out of attempts. Each `Some` counts as one attempt.
    pub fn next(&mut self, failure: &TrackFailure, now: Instant) -> Option<Duration> {
        if !failure.transient {
            return None;
        }
        let fresh = self.uri.as_deref() != Some(failure.uri.as_str())
            || self
                .since
                .is_none_or(|t| now.saturating_duration_since(t) > RETRY_WINDOW);
        if fresh {
            self.uri = Some(failure.uri.clone());
            self.attempts = 0;
            self.since = Some(now);
        }
        let delay = RETRY_DELAYS.get(self.attempts).copied();
        if delay.is_some() {
            self.attempts += 1;
        }
        delay
    }
}

// ---------------------------------------------------------------------------- context

/// The tracks of the context being played, to keep what the Web API queue does not show.
///
/// The list Velo itself loaded last wins when it contains `current` (liked songs, search,
/// earlier edits: Spotify reports no context for them). Otherwise `context_uri` (from the Web
/// API) is resolved through the session's spclient (`context-resolve/v1`, internal and
/// undocumented, as in E4), bounded in pages, tracks and time. Lo desconocido/fallido retorna error; nunca una lista parcial.
pub async fn context_tracks(
    engine: &Engine,
    context_uri: Option<&str>,
    current: &str,
) -> Result<Vec<String>, PlayerError> {
    if let Some(known) = engine
        .known_tracks()
        .filter(|k| k.iter().any(|u| u == current))
    {
        return Ok(known);
    }
    let uri = context_uri
        .filter(|u| u.starts_with("spotify:"))
        .ok_or(PlayerError::QueueContext)?;
    let tracks = complete_with_timeout(resolve(engine, uri)).await?;
    if tracks.iter().any(|u| u == current) {
        Ok(tracks)
    } else {
        Err(PlayerError::QueueContext)
    }
}
async fn complete_with_timeout<
    F: std::future::Future<Output = Result<Vec<String>, PlayerError>>,
>(
    work: F,
) -> Result<Vec<String>, PlayerError> {
    tokio::time::timeout(CONTEXT_TIMEOUT, work)
        .await
        .map_err(|_| PlayerError::QueueContext)?
}
async fn resolve(engine: &Engine, uri: &str) -> Result<Vec<String>, PlayerError> {
    let sp = engine.session().spclient();
    let context = sp
        .get_context(uri)
        .await
        .map_err(|_| PlayerError::QueueContext)?;
    let mut tracks = Vec::new();
    let mut pending = Vec::new();
    if context.pages.iter().map(|p| p.tracks.len()).sum::<usize>() > MAX_CONTEXT_TRACKS {
        return Err(PlayerError::QueueContext);
    }
    for page in &context.pages {
        if page.tracks.is_empty() {
            if !page.page_url().is_empty() {
                pending.push(page.page_url().to_string());
            }
        } else {
            for t in &page.tracks {
                push_context_track(&mut tracks, t.uri(), t.gid());
            }
            if !page.next_page_url().is_empty() {
                pending.push(page.next_page_url().to_string());
            }
        }
    }
    complete_pages(tracks, pending, |url| async move {
        let bytes = sp
            .get_next_page(&url)
            .await
            .map_err(|_| PlayerError::QueueContext)?;
        serde_json::from_slice::<Value>(&bytes).map_err(|_| PlayerError::QueueContext)
    })
    .await
}
async fn complete_pages<F, Fut>(
    mut tracks: Vec<String>,
    mut pending: Vec<String>,
    mut fetch: F,
) -> Result<Vec<String>, PlayerError>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Value, PlayerError>>,
{
    let mut visited = HashSet::new();
    let mut fetched = 0;
    while let Some(url) = pending.first().cloned() {
        pending.remove(0);
        if fetched >= MAX_CONTEXT_PAGES
            || tracks.len() >= MAX_CONTEXT_TRACKS
            || !crate::discover::safe_page_url(&url)
            || !visited.insert(url.clone())
        {
            return Err(PlayerError::QueueContext);
        }
        fetched += 1;
        let page = fetch(url).await?;
        if page.get("tracks").and_then(Value::as_array).is_none() {
            return Err(PlayerError::QueueContext);
        }
        if page
            .get("tracks")
            .and_then(Value::as_array)
            .is_some_and(|rows| rows.len() + tracks.len() > MAX_CONTEXT_TRACKS)
        {
            return Err(PlayerError::QueueContext);
        }
        if let Some(next) = append_page(&mut tracks, &page) {
            pending.insert(0, next);
        }
    }
    Ok(tracks)
}
fn push_context_track(tracks: &mut Vec<String>, uri: &str, gid: &[u8]) {
    if tracks.len() >= MAX_CONTEXT_TRACKS {
        return;
    }
    if !uri.is_empty() {
        if playable(uri) && (!uri.starts_with("spotify:track:") || parse_track_uri(uri).is_ok()) {
            tracks.push(uri.to_string());
        }
    } else if let Ok(id) = SpotifyId::from_raw(gid).and_then(|id| id.to_base62()) {
        tracks.push(format!("spotify:track:{id}"));
    }
}

/// Appends a context page (JSON: `{"tracks":[{"uri":..}], "next_page_url":..}`) and returns
/// its next page, if any. Duplicates are kept: they are distinct rows of the context.
fn append_page(tracks: &mut Vec<String>, page: &Value) -> Option<String> {
    for item in page
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(uri) = item.get("uri").and_then(Value::as_str) {
            push_context_track(tracks, uri, &[]);
        }
    }
    page.get("next_page_url")
        .or_else(|| page.get("nextPageUrl"))
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineTrack;

    #[test]
    fn s9_playlist_delta_has_revision_and_keeps_duplicate_occurrences() {
        let uri = "spotify:track:4uLU6hMCjMI75M1A2tKUQC".to_string();
        let plan = playlist_changes(&[uri.clone(), uri.clone()], vec![1, 2, 3]).unwrap();
        assert_eq!(plan.base_revision, Some(vec![1, 2, 3]));
        let add = plan.deltas[0].ops[0].add.as_ref().unwrap();
        assert_eq!(add.add_last, Some(true));
        assert_eq!(add.items.len(), 2);
        assert_eq!(add.items[0].uri.as_deref(), Some(uri.as_str()));
        assert!(playlist_changes(&["spotify:local:a".into()], vec![]).is_err());
        assert!(playlist_changes(&[], vec![]).is_err());
    }

    #[tokio::test]
    #[ignore = "Manual mutation probe: disposable playlist, keyring login and explicit flag required"]
    async fn s9_session_playlist_probe() {
        assert!(
            session_writes_enabled(),
            "Set VELO_SESSION_PLAYLIST_WRITES=1"
        );
        let engine = Engine::from_keyring(crate::EngineConfig::velo("Velo playlist probe"))
            .await
            .unwrap();
        let track = std::env::var("VELO_PROBE_TRACK_URI").expect("Set VELO_PROBE_TRACK_URI");
        let result = match std::env::var("VELO_PROBE_PLAYLIST_ID") {
            Ok(id) => session_add_to_playlist(engine.session(), &id, &[track])
                .await
                .map(|_| format!("spotify:playlist:{id}")),
            Err(_) => {
                session_create_playlist_probe(engine.session(), "Velo S9 probe", &[track]).await
            }
        };
        engine.shutdown().await;
        println!("{}", result.unwrap());
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    fn track(uri: &str) -> Track {
        Track {
            id: None,
            uri: uri.into(),
            title: uri.into(),
            artists: vec![],
            album: String::new(),
            album_id: None,
            duration_ms: 1000,
            explicit: false,
            images: vec![],
            added_at: None,
        }
    }

    #[test]
    fn d3_lagging_queue_consumes_only_the_played_occurrence() {
        let rows = || ["b", "b", "c", "b"].map(track).to_vec();
        assert_eq!(
            align_upcoming("b", Some("a"), rows()),
            Some(["b", "c", "b"].map(track).to_vec())
        );
        assert_eq!(align_upcoming("b", Some("b"), rows()), Some(rows()));
        assert_eq!(
            align_upcoming("c", Some("a"), rows()),
            Some(vec![track("b")])
        );
        assert_eq!(align_upcoming("d", Some("a"), rows()), None);
        assert_eq!(align_upcoming("b", Some("b"), vec![]), Some(vec![]));
    }

    #[test]
    fn moves_both_directions_and_removes_one_duplicate_occurrence() {
        let mut q = vec!["a", "b", "a", "c"];
        assert!(edit_queue(&mut q, QueueEdit::Move { from: 0, to: 3 }));
        assert_eq!(q, ["b", "a", "c", "a"]);
        assert!(edit_queue(&mut q, QueueEdit::Move { from: 2, to: 0 }));
        assert_eq!(q, ["c", "b", "a", "a"]);
        assert!(edit_queue(&mut q, QueueEdit::Remove(2)));
        assert_eq!(q, ["c", "b", "a"]);
    }

    #[test]
    fn skip_to_drops_the_target_and_the_rows_before_it() {
        let mut q = vec!["a", "b", "c", "d"];
        assert!(edit_queue(&mut q, QueueEdit::SkipTo(1)));
        assert_eq!(q, ["c", "d"]);
        assert!(!edit_queue(&mut q, QueueEdit::SkipTo(2)), "out of range");
        assert_eq!(q, ["c", "d"]);
        assert!(edit_queue(&mut q, QueueEdit::SkipTo(1)));
        assert!(q.is_empty());
    }

    #[test]
    fn invalid_and_same_row_edits_are_noops_even_for_empty_queue() {
        for mut q in [vec![], vec!["a"]] {
            let original = q.clone();
            for edit in [
                QueueEdit::Remove(1),
                QueueEdit::Move { from: 0, to: 1 },
                QueueEdit::Move { from: 1, to: 0 },
                QueueEdit::Move { from: 0, to: 0 },
            ] {
                assert!(!edit_queue(&mut q, edit));
                assert_eq!(q, original);
            }
        }
        let mut q = vec!["a"];
        assert!(edit_queue(&mut q, QueueEdit::Remove(0)));
        assert!(q.is_empty());
    }

    #[test]
    fn tail_keeps_the_context_after_the_visible_rows() {
        let ctx = s(&["a", "b", "c", "d", "e", "f"]);
        // Playing b; the user queued "x" and "d"; the view showed x, d, c, d (cut at 4 rows).
        let visible = s(&["x", "d", "c", "d"]);
        assert_eq!(
            tail_after(&ctx, "b", &visible, false, None).unwrap(),
            s(&["e", "f"])
        );
        // A second edit runs against the list Velo loaded: current, edited rows, old tail.
        let loaded = s(&["b", "x", "d", "d", "e", "f"]);
        assert_eq!(
            tail_after(&loaded, "b", &s(&["x", "d", "d"]), false, None).unwrap(),
            s(&["e", "f"])
        );
        // Unknown current song or empty context: nothing invented.
        assert!(tail_after(&ctx, "zz", &visible, false, None).is_err());
        assert!(
            tail_after(&[], "b", &visible, false, None)
                .unwrap()
                .is_empty()
        );
        // Everything visible: no tail.
        assert!(
            tail_after(&ctx, "e", &s(&["f"]), false, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn shuffled_tail_is_what_was_not_shown_and_stays_complete() {
        let ctx = s(&["a", "b", "c", "d", "e"]);
        let mut tail = tail_after(&ctx, "c", &s(&["e", "a"]), true, None).unwrap();
        assert_eq!(tail, s(&["b", "d"]));
        shuffle_in_place(&mut tail, 42);
        tail.sort();
        assert_eq!(tail, s(&["b", "d"]));
        let mut many: Vec<u32> = (0..50).collect();
        shuffle_in_place(&mut many, 7);
        assert_ne!(many, (0..50).collect::<Vec<_>>());
        many.sort_unstable();
        assert_eq!(many, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn rebuilt_list_puts_current_first_and_drops_local_files() {
        let list = rebuilt_list(
            "spotify:track:a",
            &s(&["spotify:local:x", "spotify:track:b"]),
            &s(&["spotify:episode:c"]),
        );
        assert_eq!(
            list,
            s(&["spotify:track:a", "spotify:track:b", "spotify:episode:c"])
        );
    }

    #[test]
    fn clear_reloads_only_current_at_its_position_with_pause_and_repeat_preserved() {
        // Vaciar descarta filas visibles y contexto completo, sin intentar resolver su tail.
        let current = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
        let tracks = rebuilt_list(current, &[], &[]);
        assert_eq!(tracks, vec![current]);
        let request = rebuild_request(tracks, 42_000, false, RepeatMode::Track).unwrap();
        assert_eq!(request.seek_to, 42_000);
        assert!(!request.start_playing);
        assert!(matches!(
            request.playing_track,
            Some(PlayingTrack::Index(0))
        ));
        assert!(matches!(
            request.context_options,
            Some(LoadContextOptions::Options(Options {
                shuffle: false,
                repeat: false,
                repeat_track: true
            }))
        ));
    }

    #[test]
    fn rebuild_request_keeps_position_pause_and_repeat_but_never_shuffle() {
        assert!(rebuild_request(vec![], 0, true, RepeatMode::Off).is_none());
        assert!(rebuild_request(s(&["spotify:local:x"]), 0, true, RepeatMode::Off).is_none());
        let r = rebuild_request(
            s(&["spotify:track:a", "spotify:track:b"]),
            42_000,
            false,
            RepeatMode::Track,
        )
        .unwrap();
        assert!(!r.start_playing);
        assert_eq!(r.seek_to, 42_000);
        assert!(matches!(r.playing_track, Some(PlayingTrack::Index(0))));
        assert!(matches!(
            r.context_options,
            Some(LoadContextOptions::Options(Options {
                shuffle: false,
                repeat: false,
                repeat_track: true
            }))
        ));
    }

    #[test]
    fn position_now_advances_only_while_playing_and_stops_at_the_end() {
        let at = Instant::now();
        let mut st = EngineState::new(50, at);
        st.track = Some(EngineTrack {
            uri: "spotify:track:a".into(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 10_000,
            cover_url: None,
        });
        st.position_ms = 4_000;
        assert_eq!(position_now(&st, at + Duration::from_secs(2)), 4_000);
        st.playing = true;
        assert_eq!(position_now(&st, at + Duration::from_secs(2)), 6_000);
        assert_eq!(position_now(&st, at + Duration::from_secs(60)), 10_000);
    }

    #[test]
    fn retries_twice_with_growing_waits_then_gives_up() {
        let now = Instant::now();
        let fail = |uri: &str, transient| TrackFailure {
            uri: uri.into(),
            position_ms: 0,
            transient,
        };
        let mut book = RetryBook::default();
        assert_eq!(
            book.next(&fail("a", true), now),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            book.next(&fail("a", true), now),
            Some(Duration::from_secs(3))
        );
        assert_eq!(book.next(&fail("a", true), now), None);
        // Another song starts its own count; a non-transient failure is never retried.
        assert_eq!(
            book.next(&fail("b", true), now),
            Some(Duration::from_secs(1))
        );
        assert_eq!(book.next(&fail("c", false), now), None);
        // The same song much later counts again.
        assert_eq!(
            book.next(&fail("b", true), now + Duration::from_secs(120)),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn context_pages_keep_duplicates_and_report_the_next_page() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        let uri = format!("spotify:track:{id}");
        let page = serde_json::json!({
            "tracks": [{"uri": uri}, {"uri": "spotify:local:a:b:c:1"}, {"uri": uri}, {"uid": "x"}],
            "next_page_url": "hm://context-resolve/v1/page/2"
        });
        let mut tracks = Vec::new();
        assert_eq!(
            append_page(&mut tracks, &page).as_deref(),
            Some("hm://context-resolve/v1/page/2")
        );
        assert_eq!(tracks, vec![uri.clone(), uri]);
        assert_eq!(
            append_page(&mut tracks, &serde_json::json!({"tracks": []})),
            None
        );
    }
    #[test]
    fn q1b_duplicate_occurrences_survive_shuffle_and_current_uses_exact_index() {
        let context = s(&["a", "b", "b", "c"]);
        assert_eq!(
            tail_after(&context, "a", &s(&["b"]), true, None).unwrap(),
            s(&["b", "c"])
        );
        let context = s(&["a", "x", "a", "b", "b", "c"]);
        assert_eq!(
            tail_after(&context, "a", &s(&["b"]), false, Some(2)).unwrap(),
            s(&["b", "c"])
        );
        assert!(tail_after(&context, "a", &s(&["b"]), false, None).is_err());
        assert_eq!(
            tail_after(&context, "a", &s(&["b", "b"]), true, None).unwrap(),
            s(&["x", "a", "c"])
        );
    }
    #[tokio::test(start_paused = true)]
    async fn q1a_page_error_and_timeout_never_confirm_a_partial_context_over_twenty_rows() {
        let initial = vec!["spotify:track:4uLU6hMCjMI75M1A2tKUQC".to_string(); 30];
        let pages = vec!["hm://context-resolve/v1/page/2".to_string()];
        let failed = complete_pages(initial.clone(), pages.clone(), |_| async {
            Err(PlayerError::QueueContext)
        })
        .await;
        assert!(failed.is_err());
        let timed_out =
            complete_with_timeout(complete_pages(initial.clone(), pages.clone(), |_| {
                std::future::pending()
            }))
            .await;
        assert!(timed_out.is_err());
        let malformed = complete_pages(initial.clone(), pages.clone(), |_| async {
            Ok(serde_json::json!({}))
        })
        .await;
        assert!(malformed.is_err());
        let complete = complete_pages(initial.clone(), pages, |_| async {
            Ok(serde_json::json!({"tracks": []}))
        })
        .await
        .unwrap();
        assert_eq!(complete, initial);
    }
    #[test]
    fn q1a_known_last_song_has_a_complete_empty_tail_for_retry() {
        assert_eq!(
            tail_after(&s(&["a"]), "a", &[], false, Some(0)).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            tail_after(&s(&["a", "b"]), "a", &[], false, Some(0)).unwrap(),
            s(&["b"])
        );
        assert!(tail_after(&s(&["b"]), "a", &[], false, None).is_err());
    }
}
#[path = "connect_snapshot.rs"]
pub mod connect_snapshot;
