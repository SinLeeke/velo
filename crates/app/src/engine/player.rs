//! `Player` engine: merges the registered playback sources into one view, owns the position
//! clock, the cover/palette pipeline, and executes the user's actions.
//!
//! The only periodic work is the position ticker: it runs at 4 Hz while the music plays and
//! the window is visible, and is not even a sleeping task otherwise (the task waits on the
//! `flags` channel, which has no timer).
//!
//! Sources, in priority order: Velo's own engine (Connect device "Velo"), the official app on
//! this PC, another Connect device. New playback goes to Velo by default (`play_route`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::sync::{mpsc, watch};
use tokio::task::AbortHandle;
use velo_domain::{MusicProvider, Palette, RepeatMode, Track, VeloError};
use velo_player::queue::{self, QueueEdit, edit_queue};
use velo_player::{Engine, EngineState, PlayerError, TrackFailure};

#[path = "history.rs"]
pub mod history;

use super::audio::audio_message;

use super::clock::{Clock, PositionGate, TICK, should_tick};
use super::images::{Generation, Thumbnailer};
use super::rows::{DEVICE_ROLES, THUMB_PX, device_row, file_url, image_of, track_row};
use super::{Bus, Env};
use crate::bridge::source::{
    PlayRoute, PlaybackSource, Refresh, SourceAction, SourceState, SourceTrack, can_control,
    either_active, play_route,
};
use crate::bridge::source_local::LocalSource;
use crate::bridge::source_remote::RemoteSource;
use crate::bridge::source_velo::VeloSource;
use crate::model::ModelOp;

/// Pause between volume requests while the slider is being dragged.
const VOLUME_COALESCE: Duration = Duration::from_millis(150);
const VOLUME_COALESCE_LOCAL: Duration = Duration::from_millis(16);
/// After the user moves the volume, source reports do not overwrite it for this long.
const VOLUME_HOLD: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerView {
    pub title: String,
    /// Spotify URI of the song, when the source knows it (M1 menu of the playing song).
    pub uri: String,
    pub artist: String,
    pub artist_id: String,
    pub album_id: String,
    pub album: String,
    pub duration_ms: u64,
    pub is_playing: bool,
    pub shuffle: bool,
    /// 0 off, 1 context, 2 track.
    pub repeat: i32,
    pub volume: i32,
    pub volume_memory: super::volume_memory::VolumeMemory,
    pub liked: bool,
    pub can_control: bool,
    pub queue_status: String,
    pub queue_error: String,
    pub queue_editable: bool,
    pub queue_clearable: bool,
    pub device_name: String,
    pub has_session: bool,
    /// `file:///` URLs; empty without a track.
    pub cover_url: String,
    pub blur_url: String,
    pub palette: Option<Palette>,
    pub dj_available: bool,
    pub dj_active: bool,
    pub dj_speaking: bool,
    pub dj_line: String,
}

pub const LIVI_ART_URL: &str = "qrc:/velo/src/assets/livi.png";
const LIVI_ART: &[u8] = include_bytes!("../assets/livi.png");

/// Ordered model changes for `Player.queue` and `Player.devices`.
#[derive(Debug)]
pub enum PlayerUi {
    Queue(ModelOp),
    Devices(ModelOp),
    RefreshPlaylists,
    QueueSaved { playlist_id: String, name: String },
}

struct Inner {
    active: usize,
    clock: Clock,
    position_gate: PositionGate,
    track: Option<SourceTrack>,
    cover_key: Option<String>,
    cover_task: Option<AbortHandle>,
    liked_uri: Option<String>,
    volume_set_at: Option<Instant>,
    /// Hay una tarea de un solo disparo esperando el fin del hold de volumen.
    volume_recheck: bool,
    devices_available: bool,
    queue_identity: (usize, bool, String, Option<String>, bool, bool, u64, u64),
    metadata_key: (String, usize, bool),
}

pub struct PlayerCtl {
    env: Arc<Env>,
    sources: Vec<Arc<dyn PlaybackSource>>,
    pub view: watch::Sender<PlayerView>,
    pub history: Arc<history::HistoryCtl>,
    pub listening: Bus<history::ProgressSample>,
    metadata_generation: AtomicU64,
    cover_generation: AtomicU64,
    /// Authoritative source events for native media and PCM routing (never the UI ticker).
    pub media_state: watch::Sender<SourceState>,
    pub ui: Bus<PlayerUi>,
    inner: Mutex<Inner>,
    volume: watch::Sender<Option<u8>>,
    queue_gen: Arc<Generation>,
    thumbs: Thumbnailer,
    /// The rows of `Player.queue` as last fetched or edited, and the song they follow.
    queue_snapshot: Mutex<QueueSnapshot>,
    /// Serializes queue fetches, edits and retries (each reloads or reads the same rows).
    queue_operation: tokio::sync::Mutex<()>,
    /// Invalida respuestas pendientes incluso si se vuelve rápidamente a la misma URI.
    queue_refresh_generation: AtomicU64,
    /// Bumped by every user playback action; a pending audio login or retry that sees it move
    /// gives up (the user asked for something else meanwhile).
    play_generation: AtomicU64,
}

#[derive(Debug, Default)]
struct QueueSnapshot {
    /// The engine's song when the rows were fetched; `None`: rows not editable (stale).
    current: Option<String>,
    /// Pista autoritativa de cualquier dispositivo a la que siguen las filas visibles.
    anchor: Option<String>,
    tracks: Vec<Track>,
}

/// Ancla del vaciado pendiente: el avance normal, volumen, seek o pausa no cambian
/// la ocurrencia. Spirc conserva el transporte más reciente al recargar.
struct ClearGuard {
    current: Option<String>,
    occurrence: Option<usize>,
    generation: u64,
    failures: u32,
}

impl ClearGuard {
    fn accepts(&self, generation: u64, state: &EngineState, occurrence: Option<usize>) -> bool {
        generation == self.generation
            && state.active
            && state.failures == self.failures
            && state.track.as_ref().map(|t| &t.uri) == self.current.as_ref()
            && occurrence == self.occurrence
    }
}

/// Role names of `Player.queue`: the track roles plus ids for "Ir al artista/álbum".
pub const QUEUE_ROLES: &str =
    "trackId,uri,title,artist,album,durationMs,imageUrl,liked,artistId,albumId";

const ONLY_VELO: &str = "Solo con Velo como dispositivo.";

/// Filas pendientes tras un cambio de pista, sin red: las anteriores sin la que ya suena.
fn advance_rows(live: bool, uri: Option<&str>, anchor: Option<&str>, rows: &[Track]) -> Vec<Track> {
    if !live {
        return Vec::new();
    }
    uri.and_then(|uri| queue::align_upcoming(uri, anchor, rows.to_vec()))
        .unwrap_or_default()
}

/// After a confirmed skip the chosen song is the new anchor of the remaining rows.
fn skip_anchor(snapshot: &mut QueueSnapshot, target: &str) {
    snapshot.anchor = Some(target.to_owned());
}

fn queue_row(t: &Track) -> serde_json::Value {
    let mut row = track_row(t, false);
    row["artistId"] = json!(
        t.artists
            .first()
            .and_then(|a| a.id.clone())
            .unwrap_or_default()
    );
    row["albumId"] = json!(t.album_id.clone().unwrap_or_default());
    row
}

/// `Player.queueEditable`: removing and reordering only work on Velo's own engine (the Web
/// API has no such calls), signed in and with a Spotify song playing or paused here.
pub fn queue_editable(state: &SourceState, signed_in: bool, audio_ready: bool) -> bool {
    signed_in
        && audio_ready
        && state.active
        && state.own_engine
        && state.track.as_ref().is_some_and(|t| t.uri.is_some())
}

/// E2b: play with Velo's audio off and nothing else playing opens the audio login.
pub fn should_enable_audio(signed_in: bool, audio_ready: bool, other_active: bool) -> bool {
    signed_in && !audio_ready && !other_active
}

/// The toast when a song is skipped for good (no silent skips).
fn skipped_message(failure: &TrackFailure) -> &'static str {
    if failure.transient {
        "Spotify tardó en entregar la canción; se saltó."
    } else {
        "Spotify no tiene disponible esta canción; se saltó."
    }
}

/// After a failed song Spirc plays the next one (`now_playing`); the Web API queue may still
/// list it first or even list the failed song (it lags a moment). The retry plays the failed
/// song, then `now_playing`, then the rest, without repeating either.
fn retry_upcoming(now_playing: Option<String>, queued: Vec<String>, failed: &str) -> Vec<String> {
    let mut upcoming: Vec<String> = now_playing.into_iter().collect();
    let mut rows = queued.into_iter().peekable();
    if rows.peek().map(String::as_str) == Some(failed) {
        rows.next();
    }
    if !upcoming.is_empty() && rows.peek() == upcoming.first() {
        rows.next();
    }
    upcoming.extend(rows);
    upcoming
}

/// Una respuesta de red pendiente no puede deshacer una acción ni otro fallo de audio.
struct RetryGuard {
    generation: u64,
    failures: u32,
    failure: Option<TrackFailure>,
    current: Option<String>,
    playing: bool,
    position_at: Instant,
}

impl RetryGuard {
    fn new(generation: u64, state: &EngineState) -> Self {
        Self {
            generation,
            failures: state.failures,
            failure: state.last_failure.clone(),
            current: state.track.as_ref().map(|t| t.uri.clone()),
            playing: state.playing,
            position_at: state.position_at,
        }
    }

    fn accepts(&self, generation: u64, state: &EngineState) -> bool {
        generation == self.generation
            && state.active
            && state.failures == self.failures
            && state.last_failure == self.failure
            && state.track.as_ref().map(|t| &t.uri) == self.current.as_ref()
            && state.playing == self.playing
            && state.position_at == self.position_at
    }
}

/// The playback sources, in priority order: the first one reporting an active device drives the
/// player. Velo's own engine first, then the official app on this PC, then any other Connect
/// device (silent while one of the first two is active).
fn register_sources(env: &Arc<Env>) -> Vec<Arc<dyn PlaybackSource>> {
    let velo = VeloSource::start(env);
    let local = LocalSource::start(env);
    let outranked = either_active(&env.rt, velo.state(), local.state());
    let remote = RemoteSource::start(env, outranked);
    vec![velo, local, remote]
}

fn active_source_index(sources: &[Arc<dyn PlaybackSource>]) -> usize {
    sources
        .iter()
        .position(|s| s.state().borrow().active)
        .unwrap_or(sources.len() - 1)
}

fn cover_key(t: &SourceTrack) -> Option<String> {
    if let Some(url) = &t.cover_url {
        Some(format!("url:{url}"))
    } else if t.thumbnail.is_some() {
        Some(bytes_key(t))
    } else {
        None
    }
}

fn bytes_key(t: &SourceTrack) -> String {
    format!("smtc:{}|{}|{}", t.artist, t.album, t.title)
}

fn repeat_to_i32(r: RepeatMode) -> i32 {
    match r {
        RepeatMode::Off => 0,
        RepeatMode::Context => 1,
        RepeatMode::Track => 2,
    }
}

pub fn repeat_from_i32(v: i32) -> RepeatMode {
    match v {
        1 => RepeatMode::Context,
        2 => RepeatMode::Track,
        _ => RepeatMode::Off,
    }
}

/// Repeat mode after one press: off, context, track, off, ...
pub fn next_repeat(current: i32) -> RepeatMode {
    repeat_from_i32((current + 1).rem_euclid(3))
}

impl PlayerCtl {
    pub fn start(env: &Arc<Env>) -> Arc<Self> {
        let sources = register_sources(env);
        let this = Self::new(env, sources);

        // Merge: any source or account change recomputes the view.
        let (kick, kicks) = mpsc::unbounded_channel::<()>();
        for source in &this.sources {
            tracing::debug!(source = source.name(), "registered playback source");
            let mut rx = source.state();
            let kick = kick.clone();
            env.rt.spawn(async move {
                while rx.changed().await.is_ok() {
                    if kick.send(()).is_err() {
                        break;
                    }
                }
            });
        }
        Self::start_loops(env, &this, kick, kicks);
        Self::attach_listening(env, &this);
        this
    }

    /// Feeds the listening history with the real PCM progress of Velo's own engine. It is
    /// event driven (broadcast of `PlaybackProgress`), with no timer and no polling.
    fn attach_listening(env: &Arc<Env>, this: &Arc<Self>) {
        let weak = Arc::downgrade(this);
        let env = env.clone();
        let mut ready = env.audio.view.subscribe();
        env.rt.clone().spawn(async move {
            let mut attached: Option<(usize, AbortHandle)> = None;
            loop {
                let engine = env.audio.engine();
                let id = engine.as_ref().map_or(0, |e| Arc::as_ptr(e) as usize);
                if attached.as_ref().map(|a| a.0) != Some(id) {
                    if let Some((_, task)) = attached.take() {
                        task.abort();
                    }
                    if let Some(engine) = engine {
                        let task = env.rt.spawn(listen_to_engine(weak.clone(), engine));
                        attached = Some((id, task.abort_handle()));
                    }
                }
                if ready.changed().await.is_err() {
                    break;
                }
            }
        });
    }

    #[cfg(test)]
    pub(super) fn for_test(env: &Arc<Env>) -> Arc<Self> {
        tests::controller(env)
    }

    fn new(env: &Arc<Env>, sources: Vec<Arc<dyn PlaybackSource>>) -> Arc<Self> {
        let this = Arc::new(Self {
            env: env.clone(),
            sources,
            view: watch::channel(PlayerView {
                queue_status: "unavailable".into(),
                ..PlayerView::default()
            })
            .0,
            history: history::HistoryCtl::start(env),
            listening: Bus::new(),
            metadata_generation: AtomicU64::new(0),
            cover_generation: AtomicU64::new(0),
            media_state: watch::channel(SourceState::default()).0,
            ui: Bus::new(),
            inner: Mutex::new(Inner {
                active: 0,
                clock: Clock::new(Instant::now()),
                position_gate: PositionGate::default(),
                track: None,
                cover_key: None,
                cover_task: None,
                liked_uri: None,
                volume_set_at: None,
                volume_recheck: false,
                devices_available: false,
                queue_identity: (0, false, String::new(), None, false, false, 0, 0),
                metadata_key: (String::new(), 0, false),
            }),
            volume: watch::channel(None).0,
            queue_gen: Arc::new(Generation::default()),
            thumbs: Thumbnailer::new(env.rt.clone(), env.covers.clone()),
            queue_snapshot: Mutex::new(QueueSnapshot::default()),
            queue_operation: tokio::sync::Mutex::new(()),
            queue_refresh_generation: AtomicU64::new(0),
            play_generation: AtomicU64::new(0),
        });
        this.history.attach(&env.rt, &this.listening);
        this
    }

    fn start_loops(
        env: &Arc<Env>,
        this: &Arc<Self>,
        kick: mpsc::UnboundedSender<()>,
        mut kicks: mpsc::UnboundedReceiver<()>,
    ) {
        {
            let mut rx = env.account.view.subscribe();
            let kick = kick.clone();
            env.rt.spawn(async move {
                while rx.changed().await.is_ok() {
                    if kick.send(()).is_err() {
                        break;
                    }
                }
            });
        }
        {
            let mut rx = env.account.queue_changes();
            let kick = kick.clone();
            let account = env.account.clone();
            env.rt.spawn(async move {
                while rx.changed().await.is_ok() {
                    account.note_queue_change();
                    if kick.send(()).is_err() {
                        break;
                    }
                }
            });
        }
        // Velo's engine coming up or going away changes `canControl`.
        {
            let mut rx = env.audio.view.subscribe();
            let kick = kick.clone();
            env.rt.spawn(async move {
                while rx.changed().await.is_ok() {
                    if kick.send(()).is_err() {
                        break;
                    }
                }
            });
        }
        let me = this.clone();
        let _ = kick.send(());
        env.rt.spawn(async move {
            while kicks.recv().await.is_some() {
                while kicks.try_recv().is_ok() {}
                me.recompute();
            }
        });

        let me = this.clone();
        env.rt.spawn(async move { me.ticker().await });
        let me = this.clone();
        env.rt.spawn(async move { me.failure_loop().await });
        let me = this.clone();
        env.rt.spawn(async move { me.visibility_refresh().await });
        let me = this.clone();
        let mut prefs = env.prefs.watch_settings();
        env.rt.spawn(async move {
            let mut blur = prefs.borrow_and_update().background.cover_blur;
            while prefs.changed().await.is_ok() {
                let next = prefs.borrow_and_update().background.cover_blur;
                if next != blur {
                    blur = next;
                    me.start_cover();
                }
            }
        });
        let me = this.clone();
        let volume_rx = this.volume.subscribe();
        env.rt.spawn(async move { me.volume_loop(volume_rx).await });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn active_source(&self) -> Arc<dyn PlaybackSource> {
        self.sources[self.lock().active].clone()
    }

    /// Asks every source to re-read its authority (after actions done outside the player).
    pub fn resync(&self) {
        for s in &self.sources {
            s.refresh(Refresh::AfterAction);
        }
    }

    // ------------------------------------------------------------------ state

    fn recompute(self: &Arc<Self>) {
        let idx = active_source_index(&self.sources);
        let state = self.sources[idx].state().borrow().clone();
        self.apply(idx, state);
    }

    fn apply(self: &Arc<Self>, idx: usize, mut st: SourceState) {
        let now = Instant::now();
        let account = self.env.account.snapshot();
        let dj_available = self.env.audio.state.borrow().dj_available;
        if !account.signed_in {
            st = SourceState::default();
            self.lock().devices_available = false;
            self.ui
                .send(PlayerUi::Devices(ModelOp::Replace(Vec::new())));
        }
        let new_key = st.track.as_ref().and_then(cover_key);
        let (track_changed, cover_changed, check_liked, queue_changed) = {
            let mut i = self.lock();
            let identity = (
                idx,
                st.active,
                st.device_name.clone(),
                st.track.as_ref().and_then(|t| t.uri.clone()),
                account.signed_in,
                st.shuffle,
                if st.own_engine {
                    self.env.audio.state.borrow().occurrence
                } else {
                    0
                },
                self.env.account.queue_revision(),
            );
            st.device_available |= i.devices_available;
            i.active = idx;
            let track_changed = match (&i.track, &st.track) {
                (Some(old), Some(new)) => !old.same_track(new),
                (None, None) => false,
                _ => true,
            };
            let queue_changed = i.queue_identity != identity || track_changed;
            i.queue_identity = identity;
            if queue_changed {
                self.queue_refresh_generation.fetch_add(1, Ordering::SeqCst);
            }
            i.track = st.track.clone();
            i.clock
                .set_duration(st.track.as_ref().map_or(0, |t| t.duration_ms));
            i.clock.set(st.position_ms, st.position_at, st.is_playing);
            let cover_changed = new_key != i.cover_key;
            if cover_changed {
                i.cover_key = new_key;
            }
            // Membership must be fetched again after signing in, even on the same track.
            let uri = account
                .signed_in
                .then(|| st.track.as_ref().and_then(|t| t.uri.clone()))
                .flatten();
            let uri_changed = uri != i.liked_uri;
            if uri_changed {
                i.liked_uri = uri.clone();
            }
            let check_liked = if uri_changed && account.signed_in {
                uri
            } else {
                None
            };
            (track_changed, cover_changed, check_liked, queue_changed)
        };

        self.env.update_flags(|f| {
            f.playing = st.is_playing && st.active;
            f.pid = st.pid;
            f.remote = st.remote;
        });

        let volume_guard = self.lock();
        let hold_volume = volume_guard
            .volume_set_at
            .is_some_and(|t| now.saturating_duration_since(t) < VOLUME_HOLD);
        let can_control = can_control(
            &st,
            account.signed_in,
            account.is_premium,
            self.env.audio.is_ready(),
        );
        self.view.send_if_modified(|v| {
            let mut next = v.clone();
            next.title = st
                .track
                .as_ref()
                .map(|t| t.title.clone())
                .unwrap_or_default();
            next.artist = st
                .track
                .as_ref()
                .map(|t| t.artist.clone())
                .unwrap_or_default();
            next.album = st
                .track
                .as_ref()
                .map(|t| t.album.clone())
                .unwrap_or_default();
            next.uri = st
                .track
                .as_ref()
                .and_then(|t| t.uri.clone())
                .unwrap_or_default();
            next.duration_ms = st.track.as_ref().map_or(0, |t| t.duration_ms);
            next.is_playing = st.is_playing;
            next.shuffle = st.shuffle;
            next.repeat = repeat_to_i32(st.repeat);
            if let (Some(volume), false) = (st.volume, hold_volume) {
                next.volume = i32::from(volume);
                next.volume_memory.observe(next.volume);
            }
            next.can_control = can_control;
            next.queue_editable = queue_editable(&st, account.signed_in, self.env.audio.is_ready());
            next.queue_clearable = next.queue_editable;
            next.device_name = st.device_name.clone();
            next.has_session = st.active;
            next.dj_available = account.signed_in
                && account.is_premium
                && self.env.audio.is_ready()
                && dj_available;
            next.dj_active = st.own_engine && st.active && st.dj.active;
            next.dj_speaking = next.dj_active && st.dj.speech.is_some();
            next.dj_line = st
                .dj
                .speech
                .as_ref()
                .filter(|_| next.dj_speaking)
                .map(|s| s.line.clone())
                .unwrap_or_default();
            if track_changed {
                next.liked = false;
            }
            if !account.signed_in {
                next.liked = false;
            }
            if *v == next {
                false
            } else {
                *v = next;
                true
            }
        });

        drop(volume_guard);
        self.refresh_metadata();
        self.publish_event_position();

        if cover_changed {
            self.start_cover();
        }
        if let Some(uri) = check_liked {
            self.check_liked(uri);
        }
        self.media_state.send_replace(st);
        if queue_changed {
            self.advance_queue();
            self.refresh_queue();
        }
    }

    pub fn metadata_current(&self, generation: u64) -> bool {
        self.metadata_generation.load(Ordering::SeqCst) == generation
    }
    pub fn current_metadata_generation(&self) -> u64 {
        self.metadata_generation.load(Ordering::SeqCst)
    }
    fn refresh_metadata(self: &Arc<Self>) {
        let view = self.view.borrow().clone();
        let engine = self.env.audio.engine();
        let key = (
            view.uri.clone(),
            engine.as_ref().map_or(0, |e| Arc::as_ptr(e) as usize),
            self.env.account.snapshot().signed_in,
        );
        let generation = {
            let mut inner = self.lock();
            if inner.metadata_key == key {
                return;
            }
            inner.metadata_key = key;
            self.metadata_generation.fetch_add(1, Ordering::SeqCst) + 1
        };
        self.view.send_modify(|v| {
            v.artist_id.clear();
            v.album_id.clear();
        });
        let Some(engine) = engine else { return };
        if view.uri.is_empty() {
            return;
        }
        let this = self.clone();
        self.env.rt.spawn(async move {
            let item = velo_player::playlist::PlaylistItem {
                uri: view.uri.clone(),
                added_at: None,
            };
            let track = velo_player::playlist::tracks_in(engine.session(), &[item])
                .await
                .into_iter()
                .next();
            if !this.metadata_current(generation) || this.view.borrow().uri != view.uri {
                return;
            }
            if let Some(track) = track {
                this.apply_metadata(generation, &view.uri, &track);
            }
        });
    }
    fn apply_metadata(&self, generation: u64, uri: &str, track: &Track) {
        let _guard = self.lock();
        if !self.metadata_current(generation) || self.view.borrow().uri != uri {
            return;
        }
        self.view.send_modify(|v| {
            v.artist_id = track
                .artists
                .first()
                .and_then(|a| a.id.clone())
                .unwrap_or_default();
            v.album_id = track.album_id.clone().unwrap_or_default();
        });
    }
    pub fn play_history_entry(self: &Arc<Self>, id: &str) {
        let Some(entry) = self.history.entry(id) else {
            return;
        };
        let this = self.clone();
        let generation = self.begin_playback();
        self.env.rt.spawn(async move {
            let result: velo_domain::Result<()> = async {
                let Some(route) = this.prepare_playback().await? else {
                    return Ok(());
                };
                if !this.playback_current(generation) {
                    return Ok(());
                }
                match route {
                    PlayRoute::Velo => {
                        let engine = this.env.audio.engine().ok_or(VeloError::NoActiveDevice)?;
                        engine
                            .load(velo_player::LoadTarget::Tracks {
                                uris: vec![entry.uri.clone()],
                                index: 0,
                                shuffle: false,
                            })
                            .map_err(|e| VeloError::Other(audio_message(&e)))?;
                    }
                    PlayRoute::WebApi => {
                        let api = this.env.account.api().ok_or(VeloError::NotAuthenticated)?;
                        api.play_uris(&[entry.uri], None).await?;
                    }
                }
                Ok(())
            }
            .await;
            if let Err(e) = result {
                this.env.report(&e);
            }
        });
    }

    fn set_position(&self, pos: u64) {
        self.env.position.send_if_modified(|p| {
            if *p == pos {
                false
            } else {
                *p = pos;
                true
            }
        });
    }

    fn publish_position(&self) {
        let flags = self.env.flags.borrow();
        let mut inner = self.lock();
        let now = Instant::now();
        if !inner.position_gate.accept(&flags, now) {
            return;
        }
        let pos = inner.clock.now_ms(now);
        drop(inner);
        self.set_position(pos);
    }

    /// An action or source event must reach lyrics and Qt even while paused or hidden.
    fn publish_event_position(&self) {
        let pos = self.lock().clock.now_ms(Instant::now());
        self.set_position(pos);
    }

    fn start_cover(self: &Arc<Self>) {
        let mut i = self.lock();
        let track = i.track.clone();
        let expected_key = track.as_ref().and_then(cover_key);
        let this = self.clone();
        let generation = self.cover_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let blur = self.env.prefs.settings().background.cover_blur;
        let speech = self.view.borrow().dj_speaking;
        if expected_key.is_none() || (speech && self.view.borrow().cover_url.is_empty()) {
            self.view.send_modify(|v| {
                v.cover_url = if speech {
                    LIVI_ART_URL.into()
                } else {
                    String::new()
                };
                v.blur_url.clear();
                v.palette = None;
            });
        }
        let handle = self.env.rt.spawn(async move {
            let Some(track) = track else {
                return;
            };
            let mut assets = None;
            if let Some(url) = &track.cover_url {
                match this.env.covers.process_with_blur(url, blur).await {
                    Ok(a) => assets = Some(a),
                    Err(e) => tracing::debug!(%e, "cover download failed"),
                }
            }
            if assets.is_none() {
                if let Some(bytes) = &track.thumbnail {
                    assets = this
                        .env
                        .covers
                        .process_bytes_with_blur(&bytes_key(&track), bytes.as_ref().clone(), blur)
                        .await
                        .ok();
                }
            }
            if assets.is_none() && speech {
                assets = this
                    .env
                    .covers
                    .process_bytes_with_blur("velo-livi-fallback-v1", LIVI_ART.to_vec(), blur)
                    .await
                    .ok();
            }
            let guard = this.lock();
            if this.cover_generation.load(Ordering::SeqCst) != generation
                || guard.cover_key != expected_key
            {
                return;
            }
            // Cover, blur and palette change in one step, including confirmed failure.
            this.view.send_modify(|v| {
                if let Some(a) = assets {
                    v.cover_url = file_url(&a.cover);
                    v.blur_url = if a.blurred.as_os_str().is_empty() {
                        String::new()
                    } else {
                        file_url(&a.blurred)
                    };
                    v.palette = Some(a.palette);
                } else {
                    v.cover_url = if speech {
                        LIVI_ART_URL.into()
                    } else {
                        String::new()
                    };
                    v.blur_url.clear();
                    v.palette = None;
                }
            });
        });
        if let Some(old) = i.cover_task.replace(handle.abort_handle()) {
            old.abort();
        }
    }

    fn check_liked(self: &Arc<Self>, uri: String) {
        let Some(api) = self.env.account.api() else {
            return;
        };
        let this = self.clone();
        self.env.rt.spawn(async move {
            if let Ok(saved) = api.is_saved(&uri).await {
                if this.lock().liked_uri.as_deref() == Some(uri.as_str()) {
                    this.view.send_modify(|v| v.liked = saved);
                }
            }
        });
    }

    // ------------------------------------------------------------------ background loops

    /// 4 Hz position clock, alive only while playing with the window visible.
    async fn ticker(self: Arc<Self>) {
        let mut flags = self.env.flags.subscribe();
        loop {
            let f = *flags.borrow_and_update();
            if should_tick(&f) {
                self.publish_position();
                let mut interval = tokio::time::interval(TICK);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                interval.tick().await;
                loop {
                    tokio::select! {
                        _ = interval.tick() => self.publish_position(),
                        changed = flags.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            break;
                        }
                    }
                }
            } else if flags.changed().await.is_err() {
                return;
            }
        }
    }

    async fn visibility_refresh(self: Arc<Self>) {
        let mut flags = self.env.flags.subscribe();
        let mut visible = flags.borrow_and_update().visible;
        while flags.changed().await.is_ok() {
            let now = flags.borrow_and_update().visible;
            if now && !visible {
                for s in &self.sources {
                    s.refresh(Refresh::Visible);
                }
            }
            visible = now;
        }
    }

    async fn volume_loop(self: Arc<Self>, mut rx: watch::Receiver<Option<u8>>) {
        while rx.changed().await.is_ok() {
            let Some(v) = *rx.borrow_and_update() else {
                continue;
            };
            let source = self.active_source();
            // Velo's own engine applies the volume locally (Spirc already debounces the
            // network update), so a drag follows the hand; remote devices go over the Web API.
            let coalesce = if source.state().borrow().own_engine {
                VOLUME_COALESCE_LOCAL
            } else {
                VOLUME_COALESCE
            };
            if let Err(e) = source.act(SourceAction::Volume(v)).await {
                self.env.report(&e);
            }
            // Coalesces a drag into a few requests; this sleep only follows a request.
            tokio::time::sleep(coalesce).await;
        }
    }

    // ------------------------------------------------------------------ actions

    fn dispatch(self: &Arc<Self>, action: SourceAction) {
        let this = self.clone();
        let generation = self.play_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.env.rt.spawn(async move {
            let route = if action == SourceAction::Play {
                match this.prepare_playback().await {
                    Ok(Some(route)) if this.playback_current(generation) => route,
                    Ok(_) => {
                        this.recompute();
                        return;
                    }
                    Err(e) => {
                        this.env.report(&e);
                        this.recompute();
                        return;
                    }
                }
            } else {
                this.play_route()
            };
            let source = if action == SourceAction::Play && route == PlayRoute::Velo {
                this.sources
                    .iter()
                    .find(|s| s.state().borrow().own_engine)
                    .cloned()
                    .unwrap_or_else(|| this.active_source())
            } else {
                this.active_source()
            };
            if let Err(e) = source.act(action).await {
                this.env.report(&e);
            }
            source.refresh(Refresh::AfterAction);
        });
    }

    fn can_control(&self) -> bool {
        self.view.borrow().can_control
    }

    pub fn play_pause(self: &Arc<Self>) {
        let account = self.env.account.snapshot();
        if !self.can_control() && !(account.signed_in && account.is_premium) {
            return;
        }
        let play = !self.view.borrow().is_playing;
        self.view.send_modify(|v| v.is_playing = play);
        let has_session = self.view.borrow().has_session;
        self.lock().clock.set_playing(play, Instant::now());
        self.media_state.send_modify(|state| {
            let now = Instant::now();
            if state.is_playing {
                state.position_ms = state.position_ms.saturating_add(
                    now.saturating_duration_since(state.position_at).as_millis() as u64,
                );
            }
            state.position_at = now;
            state.is_playing = play;
        });
        self.env.update_flags(|f| f.playing = play && has_session);
        self.publish_event_position();
        self.dispatch(if play {
            SourceAction::Play
        } else {
            SourceAction::Pause
        });
    }

    pub fn next(self: &Arc<Self>) {
        if self.can_control() {
            self.dispatch(SourceAction::Next);
        }
    }

    pub fn previous(self: &Arc<Self>) {
        if self.can_control() {
            self.dispatch(SourceAction::Previous);
        }
    }

    pub fn seek(self: &Arc<Self>, ms: i64) {
        if !self.can_control() {
            return;
        }
        let (duration, playing) = {
            let v = self.view.borrow();
            (v.duration_ms, v.is_playing)
        };
        let mut ms = ms.max(0) as u64;
        if duration > 0 {
            ms = ms.min(duration);
        }
        self.lock().clock.set(ms, Instant::now(), playing);
        self.media_state.send_modify(|state| {
            state.position_ms = ms;
            state.position_at = Instant::now();
        });
        self.publish_event_position();
        self.dispatch(SourceAction::Seek(ms));
    }

    pub fn set_volume(self: &Arc<Self>, volume: i32) {
        if !self.can_control() {
            return;
        }
        let v = volume.clamp(0, 100) as u8;
        let mut guard = self.lock();
        guard.volume_set_at = Some(Instant::now());
        self.view.send_modify(|view| {
            view.volume = i32::from(v);
            view.volume_memory.observe(view.volume);
        });
        self.volume.send_replace(Some(v));
        let spawn = !std::mem::replace(&mut guard.volume_recheck, true);
        drop(guard);
        if spawn {
            self.recheck_volume_after_hold();
        }
    }

    /// Al vencer el hold se reconcilia una vez con el volumen real (si la orden falló, se
    /// revierte el valor optimista). Tarea de un solo disparo: sin timer en reposo.
    fn recheck_volume_after_hold(self: &Arc<Self>) {
        let this = self.clone();
        self.env.rt.spawn(async move {
            loop {
                let remaining = {
                    let mut g = this.lock();
                    let left = g.volume_set_at.map_or(Duration::ZERO, |t| {
                        (t + VOLUME_HOLD).saturating_duration_since(Instant::now())
                    });
                    if left.is_zero() {
                        g.volume_recheck = false;
                        break;
                    }
                    left
                };
                tokio::time::sleep(remaining.max(Duration::from_millis(10))).await;
            }
            this.recompute_volume();
        });
    }

    fn recompute_volume(&self) {
        let held = self
            .lock()
            .volume_set_at
            .is_some_and(|t| Instant::now().saturating_duration_since(t) < VOLUME_HOLD);
        let Some(volume) = self.media_state.borrow().volume.filter(|_| !held) else {
            return;
        };
        self.view.send_if_modified(|v| {
            if v.volume == i32::from(volume) {
                return false;
            }
            v.volume = i32::from(volume);
            v.volume_memory.observe(v.volume);
            true
        });
    }

    pub fn toggle_shuffle(self: &Arc<Self>) {
        if !self.can_control() {
            return;
        }
        let on = !self.view.borrow().shuffle;
        self.view.send_modify(|v| v.shuffle = on);
        self.dispatch(SourceAction::Shuffle(on));
    }

    pub fn cycle_repeat(self: &Arc<Self>) {
        if !self.can_control() {
            return;
        }
        let mode = next_repeat(self.view.borrow().repeat);
        self.view.send_modify(|v| v.repeat = repeat_to_i32(mode));
        self.dispatch(SourceAction::Repeat(mode));
    }

    pub fn toggle_liked(self: &Arc<Self>) {
        let Some(uri) = self.lock().liked_uri.clone() else {
            return;
        };
        let Some(api) = self.env.account.api() else {
            return;
        };
        let liked = !self.view.borrow().liked;
        self.view.send_modify(|v| v.liked = liked);
        let this = self.clone();
        self.env.rt.spawn(async move {
            if let Err(e) = api.set_saved(&uri, liked).await {
                this.env.report(&e);
                if this.lock().liked_uri.as_deref() == Some(uri.as_str()) {
                    this.view.send_modify(|v| v.liked = !liked);
                }
            }
        });
    }

    /// M1 sleep timer: pauses if playing; never starts playback.
    pub fn pause(self: &Arc<Self>) {
        if self.view.borrow().is_playing {
            self.play_pause();
        }
    }

    /// M1: a menu saved/removed `uri`; keep the heart of the playing song in sync.
    pub fn refresh_liked_if_current(&self, uri: &str, saved: bool) {
        if self.lock().liked_uri.as_deref() == Some(uri) {
            self.view
                .send_if_modified(|v| std::mem::replace(&mut v.liked, saved) != saved);
        }
    }

    pub fn refresh_queue(self: &Arc<Self>) {
        let generation = self.queue_refresh_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if !self.env.account.snapshot().signed_in {
            self.set_queue_status("unavailable", "");
            return;
        }
        self.set_queue_status("loading", "");
        let this = self.clone();
        self.env.rt.spawn(async move {
            let _operation = this.queue_operation.lock().await;
            if this.queue_refresh_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let before = this.media_state.borrow().clone();
            match this
                .env
                .account
                .queue_snapshot(before.own_engine, before.shuffle)
                .await
            {
                Ok((ticket, snapshot)) => {
                    this.env.account.commit_if_current(ticket, || {
                        let _state_guard = this.lock();
                        if this.queue_refresh_generation.load(Ordering::SeqCst) != generation {
                            return;
                        }
                        let current = before
                            .track
                            .as_ref()
                            .filter(|_| before.active)
                            .and_then(|t| t.uri.as_deref());
                        let tracks = match current {
                            Some(uri) => queue::align_upcoming(
                                uri,
                                snapshot.current.as_deref(),
                                snapshot.tracks,
                            ),
                            None if before.active => Some(snapshot.tracks),
                            None => Some(Vec::new()),
                        };
                        let Some(tracks) = tracks else {
                            this.set_queue_status("unavailable", "");
                            return;
                        };
                        let editable_current = current
                            .filter(|uri| {
                                before.own_engine && this.engine_track_uri().as_deref() == Some(uri)
                            })
                            .map(str::to_owned);
                        *this.queue_snapshot() = QueueSnapshot {
                            current: editable_current,
                            anchor: current.map(str::to_owned),
                            tracks: tracks.clone(),
                        };
                        this.publish_queue(&tracks);
                        this.set_queue_status("ready", "");
                    });
                }
                Err(e) if this.queue_refresh_generation.load(Ordering::SeqCst) == generation => {
                    tracing::debug!(error = %e, "queue snapshot unavailable");
                    this.set_queue_status("unavailable", "No se pudo actualizar la cola.");
                }
                Err(_) => {}
            }
        });
    }

    /// Avance inmediato por evento, incluso si la Web API todavía describe la pista anterior.
    fn advance_queue(&self) {
        let state = self.media_state.borrow();
        let uri = state.track.as_ref().and_then(|t| t.uri.as_deref());
        let mut snapshot = self.queue_snapshot();
        // También con el motor propio: se conservan las filas quitando la que pasó a sonar,
        // así la cola no parpadea vacía hasta el próximo ClusterUpdate.
        let tracks = advance_rows(
            state.active && self.env.account.snapshot().signed_in,
            uri,
            snapshot.anchor.as_deref(),
            &snapshot.tracks,
        );
        // La respuesta de red confirmará el ancla antes de permitir una edición.
        *snapshot = QueueSnapshot {
            current: None,
            anchor: uri.map(str::to_owned),
            tracks: tracks.clone(),
        };
        drop(snapshot);
        self.publish_queue(&tracks);
    }

    fn set_queue_status(&self, status: &str, error: &str) {
        self.view.send_if_modified(|v| {
            if v.queue_status == status && v.queue_error == error {
                false
            } else {
                v.queue_status = status.into();
                v.queue_error = error.into();
                true
            }
        });
    }

    fn queue_snapshot(&self) -> std::sync::MutexGuard<'_, QueueSnapshot> {
        self.queue_snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn engine_track_uri(&self) -> Option<String> {
        self.env
            .audio
            .state
            .borrow()
            .track
            .as_ref()
            .map(|t| t.uri.clone())
    }

    fn publish_queue(&self, tracks: &[Track]) {
        let rows = tracks.iter().map(queue_row).collect();
        let gen_id = self.queue_gen.bump();
        self.ui.send(PlayerUi::Queue(ModelOp::Replace(rows)));
        self.fill_queue_thumbs(gen_id, tracks);
    }

    fn fill_queue_thumbs(&self, gen_id: u64, tracks: &[Track]) {
        let items = tracks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| image_of(&t.images).map(|u| (i, u)))
            .collect();
        let sender = self.ui.sender();
        self.thumbs.fill(
            self.queue_gen.clone(),
            gen_id,
            items,
            THUMB_PX,
            move |row, url| {
                let _ = sender.send(PlayerUi::Queue(ModelOp::SetField {
                    row,
                    field: "imageUrl",
                    value: json!(url),
                }));
            },
        );
    }

    /// Vacía TODO lo posterior a la pista actual, también el contexto no visible.
    /// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
    /// (`can_clear_queue`/`clear_queue`); Velo usa la recarga pública de Spirc de Q1.
    pub fn clear_queue(self: &Arc<Self>) {
        if !self.view.borrow().queue_clearable {
            let _ = self.env.errors.send(ONLY_VELO.into());
            return;
        }
        let Some(engine) = self.env.audio.engine() else {
            return;
        };
        let generation = self.begin_playback();
        let state = engine.state().borrow().clone();
        let current = state.track.as_ref().map(|t| t.uri.clone());
        let guard = ClearGuard {
            occurrence: current
                .as_deref()
                .and_then(|uri| engine.known_occurrence(uri)),
            current,
            generation,
            failures: state.failures,
        };
        let this = self.clone();
        self.env.rt.spawn(async move {
            let _operation = this.queue_operation.lock().await;
            let occurrence = guard
                .current
                .as_deref()
                .and_then(|uri| engine.known_occurrence(uri));
            if !this.retry_engine_current(&engine, generation)
                || !guard.accepts(
                    this.play_generation.load(Ordering::SeqCst),
                    &engine.state().borrow(),
                    occurrence,
                )
            {
                let _ = this
                    .env
                    .errors
                    .send(audio_message(&PlayerError::QueueChanged));
                return;
            }
            let Some(current) = guard.current else { return };
            // No hay cola parcial que preservar/resolver: descartamos también el tail entero.
            // Solo publicamos el vacío si Spirc aceptó el comando (la posición/pausa se conservan).
            match engine.replace_queue(&current, &[], &[]) {
                Ok(()) => {
                    *this.queue_snapshot() = QueueSnapshot {
                        anchor: Some(current.clone()),
                        current: Some(current),
                        tracks: Vec::new(),
                    };
                    this.publish_queue(&[]);
                    this.set_queue_status("ready", "");
                }
                Err(error) => {
                    let _ = this.env.errors.send(audio_message(&error));
                }
            }
        });
    }

    /// Guarda exactamente lo visible al invocar: canción actual + filas, incluso repetidas.
    pub fn save_queue_as_playlist(self: &Arc<Self>, name: String) {
        let Some(api) = self.env.account.api() else {
            self.env.report(&VeloError::NotAuthenticated);
            return;
        };
        let current = self.view.borrow().uri.clone();
        let visible = self
            .queue_snapshot()
            .tracks
            .iter()
            .map(|t| t.uri.clone())
            .collect::<Vec<_>>();
        let user = self.env.account.snapshot().user_id;
        let this = self.clone();
        self.env.rt.spawn(async move {
            match queue::save_as_playlist(api.as_ref(), &name, &current, &visible).await {
                Ok(saved) => {
                    // No publicar datos de una cuenta anterior tras cerrar/cambiar sesión.
                    let account = this.env.account.snapshot();
                    if !account.signed_in || account.user_id != user {
                        return;
                    }
                    // Incluso si falla un lote: la playlist creada (quizá parcial) existe.
                    this.ui.send(PlayerUi::RefreshPlaylists);
                    match saved.items_result {
                        Ok(()) => this.ui.send(PlayerUi::QueueSaved {
                            playlist_id: saved.playlist.id,
                            name: saved.playlist.name,
                        }),
                        Err(error) => {
                            let _ = this.env.errors.send(format!(
                                "Se creó la playlist «{}», pero no se pudo guardar toda la cola. {}",
                                saved.playlist.name, super::user_message(&error)
                            ));
                        }
                    }
                }
                Err(error) => this.env.report(&error),
            }
        });
    }

    /// `Player.removeFromQueue(index)`: index into `Player.queue` ("A continuación").
    pub fn remove_from_queue(self: &Arc<Self>, index: i32) {
        if let Ok(index) = usize::try_from(index) {
            self.edit_queue(QueueEdit::Remove(index));
        }
    }

    /// `Player.moveInQueue(from, to)`: `to` is the row's final index.
    pub fn move_in_queue(self: &Arc<Self>, from: i32, to: i32) {
        if let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) {
            self.edit_queue(QueueEdit::Move { from, to });
        }
    }

    /// `Player.skipToQueue(index)`: plays row `index` of `Player.queue`; the rows before it are
    /// dropped as if "next" had been pressed `index + 1` times. Only the local engine (Velo).
    pub fn skip_to_queue(self: &Arc<Self>, index: i32) {
        if let Ok(index) = usize::try_from(index) {
            self.edit_queue(QueueEdit::SkipTo(index));
        }
    }

    /// Shows the edit at once (model move/remove, no reset), then reloads Spirc with the new
    /// order (see `velo_player::queue`); on failure the previous rows come back.
    fn edit_queue(self: &Arc<Self>, edit: QueueEdit) {
        if !self.view.borrow().queue_editable {
            let _ = self.env.errors.send(ONLY_VELO.into());
            return;
        }
        let Some(engine) = self.env.audio.engine() else {
            return;
        };
        let api = self.env.account.api();
        let this = self.clone();
        self.env.rt.spawn(async move {
            let operation = this.queue_operation.lock().await;
            let queue_generation = this.queue_refresh_generation.load(Ordering::SeqCst);
            let (current, before) = {
                let s = this.queue_snapshot();
                (s.current.clone(), s.tracks.clone())
            };
            let Some(current) = current.filter(|c| this.engine_track_uri().as_ref() == Some(c))
            else {
                let _ = this
                    .env
                    .errors
                    .send(audio_message(&PlayerError::QueueChanged));
                drop(operation);
                this.refresh_queue();
                return;
            };
            let mut edited = before.clone();
            if !edit_queue(&mut edited, edit) {
                return;
            }
            // A valid skip is a user playback request: it cancels the DJ and any load still
            // resolving, whose late answer would otherwise replace the chosen song.
            let playback = matches!(edit, QueueEdit::SkipTo(_)).then(|| this.begin_playback());
            this.queue_snapshot().tracks = edited.clone();
            this.ui.send(PlayerUi::Queue(match edit {
                QueueEdit::Remove(row) => ModelOp::Remove(row),
                QueueEdit::Move { from, to } => ModelOp::Move { from, to },
                QueueEdit::SkipTo(_) => ModelOp::Replace(edited.iter().map(queue_row).collect()),
            }));
            // Thumbnails still loading would land on the old row numbers.
            let gen_id = this.queue_gen.bump();
            this.fill_queue_thumbs(gen_id, &edited);

            let visible: Vec<String> = before.iter().map(|t| t.uri.clone()).collect();
            let upcoming: Vec<String> = edited.iter().map(|t| t.uri.clone()).collect();
            let tail = this
                .context_tail(&engine, api.as_deref(), &current, &visible)
                .await;
            if playback.is_some_and(|g| !this.playback_current(g)) {
                // The user asked for something else meanwhile: that request owns the queue.
                drop(operation);
                this.refresh_queue();
                return;
            }
            let previous_anchor = this.queue_snapshot().anchor.clone();
            let applied = tail.and_then(|tail| match edit {
                QueueEdit::SkipTo(row) => {
                    // The target becomes the anchor before the track change arrives, so
                    // `advance_queue` does not consume it (or rows up to a duplicate) again.
                    skip_anchor(&mut this.queue_snapshot(), &before[row].uri);
                    engine.skip_to_queue(&current, &before[row].uri, &upcoming, &tail)
                }
                _ => engine.replace_queue(&current, &upcoming, &tail),
            });
            if let Err(e) = applied {
                this.queue_snapshot().anchor = previous_anchor;
                tracing::debug!(error = %e, "queue edit refused");
                let _ = this.env.errors.send(audio_message(&e));
                if this.queue_refresh_generation.load(Ordering::SeqCst) == queue_generation {
                    this.queue_snapshot().tracks = before.clone();
                    this.publish_queue(&before);
                }
            }
        });
    }

    /// The context songs after `visible` (the Web API queue cuts at about 20 rows), so an edit
    /// or a retry does not end the playlist early. Shuffled contexts stay shuffled.
    async fn context_tail(
        &self,
        engine: &Engine,
        _api: Option<&dyn MusicProvider>,
        current: &str,
        visible: &[String],
    ) -> Result<Vec<String>, PlayerError> {
        let known = engine
            .known_tracks()
            .is_some_and(|k| k.iter().any(|u| u == current));
        let context_uri = self
            .env
            .account
            .session_queue_snapshot()
            .filter(|s| s.current == current)
            .map(|s| s.context_uri);
        if !known && context_uri.as_ref().is_none_or(|s| s.is_empty()) {
            return Err(PlayerError::QueueContext);
        }
        let shuffle = engine.state().borrow().shuffle;
        // No reorder can preserve an unconfirmed shuffled tail; wait for a complete source.
        if shuffle {
            return Err(PlayerError::QueueContext);
        }
        let context = queue::context_tracks(engine, context_uri.as_deref(), current).await?;
        let tail = queue::tail_after(
            &context,
            current,
            visible,
            false,
            engine.known_occurrence(current),
        )?;
        Ok(tail)
    }
    /// `Player.inspectQueueTrack(index)`: membership only for the menu's row, not every row.
    pub fn inspect_queue_track(self: &Arc<Self>, index: i32) {
        self.queue_membership(index, None);
    }

    /// `Player.setQueueLiked(index, saved)`: "Me gusta" from the queue menu.
    pub fn set_queue_liked(self: &Arc<Self>, index: i32, saved: bool) {
        self.queue_membership(index, Some(saved));
    }

    fn queue_membership(self: &Arc<Self>, index: i32, saved: Option<bool>) {
        let Some(api) = self.env.account.api() else {
            return;
        };
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let Some(uri) = self
            .queue_snapshot()
            .tracks
            .get(index)
            .map(|t| t.uri.clone())
        else {
            return;
        };
        let generation = self.queue_gen.current();
        if let Some(saved) = saved {
            self.ui.send(PlayerUi::Queue(ModelOp::SetField {
                row: index,
                field: "liked",
                value: json!(saved),
            }));
        }
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result = match saved {
                Some(saved) => api.set_saved(&uri, saved).await.map(|()| saved),
                None => api.is_saved(&uri).await,
            };
            let liked = match (result, saved) {
                (Ok(liked), _) => liked,
                (Err(e), Some(saved)) => {
                    this.env.report(&e);
                    !saved
                }
                (Err(e), None) => {
                    this.env.report(&e);
                    return;
                }
            };
            if this.queue_gen.current() == generation {
                this.ui.send(PlayerUi::Queue(ModelOp::SetField {
                    row: index,
                    field: "liked",
                    value: json!(liked),
                }));
            }
            // The bar's heart follows when it is the song playing.
            if this.lock().liked_uri.as_deref() == Some(uri.as_str()) {
                this.view.send_modify(|v| v.liked = liked);
            }
        });
    }

    // ------------------------------------------------------------------ E2b: retries

    /// Watches the engine for songs Spotify failed to deliver. A transient failure (audio key
    /// timeout, broken stream) is reloaded after 1 s and then 3 s; otherwise, or when the
    /// retries are spent, a toast says the song was skipped (never silently). Event driven:
    /// the only sleeps are the retry waits.
    async fn failure_loop(self: Arc<Self>) {
        let mut state = self.env.audio.state.subscribe();
        let mut seen = state.borrow_and_update().failures;
        let mut book = queue::RetryBook::default();
        while state.changed().await.is_ok() {
            let (failures, failure) = {
                let s = state.borrow_and_update();
                (s.failures, s.last_failure.clone())
            };
            if failures == seen {
                continue;
            }
            seen = failures;
            let Some(failure) = failure else { continue };
            match book.next(&failure, Instant::now()) {
                Some(delay) => {
                    tracing::info!(?delay, "Spotify did not deliver the song; retrying");
                    let this = self.clone();
                    // Capturar antes de spawn: una acción anterior al primer poll también cancela.
                    let generation = self.play_generation.load(Ordering::SeqCst);
                    self.env.rt.spawn(async move {
                        this.retry(failure, failures, generation, delay).await
                    });
                }
                None => {
                    let _ = self.env.errors.send(skipped_message(&failure).into());
                }
            }
        }
    }

    fn retry_engine_current(&self, engine: &Arc<Engine>, generation: u64) -> bool {
        self.playback_current(generation)
            && self
                .env
                .audio
                .engine()
                .is_some_and(|e| Arc::ptr_eq(&e, engine))
    }

    async fn retry(
        self: Arc<Self>,
        failure: TrackFailure,
        failures: u32,
        generation: u64,
        delay: Duration,
    ) {
        let Some(engine) = self.env.audio.engine() else {
            return;
        };
        tokio::time::sleep(delay).await;
        // The user chose something else meanwhile, or the engine was replaced: leave it.
        if !self.retry_engine_current(&engine, generation) {
            return;
        }
        let _operation = self.queue_operation.lock().await;
        // El mutex también puede haber esperado a otra consulta/edición de cola.
        if !self.retry_engine_current(&engine, generation) {
            return;
        }
        let guard = {
            let state = engine.state();
            let state = state.borrow();
            if !state.active
                || state.failures != failures
                || state.last_failure.as_ref() != Some(&failure)
            {
                return;
            }
            RetryGuard::new(generation, &state)
        };
        let api = self.env.account.api();
        // What Spirc moved on to (the song after the failed one), then the rest of the queue.
        let now_playing = guard.current.clone().filter(|u| *u != failure.uri);
        let queued = match api.as_deref() {
            Some(api) => match api.queue().await {
                Ok(tracks) => tracks.into_iter().map(|t| t.uri).collect(),
                Err(e) => {
                    self.env.report(&e);
                    return;
                }
            },
            // Una lista explícita conocida permite resolver todo sin Web API. Lo desconocido
            // sigue fallando en context_tail; nunca se acepta una lista parcial por esta vía.
            None => Vec::new(),
        };
        let upcoming = retry_upcoming(now_playing, queued, &failure.uri);
        let (anchor, rest) = upcoming
            .split_first()
            .map_or((failure.uri.as_str(), &[][..]), |(uri, rest)| {
                (uri.as_str(), rest)
            });
        let tail = match self
            .context_tail(&engine, api.as_deref(), anchor, rest)
            .await
        {
            Ok(tail) => tail,
            Err(e) => {
                let _ = self.env.errors.send(audio_message(&e));
                return;
            }
        };
        // Última comprobación tras TODOS los await, justo antes de encolar la recarga.
        if !self.retry_engine_current(&engine, generation)
            || !guard.accepts(
                self.play_generation.load(Ordering::SeqCst),
                &engine.state().borrow(),
            )
        {
            return;
        }
        if let Err(e) = engine.retry_track(&failure, &upcoming, &tail) {
            tracing::debug!(error = %e, "retry refused");
            let _ = self.env.errors.send(skipped_message(&failure).into());
        }
    }

    pub fn begin_playback(&self) -> u64 {
        // Toda lista/canción pedida por el usuario invalida también un DJ que aún resuelve.
        if let Some(engine) = self.env.audio.engine() {
            engine.stop_dj();
        }
        self.play_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn playback_current(&self, generation: u64) -> bool {
        self.play_generation.load(Ordering::SeqCst) == generation
            && self.env.account.snapshot().signed_in
    }

    /// E2b: before new playback, with Velo's audio not enabled yet and no other device in
    /// charge, open the one-time audio login instead of failing with "no device". `None`:
    /// the login was cancelled or failed (its own toast already said why).
    pub async fn prepare_playback(self: &Arc<Self>) -> velo_domain::Result<Option<PlayRoute>> {
        let account = self.env.account.snapshot();
        if !account.signed_in {
            return Err(VeloError::NotAuthenticated);
        }
        if !account.is_premium {
            return Err(VeloError::PremiumRequired);
        }
        let other_active = self.sources.iter().any(|s| {
            let rx = s.state();
            let st = rx.borrow();
            st.active && !st.own_engine
        });
        if should_enable_audio(account.signed_in, self.env.audio.is_ready(), other_active) {
            // Confirm with Spotify once: a phone or speaker may be playing unseen by the sources.
            let api = self.env.account.api().ok_or(VeloError::NotAuthenticated)?;
            if api.devices().await?.iter().any(|d| d.is_active) {
                return Ok(Some(PlayRoute::WebApi));
            }
            if !self.env.audio.ensure_ready().await {
                return Ok(None);
            }
            return Ok(Some(PlayRoute::Velo));
        }
        Ok(Some(self.play_route()))
    }

    pub fn refresh_devices(self: &Arc<Self>) {
        let Some((api, ticket)) = self.env.account.api_ticket() else {
            return;
        };
        let this = self.clone();
        self.env.rt.spawn(async move {
            match api.devices().await {
                Ok(devices) => {
                    this.env.account.commit_if_current(ticket, || {
                        this.lock().devices_available = devices.iter().any(|d| d.id.is_some());
                        let rows = devices.iter().map(device_row).collect();
                        this.ui.send(PlayerUi::Devices(ModelOp::Replace(rows)));
                    });
                    this.recompute();
                }
                Err(e) => this.env.report(&e),
            }
        });
    }

    /// Where new playback (a playlist, an album...) should start. See `source::play_route`.
    pub fn play_route(&self) -> PlayRoute {
        let engine_ready = self.env.audio.engine().is_some();
        let mut velo_active = false;
        let mut other_active = false;
        for source in &self.sources {
            let st = source.state().borrow().clone();
            if st.own_engine {
                velo_active |= st.active;
            } else {
                other_active |= st.active;
            }
        }
        play_route(
            engine_ready,
            velo_active,
            other_active,
            self.env.audio.chosen_device().is_some(),
        )
    }

    pub fn transfer_to(self: &Arc<Self>, device_id: String) {
        let Some(api) = self.env.account.api() else {
            return;
        };
        self.begin_playback();
        // An explicit pick is respected later, when playback starts with no active device.
        let velo = self.env.audio.device_id();
        self.env.audio.set_chosen_device(
            (velo.as_deref() != Some(device_id.as_str())).then(|| device_id.clone()),
        );
        let this = self.clone();
        self.env.rt.spawn(async move {
            match api.transfer(&device_id, true).await {
                Ok(()) => {
                    // The Web API needs a moment to report the new active device.
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    this.refresh_devices();
                    this.resync();
                }
                Err(e) => this.env.report(&e),
            }
        });
    }
}

/// Role names of `Player.devices` (re-exported for the bridge).
pub const DEVICES_ROLES: &str = DEVICE_ROLES;

/// Joins `Engine::playback_progress` (request id, consumed PCM) with the engine state of the
/// same request (song) into one `ProgressSample`. Samples of a stale request are dropped.
async fn listen_to_engine(player: std::sync::Weak<PlayerCtl>, engine: Arc<Engine>) {
    let mut progress = engine.playback_progress();
    let state = engine.state();
    // Request ids restart with every engine: the session prefix keeps entry ids unique.
    let session = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_nanos());
    loop {
        let p = match progress.recv().await {
            Ok(p) => p,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        let Some(player) = player.upgrade() else {
            break;
        };
        let st = state.borrow().clone();
        let Some(track) = st.track.filter(|_| st.occurrence == p.play_request_id) else {
            continue;
        };
        let view = player.view.borrow().clone();
        let same = view.uri == track.uri;
        let context_uri = player
            .env
            .account
            .session_queue_snapshot()
            .filter(|s| s.current == track.uri)
            .map(|s| s.context_uri)
            .unwrap_or_default();
        let remote_image_url = track.cover_url.clone().unwrap_or_default();
        player.listening.send(history::ProgressSample {
            occurrence: format!("{session:x}-{}", p.play_request_id),
            effective_ms: p.played_ms,
            dj_voice: st.dj.speech.is_some(),
            entry: history::Entry {
                entry_id: String::new(),
                uri: track.uri,
                title: track.title,
                artist: track.artist,
                album: track.album,
                image_url: if same { view.cover_url } else { String::new() },
                remote_image_url,
                artist_id: if same { view.artist_id } else { String::new() },
                album_id: if same { view.album_id } else { String::new() },
                context_uri,
                duration_ms: track.duration_ms,
                played_at_ms: 0,
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_transition_keeps_assets_until_ready_and_clears_confirmed_absence_or_failure() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        env.account.view.send_modify(|v| v.signed_in = true);
        let player = controller(&env);
        player.view.send_modify(|v| {
            v.cover_url = "file:///previous.jpg".into();
            v.blur_url = "file:///previous.png".into();
        });
        let mut state = snapshot(0, false);
        state.track.as_mut().unwrap().thumbnail = Some(Arc::new(LIVI_ART.to_vec()));
        player.apply(0, state.clone());
        assert_eq!(player.view.borrow().cover_url, "file:///previous.jpg");
        assert_eq!(player.view.borrow().blur_url, "file:///previous.png");
        let mut view = player.view.subscribe();
        rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while view.borrow_and_update().cover_url == "file:///previous.jpg" {
                    view.changed().await.unwrap();
                }
            })
            .await
            .unwrap();
        });
        assert!(player.view.borrow().cover_url.starts_with("file:///"));
        assert!(!player.view.borrow().blur_url.is_empty());
        assert!(player.view.borrow().palette.is_some());
        state.track.as_mut().unwrap().thumbnail = None;
        player.apply(0, state.clone());
        assert!(player.view.borrow().cover_url.is_empty());
        assert!(player.view.borrow().blur_url.is_empty());

        player.view.send_modify(|v| {
            v.cover_url = "file:///retained.jpg".into();
            v.blur_url = "file:///retained.png".into();
        });
        state.track.as_mut().unwrap().title = "Invalid thumbnail".into();
        state.track.as_mut().unwrap().thumbnail = Some(Arc::new(vec![1, 2, 3]));
        player.apply(0, state.clone());
        assert_eq!(player.view.borrow().cover_url, "file:///retained.jpg");
        rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !view.borrow_and_update().cover_url.is_empty() {
                    view.changed().await.unwrap();
                }
            })
            .await
            .unwrap();
        });
        assert!(player.view.borrow().blur_url.is_empty());
        assert!(player.view.borrow().palette.is_none());
    }

    fn queue_json(current: &str, upcoming: &[&str]) -> serde_json::Value {
        let track =
            |id: &str| json!({"uri": format!("spotify:track:{id}"), "name":id, "type":"track"});
        json!({"currently_playing":track(current), "queue":upcoming.iter().map(|id| track(id)).collect::<Vec<_>>()})
    }

    fn queue_state(player: &Arc<PlayerCtl>, uri: &str, own_engine: bool) {
        let uri = format!("spotify:track:{uri}");
        // Las consultas de pertenencia no forman parte del escenario de cola.
        player.lock().liked_uri = Some(uri.clone());
        let mut state = snapshot(0, false);
        state.own_engine = own_engine;
        state.remote = !own_engine;
        state.device_name = if own_engine { "Velo" } else { "Teléfono" }.into();
        state.track.as_mut().unwrap().uri = Some(uri);
        player.apply(0, state);
    }

    async fn next_queue_rows(ui: &mut mpsc::UnboundedReceiver<PlayerUi>) -> Vec<String> {
        let PlayerUi::Queue(ModelOp::Replace(rows)) = ui.recv().await.unwrap() else {
            panic!("se esperaba actualizar las filas de cola");
        };
        rows.iter()
            .map(|r| r["uri"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn advance_rows_keeps_upcoming_minus_the_track_now_playing() {
        let row = |id: &str| Track {
            id: None,
            uri: format!("spotify:track:{id}"),
            title: id.into(),
            artists: vec![],
            album: String::new(),
            album_id: None,
            duration_ms: 1000,
            explicit: false,
            images: vec![],
            added_at: None,
        };
        let rows = ["b", "c", "d"].map(row).to_vec();
        let next = advance_rows(
            true,
            Some("spotify:track:b"),
            Some("spotify:track:a"),
            &rows,
        );
        assert_eq!(next, ["c", "d"].map(row).to_vec());
        assert!(advance_rows(false, Some("spotify:track:b"), None, &rows).is_empty());
    }

    #[test]
    fn d3_track_events_refresh_local_and_connect_queues_without_a_timer() {
        use super::super::r4b_test_support::{ManualTime, Server, runtime, settle};
        #[allow(clippy::single_element_loop)]
        // the local-engine queue now comes from the session (B3)
        for own_engine in [false] {
            let rt = runtime();
            let (_dir, env) = super::super::test_env(&rt);
            rt.block_on(async {
                let _time = ManualTime::new();
                let mut server = Server::new().await;
                env.account.set_test_provider(server.provider());
                let player = controller(&env);
                let mut ui = player.ui.take().unwrap();
                queue_state(&player, "a", own_engine);
                assert!(next_queue_rows(&mut ui).await.is_empty());
                let initial = server.next().await;
                assert_eq!(initial.path, "/me/player/queue");
                initial.respond(200, queue_json("a", &["b", "b", "c"]));
                assert_eq!(
                    next_queue_rows(&mut ui).await,
                    ["spotify:track:b", "spotify:track:b", "spotify:track:c"]
                );

                // TrackChanged tras Siguiente: avance inmediato y una petición por evento.
                queue_state(&player, "b", own_engine);
                assert_eq!(
                    next_queue_rows(&mut ui).await,
                    ["spotify:track:b", "spotify:track:c"]
                );
                let lagging = server.next().await;
                assert_eq!(lagging.path, "/me/player/queue");
                lagging.respond(200, queue_json("a", &["b", "b", "c"]));
                assert_eq!(
                    next_queue_rows(&mut ui).await,
                    ["spotify:track:b", "spotify:track:c"]
                );

                // Una respuesta actual conserva el duplicado pendiente de la canción que suena.
                player.refresh_queue();
                server
                    .next()
                    .await
                    .respond(200, queue_json("b", &["b", "c"]));
                assert_eq!(
                    next_queue_rows(&mut ui).await,
                    ["spotify:track:b", "spotify:track:c"]
                );
                settle().await;
                queue_state(&player, "b", own_engine); // Pausa/posición/volumen: misma pista.
                tokio::time::advance(Duration::from_secs(120)).await;
                settle().await;
                assert!(server.requests.try_recv().is_err(), "sin polling de cola");
                assert!(ui.try_recv().is_err(), "sin reset al actualizar transporte");
            });
        }
    }

    #[test]
    fn d3_obsolete_queue_reply_cannot_restore_a_previous_track_or_account() {
        use super::super::r4b_test_support::{ManualTime, Server, runtime, settle};
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        rt.block_on(async {
            let _time = ManualTime::new();
            let mut server = Server::new().await;
            env.account.set_test_provider(server.provider());
            let player = controller(&env);
            let mut ui = player.ui.take().unwrap();
            queue_state(&player, "a", false);
            assert!(next_queue_rows(&mut ui).await.is_empty());
            let old = server.next().await;
            queue_state(&player, "b", false);
            assert!(next_queue_rows(&mut ui).await.is_empty());
            queue_state(&player, "a", false); // A → B → A: la URI por sí sola no basta.
            assert!(next_queue_rows(&mut ui).await.is_empty());
            old.respond(200, queue_json("a", &["vieja"]));
            let current = server.next().await;
            assert!(
                ui.try_recv().is_err(),
                "no publicar la respuesta A anterior"
            );
            current.respond(200, queue_json("a", &["nueva"]));
            assert_eq!(next_queue_rows(&mut ui).await, ["spotify:track:nueva"]);

            player.refresh_queue();
            let old_account = server.next().await;
            env.account.set_test_provider(server.provider());
            old_account.respond(200, queue_json("a", &["otra-cuenta"]));
            settle().await;
            assert!(
                ui.try_recv().is_err(),
                "el ticket de sesión invalida la respuesta"
            );
            assert_eq!(player.queue_snapshot().tracks[0].uri, "spotify:track:nueva");
        });
    }

    pub(super) fn controller(env: &Arc<Env>) -> Arc<PlayerCtl> {
        let source = Arc::new(TestSource(watch::channel(SourceState::default()).0));
        PlayerCtl::new(env, vec![source])
    }

    #[test]
    fn skip_keeps_the_rest_and_a_duplicate_of_the_target_after_the_track_change() {
        let row = |u: &str| Track {
            id: None,
            uri: u.into(),
            title: String::new(),
            artists: vec![],
            album: String::new(),
            album_id: None,
            duration_ms: 1,
            explicit: false,
            images: vec![],
            added_at: None,
        };
        // Queue was [a, b, c, b]; skipping to the first `b` leaves [c, b].
        let mut snapshot = QueueSnapshot {
            current: Some("cur".into()),
            anchor: Some("cur".into()),
            tracks: vec![row("c"), row("b")],
        };
        // Old anchor: the new song is not found and the rows would be emptied.
        assert!(
            advance_rows(
                true,
                Some("b"),
                snapshot.anchor.as_deref(),
                &snapshot.tracks
            )
            .is_empty()
        );
        skip_anchor(&mut snapshot, "b");
        let rows = advance_rows(
            true,
            Some("b"),
            snapshot.anchor.as_deref(),
            &snapshot.tracks,
        );
        assert_eq!(
            rows.iter().map(|t| t.uri.as_str()).collect::<Vec<_>>(),
            ["c", "b"]
        );
    }

    #[test]
    fn a_new_playback_request_invalidates_a_pending_skip() {
        let rt = super::super::r4b_test_support::runtime();
        let (_dir, env) = super::super::test_env(&rt);
        let player = controller(&env);
        env.account.view.send_modify(|v| v.signed_in = true);
        let skip = player.begin_playback();
        assert!(player.playback_current(skip));
        let other = player.begin_playback();
        assert!(!player.playback_current(skip) && player.playback_current(other));
    }

    #[test]
    fn dj_view_gates_availability_and_voice_metadata_and_clears_for_remote_or_logout() {
        let rt = super::super::r4b_test_support::runtime();
        let (_dir, env) = super::super::test_env(&rt);
        let player = controller(&env);
        assert!(!player.view.borrow().dj_available);
        env.account.view.send_modify(|v| {
            v.signed_in = true;
            v.is_premium = true;
        });
        env.audio.view.send_modify(|v| v.ready = true);
        let mut engine = EngineState {
            active: true,
            dj_available: true,
            ..Default::default()
        };
        engine.dj.active = true;
        engine.dj.speech = Some(velo_player::DjSpeech {
            title: "Tu nueva onda".into(),
            artist: "DJ Livi".into(),
            album: "DJ".into(),
            image_url: None,
            line: "Vamos con música".into(),
            duration_ms: 5000,
            position_ms: 1000,
            position_at: Instant::now(),
            playing: true,
        });
        env.audio.state.send_replace(engine.clone());
        player.apply(0, crate::bridge::source_velo::state_of(&engine, "Velo"));
        {
            let view = player.view.borrow();
            assert!(view.dj_available && view.dj_active && view.dj_speaking && view.is_playing);
            assert_eq!(
                (
                    view.title.as_str(),
                    view.artist.as_str(),
                    view.dj_line.as_str()
                ),
                ("Tu nueva onda", "DJ Livi", "Vamos con música")
            );
            assert_eq!(view.cover_url, LIVI_ART_URL);
            assert!(view.uri.is_empty() && !view.liked && !view.queue_editable);
        }
        assert_eq!(*env.position.borrow(), 1000);
        engine.dj.speech.as_mut().unwrap().playing = false;
        env.audio.state.send_replace(engine.clone());
        player.apply(0, crate::bridge::source_velo::state_of(&engine, "Velo"));
        assert!(player.view.borrow().dj_speaking && !player.view.borrow().is_playing);
        env.account.view.send_modify(|v| v.is_premium = false);
        player.apply(0, snapshot(0, false));
        assert!(!player.view.borrow().dj_available);
        let remote = SourceState {
            own_engine: false,
            remote: true,
            ..snapshot(0, true)
        };
        player.apply(0, remote);
        assert!(!player.view.borrow().dj_active && player.view.borrow().dj_line.is_empty());
        env.account.view.send_modify(|v| v.signed_in = false);
        player.apply(0, crate::bridge::source_velo::state_of(&engine, "Velo"));
        assert!(!player.view.borrow().dj_available && !player.view.borrow().dj_active);
    }

    #[test]
    fn d2_logout_discards_metadata_and_rejects_later_source_events() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        env.account.view.send_modify(|v| v.signed_in = true);
        let player = controller(&env);
        player.apply(0, snapshot(1200, true));
        assert_eq!(player.view.borrow().title, "Marea");
        env.account.view.send_modify(|v| v.signed_in = false);
        player.apply(0, snapshot(1400, true));
        let view = player.view.borrow();
        assert!(view.title.is_empty() && view.artist.is_empty() && view.album.is_empty());
        assert!(!view.is_playing && !view.has_session && !view.can_control);
        assert!(player.lock().track.is_none());
        assert!(player.queue_snapshot().tracks.is_empty());
        assert_eq!(*env.position.borrow(), 0);
    }

    fn snapshot(position_ms: u64, playing: bool) -> SourceState {
        SourceState {
            active: true,
            controllable: true,
            own_engine: true,
            is_playing: playing,
            position_ms,
            track: Some(SourceTrack {
                title: "Marea".into(),
                artist: "Lumen".into(),
                album: "Atlas".into(),
                duration_ms: 200_000,
                uri: None,
                thumbnail: None,
                cover_url: None,
            }),
            ..SourceState::default()
        }
    }

    #[test]
    fn paused_seek_and_final_pause_publish_even_with_the_window_hidden() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        env.account.view.send_modify(|v| v.signed_in = true);
        let player = controller(&env);
        player.apply(0, snapshot(1000, false));
        player.seek(42_000);
        assert_eq!(*env.position.borrow(), 42_000);
        assert!(!env.flags.borrow().visible);
        player.apply(0, snapshot(10_000, true));
        let before = Instant::now() - Duration::from_secs(1);
        player.lock().clock.set(10_000, before, true);
        player.play_pause();
        let final_position = *env.position.borrow();
        assert!(!player.view.borrow().is_playing);
        assert!(final_position >= 11_000);
        player.publish_position();
        assert_eq!(*env.position.borrow(), final_position);
    }

    #[test]
    fn source_anchors_and_track_changes_publish_without_a_tick() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        env.account.view.send_modify(|v| v.signed_in = true);
        let player = controller(&env);
        player.apply(0, snapshot(80_000, false));
        assert_eq!(*env.position.borrow(), 80_000);
        let mut next = snapshot(1200, false);
        next.track.as_mut().unwrap().title = "Otra pista".into();
        player.apply(0, next);
        assert_eq!(*env.position.borrow(), 1200);
        player.apply(0, snapshot(7000, false));
        assert_eq!(*env.position.borrow(), 7000);
        player.apply(0, SourceState::default());
        assert_eq!(*env.position.borrow(), 0);
    }

    #[test]
    fn available_devices_enable_controls_without_an_active_session() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        env.account.view.send_modify(|v| {
            v.signed_in = true;
            v.is_premium = true;
        });
        let player = controller(&env);
        player.apply(0, SourceState::default());
        assert!(!player.can_control());
        player.lock().devices_available = true;
        player.apply(0, SourceState::default());
        assert!(player.can_control());
        assert!(!player.view.borrow().has_session);
        env.account.view.send_modify(|v| v.is_premium = false);
        player.apply(0, SourceState::default());
        assert!(!player.can_control());
    }

    #[test]
    fn s9_session_metadata_ids_and_a_b_a_generation_discard() {
        let rt = super::super::r4b_test_support::runtime();
        let (_dir, env) = super::super::test_env(&rt);
        env.account.view.send_modify(|v| v.signed_in = true);
        let player = controller(&env);
        let mut a = snapshot(0, true);
        a.track.as_mut().unwrap().uri = Some("spotify:track:a".into());
        player.apply(0, a.clone());
        let first = player.current_metadata_generation();
        let t = Track {
            id: None,
            uri: "spotify:track:a".into(),
            title: "a".into(),
            artists: vec![velo_domain::ArtistRef {
                id: Some("artist-session".into()),
                name: "Artist".into(),
            }],
            album: "Album".into(),
            album_id: Some("album-session".into()),
            duration_ms: 1000,
            explicit: false,
            images: vec![],
            added_at: None,
        };
        player.apply_metadata(first, &t.uri, &t);
        assert_eq!(player.view.borrow().artist_id, "artist-session");
        assert_eq!(player.view.borrow().album_id, "album-session");
        let mut b = a.clone();
        b.track.as_mut().unwrap().uri = Some("spotify:track:b".into());
        player.apply(0, b);
        assert!(player.view.borrow().artist_id.is_empty());
        player.apply(0, a);
        assert!(!player.metadata_current(first));
        player.apply_metadata(first, &t.uri, &t);
        assert!(player.view.borrow().album_id.is_empty());
        player.apply_metadata(player.current_metadata_generation(), &t.uri, &t);
        assert_eq!(player.view.borrow().album_id, "album-session");
    }

    #[test]
    fn s9_local_queue_without_session_snapshot_is_unavailable_not_ready_empty() {
        use super::super::r4b_test_support::{runtime, settle};
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        rt.block_on(async {
            env.account.view.send_modify(|v| v.signed_in = true);
            let player = controller(&env);
            queue_state(&player, "a", true);
            assert_eq!(player.view.borrow().queue_status, "loading");
            settle().await;
            assert_eq!(player.view.borrow().queue_status, "unavailable");
            assert!(player.queue_snapshot().tracks.is_empty());
            let generation = player.queue_refresh_generation.load(Ordering::SeqCst);
            let mut state = player.media_state.borrow().clone();
            state.shuffle = true;
            player.apply(0, state);
            assert!(player.queue_refresh_generation.load(Ordering::SeqCst) > generation);
            settle().await;
            assert_eq!(player.view.borrow().queue_status, "unavailable");
        });
    }

    struct VolumeSource {
        state: watch::Sender<SourceState>,
        requests: mpsc::UnboundedSender<u8>,
    }
    #[async_trait::async_trait]
    impl PlaybackSource for VolumeSource {
        fn name(&self) -> &'static str {
            "volume-fixture"
        }
        fn state(&self) -> watch::Receiver<SourceState> {
            self.state.subscribe()
        }
        async fn act(&self, a: SourceAction) -> velo_domain::Result<()> {
            if let SourceAction::Volume(v) = a {
                self.requests.send(v).unwrap();
            }
            Ok(())
        }
        fn refresh(&self, _: Refresh) {}
    }
    #[test]
    fn s9_failed_volume_command_reverts_when_the_hold_expires() {
        use super::super::r4b_test_support::{ManualTime, runtime, settle};
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        rt.block_on(async {
            let _time = ManualTime::new();
            env.account.view.send_modify(|v| {
                v.signed_in = true;
                v.is_premium = true;
            });
            let (tx, _requests) = mpsc::unbounded_channel();
            let mut state = snapshot(0, true);
            state.volume = Some(10);
            let source = Arc::new(VolumeSource {
                state: watch::channel(state.clone()).0,
                requests: tx,
            });
            let player = PlayerCtl::new(&env, vec![source]);
            player.apply(0, state);
            player.set_volume(50);
            assert_eq!(player.view.borrow().volume, 50);
            // Tokio virtual time does not move std::Instant (the production hold clock).
            player.lock().volume_set_at = Some(Instant::now() - VOLUME_HOLD);
            tokio::time::advance(Duration::from_millis(20)).await;
            settle().await;
            assert_eq!(player.view.borrow().volume, 10, "optimistic value reverted");
            assert!(!player.lock().volume_recheck);
        });
    }
    #[test]
    fn s9_volume_bursts_hold_and_last_command_reaches_local_and_remote() {
        use super::super::r4b_test_support::{ManualTime, runtime, settle};
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        rt.block_on(async {
            let _time = ManualTime::new();
            env.account.view.send_modify(|v| {
                v.signed_in = true;
                v.is_premium = true;
            });
            for local in [true, false] {
                let (tx, mut requests) = mpsc::unbounded_channel();
                let mut state = snapshot(0, true);
                state.own_engine = local;
                state.volume = Some(10);
                let source = Arc::new(VolumeSource {
                    state: watch::channel(state.clone()).0,
                    requests: tx,
                });
                let player = PlayerCtl::new(&env, vec![source]);
                player.apply(0, state.clone());
                let me = player.clone();
                let rx = player.volume.subscribe();
                let task = env.rt.spawn(async move {
                    me.volume_loop(rx).await;
                });
                player.set_volume(20);
                settle().await;
                assert_eq!(requests.recv().await, Some(20));
                for v in 21..=93 {
                    player.set_volume(v);
                }
                player.set_volume(0);
                player.apply(0, state.clone());
                assert_eq!(player.view.borrow().volume, 0);
                assert_eq!(player.view.borrow().volume_memory.audible(), 93);
                tokio::time::advance(if local {
                    VOLUME_COALESCE_LOCAL
                } else {
                    VOLUME_COALESCE
                })
                .await;
                settle().await;
                assert_eq!(requests.recv().await, Some(0));
                assert!(requests.try_recv().is_err());
                // Tokio virtual time does not move std::Instant (the production hold clock).
                player.lock().volume_set_at = Some(Instant::now() - VOLUME_HOLD);
                player.apply(0, state);
                assert_eq!(player.view.borrow().volume, 10);
                task.abort();
            }
        });
    }

    struct TestSource(watch::Sender<SourceState>);

    #[async_trait::async_trait]
    impl PlaybackSource for TestSource {
        fn name(&self) -> &'static str {
            "test-source"
        }
        fn state(&self) -> watch::Receiver<SourceState> {
            self.0.subscribe()
        }
        async fn act(&self, _: SourceAction) -> velo_domain::Result<()> {
            Ok(())
        }
        fn refresh(&self, _: Refresh) {}
    }

    #[test]
    fn a_third_source_can_take_priority_without_provider_specific_dispatch() {
        let own = Arc::new(TestSource(watch::channel(SourceState::default()).0));
        let local = Arc::new(TestSource(watch::channel(SourceState::default()).0));
        let remote = Arc::new(TestSource(watch::channel(SourceState::default()).0));
        let sources: Vec<Arc<dyn PlaybackSource>> =
            vec![own.clone(), local.clone(), remote.clone()];
        assert_eq!(active_source_index(&sources), 2);
        remote.0.send_modify(|s| s.active = true);
        assert_eq!(active_source_index(&sources), 2);
        local.0.send_modify(|s| s.active = true);
        assert_eq!(active_source_index(&sources), 1);
        own.0.send_modify(|s| s.active = true);
        assert_eq!(active_source_index(&sources), 0);
        own.0.send_modify(|s| s.active = false);
        assert_eq!(active_source_index(&sources), 1);
    }

    #[test]
    fn queue_is_editable_only_on_velo_with_a_spotify_song() {
        let mut st = snapshot(0, true);
        assert!(!queue_editable(&st, true, true), "no Spotify URI");
        st.track.as_mut().unwrap().uri = Some("spotify:track:a".into());
        assert!(queue_editable(&st, true, true));
        assert!(!queue_editable(&st, false, true), "signed out");
        assert!(!queue_editable(&st, true, false), "engine not ready");
        st.own_engine = false;
        assert!(!queue_editable(&st, true, true), "another device");
        st.own_engine = true;
        st.active = false;
        assert!(!queue_editable(&st, true, true), "Velo not active");
    }

    #[test]
    fn clear_on_another_device_reports_error_without_changing_the_visible_queue() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        env.account.view.send_modify(|v| v.signed_in = true);
        let player = controller(&env);
        let mut state = snapshot(1200, false);
        state.own_engine = false;
        state.track.as_mut().unwrap().uri = Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC".into());
        player.apply(0, state);
        let mut errors = env.errors.subscribe();
        let mut ui = player.ui.take().unwrap();
        // apply publicó el cambio de pista/dispositivo; el rechazo del vaciado no añade nada.
        assert!(
            matches!(ui.try_recv().unwrap(), PlayerUi::Queue(ModelOp::Replace(rows)) if rows.is_empty())
        );
        assert!(!player.view.borrow().queue_clearable);
        player.clear_queue();
        assert_eq!(errors.try_recv().unwrap(), ONLY_VELO);
        assert!(ui.try_recv().is_err());
        assert_eq!(player.play_generation.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn pending_clear_preserves_latest_transport_but_rejects_another_occurrence_or_action() {
        let mut state = EngineState {
            active: true,
            playing: true,
            track: Some(velo_player::EngineTrack {
                uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                duration_ms: 200_000,
                cover_url: None,
            }),
            ..EngineState::default()
        };
        let guard = ClearGuard {
            current: state.track.as_ref().map(|t| t.uri.clone()),
            occurrence: Some(1),
            generation: 7,
            failures: 0,
        };
        state.position_ms = 42_000;
        state.position_at += Duration::from_secs(2);
        state.playing = false;
        state.volume = 20;
        assert!(
            guard.accepts(7, &state, Some(1)),
            "se conserva el transporte más reciente"
        );
        assert!(
            !guard.accepts(8, &state, Some(1)),
            "otra acción del usuario"
        );
        assert!(
            !guard.accepts(7, &state, Some(2)),
            "misma URI, otra ocurrencia"
        );
        state.active = false;
        assert!(
            !guard.accepts(7, &state, Some(1)),
            "Connect cambió de dispositivo"
        );
        state.active = true;
        state.failures = 1;
        assert!(!guard.accepts(7, &state, Some(1)), "fallo nuevo de audio");
        state.failures = 0;
        state.track.as_mut().unwrap().uri = "spotify:track:0000000000000000000001".into();
        assert!(!guard.accepts(7, &state, Some(1)), "otra canción");
    }

    #[test]
    fn first_play_opens_the_audio_login_only_when_nothing_else_plays() {
        assert!(should_enable_audio(true, false, false));
        assert!(!should_enable_audio(true, true, false), "already enabled");
        assert!(
            !should_enable_audio(true, false, true),
            "phone or app playing"
        );
        assert!(!should_enable_audio(false, false, false), "signed out");
    }

    #[test]
    fn a_retry_plays_the_failed_song_then_what_spirc_moved_to_without_repeats() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        // The Web API already reflects the skip: queue starts after "b".
        assert_eq!(
            retry_upcoming(Some("b".into()), s(&["c", "d"]), "a"),
            s(&["b", "c", "d"])
        );
        // It lags: still lists "b" (or even the failed "a") first.
        assert_eq!(
            retry_upcoming(Some("b".into()), s(&["b", "c"]), "a"),
            s(&["b", "c"])
        );
        assert_eq!(
            retry_upcoming(Some("b".into()), s(&["a", "b", "c"]), "a"),
            s(&["b", "c"])
        );
        // The context ended: only what the queue still has.
        assert_eq!(retry_upcoming(None, s(&["c"]), "a"), s(&["c"]));
        assert!(retry_upcoming(None, vec![], "a").is_empty());
    }

    #[tokio::test]
    async fn pending_retry_rechecks_user_actions_after_a_delayed_reply() {
        let generation = Arc::new(AtomicU64::new(7));
        let state = EngineState {
            active: true,
            playing: true,
            failures: 1,
            last_failure: Some(TrackFailure {
                uri: "spotify:track:a".into(),
                position_ms: 0,
                transient: true,
            }),
            ..EngineState::default()
        };
        let guard = RetryGuard::new(7, &state);
        let (reply, pending) = tokio::sync::oneshot::channel();
        let current = generation.clone();
        let retry = tokio::spawn(async move {
            let state = pending.await.unwrap();
            guard.accepts(current.load(Ordering::SeqCst), &state)
        });
        // Equivale a Pausa/Siguiente/otra lista mientras la consulta de cola está pendiente.
        generation.fetch_add(1, Ordering::SeqCst);
        reply.send(state).unwrap();
        assert!(
            !retry.await.unwrap(),
            "la respuesta no debe reactivar la pista fallida"
        );
    }

    #[test]
    fn retry_rejects_external_transport_changes_and_superseded_failures() {
        let state = EngineState {
            active: true,
            playing: true,
            failures: 1,
            last_failure: Some(TrackFailure {
                uri: "spotify:track:a".into(),
                position_ms: 0,
                transient: true,
            }),
            track: Some(velo_player::EngineTrack {
                uri: "spotify:track:b".into(),
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                duration_ms: 200_000,
                cover_url: None,
            }),
            ..EngineState::default()
        };
        let guard = RetryGuard::new(7, &state);
        assert!(guard.accepts(7, &state));
        assert!(!guard.accepts(8, &state), "acción antes del primer poll");
        for change in 0..5 {
            let mut changed = state.clone();
            match change {
                0 => changed.active = false,
                1 => changed.playing = false,
                2 => changed.track.as_mut().unwrap().uri = "spotify:track:c".into(),
                3 => changed.position_at += Duration::from_secs(1), // seek desde Connect
                _ => changed.failures += 1,
            }
            assert!(!guard.accepts(7, &changed), "cambio {change}");
        }
        let mut volume_only = state.clone();
        volume_only.volume = 20;
        assert!(
            guard.accepts(7, &volume_only),
            "volumen no cancela el reintento"
        );
    }

    #[test]
    fn skipped_songs_always_get_a_toast_naming_the_cause() {
        let fail = |transient| TrackFailure {
            uri: "spotify:track:a".into(),
            position_ms: 0,
            transient,
        };
        assert!(skipped_message(&fail(true)).contains("tardó"));
        assert!(skipped_message(&fail(false)).contains("no tiene disponible"));
    }

    #[test]
    fn queue_rows_carry_artist_and_album_ids_for_the_menu() {
        let track = Track {
            id: Some("t".into()),
            uri: "spotify:track:t".into(),
            title: "T".into(),
            artists: vec![velo_domain::ArtistRef {
                id: Some("ar".into()),
                name: "A".into(),
            }],
            album: "B".into(),
            album_id: Some("al".into()),
            duration_ms: 1,
            explicit: false,
            images: vec![],
            added_at: None,
        };
        let row = queue_row(&track);
        assert_eq!(row["artistId"], "ar");
        assert_eq!(row["albumId"], "al");
        assert_eq!(row["liked"], false);
        for role in QUEUE_ROLES.split(',') {
            assert!(row.get(role).is_some(), "{role}");
        }
    }

    #[test]
    fn repeat_cycles_off_context_track() {
        assert_eq!(next_repeat(0), RepeatMode::Context);
        assert_eq!(next_repeat(1), RepeatMode::Track);
        assert_eq!(next_repeat(2), RepeatMode::Off);
        assert_eq!(repeat_to_i32(RepeatMode::Track), 2);
        assert_eq!(repeat_from_i32(7), RepeatMode::Off);
    }

    #[test]
    fn cover_key_prefers_url_over_bytes() {
        let mut t = SourceTrack {
            title: "t".into(),
            artist: "a".into(),
            album: "b".into(),
            duration_ms: 1,
            uri: None,
            thumbnail: Some(Arc::new(vec![1])),
            cover_url: None,
        };
        assert_eq!(cover_key(&t).as_deref(), Some("smtc:a|b|t"));
        t.cover_url = Some("https://x/y.jpg".into());
        assert_eq!(cover_key(&t).as_deref(), Some("url:https://x/y.jpg"));
        t.cover_url = None;
        t.thumbnail = None;
        assert_eq!(cover_key(&t), None);
    }
}
