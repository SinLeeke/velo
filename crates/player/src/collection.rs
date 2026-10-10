//! L1: "Me gusta" (the user's collection) through the audio Session's spclient
//! (`collection/v2`, internal and undocumented, the same service the official clients use), so
//! the library does not depend on the Web API quota. Metadata only. Run on a Tokio worker.

use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, header::CONTENT_TYPE};
use librespot_core::Session;
use protobuf::Message;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/collection2v2/mod.rs"));
}
use generated::collection2v2::{CollectionItem, PageRequest, PageResponse, WriteRequest};

/// The set holding liked songs and saved albums.
pub const LIKED_SET: &str = "collection";
/// Followed artists.
pub const ARTIST_SET: &str = "artist";
/// Upper bound of rows read from one set (like the playlist reader).
pub const MAX_ITEMS: usize = 10_000;
const PAGE: i32 = 300;
const MAX_PAGES: usize = 40;
const TIMEOUT: Duration = Duration::from_secs(20);
const PROTO: &str = "application/vnd.collection-v2.spotify.proto";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CollectionError {
    #[error("Spotify did not return the collection")]
    Unavailable,
    #[error("Spotify collection request timed out")]
    Timeout,
    #[error("Not a Spotify URI that can be saved")]
    InvalidUri,
}

/// One saved row: `spotify:track:…`, `spotify:album:…` or `spotify:artist:…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedItem {
    pub uri: String,
    /// Unix ms, `None` when Spotify has no date.
    pub added_at: Option<i64>,
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(PROTO));
    headers.insert(http::header::ACCEPT, HeaderValue::from_static(PROTO));
    headers
}

/// Every row of `set`, newest first (the order Spotify's own apps show), at most [`MAX_ITEMS`].
pub async fn read_set(session: &Session, set: &str) -> Result<Vec<SavedItem>, CollectionError> {
    let username = session.username();
    tokio::time::timeout(TIMEOUT, async {
        read_pages(|token| {
            let request = PageRequest {
                username: username.clone(),
                set: set.to_string(),
                pagination_token: token,
                limit: PAGE,
                ..Default::default()
            };
            async move {
                let body = request
                    .write_to_bytes()
                    .map_err(|_| CollectionError::Unavailable)?;
                session
                    .spclient()
                    .request(
                        &Method::POST,
                        "/collection/v2/paging",
                        Some(headers()),
                        Some(&body),
                    )
                    .await
                    .map_err(|_| CollectionError::Unavailable)
            }
        })
        .await
    })
    .await
    .map_err(|_| CollectionError::Timeout)?
}

/// Bounded pagination; cycles and truncated results are errors, not a successful empty library.
async fn read_pages<F, Fut>(mut fetch: F) -> Result<Vec<SavedItem>, CollectionError>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Bytes, CollectionError>>,
{
    let mut rows = Vec::new();
    let mut token = String::new();
    let mut seen_tokens = std::collections::HashSet::new();
    let mut seen_uris = std::collections::HashSet::new();
    for _ in 0..MAX_PAGES {
        let (page, next) = parse_page(&fetch(token).await?)?;
        rows.extend(page.into_iter().filter(|r| seen_uris.insert(r.uri.clone())));
        if next.is_empty() {
            return Ok(sorted(rows));
        }
        if rows.len() >= MAX_ITEMS || !seen_tokens.insert(next.clone()) {
            return Err(CollectionError::Unavailable);
        }
        token = next;
    }
    Err(CollectionError::Unavailable)
}

/// Rows of one page and the next page token (empty at the end).
pub fn parse_page(bytes: &Bytes) -> Result<(Vec<SavedItem>, String), CollectionError> {
    let page = PageResponse::parse_from_bytes(bytes).map_err(|_| CollectionError::Unavailable)?;
    let rows = page
        .items
        .iter()
        .filter(|i| !i.is_removed && !i.uri.is_empty())
        .map(|i| SavedItem {
            uri: i.uri.clone(),
            added_at: (i.added_at > 0).then(|| i64::from(i.added_at) * 1000),
        })
        .collect();
    Ok((rows, page.next_page_token))
}

fn sorted(mut rows: Vec<SavedItem>) -> Vec<SavedItem> {
    // Stable: rows without a date keep Spotify's order after the dated ones.
    rows.sort_by_key(|r| std::cmp::Reverse(r.added_at));
    rows.truncate(MAX_ITEMS);
    rows
}

/// Saves (`saved = true`) or removes `uris` in `set`.
pub async fn write_set(
    session: &Session,
    set: &str,
    uris: &[String],
    saved: bool,
) -> Result<(), CollectionError> {
    if uris.is_empty() || uris.iter().any(|u| !savable(u)) {
        return Err(CollectionError::InvalidUri);
    }
    let body = write_request(&session.username(), set, uris, saved, now_secs())
        .write_to_bytes()
        .map_err(|_| CollectionError::Unavailable)?;
    tokio::time::timeout(TIMEOUT, async {
        session
            .spclient()
            .request(
                &Method::POST,
                "/collection/v2/write",
                Some(headers()),
                Some(&body),
            )
            .await
            .map(|_| ())
            .map_err(|_| CollectionError::Unavailable)
    })
    .await
    .map_err(|_| CollectionError::Timeout)?
}

fn write_request(
    username: &str,
    set: &str,
    uris: &[String],
    saved: bool,
    now: i32,
) -> WriteRequest {
    WriteRequest {
        username: username.to_string(),
        set: set.to_string(),
        items: uris
            .iter()
            .map(|uri| CollectionItem {
                uri: uri.clone(),
                added_at: if saved { now } else { 0 },
                is_removed: !saved,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn now_secs() -> i32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i32::try_from(d.as_secs()).ok())
        .unwrap_or_default()
}

/// Canonical `spotify:{track|album|artist}:<22 base62>`.
pub fn savable(uri: &str) -> bool {
    let mut parts = uri.split(':');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some("spotify"), Some("track" | "album" | "artist"), Some(id), None)
            if id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
    )
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn collection_paginates_beyond_two_hundred_and_keeps_dates() {
        let rows = super::read_pages(|token| async move {
            let start = if token.is_empty() { 0 } else { 300 };
            let page = PageResponse {
                items: (start..start + 300)
                    .map(|n| item(&format!("spotify:artist:{n:022}"), n + 1, false))
                    .collect(),
                next_page_token: if start == 0 {
                    "next".into()
                } else {
                    String::new()
                },
                ..Default::default()
            };
            Ok(Bytes::from(page.write_to_bytes().unwrap()))
        })
        .await
        .unwrap();
        assert_eq!(rows.len(), 600);
        assert_eq!(rows[0].added_at, Some(600_000));
        assert!(
            super::read_pages(|_| async {
                Ok(Bytes::from(
                    PageResponse {
                        next_page_token: "cycle".into(),
                        ..Default::default()
                    }
                    .write_to_bytes()
                    .unwrap(),
                ))
            })
            .await
            .is_err()
        );
    }

    use super::*;

    fn item(uri: &str, added_at: i32, removed: bool) -> CollectionItem {
        CollectionItem {
            uri: uri.into(),
            added_at,
            is_removed: removed,
            ..Default::default()
        }
    }

    #[test]
    fn pages_drop_removed_rows_and_keep_the_next_token() {
        let page = PageResponse {
            items: vec![
                item("spotify:track:4uLU6hMCjMI75M1A2tKUQC", 10, false),
                item("spotify:track:0000000000000000000000", 11, true),
                item("spotify:album:4uLU6hMCjMI75M1A2tKUQC", 0, false),
            ],
            next_page_token: "next".into(),
            ..Default::default()
        };
        let bytes = Bytes::from(page.write_to_bytes().unwrap());
        let (rows, token) = parse_page(&bytes).unwrap();
        assert_eq!(token, "next");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].added_at, Some(10_000));
        assert_eq!(rows[1].added_at, None);
        assert!(parse_page(&Bytes::from_static(b"\xff\xff")).is_err());
    }

    #[test]
    fn rows_come_newest_first() {
        let rows = sorted(vec![
            SavedItem {
                uri: "a".into(),
                added_at: Some(1),
            },
            SavedItem {
                uri: "b".into(),
                added_at: None,
            },
            SavedItem {
                uri: "c".into(),
                added_at: Some(3),
            },
        ]);
        let uris: Vec<_> = rows.iter().map(|r| r.uri.as_str()).collect();
        assert_eq!(uris, ["c", "a", "b"]);
    }

    #[test]
    fn writes_mark_removals_and_only_accept_canonical_uris() {
        let uris = vec!["spotify:track:4uLU6hMCjMI75M1A2tKUQC".to_string()];
        let add = write_request("me", LIKED_SET, &uris, true, 99);
        assert_eq!(add.items[0].added_at, 99);
        assert!(!add.items[0].is_removed);
        let remove = write_request("me", LIKED_SET, &uris, false, 99);
        assert!(remove.items[0].is_removed);
        assert!(savable("spotify:artist:4uLU6hMCjMI75M1A2tKUQC"));
        for bad in [
            "spotify:playlist:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:track:short",
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC:x",
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
        ] {
            assert!(!savable(bad), "{bad}");
        }
    }
}
