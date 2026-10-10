//! Discovery through the audio Session's spclient, never the public recommendations API.
//! These Spotify services are internal and undocumented: availability and schemas can change.
//! No audio is requested here. All calls must run on a Tokio worker, outside the Qt thread.

use std::{collections::HashSet, time::Duration};

use librespot_core::{Session, SpotifyId, SpotifyUri};
use librespot_protocol::context::Context;
use serde_json::Value;

use crate::{Engine, LoadTarget, parse_track_uri};

pub const MAX_TRACKS: usize = 50;
pub const MAX_MIXES: usize = 6;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoverError {
    #[error("Invalid radio seed (expected a canonical track, artist, album or playlist URI)")]
    InvalidSeed,
    #[error("Spotify discovery is unavailable")]
    Unavailable,
    #[error("Spotify discovery timed out")]
    Timeout,
    #[error("Spotify returned an unsupported discovery response")]
    Format,
    #[error("Spotify returned no playable discovery items")]
    Empty,
}
pub type Result<T> = std::result::Result<T, DiscoverError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Station {
    /// Provider title, empty if Spotify supplied none (the UI can use a translated fallback).
    pub title: String,
    pub uri: Option<String>,
    pub tracks: Vec<String>,
}

impl Station {
    pub fn load_target(&self) -> Result<LoadTarget> {
        if let Some(uri) = self.uri.as_deref().filter(|uri| context_uri(uri)) {
            return Ok(LoadTarget::Context {
                uri: uri.into(),
                index: None,
                shuffle: false,
            });
        }
        if self.tracks.is_empty() {
            return Err(DiscoverError::Empty);
        }
        Ok(LoadTarget::Tracks {
            uris: self.tracks.clone(),
            index: 0,
            shuffle: false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    pub uri: String,
    pub title: String,
    pub subtitle: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mix {
    /// Stable seed URI, used as mixId; never a row index.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub image_url: Option<String>,
    pub play_uri: Option<String>,
    pub tracks: Vec<String>,
}

impl Mix {
    pub fn load_target(&self) -> Result<LoadTarget> {
        Station {
            title: self.title.clone(),
            uri: self.play_uri.clone(),
            tracks: self.tracks.clone(),
        }
        .load_target()
    }
}

pub struct Discover<'a> {
    session: &'a Session,
    seeds: &'a [Seed],
}

impl<'a> Discover<'a> {
    pub fn new(engine: &'a Engine, seeds: &'a [Seed]) -> Self {
        Self {
            session: engine.session(),
            seeds,
        }
    }

    /// Same, for callers that only hold the session (no Engine).
    pub fn with_session(session: &'a Session, seeds: &'a [Seed]) -> Self {
        Self { session, seeds }
    }

    /// Tracks: inspiredby-mix/v2/seed_to_playlist; other seed kinds: radio-apollo/v3/stations.
    /// Apollo is also the fallback when inspiredby-mix has been retired for this account.
    pub async fn radio(&self, seed_uri: &str) -> Result<Station> {
        validate_seed(seed_uri)?;
        tokio::time::timeout(REQUEST_TIMEOUT, self.radio_inner(seed_uri))
            .await
            .map_err(|_| DiscoverError::Timeout)?
    }

    async fn radio_inner(&self, seed_uri: &str) -> Result<Station> {
        let sp = self.session.spclient();
        let inspired = if seed_uri.starts_with("spotify:track:") {
            let uri = SpotifyUri::from_uri(seed_uri).map_err(|_| DiscoverError::InvalidSeed)?;
            match sp.get_radio_for_track(&uri).await {
                Ok(bytes) => parse_station_json(&bytes).ok(),
                Err(_) => None,
            }
        } else {
            None
        };
        let mut station = match inspired {
            Some(station) => station,
            None => {
                let bytes = sp
                    .get_apollo_station("stations", seed_uri, Some(MAX_TRACKS), vec![], false)
                    .await
                    .map_err(|_| DiscoverError::Unavailable)?;
                parse_station_json(&bytes)?
            }
        };
        // /context-resolve/v1/{uri}: librespot parses protobuf JSON into Context.
        // Resolve only the returned radio context, never the seed's original track list.
        if station.tracks.is_empty() {
            if let Some(uri) = station.uri.clone() {
                if let Ok(context) = sp.get_context(&uri).await {
                    let resolved = station_from_context(&context);
                    if station.title.is_empty() {
                        station.title = resolved.title;
                    }
                    station.tracks = resolved.tracks;
                    // At most three pages; do not walk an unbounded personalized context.
                    let mut next = context.pages.last().map(|p| {
                        if p.tracks.is_empty() && !p.page_url().is_empty() {
                            p.page_url().to_string()
                        } else {
                            p.next_page_url().to_string()
                        }
                    });
                    let mut visited = HashSet::new();
                    for _ in 0..3 {
                        let Some(url) = next
                            .take()
                            .filter(|u| safe_page_url(u) && visited.insert(u.clone()))
                        else {
                            break;
                        };
                        if station.tracks.len() >= MAX_TRACKS {
                            break;
                        }
                        // Keep useful tracks from earlier pages when pagination is unavailable.
                        let Ok(bytes) = sp.get_next_page(&url).await else {
                            break;
                        };
                        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                            break;
                        };
                        append_tracks(&mut station.tracks, &value);
                        next = value
                            .get("next_page_url")
                            .or_else(|| value.get("nextPageUrl"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                }
            }
        }
        // /radio-apollo/v3/tracks supplies explicit tracks when stations returned metadata only.
        if station.tracks.is_empty() {
            let bytes = sp
                .get_apollo_station("tracks", seed_uri, Some(MAX_TRACKS), vec![], false)
                .await
                .map_err(|_| DiscoverError::Unavailable)?;
            let tracks = parse_station_json(&bytes)?;
            station.tracks = tracks.tracks;
            if station.title.is_empty() {
                station.title = tracks.title;
            }
            if station.uri.is_none() {
                station.uri = tracks.uri;
            }
        }
        if station.tracks.is_empty() {
            return Err(DiscoverError::Empty);
        }
        Ok(station)
    }

    /// "For you" is a bounded collection of radios seeded by the caller's Web API top items.
    /// It is not Spotify's Home feed or Daily Mix. There is no public recommendation request.
    /// Failed seeds are omitted; no successful seed means Err, so callers can hide the section.
    pub async fn for_you(&self) -> Result<Vec<Mix>> {
        let mut mixes = Vec::new();
        let mut seen = HashSet::new();
        let mut failure = DiscoverError::Empty;
        for seed in self
            .seeds
            .iter()
            .filter(|s| seen.insert(s.uri.clone()))
            .take(MAX_MIXES)
        {
            match self.radio(&seed.uri).await {
                Ok(station) => mixes.push(Mix {
                    id: seed.uri.clone(),
                    title: if station.title.is_empty() {
                        seed.title.clone()
                    } else {
                        station.title
                    },
                    subtitle: seed.subtitle.clone(),
                    image_url: seed.image_url.clone(),
                    play_uri: station.uri,
                    tracks: station.tracks,
                }),
                Err(error) => failure = error,
            }
        }
        if mixes.is_empty() {
            Err(failure)
        } else {
            Ok(mixes)
        }
    }
}

/// "Recomendado para ti": track radios of a few seed tracks (the user's most played or
/// recently played), interleaved so every seed weighs the same. The seeds themselves and
/// `exclude` are left out. Empty on any failure: the Inicio section is then hidden.
pub async fn recommended_tracks(
    session: &Session,
    seeds: &[String],
    exclude: &HashSet<String>,
    limit: usize,
) -> Vec<velo_domain::Track> {
    const SEEDS: usize = 3;
    let discover = Discover::with_session(session, &[]);
    let mut lists = Vec::new();
    for seed in seeds
        .iter()
        .filter(|s| s.starts_with("spotify:track:"))
        .take(SEEDS)
    {
        if let Ok(station) = discover.radio(seed).await {
            lists.push(station.tracks);
        }
    }
    let mut skip = exclude.clone();
    skip.extend(seeds.iter().cloned());
    let items: Vec<_> = merge_recommended(lists, &skip, limit)
        .into_iter()
        .map(|uri| crate::playlist::PlaylistItem {
            uri,
            added_at: None,
        })
        .collect();
    crate::playlist::tracks_in(session, &items).await
}

/// Round-robin over the radios, deduplicated, without `skip`, at most `limit` URIs.
pub fn merge_recommended(
    lists: Vec<Vec<String>>,
    skip: &HashSet<String>,
    limit: usize,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..longest {
        for list in &lists {
            if let Some(uri) = list.get(i) {
                if out.len() < limit && !skip.contains(uri) && seen.insert(uri.clone()) {
                    out.push(uri.clone());
                }
            }
        }
    }
    out
}

pub fn validate_seed(uri: &str) -> Result<()> {
    let mut parts = uri.split(':');
    let valid = parts.next() == Some("spotify")
        && matches!(
            parts.next(),
            Some("track" | "artist" | "album" | "playlist")
        )
        && parts
            .next()
            .is_some_and(|id| id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        && parts.next().is_none()
        && SpotifyUri::from_uri(uri).is_ok();
    if valid {
        Ok(())
    } else {
        Err(DiscoverError::InvalidSeed)
    }
}

fn context_uri(uri: &str) -> bool {
    let mut parts = uri.split(':');
    parts.next() == Some("spotify")
        && matches!(parts.next(), Some("playlist" | "station"))
        && parts.next().is_some_and(|s| !s.is_empty())
        && uri.bytes().all(|b| b.is_ascii_alphanumeric() || b == b':')
}

pub(crate) fn safe_page_url(url: &str) -> bool {
    let path = url
        .strip_prefix("hm://")
        .map(|rest| format!("/{rest}"))
        .unwrap_or_else(|| url.to_string());
    ["/context-resolve/", "/playlist/v2/", "/radio-apollo/v3/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        && !path.contains("..")
        && !path.contains('#')
}

fn push_track(tracks: &mut Vec<String>, uri: &str) {
    if tracks.len() < MAX_TRACKS && parse_track_uri(uri).is_ok() && !tracks.iter().any(|t| t == uri)
    {
        tracks.push(uri.to_string());
    }
}

fn append_tracks(tracks: &mut Vec<String>, value: &Value) {
    if let Some(items) = value.as_array() {
        for item in items.iter().take(500) {
            if let Some(uri) = item
                .as_str()
                .or_else(|| item.get("uri").and_then(Value::as_str))
            {
                push_track(tracks, uri);
            }
        }
    }
    for key in ["tracks", "items"] {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            for item in items.iter().take(500) {
                let item = item.get("track").unwrap_or(item);
                if let Some(uri) = item
                    .as_str()
                    .or_else(|| item.get("uri").and_then(Value::as_str))
                {
                    push_track(tracks, uri);
                }
            }
        }
    }
    if let Some(pages) = value.get("pages").and_then(Value::as_array) {
        for page in pages.iter().take(3) {
            if let Some(items) = page.get("tracks") {
                append_tracks(tracks, items);
            }
        }
    }
}

/// Pure parsing; tolerate added fields, reject malformed JSON and responses with no playable data.
pub fn parse_station_json(bytes: &[u8]) -> Result<Station> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| DiscoverError::Format)?;
    let station = value
        .get("stations")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .or_else(|| value.get("station"))
        .or_else(|| value.get("playlist"))
        .unwrap_or(&value);
    let uri = [
        "uri",
        "playlist_uri",
        "playlistUri",
        "context_uri",
        "contextUri",
    ]
    .into_iter()
    .find_map(|key| station.get(key).and_then(Value::as_str))
    .filter(|u| context_uri(u))
    .map(str::to_string);
    let title = station
        .get("title")
        .or_else(|| station.get("name"))
        .or_else(|| station.pointer("/metadata/context_description"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut tracks = Vec::new();
    append_tracks(&mut tracks, station);
    if station != &value {
        append_tracks(&mut tracks, &value);
    }
    if uri.is_none() && tracks.is_empty() {
        return Err(DiscoverError::Empty);
    }
    Ok(Station { title, uri, tracks })
}

/// Pure protobuf extraction; only canonical tracks are exposed (no episodes/local files).
pub fn station_from_context(context: &Context) -> Station {
    let mut tracks = Vec::new();
    for page in &context.pages {
        for track in &page.tracks {
            if !track.uri().is_empty() {
                push_track(&mut tracks, track.uri());
            } else if let Ok(id) = SpotifyId::from_raw(track.gid()) {
                if let Ok(id) = id.to_base62() {
                    push_track(&mut tracks, &format!("spotify:track:{id}"));
                }
            }
        }
    }
    Station {
        title: context
            .metadata
            .get("context_description")
            .or_else(|| context.metadata.get("name"))
            .cloned()
            .unwrap_or_default(),
        uri: context
            .uri
            .as_deref()
            .filter(|u| context_uri(u))
            .map(str::to_string),
        tracks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Hand-written expected JSON shapes, not captures. The leader validates real formats live.
    const TRACK: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    #[test]
    fn merge_recommended_interleaves_dedups_and_skips() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let skip = HashSet::from(["b".to_string()]);
        let out = merge_recommended(vec![l(&["a", "b", "c"]), l(&["a", "d"])], &skip, 3);
        assert_eq!(out, ["a", "d", "c"]);
        assert!(merge_recommended(Vec::new(), &skip, 5).is_empty());
    }

    #[test]
    fn inspired_playlist_fixture() {
        let station =
            parse_station_json(include_bytes!("discover_fixtures/inspired.json")).unwrap();
        assert_eq!(station.title, "Seed radio");
        assert!(matches!(
            station.load_target().unwrap(),
            LoadTarget::Context { .. }
        ));
        assert_eq!(station.tracks, [TRACK]);
    }
    #[test]
    fn apollo_fixture_filters_and_deduplicates() {
        let station = parse_station_json(include_bytes!("discover_fixtures/apollo.json")).unwrap();
        assert_eq!(station.tracks, [TRACK]);
        assert!(matches!(
            station.load_target().unwrap(),
            LoadTarget::Tracks { index: 0, .. }
        ));
    }
    #[test]
    fn context_fixture_is_pure_and_tracks_are_capped() {
        let fixture: Value =
            serde_json::from_slice(include_bytes!("discover_fixtures/context.json")).unwrap();
        let parsed = parse_station_json(&serde_json::to_vec(&fixture).unwrap()).unwrap();
        let mut context = Context::new();
        context.uri = parsed.uri.clone();
        context
            .metadata
            .insert("name".into(), "Context radio".into());
        let mut page = librespot_protocol::context_page::ContextPage::new();
        for i in 0..75 {
            let mut track = librespot_protocol::context_track::ContextTrack::new();
            track.uri = Some(format!("spotify:track:{i:022}"));
            page.tracks.push(track);
        }
        context.pages.push(page);
        assert_eq!(station_from_context(&context).tracks.len(), MAX_TRACKS);
        assert_eq!(parsed.tracks, [TRACK]);
    }
    #[test]
    fn failures_are_results_not_empty_success() {
        assert_eq!(parse_station_json(b"not json"), Err(DiscoverError::Format));
        for json in [
            b"{}".as_slice(),
            b"{\"error\":\"denied\"}",
            b"{\"tracks\":[]}",
        ] {
            assert_eq!(parse_station_json(json), Err(DiscoverError::Empty));
        }
        assert_eq!(
            Station {
                title: String::new(),
                uri: None,
                tracks: vec![]
            }
            .load_target(),
            Err(DiscoverError::Empty)
        );
    }
    #[test]
    fn protobuf_gid_tracks_are_supported_without_relabeling_episodes() {
        let mut context = Context::new();
        let mut page = librespot_protocol::context_page::ContextPage::new();
        let mut track = librespot_protocol::context_track::ContextTrack::new();
        track.gid = Some(vec![0; 16]);
        page.tracks.push(track.clone());
        track.uri = Some("spotify:episode:4uLU6hMCjMI75M1A2tKUQC".into());
        page.tracks.push(track);
        context.pages.push(page);
        assert_eq!(
            station_from_context(&context).tracks,
            ["spotify:track:0000000000000000000000"]
        );
    }
    #[test]
    fn pagination_stays_on_known_spclient_paths() {
        assert!(safe_page_url("hm://playlist/v2/playlist/id?from=50"));
        assert!(safe_page_url("/context-resolve/v1/page"));
        for url in [
            "https://other.test/page",
            "//other.test/page",
            "hm://playlist/v2/../secret",
            "hm://unknown/page",
        ] {
            assert!(!safe_page_url(url));
        }
    }
    #[test]
    fn seeds_reject_injection_urls_and_unsupported_kinds() {
        for kind in ["track", "artist", "album", "playlist"] {
            assert!(validate_seed(&format!("spotify:{kind}:4uLU6hMCjMI75M1A2tKUQC")).is_ok());
        }
        for uri in [
            "",
            "spotify:track:short",
            "spotify:episode:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:artist:x?foo=bar",
            "https://open.spotify.com/track/x",
            "spotify:track:ZZZZZZZZZZZZZZZZZZZZZZ",
        ] {
            assert!(validate_seed(uri).is_err());
        }
    }
}
