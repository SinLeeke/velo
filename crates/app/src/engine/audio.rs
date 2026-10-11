//! Velo's own audio: the librespot engine (`velo-player`) published as the Spotify Connect
//! device "Velo". Owns its lifecycle (start at launch when the keyring holds the audio login,
//! one-time browser login, reconnection, sign out, exit) and mirrors its state for the
//! `velo-engine` playback source and the `Account` singleton.
//!
//! Everything runs on the tokio runtime. No timer: the forwarder waits on the engine's watch
//! channels; the only sleeps are a short, bounded reconnection backoff after the engine died.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use super::reconnect::ReconnectBudget;
use tokio::sync::{broadcast, watch};
use velo_player::session_api::{Session, SessionSource};
use velo_player::{Engine, EngineConfig, EngineState, PlayerError, login};
use velo_spotify::{MemoryTokenStore, TokenStore};

#[path = "settings_runtime.rs"]
pub mod settings_runtime;
use settings_runtime::{EngineHooks, SettingsRuntime};

/// The running engine's session; waits for a (re)connection, bounded.
struct EngineSession(std::sync::Weak<AudioCtl>);

/// Startup and the reconnection backoff fit in this wait.
const SESSION_WAIT: Duration = Duration::from_secs(20);

#[async_trait::async_trait]
impl SessionSource for EngineSession {
    fn connect_devices(&self) -> Option<Vec<velo_domain::Device>> {
        self.0.upgrade()?.engine()?.connect_devices()
    }

    async fn session(&self) -> Option<Session> {
        let audio = self.0.upgrade()?;
        let mut view = audio.view.subscribe();
        if !view.borrow().ready {
            tokio::time::timeout(SESSION_WAIT, view.wait_for(|v| v.ready))
                .await
                .ok()?
                .ok()?;
        }
        audio.engine().map(|engine| engine.session().clone())
    }
}

/// `Account.audioReady` / `Account.audioBusy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AudioView {
    pub ready: bool,
    pub busy: bool,
}

/// After an unexpected stop (network loss): retry this many times, then wait for the user.
const RECONNECT_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(10),
    Duration::from_secs(30),
];
const EXIT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct AudioCtl {
    rt: tokio::runtime::Handle,
    name: String,
    engine: RwLock<Option<Arc<Engine>>>,
    pub view: watch::Sender<AudioView>,
    /// Reason text for `Account.audioFailed`.
    pub failures: broadcast::Sender<String>,
    /// State of the running engine; inactive default without one.
    pub state: watch::Sender<EngineState>,
    /// The app's toast channel (`Player.error`).
    errors: broadcast::Sender<String>,
    /// Serializes start, login, sign out and exit.
    operation: tokio::sync::Mutex<()>,
    /// Bumped whenever the current engine is replaced or removed on purpose.
    generation: AtomicU64,
    cancellation: watch::Sender<u64>,
    /// Device the user picked in the device selector this session (None: Velo / default).
    chosen_device: Mutex<Option<String>>,
    reconnects: Mutex<ReconnectBudget>,
    /// Volumen que estaba sonando antes de una caída; nunca sustituir mute por 50 %.
    reconnect_volume: Mutex<Option<u8>>,
    preferences: RwLock<Option<Arc<super::prefs::Prefs>>>,
    /// Mixear por contexto: la playlist marcada pide fundido cruzado.
    mix_context: AtomicBool,
    hooks: RwLock<Option<EngineHooks>>,
    hot_settings: Mutex<Option<HotSettings>>,
    pub settings_runtime: Arc<SettingsRuntime>,
}

/// Spanish text for an engine error (shown in Settings and as a toast).
pub fn audio_message(e: &PlayerError) -> String {
    match e {
        PlayerError::NotSignedIn => "Activa la reproducción en Velo para escuchar aquí.",
        PlayerError::LoginCancelled => "Se canceló el inicio de sesión de audio.",
        PlayerError::LoginTimeout => {
            "El inicio de sesión de audio tardó demasiado. Vuelve a intentarlo."
        }
        PlayerError::LoginPortBusy => {
            "No se pudo abrir el inicio de sesión: otra aplicación usa el puerto 8898."
        }
        PlayerError::LoginFailed | PlayerError::OAuthRejected => {
            "Spotify no aceptó el inicio de sesión de audio."
        }
        PlayerError::TokenStore => {
            "No se pudo guardar la sesión de audio en el Administrador de credenciales."
        }
        PlayerError::OAuthNetwork | PlayerError::SessionTimeout => {
            "No se pudo conectar con Spotify para reproducir."
        }
        PlayerError::Session => {
            "Spotify no abrió la sesión de audio. Reproducir en Velo requiere Spotify Premium."
        }
        PlayerError::AudioBackend => "No hay ninguna salida de audio disponible en Windows.",
        PlayerError::Stopped => "El motor de audio de Velo se detuvo.",
        PlayerError::InvalidUri => "Esto no se puede reproducir en Velo.",
        PlayerError::QueueContext => "No se pudo recuperar la cola completa. Vuelve a intentarlo.",
        PlayerError::QueueChanged => {
            "La canción cambió mientras editabas la cola. Vuelve a intentarlo."
        }
        PlayerError::DjUnavailable => "No se pudo iniciar DJ Livi. Vuelve a intentarlo.",
        PlayerError::DjInactive => "DJ Livi no está sonando en Velo.",
        PlayerError::InvalidOptions => "Esos ajustes de audio no son válidos.",
        PlayerError::AudioCache => "No se pudo abrir la caché de audio.",
    }
    .into()
}

impl AudioCtl {
    pub fn new(
        rt: tokio::runtime::Handle,
        errors: broadcast::Sender<String>,
        name: &str,
    ) -> Arc<Self> {
        Arc::new(Self {
            rt,
            name: name.to_string(),
            engine: RwLock::new(None),
            view: watch::channel(AudioView::default()).0,
            failures: broadcast::channel(4).0,
            state: watch::channel(EngineState::default()).0,
            errors,
            operation: tokio::sync::Mutex::new(()),
            generation: AtomicU64::new(0),
            cancellation: watch::channel(0).0,
            chosen_device: Mutex::new(None),
            reconnects: Mutex::new(ReconnectBudget::default()),
            reconnect_volume: Mutex::new(None),
            preferences: RwLock::new(None),
            mix_context: AtomicBool::new(false),
            hooks: RwLock::new(Some(EngineHooks::player())),
            hot_settings: Mutex::new(None),
            settings_runtime: Arc::new(SettingsRuntime::new()),
        })
    }

    /// The profile is known: the audio cache belongs to this Spotify account. On an account
    /// switch the engine reopens so it stops writing into the previous account's cache.
    pub async fn note_account_owner(self: &Arc<Self>, user_id: &str) {
        if let Some(previous) = settings_runtime::write_audio_owner(user_id).await {
            if previous != user_id && self.engine().is_some() {
                let _ = self.reconnect_playback(false).await;
            }
        }
    }

    /// The running engine, if any.
    pub fn engine(&self) -> Option<Arc<Engine>> {
        self.engine
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn bind_settings(&self, prefs: Arc<super::prefs::Prefs>) {
        self.settings_runtime.saved(&prefs.settings());
        *self.preferences.write().unwrap_or_else(|e| e.into_inner()) = Some(prefs);
    }

    /// C2's construction/DSP/output boundary; registration does no work on the UI thread.
    #[cfg(test)]
    pub fn set_engine_hooks(&self, hooks: EngineHooks) {
        *self.hooks.write().unwrap_or_else(|e| e.into_inner()) = Some(hooks);
    }
    pub fn settings_supported(&self) -> bool {
        self.hooks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Reevalúa Mixear para el contexto que va a sonar y lo empuja al motor en caliente.
    pub fn set_mix_context(&self, wanted: bool) {
        if self.mix_context.swap(wanted, Ordering::SeqCst) == wanted {
            return;
        }
        let prefs = self
            .preferences
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(prefs) = prefs {
            self.publish_hot_settings(&prefs.settings());
        }
    }

    pub fn publish_hot_settings(&self, settings: &velo_domain::Settings) {
        let hooks = *self.hooks.read().unwrap_or_else(|e| e.into_inner());
        if let (Some(hooks), Some(engine)) = (hooks, self.engine()) {
            let generation = self.generation.load(Ordering::SeqCst);
            let eq = settings.equalizer.config();
            let buffer = settings.playback.output_buffer_ms;
            let private = settings.playback.private_session;
            let mix = self.mix_config(settings);
            let mut hot = self.hot_settings.lock().unwrap_or_else(|e| e.into_inner());
            if hot
                .as_ref()
                .is_none_or(|previous| previous.0 != generation || previous.1 != eq)
            {
                (hooks.equalizer)(&engine, eq);
            }
            if hot
                .as_ref()
                .is_none_or(|previous| previous.0 != generation || previous.2 != buffer)
            {
                if let Err(e) = (hooks.output_buffer)(&engine, buffer) {
                    self.fail(&e);
                    return;
                }
            }
            // The initial value travels in EngineConfig; here only changes (or a new engine
            // generation) are forwarded to Spirc.
            if private_session_needs_apply(hot.as_ref(), generation, private) {
                if let Err(e) = (hooks.private_session)(&engine, private) {
                    self.fail(&e);
                    return;
                }
            }
            // El valor inicial viaja en EngineOptions.mix; aquí solo cambios o motor nuevo.
            if mix_needs_apply(hot.as_ref(), generation, mix) {
                (hooks.mix)(&engine, mix);
            }
            *hot = Some((generation, eq, buffer, private, mix));
        }
    }

    /// El ajuste automático se lee en caliente; el contexto solo guarda la elección por lista.
    pub(super) fn mix_config(&self, settings: &velo_domain::Settings) -> velo_domain::MixConfig {
        let mut mix = settings.playback.mix();
        mix.enabled |= self.mix_context.load(Ordering::SeqCst);
        mix
    }

    fn engine_options(
        &self,
        settings: &velo_domain::Settings,
        dir: std::path::PathBuf,
    ) -> velo_domain::EngineOptions {
        let mut options =
            settings.engine_options(dir, velo_domain::proxy_route().borrow().url.clone());
        options.mix = self.mix_config(settings);
        options
    }

    /// Settings reconnect preserves the keyring login and rejects stale work after logout.
    pub async fn reconnect_playback(
        self: &Arc<Self>,
        clear_cache: bool,
    ) -> Result<(), PlayerError> {
        let ticket;
        let reopen;
        {
            let _operation = self.operation.lock().await;
            reopen = !clear_cache || self.engine().is_some() || self.view.borrow().busy;
            if let Some(engine) = self.engine() {
                *self
                    .reconnect_volume
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(engine.state().borrow().volume);
            }
            let detached = self.detach();
            ticket = self.generation.load(Ordering::SeqCst);
            self.cancellation.send_modify(|v| *v = v.wrapping_add(1));
            self.view.send_modify(|v| v.busy = true);
            self.settings_runtime
                .view
                .send_modify(|v| v.reconnecting = reopen);
            if let Some(engine) = detached {
                engine.shutdown().await;
            }
            if clear_cache && settings_runtime::clear_audio_directory().await.is_err() {
                self.view.send_modify(|v| v.busy = false);
                return Err(PlayerError::AudioBackend);
            }
        }
        if !reopen {
            self.view.send_modify(|v| v.busy = false);
            return Ok(());
        }
        let result = self.launch_if_current(false, Some(ticket)).await;
        if let Err(e) = &result {
            self.fail(e);
        }
        result
    }

    /// The Connect device id of the running engine.
    pub fn device_id(&self) -> Option<String> {
        self.engine().map(|e| e.device_id().to_string())
    }

    pub fn is_ready(&self) -> bool {
        self.view.borrow().ready
    }

    pub fn chosen_device(&self) -> Option<String> {
        self.chosen_device
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Remembers the user's pick in the device selector (`None` when it is Velo itself).
    pub fn set_chosen_device(&self, id: Option<String>) {
        *self.chosen_device.lock().unwrap_or_else(|e| e.into_inner()) = id;
    }

    /// At launch: starts the engine in the background when an audio login is stored.
    pub fn start(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move {
            if !login::has_stored_login(login::keyring_store()).await {
                return;
            }
            if let Err(e) = this.launch(false).await {
                tracing::warn!(error = %e, "audio engine unavailable at startup");
            }
        });
    }

    /// `Account.enableAudio()`: the one-time browser login, then the engine.
    pub fn enable(self: &Arc<Self>) {
        if !self.view.send_if_modified(|v| {
            if v.busy || v.ready {
                false
            } else {
                v.busy = true;
                true
            }
        }) {
            return;
        }
        let this = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = this.launch(true).await {
                tracing::warn!(error = %e, "enabling audio failed");
                this.fail(&e);
            }
            this.view.send_modify(|v| v.busy = false);
        });
    }

    /// All play entry points share one browser attempt. Subscribe before starting so a
    /// fast completion/cancellation cannot be missed; failure is already sent as a toast.
    pub async fn ensure_ready(self: &Arc<Self>) -> bool {
        let mut view = self.view.subscribe();
        self.enable();
        if view.wait_for(|v| v.ready || !v.busy).await.is_err() {
            return false;
        }
        self.engine().is_some()
    }

    fn fail(&self, e: &PlayerError) {
        self.settings_runtime
            .view
            .send_modify(|v| v.playback_error = true);
        let message = audio_message(e);
        let _ = self.failures.send(message.clone());
        let _ = self.errors.send(message);
    }

    async fn launch(self: &Arc<Self>, interactive: bool) -> Result<(), PlayerError> {
        self.launch_if_current(interactive, None).await
    }

    fn config(&self) -> EngineConfig {
        // Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
        // backend.rs:2599 (carry_volume): la nueva sesión arranca al nivel anterior.
        let mut config = EngineConfig::velo(&self.name);
        // Name is deliberately assigned after deriving the old stable Velo device id.
        // Renaming the device therefore never changes its identity.
        if let Some(prefs) = self
            .preferences
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let playback = prefs.settings().playback;
            config.name = playback.device_name;
            config.private_session = playback.private_session;
        }
        if let Some(volume) = *self
            .reconnect_volume
            .lock()
            .unwrap_or_else(|e| e.into_inner())
        {
            config.initial_volume = volume;
        }
        config
    }

    async fn launch_if_current(
        self: &Arc<Self>,
        interactive: bool,
        expected: Option<u64>,
    ) -> Result<(), PlayerError> {
        let _operation = self.operation.lock().await;
        // Comprobar dentro de la exclusión: cerrar sesión puede haber ganado el lock
        // después de que el backoff leyera la generación. No reabrir con un ticket viejo.
        if expected.is_some_and(|g| self.generation.load(Ordering::SeqCst) != g) {
            return Ok(());
        }
        if self.engine().is_some() {
            return Ok(());
        }
        let ticket = self.generation.load(Ordering::SeqCst);
        self.view.send_modify(|v| v.busy = true);
        self.settings_runtime.view.send_modify(|v| {
            v.playback_error = false;
            v.reconnecting = expected.is_some();
        });
        if interactive {
            self.reconnects
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .reset();
        }
        let mut config = self.config();
        let snapshot = self
            .preferences
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|p| p.settings());
        let hooks = *self.hooks.read().unwrap_or_else(|e| e.into_inner());
        if let Some(settings) = &snapshot {
            if self
                .settings_runtime
                .restore_proxy(settings, hooks.is_some())
                .await
                .is_err()
            {
                self.view.send_modify(|v| v.busy = false);
                return Err(PlayerError::Session);
            }
        }
        let mut options = None;
        if let (Some(settings), Some(hooks)) = (&snapshot, hooks) {
            // Encrypted audio cache: absolute and scoped by the stable device id of this account.
            let owner = settings_runtime::read_audio_owner().await;
            let dir = settings_runtime::audio_cache_dir(&config.device_id, owner.as_deref());
            let mut built = self.engine_options(settings, dir);
            config.name = settings.playback.device_name.clone();
            (hooks.configure)(&mut config, &built);
            built.device_name = config.name.clone();
            options = Some(built);
        }
        // Un logout cancela navegador/conexión. Las rotaciones y el grant nuevo sólo
        // se guardan tras confirmar que esta generación sigue siendo la actual.
        let mut cancel = self.cancellation.subscribe();
        let staged = Arc::new(MemoryTokenStore::new());
        let durable = login::keyring_store();
        let load_store = durable.clone();
        if let Ok(Some(token)) = tokio::task::spawn_blocking(move || load_store.load()).await {
            staged.save(&token).map_err(|_| PlayerError::TokenStore)?;
        }
        let result = tokio::select! {
            biased;
            _ = cancel.changed() => { self.view.send_modify(|v| v.busy = false); return Ok(()); },
            result = async {
                match (interactive, options) {
                    (true, Some(o)) => Engine::start_login_with_store_and_options(config, staged.clone(), o).await,
                    (true, None) => Engine::start_login_with_store(config, staged.clone()).await,
                    (false, Some(o)) => Engine::from_store_with_options(config, staged.clone(), o).await,
                    (false, None) => Engine::from_store(config, staged.clone()).await,
                }
            } => result,
        };
        match result {
            Ok(engine) => {
                // Salir de la app puede invalidar una conexión en vuelo sin esperar
                // este lock asíncrono. Descartar el motor antes de publicarlo a Qt.
                if self.generation.load(Ordering::SeqCst) != ticket {
                    engine.shutdown().await;
                    self.view.send_modify(|v| v.busy = false);
                    return Ok(());
                }
                if let Some(refresh) = staged.load() {
                    let saved = tokio::task::spawn_blocking(move || durable.save(&refresh)).await;
                    if !matches!(saved, Ok(Ok(()))) {
                        engine.shutdown().await;
                        return Err(PlayerError::TokenStore);
                    }
                }
                let engine = Arc::new(engine);
                if !self.install(engine.clone(), ticket) {
                    engine.shutdown().await;
                } else if let (Some(snapshot), Some(_)) = (snapshot, hooks) {
                    self.settings_runtime.applied(&snapshot);
                }
                self.view.send_modify(|v| v.busy = false);
                Ok(())
            }
            Err(e) => {
                self.settings_runtime
                    .view
                    .send_modify(|v| v.playback_error = true);
                if matches!(e, PlayerError::NotSignedIn) {
                    let _ = login::forget_login(durable).await;
                }
                if !interactive {
                    self.view.send_modify(|v| v.busy = false);
                }
                Err(e)
            }
        }
    }

    fn install(self: &Arc<Self>, engine: Arc<Engine>, expected: u64) -> bool {
        // El mismo lock protege generación, reemplazo y publicación del forwarder.
        let mut slot = self.engine.write().unwrap_or_else(|e| e.into_inner());
        if self.generation.load(Ordering::SeqCst) != expected {
            return false;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut state = engine.state();
        let mut stopped = engine.stopped();
        let availability = Arc::downgrade(&engine);
        *slot = Some(engine);
        self.view.send_modify(|v| v.ready = true);
        drop(slot);
        self.rt.spawn(async move {
            if let Some(engine) = availability.upgrade() {
                engine.check_dj_available().await;
            }
        });
        let this = self.clone();
        self.rt.spawn(async move {
            let mut seen = {
                let s = state.borrow();
                (s.audio_errors, s.unavailable, s.dj_errors)
            };
            loop {
                let s = state.borrow_and_update().clone();
                let next_seen = (s.audio_errors, s.unavailable, s.dj_errors);
                if !this.publish_forwarded(generation, s, seen) {
                    return;
                }
                seen = next_seen;
                tokio::select! {
                    changed = state.changed() => if changed.is_err() { break },
                    _ = stopped.wait_for(|s| *s) => break,
                }
            }
            this.stopped(generation).await;
        });
        true
    }

    /// Comprobar dentro de la exclusión: un snapshot leído antes de detach ya no es publicable.
    fn publish_forwarded(
        &self,
        generation: u64,
        state: EngineState,
        seen: (u32, u32, u32),
    ) -> bool {
        self.with_current_engine(generation, || {
            if state.audio_errors != seen.0 {
                let _ = self
                    .errors
                    .send("No se pudo abrir la salida de audio de Windows.".into());
            }
            if state.unavailable != seen.1 {
                let _ = self
                    .errors
                    .send("Spotify no entregó una canción de la cola; se saltó.".into());
            }
            if state.dj_errors != seen.2 {
                if let Some(error) = state.dj_error {
                    let _ = self.errors.send(error.to_string());
                }
            }
            self.state.send_replace(state);
        })
    }

    fn with_current_engine(&self, generation: u64, publish: impl FnOnce()) -> bool {
        let _slot = self.engine.read().unwrap_or_else(|e| e.into_inner());
        if self.generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        publish();
        true
    }

    /// The engine's Connect task ended. On purpose (sign out, exit, replacement) the generation
    /// moved on and nothing happens; otherwise reconnect a few times with the stored login.
    async fn stopped(self: &Arc<Self>, generation: u64) {
        let operation = self.operation.lock().await;
        if !self.retire(generation) {
            return;
        }
        if !self
            .reconnects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record(std::time::Instant::now())
        {
            let message = "La sesión de audio se desconecta repetidamente. Reactiva la reproducción en Ajustes.".to_string();
            let _ = self.failures.send(message.clone());
            let _ = self.errors.send(message);
            return;
        }
        drop(operation);
        tracing::warn!("audio engine stopped unexpectedly; reconnecting");
        for delay in RECONNECT_DELAYS {
            tokio::time::sleep(delay).await;
            if self.generation.load(Ordering::SeqCst) != generation || self.engine().is_some() {
                return;
            }
            match self.launch_if_current(false, Some(generation)).await {
                Ok(()) => return,
                Err(PlayerError::NotSignedIn) => break,
                Err(e) => tracing::debug!(error = %e, "audio reconnection failed"),
            }
        }
        if self.generation.load(Ordering::SeqCst) == generation && self.engine().is_none() {
            self.fail(&PlayerError::Stopped);
        }
    }

    /// Drops the engine of `generation` if it is still the current one.
    fn retire(&self, generation: u64) -> bool {
        let mut slot = self.engine.write().unwrap_or_else(|e| e.into_inner());
        if self.generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        if let Some(engine) = slot.as_ref() {
            *self
                .reconnect_volume
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(engine.state().borrow().volume);
        }
        *slot = None;
        self.state.send_replace(EngineState::default());
        self.view.send_modify(|v| v.ready = false);
        true
    }

    /// Takes the engine out without triggering a reconnection.
    fn detach(&self) -> Option<Arc<Engine>> {
        let mut slot = self.engine.write().unwrap_or_else(|e| e.into_inner());
        self.generation.fetch_add(1, Ordering::SeqCst);
        let engine = slot.take();
        self.state.send_replace(EngineState::default());
        self.view.send_modify(|v| v.ready = false);
        engine
    }

    /// L1: abandons a browser login in progress (the listener releases its port). A running
    /// engine is not affected.
    pub fn cancel_login(&self) {
        if self.view.borrow().busy && !self.view.borrow().ready {
            self.cancellation.send_modify(|v| *v = v.wrapping_add(1));
        }
    }

    /// L1: the engine's librespot session for the data provider (one login for audio and data).
    pub fn session_source(self: &Arc<Self>) -> Arc<dyn SessionSource> {
        Arc::new(EngineSession(Arc::downgrade(self)))
    }

    /// Signing out of Spotify also forgets the audio login and removes the device.
    pub fn sign_out(self: &Arc<Self>) {
        let detached = self.detach();
        self.cancellation.send_modify(|v| *v = v.wrapping_add(1));
        self.view.send_replace(AudioView::default());
        let this = self.clone();
        self.rt.spawn(async move {
            let _operation = this.operation.lock().await;
            this.reconnects
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .reset();
            *this
                .reconnect_volume
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            if let Some(engine) = detached {
                engine.shutdown().await;
            }
            this.set_chosen_device(None);
            if let Err(e) = login::forget_login(login::keyring_store()).await {
                tracing::warn!(error = %e, "could not clear the audio login");
            }
        });
    }

    /// At exit (after the Qt event loop, never on a runtime worker): pause and leave Spotify
    /// Connect so other clients stop listing "Velo". Bounded wait.
    pub fn shutdown_blocking(&self) {
        let Some(engine) = self.detach() else {
            return;
        };
        self.rt.block_on(async move {
            let _ = tokio::time::timeout(EXIT_TIMEOUT, engine.shutdown()).await;
        });
    }
}

/// Last hot settings pushed to an engine: (generation, eq, output buffer, private session, mix).
type HotSettings = (
    u64,
    velo_domain::EqConfig,
    u32,
    bool,
    velo_domain::MixConfig,
);

/// Mixear: se empuja al motor una vez por cambio (o por motor nuevo), nunca en cada ajuste.
fn mix_needs_apply(
    previous: Option<&HotSettings>,
    generation: u64,
    mix: velo_domain::MixConfig,
) -> bool {
    previous.is_none_or(|p| p.0 != generation || p.4 != mix)
}

/// A new engine generation starts from `EngineConfig.private_session`, so it only needs a push
/// when the value differs from what that engine already got; within a generation, on change.
fn private_session_needs_apply(
    previous: Option<&HotSettings>,
    generation: u64,
    private: bool,
) -> bool {
    match previous {
        Some(p) if p.0 == generation => p.3 != private,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mix_marked_context_reaches_initial_options_before_an_engine_exists() {
        let audio = AudioCtl::new(
            tokio::runtime::Handle::current(),
            broadcast::channel(1).0,
            "Velo",
        );
        let mut settings = velo_domain::Settings::default();
        settings.playback.mix_seconds = 9;
        audio.set_mix_context(true);
        assert!(!audio.is_ready());
        let options = audio.engine_options(&settings, "encrypted-audio".into());
        assert!(options.mix.enabled);
        assert_eq!(options.mix.seconds, 9);
        audio.set_mix_context(false);
        assert!(
            !audio
                .engine_options(&settings, "encrypted-audio".into())
                .mix
                .enabled
        );
        settings.playback.mix_auto = true;
        assert!(
            audio
                .engine_options(&settings, "encrypted-audio".into())
                .mix
                .enabled
        );
    }

    #[tokio::test]
    async fn s9_name_changes_keep_device_id_and_hooks_are_an_explicit_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = super::super::prefs::Prefs::for_test(
            tokio::runtime::Handle::current(),
            dir.path().to_path_buf(),
        );
        let audio = AudioCtl::new(
            tokio::runtime::Handle::current(),
            broadcast::channel(4).0,
            "Velo",
        );
        audio.bind_settings(prefs.clone());
        let old_id = audio.config().device_id;
        prefs.update_settings(|s| s.playback.device_name = "Kitchen".into());
        assert_eq!(audio.config().name, "Kitchen");
        assert_eq!(audio.config().device_id, old_id);
        assert!(!audio.config().private_session);
        prefs.update_settings(|s| s.playback.private_session = true);
        assert!(audio.config().private_session);
        audio.set_engine_hooks(EngineHooks {
            configure: |_, _| {},
            equalizer: |_, _| {},
            output_buffer: |_, _| Ok(()),
            private_session: |_, _| Ok(()),
            mix: |_, _| {},
        });
        assert!(audio.settings_supported());
        // A late reconnect ticket is rejected before touching keyring or the network.
        let stale = audio.generation.load(Ordering::SeqCst);
        audio.detach();
        audio.launch_if_current(false, Some(stale)).await.unwrap();
        assert!(!audio.is_ready());
    }

    #[test]
    fn private_session_change_is_pushed_hot_once_per_change() {
        let eq = velo_domain::EqConfig::default();
        assert!(private_session_needs_apply(None, 1, false));
        let same = (1, eq, 0, true, velo_domain::MixConfig::default());
        assert!(!private_session_needs_apply(Some(&same), 1, true));
        assert!(private_session_needs_apply(Some(&same), 1, false));
        // A reconnected engine (new generation) is re-synced.
        assert!(private_session_needs_apply(Some(&same), 2, true));
    }

    #[test]
    fn mix_change_is_pushed_hot_once_per_change_and_per_engine() {
        let eq = velo_domain::EqConfig::default();
        let off = velo_domain::MixConfig::default();
        let on = velo_domain::Settings {
            playback: velo_domain::PlaybackSettings {
                mix_auto: true,
                mix_seconds: 9,
                ..Default::default()
            },
            ..Default::default()
        }
        .playback
        .mix();
        assert!(on.enabled && on.seconds == 9);
        assert!(mix_needs_apply(None, 1, off));
        let applied = (1, eq, 100, false, off);
        assert!(!mix_needs_apply(Some(&applied), 1, off));
        assert!(mix_needs_apply(Some(&applied), 1, on));
        assert!(mix_needs_apply(Some(&applied), 2, off));
    }

    #[test]
    fn controller_errors_reach_the_existing_player_channel_once_per_counter() {
        let rt = super::super::r4b_test_support::runtime();
        let errors = broadcast::channel(8).0;
        let mut received = errors.subscribe();
        let audio = AudioCtl::new(rt.handle().clone(), errors, "Velo");
        let state = EngineState {
            dj_errors: 1,
            dj_error: Some(velo_player::dj::DjError::Decode),
            ..Default::default()
        };
        assert!(audio.publish_forwarded(0, state.clone(), (0, 0, 0)));
        assert_eq!(
            received.try_recv().unwrap(),
            "La voz de DJ no se pudo decodificar"
        );
        assert!(audio.publish_forwarded(0, state, (0, 0, 1)));
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn r4b_retired_forwarder_cannot_publish_snapshot_or_errors_after_detach_or_replacement() {
        let rt = super::super::r4b_test_support::runtime();
        rt.block_on(async {
            let audio = AudioCtl::new(
                tokio::runtime::Handle::current(),
                broadcast::channel(8).0,
                "Velo",
            );
            let mut errors = audio.errors.subscribe();
            let old = EngineState {
                active: true,
                playing: true,
                audio_errors: 1,
                unavailable: 1,
                ..EngineState::default()
            };
            let (captured, snapshot) = tokio::sync::oneshot::channel();
            let (release, released) = tokio::sync::oneshot::channel();
            let forwarder = {
                let audio = audio.clone();
                tokio::spawn(async move {
                    // Mismo punto que el worker real: snapshot tomado, publicación aún pendiente.
                    let snapshot = old;
                    captured.send(()).unwrap();
                    released.await.unwrap();
                    audio.publish_forwarded(0, snapshot, (0, 0, 0))
                })
            };
            snapshot.await.unwrap();
            audio.detach();
            tokio::time::advance(Duration::from_secs(30)).await;
            release.send(()).unwrap();
            assert!(!forwarder.await.unwrap());
            assert!(!audio.state.borrow().active);
            assert!(errors.try_recv().is_err());
            assert!(!audio.retire(0));

            let old = EngineState {
                active: true,
                audio_errors: 2,
                unavailable: 2,
                ..EngineState::default()
            };
            let new = EngineState {
                active: true,
                volume: 20,
                ..EngineState::default()
            };
            // Simula el reemplazo bajo el mismo lock de install sin abrir un motor real.
            {
                let _slot = audio.engine.write().unwrap();
                audio.generation.fetch_add(1, Ordering::SeqCst);
                audio.state.send_replace(new.clone());
            }
            assert!(!audio.publish_forwarded(1, old, (0, 0, 0)));
            assert_eq!(audio.state.borrow().volume, new.volume);
            assert!(errors.try_recv().is_err());
            assert!(audio.publish_forwarded(2, new, (0, 0, 0)));

            // Pausa ahora DESPUÉS de validar el ticket: el lock debe cubrir la publicación.
            // Este tramo falla también si se conserva solo una comprobación atómica.
            let (entered, entry) = std::sync::mpsc::channel();
            let (release, released) = std::sync::mpsc::channel();
            let publisher = {
                let audio = audio.clone();
                std::thread::spawn(move || {
                    audio.with_current_engine(2, || {
                        entered.send(()).unwrap();
                        released.recv().unwrap();
                        audio.state.send_replace(EngineState {
                            active: true,
                            ..EngineState::default()
                        });
                    })
                })
            };
            entry.recv().unwrap();
            let protected = audio.engine.try_write().is_err();
            let detacher = {
                let audio = audio.clone();
                std::thread::spawn(move || audio.detach())
            };
            release.send(()).unwrap();
            assert!(publisher.join().unwrap());
            assert!(detacher.join().unwrap().is_none());
            assert!(protected, "detach debe excluirse entre validar y publicar");
            assert!(!audio.state.borrow().active);
        });
    }

    #[test]
    fn every_engine_error_has_a_spanish_message() {
        for e in [
            PlayerError::NotSignedIn,
            PlayerError::LoginCancelled,
            PlayerError::LoginTimeout,
            PlayerError::LoginPortBusy,
            PlayerError::LoginFailed,
            PlayerError::TokenStore,
            PlayerError::OAuthNetwork,
            PlayerError::OAuthRejected,
            PlayerError::Session,
            PlayerError::SessionTimeout,
            PlayerError::AudioBackend,
            PlayerError::Stopped,
            PlayerError::InvalidUri,
            PlayerError::QueueChanged,
            PlayerError::DjUnavailable,
            PlayerError::DjInactive,
        ] {
            let m = audio_message(&e);
            assert!(m.ends_with('.') && !m.contains("librespot"), "{m}");
        }
    }

    #[test]
    fn without_an_engine_the_controller_is_idle_and_remembers_the_choice() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let audio = AudioCtl::new(rt.handle().clone(), broadcast::channel(1).0, "Velo");
        assert!(audio.engine().is_none() && audio.device_id().is_none());
        assert_eq!(audio.config().initial_volume, 50);
        assert!(!audio.is_ready());
        assert!(!audio.state.borrow().active);
        audio.set_chosen_device(Some("phone".into()));
        assert_eq!(audio.chosen_device().as_deref(), Some("phone"));
        // A stale generation cannot retire anything.
        assert!(!audio.retire(42));
        assert!(audio.detach().is_none());
        assert!(!audio.retire(0));
    }

    #[test]
    fn replacement_config_preserves_muted_low_and_full_volume_through_idle_state() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let audio = AudioCtl::new(rt.handle().clone(), broadcast::channel(1).0, "Velo");
        for volume in [0, 20, 100] {
            *audio.reconnect_volume.lock().unwrap() = Some(volume);
            // Retirar el estado visible no pierde el nivel retenido para la sesión nueva.
            audio.state.send_replace(EngineState::default());
            assert_eq!(audio.config().initial_volume, volume);
            assert_eq!(
                audio.config().device_id,
                EngineConfig::velo("Velo").device_id
            );
        }
    }

    #[tokio::test]
    async fn exhausted_budget_notifies_without_network_and_stale_reconnect_stays_idle() {
        let errors = broadcast::channel(4).0;
        let mut error_rx = errors.subscribe();
        let audio = AudioCtl::new(tokio::runtime::Handle::current(), errors, "Velo");
        let now = std::time::Instant::now();
        for _ in 0..6 {
            assert!(audio.reconnects.lock().unwrap().record(now));
        }
        // Simula una caída del ticket actual; no hay motor/dispositivo real en este test.
        audio.stopped(0).await;
        assert!(error_rx.try_recv().unwrap().contains("Reactiva"));
        assert!(!audio.view.borrow().ready && !audio.view.borrow().busy);
        // El signout cambia la generación incluso sin motor: ninguna consulta de keyring
        // ni sesión de red debe empezar con el ticket anterior después de ganar el lock.
        audio.detach();
        audio.launch_if_current(false, Some(0)).await.unwrap();
        assert!(!audio.view.borrow().busy && audio.engine().is_none());
        audio.stopped(0).await;
        assert!(error_rx.try_recv().is_err());
    }
}
