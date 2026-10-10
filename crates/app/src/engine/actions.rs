//! M1 context-menu actions shared by every list (`Actions` singleton):
//! library membership of any URI, "add album/playlist to queue", "go to artist/album" of the
//! song that is playing, and the sleep timer. Network work runs on tokio; results reach QML as
//! eventos. Minutos usan un sleep; fin local usa EOF/PCM; remoto sigue los eventos del transporte.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use velo_domain::{MusicProvider, Result, Track, VeloError};

use super::player::PlayerCtl;
use super::{Bus, Env};
use crate::bridge::source::SourceState;
use crate::bridge::source::{activate_device, retry_device_action};
use tokio::task::AbortHandle;

/// Tracks queued by "Añadir a la cola" on an album or playlist (one Web API call each).
const CONTEXT_QUEUE_MAX: usize = 50;
/// `sleep_minutes` value meaning "at the end of this song".
pub const SLEEP_END_OF_TRACK: i32 = -1;

pub enum ActionsUi {
    /// Answer to `inspect_saved`: `None` while Spotify has not answered (or failed).
    Inspected {
        generation: u64,
        uri: String,
        saved: Option<bool>,
    },
    /// A `set_saved` succeeded; every row showing `uri` can update its heart.
    Saved { uri: String, saved: bool },
    /// "Ir al artista / álbum" of the playing song, resolved through `/me/player`.
    Navigate {
        kind: &'static str,
        id: String,
        generation: u64,
    },
    /// Sleep timer state: 0 off, `SLEEP_END_OF_TRACK`, or the minutes chosen.
    Sleep {
        generation: u64,
        minutes: i32,
        /// Unix ms when the minutes timer fires; 0 when there is no deadline.
        ends_at_ms: f64,
    },
    /// Créditos parciales de la URI solicitada; JSON documentado en CONTRATO.
    Credits { uri: String, json: String },
}

pub struct ActionsCtl {
    env: Arc<Env>,
    player: Arc<PlayerCtl>,
    pub ui: Bus<ActionsUi>,
    /// Bumped by every set/cancel; a sleeping task only fires if it is still current.
    sleep_task: Mutex<SleepTask>,
    inspection: Mutex<Inspection>,
    credits_generation: AtomicU64,
    dj_busy: AtomicBool,
}

/// `spotify:track:ID` → ("track", "ID").
pub fn parse_uri(uri: &str) -> Option<(&str, &str)> {
    let mut parts = uri.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("spotify"), Some(kind), Some(id), None) if !kind.is_empty() && !id.is_empty() => {
            Some((kind, id))
        }
        _ => None,
    }
}

/// Public link for "Compartir → Copiar enlace".
pub fn share_url(uri: &str) -> Option<String> {
    parse_uri(uri).map(|(kind, id)| format!("https://open.spotify.com/{kind}/{id}"))
}

impl ActionsCtl {
    pub fn start(env: &Arc<Env>, player: &Arc<PlayerCtl>) -> Arc<Self> {
        Arc::new(Self {
            env: env.clone(),
            player: player.clone(),
            ui: Bus::new(),
            sleep_task: Mutex::new(SleepTask::default()),
            inspection: Mutex::new(Inspection::default()),
            credits_generation: AtomicU64::new(0),
            dj_busy: AtomicBool::new(false),
        })
    }

    fn api(&self) -> Option<Arc<dyn MusicProvider>> {
        self.env.account.api()
    }

    pub fn request_credits(self: &Arc<Self>, uri: String) {
        let generation = self.credits_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(engine) = self.env.audio.engine() else {
            let _ = self
                .env
                .errors
                .send("Activa la reproducción en Velo para consultar créditos.".into());
            return;
        };
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result = velo_player::extras::credits(engine.session(), &uri).await;
            if this.credits_generation.load(Ordering::SeqCst) != generation
                || !this
                    .env
                    .audio
                    .engine()
                    .is_some_and(|current| Arc::ptr_eq(&current, &engine))
            {
                return;
            }
            match result {
                Ok(value) => this.ui.send(ActionsUi::Credits {
                    uri,
                    json: value.to_string(),
                }),
                Err(_) => {
                    let _ = this
                        .env
                        .errors
                        .send("Spotify no entregó créditos para esta canción.".into());
                }
            }
        });
    }

    /// DJ Livi siempre en el motor local, con la misma activación asíncrona de audio.
    pub fn start_dj(self: &Arc<Self>) {
        if self.dj_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let generation = self.player.begin_playback();
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result: Result<()> = async {
                let account = this.env.account.snapshot();
                if !account.signed_in {
                    return Err(VeloError::NotAuthenticated);
                }
                if !account.is_premium {
                    return Err(VeloError::PremiumRequired);
                }
                if !this.env.audio.ensure_ready().await || !this.player.playback_current(generation)
                {
                    return Ok(());
                }
                let engine = this.env.audio.engine().ok_or(VeloError::NoActiveDevice)?;
                this.env.audio.set_chosen_device(None);
                let result = engine.start_dj().await;
                if !this.player.playback_current(generation) {
                    engine.stop_dj();
                    return Ok(());
                }
                result.map_err(|error| VeloError::Other(super::audio::audio_message(&error)))
            }
            .await;
            this.dj_busy.store(false, Ordering::SeqCst);
            if let Err(error) = result {
                this.env.report(&error);
            }
        });
    }

    pub fn dj_next_segment(self: &Arc<Self>) {
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result = this
                .env
                .audio
                .engine()
                .ok_or(velo_player::PlayerError::DjInactive)
                .and_then(|engine| engine.dj_next_segment());
            if let Err(error) = result {
                let _ = this.env.errors.send(super::audio::audio_message(&error));
            }
        });
    }

    /// Compatibilidad E5: reemplazado por start_dj(), conserva el envío al dispositivo elegido.
    pub fn start_dj_on(self: &Arc<Self>, device_id: String) {
        let (Some(api), Some(engine)) = (self.api(), self.env.audio.engine()) else {
            let _ = self.env.errors.send(
                "Inicia sesión y activa el audio de Velo para enviar DJ al dispositivo oficial."
                    .into(),
            );
            return;
        };
        if self.dj_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result: Result<()> = async {
                let devices = api.devices().await?;
                if device_id == engine.device_id()
                    || !devices
                        .iter()
                        .any(|d| d.id.as_deref() == Some(&device_id) && d.is_active)
                {
                    return Err(VeloError::Other(
                        "Selecciona primero un dispositivo oficial activo para DJ Livi.".into(),
                    ));
                }
                if !this
                    .env
                    .audio
                    .engine()
                    .is_some_and(|current| Arc::ptr_eq(&current, &engine))
                {
                    return Err(VeloError::Other(
                        "La sesión de audio cambió; vuelve a intentarlo.".into(),
                    ));
                }
                velo_player::extras::start_dj(engine.session(), &device_id)
                    .await
                    .map_err(|_| {
                        VeloError::Other(
                            "Spotify no aceptó el arranque experimental de DJ Livi.".into(),
                        )
                    })
            }
            .await;
            this.dj_busy.store(false, Ordering::SeqCst);
            if let Err(error) = result {
                this.env.report(&error);
            }
        });
    }

    /// Asks Spotify whether `uri` is in the user's library (menus ask when they open).
    pub fn inspect_saved(self: &Arc<Self>, uri: String) {
        let generation = {
            let mut inspection = self.inspection.lock().unwrap_or_else(|e| e.into_inner());
            let generation = inspection.begin(uri.clone());
            self.ui.send(ActionsUi::Inspected {
                generation,
                uri: uri.clone(),
                saved: None,
            });
            generation
        };
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            let saved = api.is_saved(&uri).await.ok();
            this.inspected(generation, uri, saved);
        });
    }

    fn inspected(&self, generation: u64, uri: String, saved: Option<bool>) {
        let inspection = self.inspection.lock().unwrap_or_else(|e| e.into_inner());
        if inspection.accepts(generation, &uri) {
            self.ui.send(ActionsUi::Inspected {
                generation,
                uri,
                saved,
            });
        }
    }
    pub fn inspection_current(&self, generation: u64, uri: &str) -> bool {
        self.inspection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accepts(generation, uri)
    }

    /// Guardar invalida inspecciones anteriores de la misma URI, incluso ya encoladas en Qt.
    pub fn set_saved(self: &Arc<Self>, uri: String, saved: bool) {
        let Some(api) = self.api() else { return };
        let generation = {
            let mut inspection = self.inspection.lock().unwrap_or_else(|e| e.into_inner());
            if inspection.uri == uri {
                Some(inspection.begin(uri.clone()))
            } else {
                None
            }
        };
        let this = self.clone();
        self.env.rt.spawn(async move {
            match api.set_saved(&uri, saved).await {
                Ok(()) => {
                    this.ui.send(ActionsUi::Saved {
                        uri: uri.clone(),
                        saved,
                    });
                    if let Some(generation) = generation {
                        this.inspected(generation, uri.clone(), Some(saved));
                    }
                    if parse_uri(&uri).is_some_and(|(kind, _)| kind == "playlist") {
                        super::core().library.refresh_playlists();
                    }
                    this.player.refresh_liked_if_current(&uri, saved);
                }
                Err(e) => this.env.report(&e),
            }
        });
    }
    /// Queues the tracks of an album or playlist (first CONTEXT_QUEUE_MAX), or one track.
    pub fn add_context_to_queue(self: &Arc<Self>, uri: String) {
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result: Result<()> = async {
                let uris = context_tracks(&*api, &uri).await?;
                for track in uris {
                    let call = || {
                        let api = api.clone();
                        let track = track.clone();
                        async move { api.add_to_queue(&track).await }
                    };
                    match call().await {
                        Err(VeloError::NoActiveDevice) => {
                            activate_device(&this.env, &*api, true).await?;
                            retry_device_action(call).await?;
                        }
                        other => other?,
                    }
                }
                Ok(())
            }
            .await;
            match result {
                Ok(()) => this.player.refresh_queue(),
                Err(e) => this.env.report(&e),
            }
        });
    }

    /// "Ir al artista / álbum" of the song that is playing on any device.
    pub fn open_current(self: &Arc<Self>, kind: String) {
        let kind = match kind.as_str() {
            "artist" => "artist",
            "album" => "album",
            _ => return,
        };
        let generation = self.player.current_metadata_generation();
        let this = self.clone();
        self.env.rt.spawn(async move {
            let mut views = this.player.view.subscribe();
            let target = tokio::time::timeout(Duration::from_secs(12), async {
                loop {
                    if !this.player.metadata_current(generation) {
                        return None;
                    }
                    let v = views.borrow_and_update().clone();
                    let id = if kind == "artist" {
                        v.artist_id
                    } else {
                        v.album_id
                    };
                    if !id.is_empty() {
                        return Some(id);
                    }
                    if v.uri.is_empty() || views.changed().await.is_err() {
                        return None;
                    }
                }
            })
            .await
            .ok()
            .flatten();
            if this.player.metadata_current(generation) {
                if let Some(id) = target {
                    this.ui.send(ActionsUi::Navigate {
                        kind,
                        id,
                        generation,
                    });
                }
            }
        });
    }

    /// 0 cancels; `SLEEP_END_OF_TRACK` pauses when the current song ends; else minutes.
    pub fn set_sleep_timer(self: &Arc<Self>, minutes: i32) {
        let mut task = self.sleep_task.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(engine) = task.engine.take().and_then(|e| e.upgrade()) {
            engine.cancel_track_end_sleep();
        }
        if let Some(previous) = task.abort.take() {
            previous.abort();
        }
        task.generation = task.generation.wrapping_add(1);
        let generation = task.generation;
        let states = self.player.media_state.subscribe();
        let identity = sleep_identity(&states.borrow());
        if minutes == 0
            || minutes < SLEEP_END_OF_TRACK
            || (minutes == SLEEP_END_OF_TRACK && identity.is_none())
        {
            self.ui.send(ActionsUi::Sleep {
                generation,
                minutes: 0,
                ends_at_ms: 0.0,
            });
            return;
        }
        let local = if minutes == SLEEP_END_OF_TRACK && states.borrow().own_engine {
            self.env.audio.engine().and_then(|engine| {
                let uri = states.borrow().track.as_ref()?.uri.clone()?;
                match engine.sleep_at_track_end(&uri) {
                    Ok(timer) => {
                        let weak = Arc::downgrade(&engine);
                        task.engine = Some(weak.clone());
                        Some((weak, timer))
                    }
                    Err(e) => {
                        let _ = self.env.errors.send(super::audio::audio_message(&e));
                        None
                    }
                }
            })
        } else {
            None
        };
        if minutes == SLEEP_END_OF_TRACK && states.borrow().own_engine && local.is_none() {
            self.ui.send(ActionsUi::Sleep {
                generation,
                minutes: 0,
                ends_at_ms: 0.0,
            });
            return;
        }
        self.ui.send(ActionsUi::Sleep {
            generation,
            minutes,
            ends_at_ms: sleep_deadline_ms(unix_now_ms(), minutes),
        });
        let this = self.clone();
        let worker = self.env.rt.spawn(async move {
            let due = if minutes > 0 {
                tokio::time::sleep(Duration::from_secs(minutes as u64 * 60)).await;
                true
            } else {
                match local {
                    Some((engine, mut timer)) => {
                        let mut states = states;
                        loop {
                            if *timer.borrow_and_update() != velo_player::TrackEndTimerState::Armed { break; }
                            tokio::select! {
                                changed = timer.changed() => { if changed.is_err() { break; } }
                                changed = states.changed() => {
                                    if changed.is_err() { break; }
                                    let state = states.borrow().clone();
                                    if let Some(engine) = engine.upgrade() {
                                        if !state.active || !state.own_engine { engine.cancel_track_end_sleep(); break; }
                                        engine.refresh_track_end_sleep();
                                    } else { break; }
                                }
                            }
                        }
                        false // El motor ya paró en el límite del PCM, o canceló por carga manual.
                    }
                    None => wait_track_end(states, identity.clone()).await,
                }
            };
            // Serializar con cancelar/reemplazar; abortar también suelta inmediatamente el Arc.
            let task = this.sleep_task.lock().unwrap_or_else(|e| e.into_inner());
            if task.generation == generation {
                if due
                    && (minutes > 0
                        || sleep_identity(&this.player.media_state.borrow()) == identity)
                {
                    this.player.pause();
                }
                this.ui.send(ActionsUi::Sleep {
                    generation,
                    minutes: 0,
                    ends_at_ms: 0.0,
                });
            }
        });
        task.abort = Some(worker.abort_handle());
    }
    pub fn sleep_current(&self, generation: u64) -> bool {
        self.sleep_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation
            == generation
    }
}

#[derive(Default)]
struct SleepTask {
    generation: u64,
    abort: Option<AbortHandle>,
    engine: Option<Weak<velo_player::Engine>>,
}
/// Unix ms at which a `minutes` timer fires; 0 for off / end-of-song.
pub fn sleep_deadline_ms(now_ms: f64, minutes: i32) -> f64 {
    if minutes > 0 {
        now_ms + f64::from(minutes) * 60_000.0
    } else {
        0.0
    }
}

fn unix_now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

type SleepIdentity = Option<(String, String, bool)>;
async fn wait_track_end(
    mut states: tokio::sync::watch::Receiver<SourceState>,
    identity: SleepIdentity,
) -> bool {
    loop {
        let state = states.borrow_and_update().clone();
        if sleep_identity(&state) != identity {
            return false;
        }
        if let Some(deadline) = track_deadline(&state, tokio::time::Instant::now().into_std()) {
            tokio::select! {
                biased;
                changed = states.changed() => { if changed.is_err() { return false; } }
                _ = tokio::time::sleep_until(deadline.into()) => {
                    let latest = states.borrow().clone();
                    let now = tokio::time::Instant::now().into_std();
                    if sleep_identity(&latest) == identity && latest.is_playing
                        && track_deadline(&latest, now).is_some_and(|d| d <= now) { return true; }
                }
            }
        } else if states.changed().await.is_err() {
            return false;
        }
    }
}
#[derive(Default)]
struct Inspection {
    generation: u64,
    uri: String,
}
impl Inspection {
    fn begin(&mut self, uri: String) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.uri = uri;
        self.generation
    }
    fn accepts(&self, generation: u64, uri: &str) -> bool {
        self.generation == generation && self.uri == uri
    }
}

fn sleep_identity(state: &SourceState) -> Option<(String, String, bool)> {
    let track = state
        .track
        .as_ref()
        .filter(|t| state.active && t.duration_ms > 0)?;
    let uri = track.uri.clone().unwrap_or_else(|| {
        format!(
            "{}|{}|{}|{}",
            track.title, track.artist, track.album, track.duration_ms
        )
    });
    Some((uri, state.device_name.clone(), state.own_engine))
}
fn track_deadline(state: &SourceState, now: Instant) -> Option<Instant> {
    if !state.is_playing {
        return None;
    }
    let duration = state.track.as_ref()?.duration_ms;
    let elapsed = now.saturating_duration_since(state.position_at).as_millis() as u64;
    Some(
        now + Duration::from_millis(
            duration.saturating_sub(state.position_ms.saturating_add(elapsed)),
        ),
    )
}
async fn context_tracks(api: &dyn MusicProvider, uri: &str) -> Result<Vec<String>> {
    let (kind, id) = parse_uri(uri).ok_or(VeloError::NotFound)?;
    let page: Vec<Track> = match kind {
        "track" | "episode" => return Ok(vec![uri.to_string()]),
        "album" => {
            api.album_tracks(id, 0, CONTEXT_QUEUE_MAX as u32)
                .await?
                .items
        }
        "playlist" => {
            api.playlist_tracks(id, 0, CONTEXT_QUEUE_MAX as u32)
                .await?
                .items
        }
        _ => return Err(VeloError::Forbidden("queue".into())),
    };
    Ok(page
        .into_iter()
        .map(|t| t.uri)
        .filter(|u| !u.is_empty())
        .take(CONTEXT_QUEUE_MAX)
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn sleep_deadline_only_for_minutes() {
        assert_eq!(super::sleep_deadline_ms(1_000.0, 2), 121_000.0);
        assert_eq!(super::sleep_deadline_ms(1_000.0, 0), 0.0);
        assert_eq!(
            super::sleep_deadline_ms(1_000.0, super::SLEEP_END_OF_TRACK),
            0.0
        );
    }

    use super::*;

    #[test]
    fn dj_actions_report_inactive_and_premium_errors_without_opening_audio() {
        let rt = super::super::r4b_test_support::runtime();
        let (_dir, env) = super::super::test_env(&rt);
        let player = PlayerCtl::for_test(&env);
        let actions = ActionsCtl::start(&env, &player);
        rt.block_on(async {
            let mut errors = env.errors.subscribe();
            actions.dj_next_segment();
            assert_eq!(
                errors.recv().await.unwrap(),
                "DJ Livi no está sonando en Velo."
            );
            env.account.view.send_modify(|v| {
                v.signed_in = true;
                v.is_premium = false;
            });
            actions.start_dj();
            assert_eq!(
                errors.recv().await.unwrap(),
                "Esta acción requiere Spotify Premium."
            );
            assert!(!env.audio.view.borrow().busy && env.audio.engine().is_none());
            assert!(!actions.dj_busy.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn uris_parse_and_share_as_open_spotify_links() {
        assert_eq!(parse_uri("spotify:album:abc"), Some(("album", "abc")));
        assert_eq!(parse_uri("spotify:user:me:collection"), None);
        assert_eq!(parse_uri("http://x"), None);
        assert_eq!(
            share_url("spotify:track:4cOdK2wGLETKBW3PvgPWqT").as_deref(),
            Some("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT")
        );
        assert_eq!(share_url("spotify:track:"), None);
    }

    #[test]
    fn m1_end_of_song_follows_pause_seek_and_cancels_on_track_or_device_change() {
        use crate::bridge::source::SourceTrack;
        let now = Instant::now();
        let mut state = SourceState {
            active: true,
            is_playing: true,
            position_ms: 40_000,
            position_at: now,
            track: Some(SourceTrack {
                title: "A".into(),
                artist: "X".into(),
                album: "Y".into(),
                duration_ms: 100_000,
                uri: Some("spotify:track:a".into()),
                thumbnail: None,
                cover_url: None,
            }),
            ..SourceState::default()
        };
        let identity = sleep_identity(&state);
        assert_eq!(
            track_deadline(&state, now),
            Some(now + Duration::from_secs(60))
        );
        state.is_playing = false;
        assert_eq!(track_deadline(&state, now + Duration::from_secs(90)), None);
        assert_eq!(sleep_identity(&state), identity);
        state.is_playing = true;
        state.position_at = now + Duration::from_secs(90);
        assert_eq!(
            track_deadline(&state, state.position_at),
            Some(now + Duration::from_secs(150))
        );
        state.position_ms = 95_000;
        assert_eq!(
            track_deadline(&state, state.position_at),
            Some(now + Duration::from_secs(95))
        );
        state.position_ms = 5_000;
        assert_eq!(
            track_deadline(&state, state.position_at),
            Some(now + Duration::from_secs(185))
        );
        state.track.as_mut().unwrap().uri = Some("spotify:track:b".into());
        assert_ne!(sleep_identity(&state), identity);
        state.track.as_mut().unwrap().uri = Some("spotify:track:a".into());
        state.device_name = "Teléfono".into();
        assert_ne!(sleep_identity(&state), identity);
    }
    #[test]
    fn m2_inverted_inspections_and_save_invalidate_older_responses() {
        let mut state = Inspection::default();
        let a = state.begin("a".into());
        let b = state.begin("b".into());
        assert!(state.accepts(b, "b"));
        assert!(!state.accepts(a, "a"));
        let saved = state.begin("b".into());
        assert!(!state.accepts(b, "b"));
        assert!(state.accepts(saved, "b"));
    }
    #[tokio::test(start_paused = true)]
    async fn m1_watch_waits_without_deadline_in_pause_recomputes_seek_and_aborts_on_change() {
        use crate::bridge::source::SourceTrack;
        let mut state = SourceState {
            active: true,
            is_playing: true,
            position_at: tokio::time::Instant::now().into_std(),
            track: Some(SourceTrack {
                title: "A".into(),
                artist: "X".into(),
                album: "Y".into(),
                duration_ms: 10_000,
                uri: Some("spotify:track:a".into()),
                thumbnail: None,
                cover_url: None,
            }),
            ..SourceState::default()
        };
        let identity = sleep_identity(&state);
        let (tx, rx) = tokio::sync::watch::channel(state.clone());
        let task = tokio::spawn(wait_track_end(rx, identity.clone()));
        tokio::task::yield_now().await;
        state.is_playing = false;
        state.position_ms = 2_000;
        tx.send_replace(state.clone());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(30)).await;
        assert!(!task.is_finished(), "No vence mientras está en pausa");
        state.is_playing = true;
        state.position_at = tokio::time::Instant::now().into_std();
        tx.send_replace(state.clone());
        tokio::task::yield_now().await;
        state.position_ms = 9_000;
        state.position_at = tokio::time::Instant::now().into_std();
        tx.send_replace(state.clone());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(500)).await;
        assert!(!task.is_finished());
        state.position_ms = 1_000;
        state.position_at = tokio::time::Instant::now().into_std();
        tx.send_replace(state.clone());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(!task.is_finished(), "Seek atrás extiende el plazo");
        state.track.as_mut().unwrap().uri = Some("spotify:track:b".into());
        tx.send_replace(state.clone());
        assert!(!task.await.unwrap(), "Cambiar cancela, no pausa B");
        let rx = tx.subscribe();
        let identity = sleep_identity(&state);
        let finish = tokio::spawn(wait_track_end(rx, identity));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(
            finish.await.unwrap(),
            "La nueva pista vence desde su ancla vigente"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn m1_cancel_aborts_wait_and_releases_captured_state() {
        let held = Arc::new(());
        let weak = Arc::downgrade(&held);
        let task = tokio::spawn(async move {
            let _held = held;
            tokio::time::sleep(Duration::from_secs(900)).await;
        });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(weak.upgrade().is_none());
    }
}
