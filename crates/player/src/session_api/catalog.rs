//! Catalog reads through the session's pathfinder queries. Errors let SessionApi try the
//! Web API. Hashes and variable shapes come from fixtures/queries.json.

use librespot_core::Session;
use serde_json::{Value, json};
use std::future::Future;
use velo_domain::{
    Album, Artist, Page, Playlist, PublicProfile, Result, SearchResults, Track, VeloError,
};

#[path = "parse.rs"]
mod parse;

const SEARCH: (&str, &str) = (
    "searchDesktop",
    "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03",
);
const ALBUM: (&str, &str) = (
    "getAlbum",
    "11c9cad9cb24c4f9e25ef9456f68621cfd6006a90f679e729c1d2f2fd70dfe16",
);
const ARTIST: (&str, &str) = (
    "queryArtistOverview",
    "9f8134ef565e78621f1e1793555bd6633c5ac144ae0f89604ed3ae3f80b3c8e6",
);
const DISCOGRAPHY: (&str, &str) = (
    "queryArtistDiscographyAll",
    "5e07d323febb57b4a56a42abbf781490e58764aa45feb6e3dc0591564fc56599",
);

/// "Not read through the session yet": the provider tries the Web API.
pub fn not_migrated() -> VeloError {
    VeloError::Other("not read through the session yet".into())
}

fn uri(id: &str, kind: &str) -> Result<String> {
    let prefix = format!("spotify:{kind}:");
    let id = id.strip_prefix(&prefix).unwrap_or(id);
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(VeloError::NotFound);
    }
    Ok(format!("{prefix}{id}"))
}

fn search_variables(query: &str, offset: u32, limit: u32) -> Value {
    json!({"searchTerm": query, "offset": offset, "limit": limit,
        "numberOfTopResults": 5, "includeAudiobooks": false,
        "includeArtistHasConcertsField": false, "includePreReleases": true,
        "includeLocalConcertsField": false, "includeAuthors": false})
}

fn album_variables(id: &str, offset: u32, limit: u32) -> Result<Value> {
    Ok(json!({"uri": uri(id, "album")?, "locale": "", "offset": offset, "limit": limit}))
}

fn artist_variables(id: &str) -> Result<Value> {
    Ok(json!({"uri": uri(id, "artist")?, "locale": "", "includePrerelease": true}))
}

fn discography_variables(id: &str, offset: u32, limit: u32) -> Result<Value> {
    Ok(json!({"uri": uri(id, "artist")?, "offset": offset, "limit": limit, "order": "DATE_DESC"}))
}

fn window<T>(mut page: Page<T>, offset: u32, limit: u32) -> Page<T> {
    page.offset = offset;
    page.limit = limit;
    page.items.truncate(limit as usize);
    page
}

pub async fn search(
    session: &Session,
    query: &str,
    offset: u32,
    limit: u32,
) -> Result<SearchResults> {
    let limit = limit.clamp(1, 10);
    if offset.saturating_add(limit) > 1000 {
        return Ok(SearchResults {
            tracks: window(Page::default(), offset, limit),
            playlists: window(Page::default(), offset, limit),
            albums: window(Page::default(), offset, limit),
            artists: window(Page::default(), offset, limit),
        });
    }
    let data = super::pathfinder::query(
        session,
        SEARCH.0,
        &search_variables(query, offset, limit),
        SEARCH.1,
    )
    .await?;
    let results = parse::parse_search(&data)?;
    Ok(SearchResults {
        tracks: window(results.tracks, offset, limit),
        playlists: window(results.playlists, offset, limit),
        albums: window(results.albums, offset, limit),
        artists: window(results.artists, offset, limit),
    })
}

pub async fn album(session: &Session, id: &str) -> Result<Album> {
    let data = tokio::time::timeout(
        super::TIMEOUT,
        super::pathfinder::query(session, ALBUM.0, &album_variables(id, 0, 50)?, ALBUM.1),
    )
    .await
    .map_err(|_| VeloError::Other(ALBUM_TIMEOUT.into()))?
    .map_err(|error| match error {
        VeloError::Network(message) if message == "pathfinder timed out" => {
            VeloError::Other(ALBUM_TIMEOUT.into())
        }
        error => error,
    })?;
    // A missing or unavailable album is a row failure, not a transport failure.
    parse::parse_album(&data).map_err(|_| VeloError::NotFound)
}

const ALBUM_TIMEOUT: &str = "album metadata timed out";

pub async fn album_tracks(
    session: &Session,
    id: &str,
    offset: u32,
    limit: u32,
) -> Result<Page<Track>> {
    let limit = limit.clamp(1, 50);
    let data = super::pathfinder::query(
        session,
        ALBUM.0,
        &album_variables(id, offset, limit)?,
        ALBUM.1,
    )
    .await?;
    Ok(window(parse::parse_album_tracks(&data)?, offset, limit))
}

pub async fn artist(session: &Session, id: &str) -> Result<Artist> {
    Ok(artist_overview(session, id).await?.artist)
}

/// Public metadata from the same query used for the artist's main header.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtistOverview {
    pub artist: Artist,
    pub monthly_listeners: i32,
    pub followers: i32,
    pub verified: bool,
    pub top_cities: Vec<Value>,
    pub biography: String,
    /// Remote source; the app caches/resizes it before exposing it to QML.
    pub header_image_url: String,
}

pub use parse::{artist_overview_value, biography_plain_text};

/// Coalesce the main header and F6 reads. The small, expiring cache holds public
/// parsed metadata only; per-key locks do not serialize unrelated artists.
pub async fn artist_overview(session: &Session, id: &str) -> Result<ArtistOverview> {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, OnceLock},
    };
    type Entry = Arc<tokio::sync::Mutex<Option<(tokio::time::Instant, ArtistOverview)>>>;
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    let variables = artist_variables(id)?;
    let key = format!(
        "{}:{}:{}:{}",
        session.device_id(),
        session.session_id(),
        session.username(),
        variables["uri"]
    );
    let entry = {
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 32 && !cache.contains_key(&key) {
            cache.retain(|_, entry| Arc::strong_count(entry) > 1);
        }
        cache.entry(key).or_default().clone()
    };
    cached_artist_overview(&entry, async {
        let data = super::pathfinder::query(session, ARTIST.0, &variables, ARTIST.1).await?;
        artist_overview_value(&data)
    })
    .await
}

async fn cached_artist_overview(
    entry: &tokio::sync::Mutex<Option<(tokio::time::Instant, ArtistOverview)>>,
    read: impl Future<Output = Result<ArtistOverview>>,
) -> Result<ArtistOverview> {
    use std::time::Duration;
    let mut entry = entry.lock().await;
    if let Some((_, value)) = entry
        .as_ref()
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
    {
        return Ok(value.clone());
    }
    let value = read.await?;
    *entry = Some((tokio::time::Instant::now(), value.clone()));
    Ok(value)
}

pub async fn artist_albums(
    session: &Session,
    id: &str,
    offset: u32,
    limit: u32,
) -> Result<Page<Album>> {
    let limit = limit.clamp(1, 50);
    let data = super::pathfinder::query(
        session,
        DISCOGRAPHY.0,
        &discography_variables(id, offset, limit)?,
        DISCOGRAPHY.1,
    )
    .await?;
    Ok(window(parse::parse_artist_albums(&data)?, offset, limit))
}

async fn optional<T>(future: Option<impl Future<Output = Result<T>>>) -> Result<Option<T>> {
    match future {
        Some(future) => future.await.map(Some),
        None => Ok(None),
    }
}

/// Four borrowed futures at a time. Output keeps input order, including duplicates;
/// any failure rejects the batch instead of hiding missing rows. No detached tasks.
async fn ordered<T>(
    futures: impl IntoIterator<Item = impl Future<Output = Result<T>>>,
) -> Result<Vec<T>> {
    let mut futures = futures.into_iter();
    let mut items = Vec::new();
    loop {
        let Some(first) = futures.next() else { break };
        let results = tokio::join!(
            optional(Some(first)),
            optional(futures.next()),
            optional(futures.next()),
            optional(futures.next())
        );
        for result in [results.0, results.1, results.2, results.3] {
            if let Some(item) = result? {
                items.push(item);
            }
        }
    }
    Ok(items)
}

/// Albums of `uris` (`spotify:album:…`), in that order (saved albums of the library).
pub async fn albums(session: &Session, uris: &[String]) -> Result<Vec<Album>> {
    surviving_albums(uris.iter().map(|uri| album(session, uri))).await
}

async fn surviving_albums<T>(
    futures: impl IntoIterator<Item = impl Future<Output = Result<T>>>,
) -> Result<Vec<T>> {
    let futures: Vec<_> = futures
        .into_iter()
        .map(|future| async {
            match future.await {
                Ok(album) => Ok(Ok(album)),
                Err(e @ VeloError::NotFound) => Ok(Err(e)),
                Err(VeloError::Other(message)) if message == ALBUM_TIMEOUT => {
                    Ok(Err(VeloError::Other(message)))
                }
                Err(e) => Err(e),
            }
        })
        .collect();
    let mut items = Vec::new();
    let mut last_error = None;
    for result in ordered(futures).await? {
        match result {
            Ok(item) => items.push(item),
            Err(error) => last_error = Some(error),
        }
    }
    if items.is_empty() {
        if let Some(error) = last_error {
            return Err(error);
        }
    }
    Ok(items)
}

pub fn validate_user_id(user_id: &str) -> Result<()> {
    if user_id.is_empty() || user_id.len() > 256 || user_id.chars().any(char::is_control) {
        return Err(VeloError::NotFound);
    }
    Ok(())
}

/// Encode the username as exactly one path segment, including legacy Unicode IDs.
pub fn encoded_user_id(user_id: &str) -> Result<String> {
    validate_user_id(user_id)?;
    let mut encoded = String::new();
    use std::fmt::Write;
    for byte in user_id.bytes() {
        if byte.is_ascii_alphanumeric() || b"._-~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("write to String");
        }
    }
    // Dot segments must also stay inside the profile path.
    if matches!(encoded.as_str(), "." | "..") {
        encoded = encoded.replace('.', "%2E");
    }
    Ok(encoded)
}

pub async fn user_profile(session: &Session, user_id: &str) -> Result<PublicProfile> {
    let encoded = encoded_user_id(user_id)?;
    let body = tokio::time::timeout(
        super::TIMEOUT,
        session
            .spclient()
            .get_user_profile(&encoded, Some(0), Some(0)),
    )
    .await
    .map_err(|_| super::unavailable())?
    .map_err(|_| super::unavailable())?;
    let value = serde_json::from_slice(&body).map_err(|_| super::unavailable())?;
    parse::parse_public_profile(&value, user_id)
}

fn playlists_path(user_id: &str, offset: u32, limit: u32) -> Result<String> {
    let user_id = encoded_user_id(user_id)?;
    Ok(format!(
        "/user-profile-view/v3/profile/{user_id}/playlists?offset={offset}&limit={limit}"
    ))
}

/// Spclient's public-profile playlists are already public; retain only this owner's rows.
pub async fn user_public_playlists(
    session: &Session,
    user_id: &str,
    offset: u32,
    limit: u32,
) -> Result<Page<Playlist>> {
    let username = if user_id == "me" {
        session.username()
    } else {
        user_id.into()
    };
    let limit = limit.clamp(1, 50);
    let path = playlists_path(&username, offset, limit)?;
    let result = tokio::time::timeout(
        super::TIMEOUT,
        session
            .spclient()
            .request_as_json(&http::Method::GET, &path, None, None),
    )
    .await;
    if let Ok(Ok(body)) = result {
        if let Ok(value) = serde_json::from_slice(&body) {
            if let Ok(page) = parse::parse_public_playlists(&value, &username, offset, limit) {
                return Ok(page);
            }
        }
    }
    // Some clients expose playlists only on the profile itself. Fetch its prefix and
    // apply the offset locally exactly once. Bound this fallback, never a rootlist
    // (which includes private/followed playlists and does not prove public ownership).
    let prefix = offset
        .checked_add(limit)
        .filter(|n| *n <= 1000)
        .ok_or_else(super::unavailable)?;
    let body = tokio::time::timeout(
        super::TIMEOUT,
        session
            .spclient()
            .get_user_profile(&encoded_user_id(&username)?, Some(prefix), Some(0)),
    )
    .await
    .map_err(|_| super::unavailable())?
    .map_err(|_| super::unavailable())?;
    let value = serde_json::from_slice(&body).map_err(|_| super::unavailable())?;
    parse::parse_public_playlists_prefix(value, &username, offset, limit)
}

/// Artists of `uris` (`spotify:artist:…`), in that order (followed artists).
pub async fn artists(session: &Session, uris: &[String]) -> Result<Vec<Artist>> {
    let futures: Vec<_> = uris.iter().map(|uri| artist(session, uri)).collect();
    ordered(futures).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn overview_coalesces_header_and_f6_and_refreshes_after_expiry_or_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/artist.json"))
                .unwrap();
        let overview = artist_overview_value(&v["data"]).unwrap();
        let entry = tokio::sync::Mutex::new(None);
        let reads = AtomicUsize::new(0);
        let read = || async {
            reads.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(overview.clone())
        };
        let (header, f6) = tokio::join!(
            cached_artist_overview(&entry, read()),
            cached_artist_overview(&entry, read())
        );
        assert_eq!(header.unwrap(), f6.unwrap());
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(
            cached_artist_overview(&entry, std::future::ready(Err(super::super::unavailable())))
                .await
                .is_err()
        );
        assert_eq!(
            cached_artist_overview(&entry, read()).await.unwrap(),
            overview
        );
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn public_profile_paths_validate_without_requesting() {
        assert_eq!(
            playlists_path("fixture.user-_", 50, 10).unwrap(),
            "/user-profile-view/v3/profile/fixture.user-_/playlists?offset=50&limit=10"
        );
        for id in ["", "a\n", "\0"] {
            assert!(validate_user_id(id).is_err());
        }
        assert!(validate_user_id(&"x".repeat(257)).is_err());
        assert_eq!(
            encoded_user_id("Señor + /?#%用户").unwrap(),
            "Se%C3%B1or%20%2B%20%2F%3F%23%25%E7%94%A8%E6%88%B7"
        );
        assert_eq!(encoded_user_id("..").unwrap(), "%2E%2E");
        assert_eq!(
            playlists_path("a/b?limit=9", 0, 2).unwrap(),
            "/user-profile-view/v3/profile/a%2Fb%3Flimit%3D9/playlists?offset=0&limit=2"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn album_errors_and_timeouts_skip_only_failed_rows_in_order() {
        let rows = surviving_albums((0..7).map(|i| async move {
            if i == 1 {
                return Err(VeloError::NotFound);
            }
            tokio::time::timeout(std::time::Duration::from_millis(5), async {
                if i == 4 {
                    std::future::pending::<()>().await;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                Ok(i)
            })
            .await
            .map_err(|_| VeloError::Other(ALBUM_TIMEOUT.into()))?
        }))
        .await
        .unwrap();
        assert_eq!(rows, [0, 2, 3, 5, 6]);
        assert!(
            surviving_albums([std::future::ready(Err::<u32, _>(VeloError::NotFound))])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn album_batches_propagate_session_errors_even_after_a_success() {
        for error in [
            VeloError::NotAuthenticated,
            VeloError::RateLimited(std::time::Duration::from_secs(1)),
            VeloError::Network("fixture transport failure".into()),
        ] {
            assert!(
                surviving_albums([std::future::ready(Ok(1)), std::future::ready(Err(error))])
                    .await
                    .is_err()
            );
        }
        assert!(
            surviving_albums(std::iter::empty::<std::future::Ready<Result<u32>>>())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn query_contracts_match_captured_variables_and_hashes() {
        let q: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/queries.json"))
                .unwrap();
        for (file, operation, vars) in [
            ("search.json", SEARCH, search_variables("queen", 0, 10)),
            (
                "album.json",
                ALBUM,
                album_variables("6i6folBtxKV28WX3msQ4FE", 0, 50).unwrap(),
            ),
            (
                "artist.json",
                ARTIST,
                artist_variables("1dfeR4HaWDbWqFHLkxsg1d").unwrap(),
            ),
            (
                "artist_discography.json",
                DISCOGRAPHY,
                discography_variables("1dfeR4HaWDbWqFHLkxsg1d", 0, 20).unwrap(),
            ),
        ] {
            assert_eq!(q[file]["operationName"], operation.0);
            assert_eq!(q[file]["sha256Hash"], operation.1);
            assert_eq!(q[file]["variables"], vars);
        }
        assert_eq!(
            album_variables("spotify:album:abc", 30, 7).unwrap(),
            json!({"uri":"spotify:album:abc","locale":"","offset":30,"limit":7})
        );
        assert!(artist_variables("spotify:album:abc").is_err());
        assert!(album_variables("", 0, 10).is_err());
    }

    #[test]
    fn request_window_preserves_total_and_does_not_skip_a_page_twice() {
        let p = window(
            Page {
                items: vec!["first", "second", "third"],
                offset: 0,
                limit: 3,
                total: 88,
            },
            20,
            2,
        );
        assert_eq!(p.items, vec!["first", "second"]);
        assert_eq!((p.offset, p.limit, p.total), (20, 2, 88));
        assert!(p.has_more());
    }

    #[tokio::test]
    async fn album_batches_parse_fixtures_and_preserve_order_with_duplicates() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/album.json"))
                .unwrap();
        let mut second = v["data"].clone();
        second["albumUnion"]["uri"] = json!("spotify:album:second");
        let rows = [v["data"].clone(), second, v["data"].clone()];
        let result = ordered(rows.iter().map(|v| async { parse::parse_album(v) }))
            .await
            .unwrap();
        assert_eq!(
            result.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["6i6folBtxKV28WX3msQ4FE", "second", "6i6folBtxKV28WX3msQ4FE"]
        );
        assert_eq!(result[0].total_tracks, 22);
        assert_eq!(result[0].images[0].width, Some(640));
    }

    #[tokio::test]
    async fn artist_batches_parse_fixtures_and_preserve_order() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/artist.json"))
                .unwrap();
        let mut second = v["data"].clone();
        second["artistUnion"]["uri"] = json!("spotify:artist:second");
        let rows = [v["data"].clone(), second];
        let result = ordered(rows.iter().map(|v| async { parse::parse_artist(v) }))
            .await
            .unwrap();
        assert_eq!(
            result.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["1dfeR4HaWDbWqFHLkxsg1d", "second"]
        );
        assert_eq!(result[0].name, "Queen");
        assert_eq!(result[0].followers, Some(58691539));
    }

    #[tokio::test(start_paused = true)]
    async fn batches_bound_concurrency_preserve_completion_order_and_reject_failures() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let result = ordered((0..9).map(|i| {
            let active = &active;
            let peak = &peak;
            async move {
                let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10 - i)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(i)
            }
        }))
        .await
        .unwrap();
        assert_eq!(result, (0..9).collect::<Vec<_>>());
        assert_eq!(peak.load(Ordering::SeqCst), 4);
        assert!(
            ordered([
                std::future::ready(Ok(1)),
                std::future::ready(Err(not_migrated()))
            ])
            .await
            .is_err()
        );
        assert!(
            ordered(std::iter::empty::<std::future::Ready<Result<u32>>>())
                .await
                .unwrap()
                .is_empty()
        );
    }
}
