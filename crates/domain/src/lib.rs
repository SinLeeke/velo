//! Velo shared contract.
//!
//! Every backend crate implements one of the traits below; the Qt bridge (`crates/app`)
//! only talks to these traits and types, never to a concrete provider. Changing anything
//! here is a contract change: it must be agreed by the lead, not done inside a task.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------ errors

#[derive(Debug, thiserror::Error)]
pub enum VeloError {
    #[error("not signed in")]
    NotAuthenticated,
    #[error("Spotify Premium is required for this action")]
    PremiumRequired,
    #[error("no active Spotify device; enable playback in Velo or pick a device")]
    NoActiveDevice,
    #[error("rate limited; retry after {0:?}")]
    RateLimited(Duration),
    #[error("not found")]
    NotFound,
    /// e.g. listing items of a playlist the user neither owns nor collaborates on (Development Mode).
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("network: {0}")]
    Network(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, VeloError>;

// ------------------------------------------------------------------ catalog

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Picks the smallest image whose width is >= `min_px`, else the largest available.
pub fn pick_image(images: &[ImageRef], min_px: u32) -> Option<&ImageRef> {
    let mut sorted: Vec<&ImageRef> = images.iter().collect();
    sorted.sort_by_key(|i| i.width.unwrap_or(0));
    sorted
        .iter()
        .find(|i| i.width.unwrap_or(0) >= min_px)
        .or_else(|| sorted.last())
        .copied()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    /// Provider id (Spotify track id). `None` for local/unknown items.
    pub id: Option<String>,
    /// Playable URI (e.g. `spotify:track:...`).
    pub uri: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: String,
    pub album_id: Option<String>,
    pub duration_ms: u64,
    pub explicit: bool,
    /// Album art, any order; use [`pick_image`].
    pub images: Vec<ImageRef>,
    /// When the item was added to the playlist / "Tus me gusta" (Unix ms, UTC); `None`
    /// outside those lists or when Spotify does not say (F3).
    #[serde(default)]
    pub added_at: Option<i64>,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub owner_id: String,
    pub collaborative: bool,
    pub images: Vec<ImageRef>,
    pub track_count: u32,
}

impl Playlist {
    /// Development Mode only lists items of playlists the user owns or collaborates on.
    pub fn items_listable_by(&self, user_id: &str) -> bool {
        self.collaborative || self.owner_id == user_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub images: Vec<ImageRef>,
    pub release_date: String,
    pub total_tracks: u32,
    pub album_type: String, // "album" | "single" | "compilation"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub images: Vec<ImageRef>,
    pub genres: Vec<String>,
    pub followers: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeRange {
    ShortTerm,
    #[default]
    MediumTerm,
    LongTerm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub display_name: String,
    pub product: String, // "premium" | "free" | ...
    pub images: Vec<ImageRef>,
}

/// U4: the public face of a Spotify user (profile page): `GET /me` or `GET /users/{id}`.
/// `followers` is `None` when Spotify omits it (never shown as 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicProfile {
    pub id: String,
    pub display_name: String,
    pub images: Vec<ImageRef>,
    pub followers: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub is_active: bool,
    pub volume_percent: Option<u8>,
    pub supports_volume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub offset: u32,
    pub limit: u32,
    pub total: u32,
}

impl<T> Page<T> {
    pub fn has_more(&self) -> bool {
        self.offset + (self.items.len() as u32) < self.total
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
}

/// Authoritative playback snapshot from the provider (Web API `/me/player`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackState {
    pub track: Option<Track>,
    pub is_playing: bool,
    pub progress_ms: u64,
    /// When `progress_ms` was sampled; position now = progress_ms + elapsed if playing.
    pub sampled_at: Instant,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub device: Option<Device>,
    /// Context being played (playlist/album URI), if any.
    pub context_uri: Option<String>,
}

impl PlaybackState {
    pub fn position_now(&self) -> u64 {
        let mut p = self.progress_ms;
        if self.is_playing {
            p += self.sampled_at.elapsed().as_millis() as u64;
        }
        match &self.track {
            Some(t) => p.min(t.duration_ms),
            None => p,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SearchResults {
    pub tracks: Page<Track>,
    pub playlists: Page<Playlist>,
    pub albums: Page<Album>,
    pub artists: Page<Artist>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            offset: 0,
            limit: 0,
            total: 0,
        }
    }
}

// ------------------------------------------------------------------ provider

/// A music source. Spotify (Web API remote control) is the first implementation.
/// All methods are async and must never block; implementations own their own HTTP client.
#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn name(&self) -> &'static str;

    // auth
    async fn is_signed_in(&self) -> bool;
    /// Interactive sign-in (opens the system browser). Resolves when tokens are stored.
    async fn sign_in(&self) -> Result<UserProfile>;
    async fn sign_out(&self) -> Result<()>;
    async fn me(&self) -> Result<UserProfile>;

    // playback (Premium)
    async fn playback(&self) -> Result<Option<PlaybackState>>;
    async fn devices(&self) -> Result<Vec<Device>>;
    async fn transfer(&self, device_id: &str, play: bool) -> Result<()>;
    async fn play(&self) -> Result<()>;
    async fn pause(&self) -> Result<()>;
    async fn next(&self) -> Result<()>;
    async fn previous(&self) -> Result<()>;
    async fn seek(&self, position_ms: u64) -> Result<()>;
    async fn set_volume(&self, percent: u8) -> Result<()>;
    async fn set_shuffle(&self, on: bool) -> Result<()>;
    async fn set_repeat(&self, mode: RepeatMode) -> Result<()>;
    /// Play a context (playlist/album URI) starting at `offset` index.
    async fn play_context(&self, context_uri: &str, offset: Option<u32>) -> Result<()>;
    async fn play_uris(&self, uris: &[String], offset: Option<u32>) -> Result<()>;
    async fn queue(&self) -> Result<Vec<Track>>;
    async fn add_to_queue(&self, uri: &str) -> Result<()>;

    // library
    async fn my_playlists(&self, offset: u32, limit: u32) -> Result<Page<Playlist>>;
    async fn playlist(&self, id: &str) -> Result<Playlist>;
    /// Fails with `Forbidden` for playlists the user neither owns nor collaborates on.
    async fn playlist_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>>;
    async fn saved_tracks(&self, offset: u32, limit: u32) -> Result<Page<Track>>;
    async fn saved_albums(&self, offset: u32, limit: u32) -> Result<Page<Album>>;
    async fn followed_artists(&self, limit: u32) -> Result<Vec<Artist>>;
    /// Library membership by Spotify URI (`/me/library/contains`, `PUT|DELETE /me/library`).
    async fn is_saved(&self, uri: &str) -> Result<bool>;
    async fn set_saved(&self, uri: &str, saved: bool) -> Result<()>;
    async fn recently_played(&self, limit: u32) -> Result<Vec<Track>>;
    async fn top_tracks(&self, range: TimeRange, offset: u32, limit: u32) -> Result<Page<Track>>;
    async fn top_artists(&self, range: TimeRange, offset: u32, limit: u32) -> Result<Page<Artist>>;

    // catalog
    async fn album(&self, id: &str) -> Result<Album>;
    async fn album_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>>;
    async fn artist(&self, id: &str) -> Result<Artist>;
    async fn artist_albums(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Album>>;

    // playlist editing
    async fn create_playlist(
        &self,
        name: &str,
        description: &str,
        public: bool,
    ) -> Result<Playlist>;
    /// M1b: "Editar detalles" of an own playlist (`PUT /playlists/{id}` with name and description).
    async fn update_playlist_details(
        &self,
        playlist_id: &str,
        name: &str,
        description: &str,
    ) -> Result<()>;
    async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()>;
    async fn remove_from_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()>;
    /// Spotify Development Mode caps `limit` at 10.
    async fn search(&self, query: &str, offset: u32, limit: u32) -> Result<SearchResults>;

    // profile pages (U4). Additive: providers without profiles keep the defaults.
    /// `user_id == "me"`: the signed-in account (`GET /me`); else `GET /users/{id}`.
    async fn user_profile(&self, _user_id: &str) -> Result<PublicProfile> {
        Err(VeloError::NotFound)
    }
    /// One page of `GET /users/{id}/playlists`, keeping only the **public playlists that user
    /// owns** (what Spotify's profile shows). `offset`/`total` refer to the unfiltered list, so
    /// `has_more()` still pages through it; a page may come back with fewer (or no) items.
    async fn user_public_playlists(
        &self,
        _user_id: &str,
        _offset: u32,
        _limit: u32,
    ) -> Result<Page<Playlist>> {
        Err(VeloError::NotFound)
    }
}

// ------------------------------------------------------------------ now playing (OS session)

/// What the OS media session says (Windows SMTC of Spotify.exe). Push-based, zero polling.
#[derive(Debug, Clone, PartialEq)]
pub enum NowPlayingEvent {
    /// Spotify's media session appeared (pid lets the visualizer target the process).
    SessionStarted {
        pid: Option<u32>,
    },
    SessionEnded,
    TrackChanged {
        title: String,
        artist: String,
        album: String,
        duration_ms: Option<u64>,
        /// Small cover thumbnail bytes from the session (JPEG/PNG), if provided.
        thumbnail: Option<Vec<u8>>,
    },
    PlaybackChanged {
        is_playing: bool,
    },
    /// Timeline update: position at the instant it was sampled.
    Position {
        position_ms: u64,
        sampled_at: Instant,
    },
}

pub trait NowPlayingSource: Send + Sync {
    /// Subscribe to events. The first events after subscribing describe the current state.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<NowPlayingEvent>;
    /// PID of the Spotify process owning the session, if known.
    fn session_pid(&self) -> Option<u32>;
}

// ------------------------------------------------------------------ cover art & palette

/// Colors are "#RRGGBB". Opaque inks have >= 4.5:1 contrast over the cover
/// composed with the global veil and the local scrim (both use the veil color).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    pub base: String,
    pub glow: String,
    pub veil_dark: String,
    pub veil_light: String,
    pub ink_dark: String,
    pub ink_light: String,
    /// Five unmodified k-means cluster colors and population fractions (0..=1),
    /// largest first. Missing clusters repeat the last color with zero weight.
    /// Empty only in legacy caches, which the cover service rebuilds offline.
    #[serde(default)]
    pub colors: Vec<(String, f32)>,
    /// Veil opacity in 0..=255; published to QML as a real in 0..=1.
    pub veil_dark_alpha: u8,
    pub veil_light_alpha: u8,
    /// Global veil opacity (0..=255) used only while the background moves: no scrim
    /// then, so this alone keeps the ink >= 4.5:1 over the lightest and darkest
    /// dominant colors. Never below the static alpha. 0 only in legacy caches,
    /// which the cover service rebuilds offline.
    #[serde(default)]
    pub veil_dark_moving_alpha: u8,
    #[serde(default)]
    pub veil_light_moving_alpha: u8,
    /// Highest opacity (0..=255) the colour lights may have while the background moves,
    /// so the ink keeps 4.5:1 over art + capped veil + light. `None` in legacy caches.
    #[serde(default)]
    pub blob_dark_moving_alpha: Option<u8>,
    #[serde(default)]
    pub blob_light_moving_alpha: Option<u8>,
    /// Local opacity behind text/controls, in 0..=255. Required fields deliberately
    /// invalidate old cached palettes so the cover cache recomputes them offline.
    pub scrim_dark_alpha: u8,
    pub scrim_light_alpha: u8,
}

/// Files ready for QML (`file:` URLs are built by the bridge).
#[derive(Debug, Clone, PartialEq)]
pub struct CoverAssets {
    /// Cover resized for display (<= 600 px), on disk.
    pub cover: PathBuf,
    /// Pre-blurred background (small, e.g. 96 px); empty when blur was not requested.
    pub blurred: PathBuf,
    pub palette: Palette,
}

#[async_trait]
pub trait CoverArtService: Send + Sync {
    /// Download (or read from disk cache) `url`, then produce resized cover, blurred
    /// background and palette. Cached by URL hash; second call for the same URL is disk-only.
    async fn process(&self, url: &str) -> Result<CoverAssets>;
    /// Same, from raw bytes (SMTC thumbnail) with a caller-chosen cache key.
    async fn process_bytes(&self, key: &str, bytes: Vec<u8>) -> Result<CoverAssets>;
    /// Thumbnail for lists (e.g. 64 or 128 px). Returns a cached file path.
    async fn thumbnail(&self, url: &str, max_px: u32) -> Result<PathBuf>;
}

// ------------------------------------------------------------------ lyrics

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Lyrics {
    /// Time-sorted synced lines; empty when only plain lyrics exist.
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
    pub instrumental: bool,
}

impl Lyrics {
    /// Index of the active synced line at `position_ms` (last line with time <= position).
    pub fn line_at(&self, position_ms: u64) -> Option<usize> {
        match self.synced.partition_point(|l| l.time_ms <= position_ms) {
            0 => None,
            n => Some(n - 1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: u32,
}

#[async_trait]
pub trait LyricsService: Send + Sync {
    /// LRCLIB lookup with local cache (including cached "not found").
    async fn lyrics(&self, q: &LyricsQuery) -> Result<Option<Lyrics>>;
}

// ------------------------------------------------------------------ visualizer

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VizStyle {
    #[default]
    Normal,
    Circular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VizShape {
    #[default]
    Bars,
    Wave,
}

pub const VIZ_BANDS: usize = 48;
/// Legacy 30 Hz reference for gain/smoothing calibration, not a publication cap.
pub const VIZ_MAX_FPS: u32 = 30;

/// Visualizer engine: per-process loopback of Spotify.exe + FFT on its own thread.
/// While inactive it holds no capture stream and no running timer.
pub trait Visualizer: Send + Sync {
    /// Start/stop capture+FFT. The bridge calls this with
    /// enabled && playing && window visible && window focused.
    fn set_active(&self, active: bool, target_pid: Option<u32>);
    fn set_sensitivity(&self, factor: f32); // 1.0 = 100 %
    /// Latest band magnitudes >= 0 (≈1 = the recent crest; louder moments exceed it),
    /// published at the display refresh rate while active.
    fn bands(&self) -> tokio::sync::watch::Receiver<[f32; VIZ_BANDS]>;
    /// True when real capture is unavailable (Windows 10) and output is simulated.
    fn is_simulated(&self) -> bool;
}

// ------------------------------------------------------------------ settings

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Settings {
    pub appearance: AppearanceSettings,
    pub visualizer: VisualizerSettings,
    pub background: BackgroundSettings,
    pub playback: PlaybackSettings,
    pub account: AccountSettings,
    pub window: WindowSettings,
    pub cache: CacheSettings,
    pub library: LibrarySettings,
    pub proxy: ProxySettings,
    pub equalizer: EqualizerSettings,
    pub updates: UpdateSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    pub check_updates: bool,
}
impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_updates: true,
        }
    }
}

/// Limits in MiB. Audio storage contains only librespot's encrypted stream cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheSettings {
    pub cover_limit_mb: u32,
    #[serde(deserialize_with = "deserialize_u32")]
    pub audio_limit_mb: u32,
    pub audio_enabled: bool,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            cover_limit_mb: 50,
            audio_limit_mb: 1024,
            audio_enabled: true,
        }
    }
}

impl CacheSettings {
    /// Acota entradas de TOML/QML sin desbordamientos ni caché ilimitada.
    pub fn normalize(&mut self) {
        self.cover_limit_mb = self.cover_limit_mb.clamp(1, 500);
        self.audio_limit_mb = audio_cache_limit(self.audio_limit_mb);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub theme: String,    // "system" | "light" | "dark"
    pub language: String, // "system" | "es" | "en"
    pub reduce_motion: bool,
    /// Modo rendimiento: sin animaciones, sin fundido de fondo, sin visualizador ni drift, y
    /// efectos en pausa cuando la ventana pierde el foco.
    pub performance_mode: bool,
    /// Pausar visualizador y reloj de posición cuando la ventana pierde el foco.
    pub pause_in_background: bool,
    /// Con Modo rendimiento activo, el visualizador sigue corriendo si esta opción está en true.
    pub performance_keep_visualizer: bool,
    pub sidebar_compact: bool,
    pub compact_tracks: bool,
    pub ui_zoom: f64,
}
impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            language: "system".into(),
            reduce_motion: false,
            performance_mode: false,
            pause_in_background: false,
            performance_keep_visualizer: false,
            sidebar_compact: false,
            compact_tracks: false,
            ui_zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualizerSettings {
    pub enabled: bool,
    /// Show the bottom band in Now Playing; independent of the reactive cover.
    #[serde(default = "default_band")]
    pub band: bool,
    pub style: VizStyle,
    pub shape: VizShape,
    pub sensitivity: f32, // 0.25 ..= 2.0
    pub player_bar_mode: String,
}
impl Default for VisualizerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            band: true,
            style: VizStyle::Normal,
            shape: VizShape::Wave,
            sensitivity: 1.2,
            player_bar_mode: "off".into(),
        }
    }
}

fn default_band() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundSettings {
    pub cover_blur: bool, // false = flat palette veil only
    pub drift: bool,      // slow movement of the blurred art while playing
    pub tint_ui: bool,    // palette tints the chrome
    /// Hypnotic background (moving art): pace and strength, 0..=100 each.
    pub hypnotic_speed: i32,
    pub hypnotic_intensity: i32,
}
impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            cover_blur: true,
            drift: true,
            tint_ui: true,
            hypnotic_speed: 50,
            hypnotic_intensity: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackSettings {
    pub preferred_device: Option<String>,
    pub show_lyrics: bool,
    /// Suppress Velo's local listening history; Spotify privacy needs Connect support.
    pub private_session: bool,
    pub device_name: String,
    #[serde(deserialize_with = "deserialize_u32")]
    pub quality: u32,
    pub normalize_volume: bool,
    pub autoplay: bool,
    pub gapless: bool,
    #[serde(deserialize_with = "deserialize_u32")]
    pub output_buffer_ms: u32,
    /// «Mixear»: segundos del fundido cruzado entre canciones (1..=12).
    #[serde(deserialize_with = "deserialize_u32")]
    pub mix_seconds: u32,
    /// Mezclar siempre las canciones consecutivas en el motor local (requiere gapless).
    pub mix_auto: bool,
}
impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            preferred_device: None,
            show_lyrics: true,
            private_session: false,
            device_name: "Velo".into(),
            quality: 320,
            normalize_volume: true,
            autoplay: true,
            gapless: true,
            output_buffer_ms: 100,
            mix_seconds: MixConfig::DEFAULT_SECONDS,
            mix_auto: false,
        }
    }
}
impl PlaybackSettings {
    /// La mezcla que el motor recibe de los ajustes (el botón por playlist puede encenderla).
    pub fn mix(&self) -> MixConfig {
        MixConfig {
            enabled: self.mix_auto,
            seconds: self.mix_seconds,
        }
        .clamped()
    }
}

/// «Mixear» (como el botón de Spotify): fundido cruzado en tiempo real entre canciones
/// consecutivas, en el reproductor local de Velo y solo en memoria. Exige gapless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MixConfig {
    pub enabled: bool,
    /// Duración del solape, 1..=12 s (el motor la recorta a 1/3 de la pista más corta).
    pub seconds: u32,
}
impl MixConfig {
    pub const MIN_SECONDS: u32 = 1;
    pub const MAX_SECONDS: u32 = 12;
    pub const DEFAULT_SECONDS: u32 = 6;
    pub fn clamped(self) -> Self {
        Self {
            enabled: self.enabled,
            seconds: self.seconds.clamp(Self::MIN_SECONDS, Self::MAX_SECONDS),
        }
    }
}
impl Default for MixConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            seconds: Self::DEFAULT_SECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountSettings {
    /// User's own Spotify Client ID (Development Mode). Tokens live in the OS keyring, never here.
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}
impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: 1100,
            height: 680,
            maximized: false,
        }
    }
}

/// Old reserved 200 MiB becomes 1 GiB; other inputs snap to a supported limit.
pub fn audio_cache_limit(value: u32) -> u32 {
    if value == 200 {
        return 1024;
    }
    [512u32, 1024, 4096]
        .into_iter()
        .min_by_key(|v| v.abs_diff(value))
        .unwrap()
}

/// Steps are anchored at 1.0 so Ctrl+0 and the default round-trip exactly;
/// the two endpoints remain selectable despite the non-integral range width.
pub fn ui_zoom(value: f64) -> f64 {
    if !value.is_finite() {
        return 1.0;
    }
    let value = value.clamp(0.75, 1.5);
    if value == 0.75 {
        return value;
    }
    ((value * 10.0).round() / 10.0).clamp(0.75, 1.5)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibrarySettings {
    pub sort: String,
    pub pinned_uris: Vec<String>,
}
impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            sort: "recent_played".into(),
            pinned_uris: Vec::new(),
        }
    }
}

pub fn library_uri(uri: &str) -> bool {
    uri.strip_prefix("spotify:")
        .and_then(|v| v.split_once(':'))
        .is_some_and(|(kind, id)| {
            matches!(kind, "playlist" | "album" | "artist" | "show")
                && id.len() == 22
                && id.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

/// Deliberately contains no password. Secrets live in a separate keyring entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxySettings {
    pub mode: String,
    pub host: String,
    #[serde(deserialize_with = "deserialize_u32")]
    pub port: u32,
    pub user: String,
}
impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            mode: "off".into(),
            host: String::new(),
            port: 8080,
            user: String::new(),
        }
    }
}
impl ProxySettings {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        if !matches!(self.mode.as_str(), "off" | "system" | "http" | "socks5") {
            return Err("Modo de proxy inválido");
        }
        if !(1..=65535).contains(&self.port) {
            return Err("El puerto debe estar entre 1 y 65535");
        }
        if matches!(self.mode.as_str(), "http" | "socks5")
            && (self.host.is_empty()
                || self.host.trim() != self.host
                || self.host.chars().any(|c| {
                    c.is_control() || c.is_whitespace() || matches!(c, '/' | '@' | '?' | '#' | '\\')
                })
                || self.host.contains("://")
                || (self.host.contains(':')
                    && !(self.host.starts_with('[') && self.host.ends_with(']'))))
        {
            return Err("Escribe un host de proxy sin esquema ni credenciales");
        }
        if self.user.chars().any(char::is_control) {
            return Err("Usuario de proxy inválido");
        }
        Ok(())
    }
}

pub const EQ_FREQUENCIES_HZ: [f32; 10] = [
    60., 170., 310., 600., 1000., 3000., 6000., 12000., 14000., 16000.,
];

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct EqConfig {
    pub enabled: bool,
    pub preamp_db: f32,
    pub gains_db: [f32; 10],
}
pub struct EqPreset {
    pub name: &'static str,
    pub preamp_db: f32,
    pub gains_db: [f32; 10],
}
macro_rules! preset {
    ($name:literal, $preamp:expr, [$($gain:expr),*]) => { EqPreset { name: $name, preamp_db: $preamp, gains_db: [$($gain as f32),*] } };
}
/// Stable data table; positive-gain presets include conservative headroom.
pub const EQ_PRESETS: [EqPreset; 26] = [
    preset!("Flat", 0., [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    preset!("Classical", -6., [0, 0, 0, 0, 0, 0, -4, -4, -4, -6]),
    preset!("Club", -6., [0, 0, 4, 6, 6, 6, 4, 0, 0, 0]),
    preset!("Dance", -6., [6, 4, 2, 0, 0, -4, -4, -4, 0, 0]),
    preset!("Full Bass", -8., [8, 8, 8, 4, 0, -4, -6, -8, -8, -8]),
    preset!("Full Bass & Treble", -6., [6, 4, 0, -4, -2, 2, 4, 6, 6, 6]),
    preset!("Full Treble", -8., [-6, -6, -6, -2, 2, 6, 8, 8, 8, 8]),
    preset!(
        "Laptop Speakers/Headphones",
        -6.,
        [3, 6, 3, -2, -1, 2, 3, 6, 8, 8]
    ),
    preset!("Large Hall", -6., [6, 6, 4, 4, 0, -2, -2, -2, 0, 0]),
    preset!("Live", -6., [-2, 0, 2, 4, 4, 4, 2, 2, 2, 2]),
    preset!("Party", -6., [4, 4, 0, 0, 0, 0, 0, 0, 4, 4]),
    preset!("Pop", -6., [-1, 2, 4, 4, 2, -1, -2, -2, -1, -1]),
    preset!("Reggae", -6., [0, 0, 0, -4, 0, 4, 4, 0, 0, 0]),
    preset!("Rock", -6., [5, 3, -3, -5, -2, 2, 5, 6, 6, 6]),
    preset!("Ska", -6., [-2, -4, -3, 0, 3, 4, 5, 5, 6, 5]),
    preset!("Soft", -6., [3, 1, 0, -2, 0, 3, 5, 6, 7, 8]),
    preset!("Soft Rock", -6., [2, 2, 1, 0, -2, -3, -1, 0, 1, 5]),
    preset!("Techno", -6., [5, 3, 0, -3, -2, 0, 5, 6, 6, 5]),
    preset!("Bass Booster", -6., [6, 5, 4, 3, 2, 0, 0, 0, 0, 0]),
    preset!("Bass Reducer", 0., [-6, -5, -4, -3, -2, 0, 0, 0, 0, 0]),
    preset!("Treble Booster", -6., [0, 0, 0, 0, 0, 1, 2, 3, 4, 6]),
    preset!("Vocal Booster", -4., [-2, -2, 0, 2, 4, 4, 2, 0, -2, -2]),
    preset!("Small Speakers", -4., [-4, -2, 2, 4, 4, 2, 0, -2, -4, -4]),
    preset!("Spoken Word", -4., [-6, -4, 0, 3, 4, 4, 2, 0, -4, -6]),
    preset!("Loudness", -6., [6, 4, 1, 0, -2, 0, 1, 3, 5, 6]),
    preset!("Night Listening", -2., [-4, -3, -1, 1, 2, 2, 0, -2, -3, -4]),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqualizerSettings {
    pub enabled: bool,
    pub preamp_db: f32,
    #[serde(deserialize_with = "deserialize_gains")]
    pub gains_db: [f32; 10],
    pub preset: String,
}
impl Default for EqualizerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.,
            gains_db: [0.; 10],
            preset: "Flat".into(),
        }
    }
}

fn deserialize_u32<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<u32, D::Error> {
    let value = i64::deserialize(deserializer)?;
    Ok(value.clamp(0, i64::from(u32::MAX)) as u32)
}
fn deserialize_gains<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<[f32; 10], D::Error> {
    let values = Vec::<f32>::deserialize(deserializer)?;
    let mut bands = [0.; 10];
    for (band, value) in bands.iter_mut().zip(values) {
        *band = finite_db(value);
    }
    Ok(bands)
}
impl EqualizerSettings {
    pub fn config(&self) -> EqConfig {
        EqConfig {
            enabled: self.enabled,
            preamp_db: self.preamp_db,
            gains_db: self.gains_db,
        }
    }
    pub fn set_band(&mut self, index: usize, db: f32) -> bool {
        if index >= 10 || !db.is_finite() {
            return false;
        }
        self.gains_db[index] = db.clamp(-12., 12.);
        self.preset = "Custom".into();
        true
    }
    pub fn set_preamp(&mut self, db: f32) -> bool {
        if !db.is_finite() {
            return false;
        }
        self.preamp_db = db.clamp(-12., 12.);
        self.preset = "Custom".into();
        true
    }
    pub fn set_preset(&mut self, name: &str) -> bool {
        let Some(p) = EQ_PRESETS.iter().find(|p| p.name == name) else {
            return false;
        };
        self.gains_db = p.gains_db;
        self.preamp_db = p.preamp_db;
        self.preset = name.into();
        true
    }
    pub fn reset(&mut self) {
        self.set_preset("Flat");
    }
}

/// The single engine construction schema (shared by settings and `velo-player`).
/// Never contains proxy authentication or login secrets.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineOptions {
    pub quality: u32,
    pub normalize_volume: bool,
    pub autoplay: bool,
    pub gapless: bool,
    /// Connect device name (the device id is never derived from it).
    pub device_name: String,
    pub output_buffer_ms: u32,
    /// Absolute, account-scoped directory; `None` disables the encrypted audio cache.
    pub audio_cache_dir: Option<PathBuf>,
    pub audio_cache_limit_bytes: u64,
    /// A credential-free HTTP CONNECT endpoint (possibly the local adapter).
    pub proxy_url: Option<String>,
    pub eq: EqConfig,
    /// Valor inicial de la mezcla; los cambios llegan en caliente (`Engine::set_mix`).
    pub mix: MixConfig,
}
impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            quality: 320,
            normalize_volume: true,
            autoplay: true,
            gapless: true,
            device_name: "Velo".into(),
            output_buffer_ms: 100,
            audio_cache_dir: None,
            audio_cache_limit_bytes: 1024 * 1024 * 1024,
            proxy_url: None,
            eq: EqConfig::default(),
            mix: MixConfig::default(),
        }
    }
}
impl Settings {
    pub fn engine_options(&self, audio_dir: PathBuf, proxy_url: Option<String>) -> EngineOptions {
        EngineOptions {
            quality: self.playback.quality,
            normalize_volume: self.playback.normalize_volume,
            autoplay: self.playback.autoplay,
            gapless: self.playback.gapless,
            device_name: self.playback.device_name.clone(),
            output_buffer_ms: self.playback.output_buffer_ms,
            audio_cache_dir: self.cache.audio_enabled.then_some(audio_dir),
            audio_cache_limit_bytes: u64::from(self.cache.audio_limit_mb) * 1024 * 1024,
            proxy_url,
            eq: self.equalizer.config(),
            mix: self.playback.mix(),
        }
    }
    pub fn normalize(&mut self) {
        self.cache.normalize();
        self.playback.device_name = self
            .playback
            .device_name
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(100)
            .collect();
        if self.playback.device_name.is_empty() {
            self.playback.device_name = "Velo".into();
        }
        if ![96, 160, 320].contains(&self.playback.quality) {
            self.playback.quality = 320;
        }
        if ![50, 100, 200].contains(&self.playback.output_buffer_ms) {
            self.playback.output_buffer_ms = 100;
        }
        self.playback.mix_seconds = self
            .playback
            .mix_seconds
            .clamp(MixConfig::MIN_SECONDS, MixConfig::MAX_SECONDS);
        self.appearance.ui_zoom = ui_zoom(self.appearance.ui_zoom);
        if !["off", "bars", "wave"].contains(&self.visualizer.player_bar_mode.as_str()) {
            self.visualizer.player_bar_mode = "off".into();
        }
        if !["recent_played", "recent_added", "alphabetical", "creator"]
            .contains(&self.library.sort.as_str())
        {
            self.library.sort = "recent_played".into();
        }
        let mut pins = std::collections::HashSet::new();
        self.library
            .pinned_uris
            .retain(|uri| library_uri(uri) && pins.insert(uri.clone()));
        if self.proxy.validate().is_err() {
            self.proxy = ProxySettings::default();
        }
        let eq = &mut self.equalizer;
        eq.preamp_db = finite_db(eq.preamp_db);
        for gain in &mut eq.gains_db {
            *gain = finite_db(*gain);
        }
        if eq.preset != "Custom"
            && !EQ_PRESETS.iter().any(|p| {
                p.name == eq.preset && p.preamp_db == eq.preamp_db && p.gains_db == eq.gains_db
            })
        {
            eq.preset = "Custom".into();
        }
    }
}
fn finite_db(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(-12., 12.)
    } else {
        0.
    }
}

/// Shared route generation. HTTP clients replace their connection pool on change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxyRoute {
    pub generation: u64,
    pub url: Option<String>,
}
pub fn proxy_route() -> &'static tokio::sync::watch::Sender<ProxyRoute> {
    static ROUTE: std::sync::OnceLock<tokio::sync::watch::Sender<ProxyRoute>> =
        std::sync::OnceLock::new();
    ROUTE.get_or_init(|| tokio::sync::watch::channel(ProxyRoute::default()).0)
}

pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Settings;
    /// Debounced, atomic write (temp file + rename) off the UI thread.
    fn save(&self, settings: &Settings);
    fn path(&self) -> PathBuf;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s9_defaults_and_engine_boundary() {
        let s = Settings::default();
        assert_eq!(s.visualizer.player_bar_mode, "off");
        assert_eq!(s.playback.device_name, "Velo");
        assert_eq!(s.playback.quality, 320);
        assert!(s.playback.normalize_volume && s.playback.autoplay && s.playback.gapless);
        assert_eq!(s.playback.output_buffer_ms, 100);
        assert!(s.cache.audio_enabled);
        assert_eq!(s.cache.audio_limit_mb, 1024);
        assert_eq!(s.appearance.ui_zoom, 1.0);
        assert!(!s.appearance.sidebar_compact && !s.appearance.compact_tracks);
        assert_eq!(s.library.sort, "recent_played");
        assert!(s.library.pinned_uris.is_empty());
        assert_eq!(s.proxy.mode, "off");
        assert!(!s.equalizer.enabled);
        let options = s.engine_options(PathBuf::from("encrypted-audio"), None);
        assert_eq!(
            options.audio_cache_dir,
            Some(PathBuf::from("encrypted-audio"))
        );
        assert_eq!(options.audio_cache_limit_bytes, 1024 * 1024 * 1024);
    }
    #[test]
    fn s9_validation_and_cache_migration() {
        assert_eq!(audio_cache_limit(200), 1024);
        for (input, expected) in [
            (0, 512),
            (512, 512),
            (1000, 1024),
            (1024, 1024),
            (4096, 4096),
            (u32::MAX, 4096),
        ] {
            assert_eq!(audio_cache_limit(input), expected);
        }
        let mut s = Settings::default();
        s.playback.quality = 255;
        s.playback.output_buffer_ms = 0;
        s.playback.device_name = " \n ".into();
        s.appearance.ui_zoom = f64::NAN;
        s.library.sort = "invalid".into();
        s.visualizer.player_bar_mode = "invalid".into();
        s.normalize();
        assert_eq!(s, Settings::default());
        for quality in [96, 160, 320] {
            s.playback.quality = quality;
            s.normalize();
            assert_eq!(s.playback.quality, quality);
        }
        for buffer in [50, 100, 200] {
            s.playback.output_buffer_ms = buffer;
            s.normalize();
            assert_eq!(s.playback.output_buffer_ms, buffer);
        }
        for (input, expected) in [
            (0.0, 0.75),
            (0.75, 0.75),
            (0.76, 0.8),
            (1.01, 1.0),
            (1.16, 1.2),
            (1.5, 1.5),
            (10., 1.5),
            (f64::INFINITY, 1.0),
        ] {
            assert_eq!(ui_zoom(input), expected);
        }
    }
    #[test]
    fn s9_equalizer_table_and_manual_edits_are_atomic() {
        let names = [
            "Flat",
            "Classical",
            "Club",
            "Dance",
            "Full Bass",
            "Full Bass & Treble",
            "Full Treble",
            "Laptop Speakers/Headphones",
            "Large Hall",
            "Live",
            "Party",
            "Pop",
            "Reggae",
            "Rock",
            "Ska",
            "Soft",
            "Soft Rock",
            "Techno",
            "Bass Booster",
            "Bass Reducer",
            "Treble Booster",
            "Vocal Booster",
            "Small Speakers",
            "Spoken Word",
            "Loudness",
            "Night Listening",
        ];
        assert_eq!(EQ_PRESETS.map(|p| p.name), names);
        let mut eq = EqualizerSettings {
            enabled: true,
            ..Default::default()
        };
        for name in names {
            assert!(eq.set_preset(name));
            assert_eq!(eq.preset, name);
            assert!(eq.preamp_db.is_finite() && (-12.0..=12.0).contains(&eq.preamp_db));
            assert!(
                eq.gains_db
                    .iter()
                    .all(|v| v.is_finite() && (-12.0..=12.0).contains(v))
            );
        }
        let before = eq.clone();
        assert!(!eq.set_band(10, 0.));
        assert!(!eq.set_band(0, f32::NAN));
        assert!(!eq.set_preamp(f32::INFINITY));
        assert!(!eq.set_preset("unknown"));
        assert_eq!(eq, before);
        assert!(eq.set_band(0, 99.));
        assert_eq!(eq.gains_db[0], 12.);
        assert_eq!(eq.preset, "Custom");
        assert!(eq.set_preamp(-99.));
        assert_eq!(eq.preamp_db, -12.);
        eq.reset();
        assert!(eq.enabled);
        assert_eq!(eq.preset, "Flat");
        assert_eq!(eq.config().gains_db, [0.; 10]);
        assert_eq!(eq.preamp_db, 0.);
    }
    #[test]
    fn s9_proxy_rejects_urls_credentials_and_bad_ports() {
        for mode in ["off", "system", "http", "socks5"] {
            let mut p = ProxySettings {
                mode: mode.into(),
                host: "localhost".into(),
                ..Default::default()
            };
            assert!(p.validate().is_ok());
            for port in [0, 65536, u32::MAX] {
                p.port = port;
                assert!(p.validate().is_err());
            }
        }
        for host in [
            "http://host",
            "user:password@host",
            "host/path",
            "host:80",
            "host\nsecret",
            "",
        ] {
            let p = ProxySettings {
                mode: "http".into(),
                host: host.into(),
                ..Default::default()
            };
            assert!(p.validate().is_err());
        }
    }

    #[test]
    fn line_at_picks_last_started_line() {
        let l = Lyrics {
            synced: vec![
                LyricLine {
                    time_ms: 1000,
                    text: "a".into(),
                },
                LyricLine {
                    time_ms: 3000,
                    text: "b".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(l.line_at(500), None);
        assert_eq!(l.line_at(1000), Some(0));
        assert_eq!(l.line_at(2999), Some(0));
        assert_eq!(l.line_at(9000), Some(1));
    }

    #[test]
    fn pick_image_prefers_smallest_sufficient() {
        let imgs = vec![
            ImageRef {
                url: "640".into(),
                width: Some(640),
                height: Some(640),
            },
            ImageRef {
                url: "64".into(),
                width: Some(64),
                height: Some(64),
            },
            ImageRef {
                url: "300".into(),
                width: Some(300),
                height: Some(300),
            },
        ];
        assert_eq!(pick_image(&imgs, 200).unwrap().url, "300");
        assert_eq!(pick_image(&imgs, 1000).unwrap().url, "640");
    }
}
