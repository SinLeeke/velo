//! L1: Velo's data provider on top of the librespot session (one login for audio and data).
//!
//! The Web API answers 429 to every app that is not registered by the user (Spotify's quota is
//! per Client ID and the public ones are exhausted), so the library comes from the same internal
//! services the official clients use, through the session: `user-profile-view` (profile),
//! `playlist/v2` (playlists of the sidebar and their songs), `collection/v2` (Me gusta, followed
//! artists) and `metadata/4` (songs). Metadata only, never audio.
//!
//! What is not migrated yet goes to the Web API with the session's login5 token
//! ([`Login5Tokens`]); when that one is rate limited the caller gets `RateLimited` and hides
//! the section. Every internal read falls back to the Web API on failure.
//!
//! - [`catalog`]: search, albums, artists, public playlists of a user.
//! - [`home`]: recently played and tops (Inicio).
//! - [`pathfinder`]: the GraphQL service of Spotify's web player.

pub mod catalog;
pub mod home;
pub mod pathfinder;
pub mod shows;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
pub use librespot_core::Session;
use librespot_protocol::playlist4_external::{MetaItem, SelectedListContent};
use protobuf::Message;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::Instant;
use velo_domain::*;
use velo_spotify::{SpotifyConfig, SpotifyProvider, TokenSource};

use crate::collection::{self, ARTIST_SET, LIKED_SET, SavedItem};
use crate::playlist::{self, PlaylistItem};

const TIMEOUT: Duration = Duration::from_secs(15);
/// Saved sets are re-read after this long (or right after Velo changes them).
const SET_TTL: Duration = Duration::from_secs(120);
const IMAGE_HOST: &str = "https://i.scdn.co/image/";

/// Where the signed-in librespot session comes from (the audio engine in the app).
#[async_trait]
pub trait SessionSource: Send + Sync {
    /// The session, waiting a little while it (re)connects; `None` when signed out.
    async fn session(&self) -> Option<Session>;
}

/// Web API tokens from the session's login5 (librespot's own client): no second login.
pub struct Login5Tokens(pub Arc<dyn SessionSource>);

#[async_trait]
impl TokenSource for Login5Tokens {
    async fn token(&self, _rejected: Option<&str>) -> Result<(String, Duration)> {
        // No session yet (connecting) is transient: never "signed out".
        let session = self.0.session().await.ok_or_else(unavailable)?;
        let token = tokio::time::timeout(TIMEOUT, session.login5().auth_token())
            .await
            .map_err(|_| VeloError::Network("login5 timed out".into()))?
            .map_err(|_| VeloError::Network("login5 failed".into()))?;
        let left = (token.timestamp + token.expires_in)
            .duration_since(SystemTime::now())
            .unwrap_or_default();
        Ok((token.access_token, left))
    }
}

pub(crate) fn unavailable() -> VeloError {
    VeloError::Network("Spotify did not answer".into())
}

struct SavedSet {
    read_at: Instant,
    rows: Arc<Vec<SavedItem>>,
}

#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub kind: String,
    pub object_id: String,
    pub uri: String,
    pub name: String,
    pub creator: String,
    pub images: Vec<ImageRef>,
    pub added_at: Option<i64>,
}

pub struct SessionApi {
    source: Arc<dyn SessionSource>,
    web: Arc<SpotifyProvider>,
    sets: Mutex<HashMap<&'static str, SavedSet>>,
    profile: Mutex<Option<UserProfile>>,
}

impl SessionApi {
    pub fn new(source: Arc<dyn SessionSource>) -> Self {
        Self::with_web_config(source, SpotifyConfig::default())
    }

    /// `web` points the Web API fallback elsewhere (tests).
    pub fn with_web_config(source: Arc<dyn SessionSource>, web: SpotifyConfig) -> Self {
        let tokens = Arc::new(Login5Tokens(source.clone()));
        Self::with_web(
            source,
            Arc::new(SpotifyProvider::with_token_source(web, tokens)),
        )
    }

    /// A provider with its own Web API fallback.
    pub fn with_web(source: Arc<dyn SessionSource>, web: Arc<SpotifyProvider>) -> Self {
        Self {
            web,
            source,
            sets: Mutex::new(HashMap::new()),
            profile: Mutex::new(None),
        }
    }

    /// Full library metadata with collection timestamps, sharing the cached collection read.
    pub async fn sidebar_collection(&self) -> Result<Vec<LibraryEntry>> {
        let session = self.session().await?;
        let albums = self.saved(LIKED_SET).await?;
        let artists = self.saved(ARTIST_SET).await?;
        let album_uris: Vec<_> = albums
            .iter()
            .filter(|r| r.uri.starts_with("spotify:album:"))
            .map(|r| r.uri.clone())
            .collect();
        let artist_uris: Vec<_> = artists
            .iter()
            .filter(|r| r.uri.starts_with("spotify:artist:"))
            .map(|r| r.uri.clone())
            .collect();
        let (album_meta, artist_meta) = tokio::try_join!(
            catalog::albums(&session, &album_uris),
            catalog::artists(&session, &artist_uris)
        )?;
        let dates: HashMap<_, _> = albums
            .iter()
            .chain(artists.iter())
            .map(|r| (r.uri.as_str(), r.added_at))
            .collect();
        Ok(album_meta
            .into_iter()
            .map(|a| LibraryEntry {
                added_at: dates.get(a.uri.as_str()).copied().flatten(),
                kind: "album".into(),
                object_id: a.id,
                uri: a.uri,
                name: a.name,
                creator: a
                    .artists
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                images: a.images,
            })
            .chain(artist_meta.into_iter().map(|a| LibraryEntry {
                added_at: dates.get(a.uri.as_str()).copied().flatten(),
                kind: "artist".into(),
                object_id: a.id,
                uri: a.uri,
                name: a.name,
                creator: String::new(),
                images: a.images,
            }))
            .collect())
    }

    /// The Web API fallback (login5 token), for calls outside [`MusicProvider`].
    pub fn web(&self) -> &Arc<SpotifyProvider> {
        &self.web
    }

    async fn session(&self) -> Result<Session> {
        self.source.session().await.ok_or_else(unavailable)
    }

    /// The internal read, or the Web API when it fails.
    async fn or_web<T>(
        &self,
        what: &'static str,
        internal: impl std::future::Future<Output = Result<T>>,
        web: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        match internal.await {
            Ok(value) => Ok(value),
            Err(e) => {
                tracing::debug!(what, error = %e, "session read failed; trying the Web API");
                web.await
            }
        }
    }

    async fn saved(&self, set: &'static str) -> Result<Arc<Vec<SavedItem>>> {
        let mut sets = self.sets.lock().await;
        if let Some(cached) = sets.get(set) {
            if cached.read_at.elapsed() < SET_TTL {
                return Ok(cached.rows.clone());
            }
        }
        let session = self.session().await?;
        let rows = Arc::new(
            collection::read_set(&session, set)
                .await
                .map_err(|_| unavailable())?,
        );
        sets.insert(
            set,
            SavedSet {
                read_at: Instant::now(),
                rows: rows.clone(),
            },
        );
        Ok(rows)
    }

    async fn forget_set(&self, set: &'static str) {
        self.sets.lock().await.remove(set);
    }

    async fn own_profile(&self) -> Result<UserProfile> {
        if let Some(profile) = self.profile.lock().await.clone() {
            return Ok(profile);
        }
        let session = self.session().await?;
        let username = session.username();
        let body = tokio::time::timeout(
            TIMEOUT,
            session.spclient().get_user_profile(&username, None, None),
        )
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        let json: Value = serde_json::from_slice(&body).map_err(|_| unavailable())?;
        let product = session
            .get_user_attribute("type")
            .unwrap_or_else(|| "premium".into());
        let profile = profile_from(&username, &product, &json);
        *self.profile.lock().await = Some(profile.clone());
        Ok(profile)
    }

    async fn rootlist(&self, offset: u32, limit: u32) -> Result<Page<Playlist>> {
        let session = self.session().await?;
        let me = self.own_profile().await.ok();
        let body = tokio::time::timeout(
            TIMEOUT,
            session
                .spclient()
                .get_rootlist(offset as usize, Some(limit.max(1) as usize)),
        )
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        let list = SelectedListContent::parse_from_bytes(&body).map_err(|_| unavailable())?;
        Ok(rootlist_page(&list, offset, limit, me.as_ref()))
    }

    async fn playlist_internal(&self, id: &str) -> Result<Playlist> {
        let session = self.session().await?;
        let sid = librespot_core::SpotifyId::from_base62(id).map_err(|_| VeloError::NotFound)?;
        let body = tokio::time::timeout(TIMEOUT, session.spclient().get_playlist(&sid))
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        let list = SelectedListContent::parse_from_bytes(&body).map_err(|_| unavailable())?;
        let me = self.own_profile().await.ok();
        Ok(playlist_from(id, &list, me.as_ref()))
    }

    async fn playlist_tracks_internal(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<Track>> {
        let session = self.session().await?;
        let items = playlist::playlist_items_in(&session, &format!("spotify:playlist:{id}"))
            .await
            .map_err(|_| unavailable())?;
        Ok(track_page(&session, &items, offset, limit).await)
    }

    async fn saved_tracks_internal(&self, offset: u32, limit: u32) -> Result<Page<Track>> {
        let rows = self.saved(LIKED_SET).await?;
        let items: Vec<PlaylistItem> = rows
            .iter()
            .filter(|r| r.uri.starts_with("spotify:track:"))
            .map(|r| PlaylistItem {
                uri: r.uri.clone(),
                added_at: r.added_at,
            })
            .collect();
        let session = self.session().await?;
        Ok(track_page(&session, &items, offset, limit).await)
    }

    async fn saved_of_kind(
        &self,
        set: &'static str,
        kind: &str,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<String>, u32)> {
        let rows = self.saved(set).await?;
        let prefix = format!("spotify:{kind}:");
        let uris: Vec<String> = rows
            .iter()
            .filter(|r| r.uri.starts_with(&prefix))
            .map(|r| r.uri.clone())
            .collect();
        let total = u32::try_from(uris.len()).unwrap_or(u32::MAX);
        let page = uris
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok((page, total))
    }

    async fn saved_albums_internal(&self, offset: u32, limit: u32) -> Result<Page<Album>> {
        let (uris, total) = self
            .saved_of_kind(LIKED_SET, "album", offset, limit)
            .await?;
        let session = self.session().await?;
        Ok(Page {
            items: catalog::albums(&session, &uris).await?,
            offset,
            limit,
            total,
        })
    }

    async fn followed_artists_internal(&self, limit: u32) -> Result<Vec<Artist>> {
        let (uris, _) = self
            .saved_of_kind(
                ARTIST_SET,
                "artist",
                0,
                limit.max(collection::MAX_ITEMS as u32),
            )
            .await?;
        let session = self.session().await?;
        catalog::artists(&session, &uris).await
    }

    async fn is_saved_internal(&self, uri: &str) -> Result<bool> {
        let set = set_of(uri).ok_or(VeloError::NotFound)?;
        Ok(self.saved(set).await?.iter().any(|r| r.uri == uri))
    }

    async fn set_saved_internal(&self, uri: &str, saved: bool) -> Result<()> {
        let set = set_of(uri).ok_or(VeloError::NotFound)?;
        let session = self.session().await?;
        let result = collection::write_set(&session, set, &[uri.to_string()], saved)
            .await
            .map_err(|_| unavailable());
        self.forget_set(set).await;
        result
    }

    async fn user_profile_internal(&self, user_id: &str) -> Result<PublicProfile> {
        catalog::validate_user_id(user_id)?;
        let session = self.session().await?;
        let username = if user_id == "me" {
            session.username()
        } else {
            user_id.to_string()
        };
        catalog::user_profile(&session, &username).await
    }
}

/// Which saved set holds `uri`.
fn set_of(uri: &str) -> Option<&'static str> {
    if !collection::savable(uri) {
        return None;
    }
    Some(if uri.starts_with("spotify:artist:") {
        ARTIST_SET
    } else {
        LIKED_SET
    })
}

/// The domain profile from `user-profile-view/v3` JSON.
pub fn profile_from(username: &str, product: &str, json: &Value) -> UserProfile {
    let name = json["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(username);
    UserProfile {
        id: username.to_string(),
        display_name: name.to_string(),
        product: product.to_string(),
        images: json["image_url"]
            .as_str()
            .filter(|s| s.starts_with("https://"))
            .map(|url| {
                vec![ImageRef {
                    url: url.to_string(),
                    width: None,
                    height: None,
                }]
            })
            .unwrap_or_default(),
    }
}

fn owner_name(owner_id: &str, me: Option<&UserProfile>) -> String {
    match me {
        Some(me) if me.id == owner_id => me.display_name.clone(),
        _ => owner_id.to_string(),
    }
}

/// Cover of a playlist: its own picture (an image file id) or the generated mosaic.
fn playlist_images<'a>(
    picture: &[u8],
    sizes: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<ImageRef> {
    if picture.len() == 20 {
        let hex: String = picture.iter().map(|b| format!("{b:02x}")).collect();
        return vec![ImageRef {
            url: format!("{IMAGE_HOST}{hex}"),
            width: Some(300),
            height: Some(300),
        }];
    }
    let sizes: Vec<_> = sizes
        .filter(|(_, url)| url.starts_with("https://"))
        .collect();
    let pick = sizes
        .iter()
        .find(|(name, _)| *name == "default")
        .or_else(|| sizes.first());
    pick.map(|(_, url)| {
        vec![ImageRef {
            url: url.to_string(),
            width: None,
            height: None,
        }]
    })
    .unwrap_or_default()
}

/// A playlist's header from `playlist/v2/playlist/<id>`.
pub fn playlist_from(id: &str, list: &SelectedListContent, me: Option<&UserProfile>) -> Playlist {
    let a = &list.attributes;
    let owner_id = list.owner_username().to_string();
    Playlist {
        id: id.to_string(),
        uri: format!("spotify:playlist:{id}"),
        name: a.name().to_string(),
        description: a.description().to_string(),
        owner: owner_name(&owner_id, me),
        owner_id,
        collaborative: a.collaborative(),
        images: playlist_images(
            a.picture(),
            a.picture_size.iter().map(|p| (p.target_name(), p.url())),
        ),
        track_count: u32::try_from(list.length()).unwrap_or_default(),
    }
}

/// One page of the sidebar from `playlist/v2/user/<me>/rootlist`; folders are skipped.
pub fn rootlist_page(
    list: &SelectedListContent,
    offset: u32,
    limit: u32,
    me: Option<&UserProfile>,
) -> Page<Playlist> {
    let contents = &list.contents;
    let empty = MetaItem::new();
    let items = contents
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let id = item.uri().strip_prefix("spotify:playlist:")?;
            let meta = contents.meta_items.get(i).unwrap_or(&empty);
            let a = &meta.attributes;
            let owner_id = meta.owner_username().to_string();
            Some(Playlist {
                id: id.to_string(),
                uri: item.uri().to_string(),
                name: a.name().to_string(),
                description: a.description().to_string(),
                owner: owner_name(&owner_id, me),
                owner_id,
                collaborative: a.collaborative(),
                images: playlist_images(
                    a.picture(),
                    a.picture_size.iter().map(|p| (p.target_name(), p.url())),
                ),
                track_count: u32::try_from(meta.length()).unwrap_or_default(),
            })
        })
        .collect();
    Page {
        items,
        offset,
        limit,
        total: u32::try_from(list.length()).unwrap_or_default(),
    }
}

async fn track_page(
    session: &Session,
    items: &[PlaylistItem],
    offset: u32,
    limit: u32,
) -> Page<Track> {
    let start = (offset as usize).min(items.len());
    let end = start.saturating_add(limit as usize).min(items.len());
    Page {
        items: playlist::tracks_in(session, &items[start..end]).await,
        offset,
        limit,
        total: u32::try_from(items.len()).unwrap_or(u32::MAX),
    }
}

#[async_trait]
impl MusicProvider for SessionApi {
    fn name(&self) -> &'static str {
        "Spotify"
    }

    async fn is_signed_in(&self) -> bool {
        true
    }

    async fn sign_in(&self) -> Result<UserProfile> {
        Err(VeloError::Other(
            "Session sign-in happens in the audio login".into(),
        ))
    }

    async fn sign_out(&self) -> Result<()> {
        self.sets.lock().await.clear();
        *self.profile.lock().await = None;
        Ok(())
    }

    async fn me(&self) -> Result<UserProfile> {
        self.or_web("me", self.own_profile(), self.web.me()).await
    }

    // Playback through the Web API (Velo's own device is driven by the engine directly).
    async fn playback(&self) -> Result<Option<PlaybackState>> {
        self.web.playback().await
    }
    async fn devices(&self) -> Result<Vec<Device>> {
        self.web.devices().await
    }
    async fn transfer(&self, device_id: &str, play: bool) -> Result<()> {
        self.web.transfer(device_id, play).await
    }
    async fn play(&self) -> Result<()> {
        self.web.play().await
    }
    async fn pause(&self) -> Result<()> {
        self.web.pause().await
    }
    async fn next(&self) -> Result<()> {
        self.web.next().await
    }
    async fn previous(&self) -> Result<()> {
        self.web.previous().await
    }
    async fn seek(&self, position_ms: u64) -> Result<()> {
        self.web.seek(position_ms).await
    }
    async fn set_volume(&self, percent: u8) -> Result<()> {
        self.web.set_volume(percent).await
    }
    async fn set_shuffle(&self, on: bool) -> Result<()> {
        self.web.set_shuffle(on).await
    }
    async fn set_repeat(&self, mode: RepeatMode) -> Result<()> {
        self.web.set_repeat(mode).await
    }
    async fn play_context(&self, context_uri: &str, offset: Option<u32>) -> Result<()> {
        self.web.play_context(context_uri, offset).await
    }
    async fn play_uris(&self, uris: &[String], offset: Option<u32>) -> Result<()> {
        self.web.play_uris(uris, offset).await
    }
    async fn queue(&self) -> Result<Vec<Track>> {
        self.web.queue().await
    }
    async fn add_to_queue(&self, uri: &str) -> Result<()> {
        self.web.add_to_queue(uri).await
    }

    async fn my_playlists(&self, offset: u32, limit: u32) -> Result<Page<Playlist>> {
        self.or_web(
            "rootlist",
            self.rootlist(offset, limit),
            self.web.my_playlists(offset, limit),
        )
        .await
    }
    async fn playlist(&self, id: &str) -> Result<Playlist> {
        self.or_web(
            "playlist",
            self.playlist_internal(id),
            self.web.playlist(id),
        )
        .await
    }
    async fn playlist_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>> {
        self.or_web(
            "playlist tracks",
            self.playlist_tracks_internal(id, offset, limit),
            self.web.playlist_tracks(id, offset, limit),
        )
        .await
    }
    async fn saved_tracks(&self, offset: u32, limit: u32) -> Result<Page<Track>> {
        self.or_web(
            "liked songs",
            self.saved_tracks_internal(offset, limit),
            self.web.saved_tracks(offset, limit),
        )
        .await
    }
    async fn saved_albums(&self, offset: u32, limit: u32) -> Result<Page<Album>> {
        self.or_web(
            "saved albums",
            self.saved_albums_internal(offset, limit),
            self.web.saved_albums(offset, limit),
        )
        .await
    }
    async fn followed_artists(&self, limit: u32) -> Result<Vec<Artist>> {
        self.or_web(
            "followed artists",
            self.followed_artists_internal(limit),
            self.web.followed_artists(limit),
        )
        .await
    }
    async fn is_saved(&self, uri: &str) -> Result<bool> {
        self.or_web(
            "is saved",
            self.is_saved_internal(uri),
            self.web.is_saved(uri),
        )
        .await
    }
    async fn set_saved(&self, uri: &str, saved: bool) -> Result<()> {
        self.or_web(
            "save",
            self.set_saved_internal(uri, saved),
            self.web.set_saved(uri, saved),
        )
        .await
    }
    async fn recently_played(&self, limit: u32) -> Result<Vec<Track>> {
        let internal = async { home::recently_played(&self.session().await?, limit).await };
        self.or_web("recently played", internal, self.web.recently_played(limit))
            .await
    }
    async fn top_tracks(&self, range: TimeRange, offset: u32, limit: u32) -> Result<Page<Track>> {
        let internal =
            async { home::top_tracks(&self.session().await?, range, offset, limit).await };
        self.or_web(
            "top tracks",
            internal,
            self.web.top_tracks(range, offset, limit),
        )
        .await
    }
    async fn top_artists(&self, range: TimeRange, offset: u32, limit: u32) -> Result<Page<Artist>> {
        let internal =
            async { home::top_artists(&self.session().await?, range, offset, limit).await };
        self.or_web(
            "top artists",
            internal,
            self.web.top_artists(range, offset, limit),
        )
        .await
    }

    async fn album(&self, id: &str) -> Result<Album> {
        let internal = async { catalog::album(&self.session().await?, id).await };
        self.or_web("album", internal, self.web.album(id)).await
    }
    async fn album_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>> {
        let internal =
            async { catalog::album_tracks(&self.session().await?, id, offset, limit).await };
        self.or_web(
            "album tracks",
            internal,
            self.web.album_tracks(id, offset, limit),
        )
        .await
    }
    async fn artist(&self, id: &str) -> Result<Artist> {
        let internal = async { catalog::artist(&self.session().await?, id).await };
        self.or_web("artist", internal, self.web.artist(id)).await
    }
    async fn artist_albums(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Album>> {
        let internal =
            async { catalog::artist_albums(&self.session().await?, id, offset, limit).await };
        self.or_web(
            "artist albums",
            internal,
            self.web.artist_albums(id, offset, limit),
        )
        .await
    }

    // Playlist editing stays on the Web API for now.
    async fn create_playlist(
        &self,
        name: &str,
        description: &str,
        public: bool,
    ) -> Result<Playlist> {
        self.web.create_playlist(name, description, public).await
    }
    async fn update_playlist_details(
        &self,
        playlist_id: &str,
        name: &str,
        description: &str,
    ) -> Result<()> {
        self.web
            .update_playlist_details(playlist_id, name, description)
            .await
    }
    async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.web.add_to_playlist(playlist_id, uris).await
    }
    async fn remove_from_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.web.remove_from_playlist(playlist_id, uris).await
    }

    async fn search(&self, query: &str, offset: u32, limit: u32) -> Result<SearchResults> {
        let internal =
            async { catalog::search(&self.session().await?, query, offset, limit).await };
        self.or_web("search", internal, self.web.search(query, offset, limit))
            .await
    }

    async fn user_profile(&self, user_id: &str) -> Result<PublicProfile> {
        self.or_web(
            "user profile",
            self.user_profile_internal(user_id),
            self.web.user_profile(user_id),
        )
        .await
    }
    async fn user_public_playlists(
        &self,
        user_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<Playlist>> {
        let internal = async {
            catalog::user_public_playlists(&self.session().await?, user_id, offset, limit).await
        };
        self.or_web(
            "public playlists",
            internal,
            self.web.user_public_playlists(user_id, offset, limit),
        )
        .await
    }
}

#[cfg(test)]
mod tests;
