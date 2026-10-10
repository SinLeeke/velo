//! Songs of playlists the Web API will not list (F4): since 2026 Development Mode only lists
//! the items of playlists the user owns or collaborates on. Velo reads the others through the
//! audio Session's spclient, like E4: `playlist/v2` (internal, undocumented) for the item list
//! with the date each song was added, and `metadata/4/track` for each song's title, artists,
//! album, cover and duration. Metadata only, never audio. Run on a Tokio worker.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use librespot_core::spotify_id::SpotifyId;
use librespot_core::{Session, SpotifyUri};
use librespot_metadata::image::ImageSize;
use librespot_metadata::{Metadata, Playlist, Track as SpTrack};
use serde_json::Value;
use tokio::sync::Semaphore;
use velo_domain::{ArtistRef, ImageRef, Track};

use crate::{Engine, parse_track_uri};

/// Same bounds as the queue context (Q1): enough for any real playlist.
pub const MAX_ITEMS: usize = 10_000;
/// Parallel metadata requests for one page of songs.
const PARALLEL: usize = 8;
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const TRACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Spotify's public image host; ids come from the metadata (`FileId` in base 16).
const IMAGE_HOST: &str = "https://i.scdn.co/image/";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlaylistError {
    #[error("Not a Spotify playlist URI")]
    InvalidUri,
    #[error("Spotify did not return the playlist")]
    Unavailable,
    #[error("Spotify playlist request timed out")]
    Timeout,
    #[error("Spotify returned only part of the playlist")]
    Incomplete,
}

/// One row of the playlist, in its order (episodes and local files are skipped: they cannot be
/// described by the track metadata service).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    pub uri: String,
    /// Unix ms; `None` when Spotify has no date for the row.
    pub added_at: Option<i64>,
}

/// The playlist's song rows, in order, at most [`MAX_ITEMS`].
pub async fn playlist_items(
    engine: &Engine,
    uri: &str,
) -> Result<Vec<PlaylistItem>, PlaylistError> {
    playlist_items_in(engine.session(), uri).await
}

/// [`playlist_items`] with any signed-in session (L1: the session data provider).
pub async fn playlist_items_in(
    session: &Session,
    uri: &str,
) -> Result<Vec<PlaylistItem>, PlaylistError> {
    let parsed = SpotifyUri::from_uri(uri).map_err(|_| PlaylistError::InvalidUri)?;
    if !matches!(parsed, SpotifyUri::Playlist { .. }) {
        return Err(PlaylistError::InvalidUri);
    }
    tokio::time::timeout(LIST_TIMEOUT, async {
        // playlist/v2 includes dates but is not guaranteed to return every slot.
        let metadata = Playlist::get(session, &parsed).await.ok();
        if let Some(p) = &metadata {
            if p.contents.items.len() == usize::try_from(p.length).unwrap_or(usize::MAX)
                && p.contents.items.len() <= MAX_ITEMS
            {
                return Ok(items_of(p));
            }
        }
        let sp = session.spclient();
        let context = sp
            .get_context(uri)
            .await
            .map_err(|_| PlaylistError::Unavailable)?;
        let mut items = Vec::new();
        let mut pending = VecDeque::new();
        for page in &context.pages {
            for t in &page.tracks {
                let uri = if t.uri().is_empty() {
                    SpotifyId::from_raw(t.gid())
                        .and_then(|id| id.to_base62())
                        .ok()
                        .map(|id| format!("spotify:track:{id}"))
                } else {
                    Some(t.uri().to_string())
                };
                if let Some(uri) = uri {
                    push_item(&mut items, &uri);
                }
            }
            if page.tracks.is_empty() && !page.page_url().is_empty() {
                pending.push_back(page.page_url().to_string());
            } else if !page.next_page_url().is_empty() {
                pending.push_back(page.next_page_url().to_string());
            }
        }
        let mut visited = HashSet::new();
        while let Some(url) = pending.pop_front() {
            if visited.len() >= 40
                || items.len() >= MAX_ITEMS
                || !crate::discover::safe_page_url(&url)
                || !visited.insert(url.clone())
            {
                return Err(PlaylistError::Incomplete);
            }
            let bytes = sp
                .get_next_page(&url)
                .await
                .map_err(|_| PlaylistError::Unavailable)?;
            let page: Value =
                serde_json::from_slice(&bytes).map_err(|_| PlaylistError::Unavailable)?;
            if let Some(next) = append_page(&mut items, &page) {
                pending.push_front(next);
            }
        }
        if items.len() > MAX_ITEMS {
            return Err(PlaylistError::Incomplete);
        }
        // Dates are safe to reuse only when the two services agree on row order.
        if let Some(p) = metadata {
            let dated = items_of(&p);
            for (item, date) in items.iter_mut().zip(dated) {
                if item.uri != date.uri {
                    break;
                }
                item.added_at = date.added_at;
            }
        }
        Ok(items)
    })
    .await
    .map_err(|_| PlaylistError::Timeout)?
}

fn push_item(items: &mut Vec<PlaylistItem>, uri: &str) {
    if items.len() <= MAX_ITEMS && parse_track_uri(uri).is_ok() {
        items.push(PlaylistItem {
            uri: uri.into(),
            added_at: None,
        });
    }
}

fn append_page(items: &mut Vec<PlaylistItem>, value: &Value) -> Option<String> {
    for row in value
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(uri) = row.get("uri").and_then(Value::as_str) {
            push_item(items, uri);
        }
    }
    value
        .get("next_page_url")
        .or_else(|| value.get("nextPageUrl"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn items_of(playlist: &Playlist) -> Vec<PlaylistItem> {
    playlist
        .contents
        .items
        .iter()
        .filter_map(|item| {
            let uri = item.id.to_uri().ok()?;
            parse_track_uri(&uri).ok()?;
            let ms = item.attributes.timestamp.as_timestamp_ms();
            Some(PlaylistItem {
                uri,
                added_at: (ms > 0).then_some(ms),
            })
        })
        .take(MAX_ITEMS)
        .collect()
}

/// Metadata of `items`, in their order. Songs whose metadata fails are left out.
pub async fn tracks(engine: &Engine, items: &[PlaylistItem]) -> Vec<Track> {
    tracks_in(engine.session(), items).await
}

/// [`tracks`] with any signed-in session.
pub async fn tracks_in(session: &Session, items: &[PlaylistItem]) -> Vec<Track> {
    let permits = Arc::new(Semaphore::new(PARALLEL));
    let mut set = tokio::task::JoinSet::new();
    for (i, item) in items.iter().enumerate() {
        let Ok(uri) = SpotifyUri::from_uri(&item.uri) else {
            continue;
        };
        let session = session.clone();
        let permits = permits.clone();
        let added_at = item.added_at;
        set.spawn(async move {
            let _permit = permits.acquire_owned().await.ok()?;
            let track = tokio::time::timeout(TRACK_TIMEOUT, SpTrack::get(&session, &uri))
                .await
                .ok()?
                .ok()?;
            let mut t = domain_track(&track)?;
            t.added_at = added_at;
            Some((i, t))
        });
    }
    let mut out: Vec<(usize, Track)> = Vec::with_capacity(items.len());
    while let Some(done) = set.join_next().await {
        if let Ok(Some(row)) = done {
            out.push(row);
        }
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, t)| t).collect()
}

/// librespot's track metadata → Velo's domain track (cover URLs on Spotify's image host).
pub fn domain_track(t: &SpTrack) -> Option<Track> {
    let uri = t.id.to_uri().ok()?;
    let id = t.id.to_id().ok();
    Some(Track {
        id,
        uri,
        title: t.name.clone(),
        artists: t
            .artists
            .iter()
            .map(|a| ArtistRef {
                id: a.id.to_id().ok(),
                name: a.name.clone(),
            })
            .collect(),
        album: t.album.name.clone(),
        album_id: t.album.id.to_id().ok().filter(|id| !id.is_empty()),
        duration_ms: u64::try_from(t.duration).unwrap_or_default(),
        explicit: t.is_explicit,
        images: t
            .album
            .covers
            .iter()
            .filter_map(|image| {
                let file = image.id.to_base16().ok()?;
                let edge = if image.width > 0 {
                    u32::try_from(image.width).ok()
                } else {
                    size_px(image.size)
                };
                Some(ImageRef {
                    url: format!("{IMAGE_HOST}{file}"),
                    width: edge,
                    height: edge,
                })
            })
            .collect(),
        added_at: None,
    })
}

/// Nominal edge of Spotify's cover sizes when the metadata gives no width.
fn size_px(size: ImageSize) -> Option<u32> {
    match size {
        ImageSize::SMALL => Some(64),
        ImageSize::DEFAULT => Some(300),
        ImageSize::LARGE | ImageSize::XLARGE => Some(640),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::metadata;

    const TRACK: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    #[test]
    fn context_pages_preserve_duplicates_and_skip_episodes() {
        let mut items = Vec::new();
        let next = append_page(
            &mut items,
            &serde_json::json!({
                "tracks": [{"uri": TRACK}, {"uri": "spotify:episode:4uLU6hMCjMI75M1A2tKUQC"}, {"uri": TRACK}],
                "nextPageUrl": "hm://playlist/v2/playlist/id?from=50"
            }),
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], items[1]);
        assert!(items[0].added_at.is_none());
        assert_eq!(
            next.as_deref(),
            Some("hm://playlist/v2/playlist/id?from=50")
        );
    }

    fn track_message() -> metadata::Track {
        let mut m = metadata::Track::new();
        m.gid = Some(SpotifyUri::from_uri(TRACK).ok().map(gid).unwrap());
        m.name = Some("Marea".into());
        m.duration = Some(261_000);
        m.explicit = Some(true);
        let mut artist = metadata::Artist::new();
        artist.gid = Some(vec![1; 16]);
        artist.name = Some("Lumen".into());
        m.artist.push(artist);
        let mut album = metadata::Album::new();
        album.gid = Some(vec![2; 16]);
        album.name = Some("Atlas".into());
        let mut image = metadata::Image::new();
        image.file_id = Some(vec![0xab; 20]);
        image.size = Some(metadata::image::Size::SMALL.into());
        album.cover_group.mut_or_insert_default().image.push(image);
        m.album = Some(album).into();
        m
    }

    fn gid(uri: SpotifyUri) -> Vec<u8> {
        match uri {
            SpotifyUri::Track { id } => id.to_raw().to_vec(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn metadata_becomes_a_domain_track_with_a_cover_url() {
        let sp = SpTrack::try_from(&track_message()).expect("valid message");
        let t = domain_track(&sp).unwrap();
        assert_eq!(t.uri, TRACK);
        assert_eq!(t.id.as_deref(), Some("4uLU6hMCjMI75M1A2tKUQC"));
        assert_eq!(t.title, "Marea");
        assert_eq!(t.artist_line(), "Lumen");
        assert!(t.artists[0].id.is_some());
        assert_eq!(t.album, "Atlas");
        assert!(t.album_id.is_some());
        assert_eq!(t.duration_ms, 261_000);
        assert!(t.explicit);
        assert_eq!(t.images.len(), 1);
        assert_eq!(t.images[0].url, format!("{IMAGE_HOST}{}", "ab".repeat(20)));
        assert_eq!(t.images[0].width, Some(64));
        assert_eq!(t.added_at, None);
    }

    #[test]
    fn playlist_rows_keep_order_and_dates_and_skip_what_is_not_a_song() {
        use librespot_protocol::playlist4_external as pl;
        let mut contents = pl::ListItems::new();
        for (uri, ms) in [
            (TRACK, 1_772_366_400_000_i64),
            ("spotify:episode:4uLU6hMCjMI75M1A2tKUQC", 5),
            ("spotify:local:Artist:Album:Song:60", 5),
            ("spotify:track:0000000000000000000000", 0),
        ] {
            let mut item = pl::Item::new();
            item.uri = Some(uri.into());
            let mut attributes = pl::ItemAttributes::new();
            attributes.timestamp = Some(ms);
            item.attributes = Some(attributes).into();
            contents.items.push(item);
        }
        let mut message = pl::SelectedListContent::new();
        message.contents = Some(contents).into();
        let uri = SpotifyUri::from_uri("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M").unwrap();
        let playlist = <Playlist as Metadata>::parse(&message, &uri).expect("valid playlist");
        assert_eq!(
            items_of(&playlist),
            [
                PlaylistItem {
                    uri: TRACK.into(),
                    added_at: Some(1_772_366_400_000)
                },
                PlaylistItem {
                    uri: "spotify:track:0000000000000000000000".into(),
                    added_at: None
                },
            ]
        );
    }
}
