//! The Qt-free application engine behind the `Velo.Backend` singletons.
//!
//! Everything heavy (network, SMTC, covers, lyrics, FFT, disk) runs on one tokio runtime owned
//! here, never on the Qt thread. State flows to the singletons through `tokio::sync::watch`
//! channels (latest value) and [`Bus`] queues (ordered model changes); the singletons are thin
//! views that forward those to the Qt thread with `qt_thread().queue(..)`.
//!
//! Nothing here keeps a periodic timer alive: the only recurring work is the 4 Hz position clock
//! ([`player`]) and the remote-device refresh, both of which exist only while the music plays and
//! the window is visible.

pub mod account;
pub mod actions;
pub mod artists;
pub mod audio;
pub mod audio_output;
pub mod cache;
pub mod clock;
pub mod discover;
pub mod home;
pub mod images;
pub mod library;
pub mod links;
pub mod lyrics;
pub mod native_media;
pub mod player;
pub mod prefs;
pub mod profile;
#[cfg(test)]
mod r4b_test_support;
mod reconnect;
pub mod rows;
pub mod search;
pub mod sidebar;
pub mod tracklist;
pub mod viz;
mod viz_pcm;
pub mod volume_memory;

use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::{broadcast, mpsc, watch};
use velo_domain::VeloError;
use velo_services::{CoverArtCache, LrcLibClient};
use velo_viz::VizEngine;

use clock::Flags;

/// Ordered queue of UI events whose single consumer (a singleton) claims it once. Events sent
/// before the singleton exists wait in the queue.
pub struct Bus<E> {
    tx: mpsc::UnboundedSender<E>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<E>>>,
}

impl<E> Bus<E> {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }

    pub fn send(&self, event: E) {
        let _ = self.tx.send(event);
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<E> {
        self.tx.clone()
    }

    /// The receiving end; only the first call gets it.
    pub fn take(&self) -> Option<mpsc::UnboundedReceiver<E>> {
        self.rx.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

impl<E> Default for Bus<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared services every controller needs.
pub struct Env {
    pub rt: tokio::runtime::Handle,
    pub prefs: Arc<prefs::Prefs>,
    /// What gates periodic work (see [`clock`]).
    pub flags: watch::Sender<Flags>,
    /// Interpolated playback position (ms): changes on events, and at 4 Hz only while ticking.
    pub position: watch::Sender<u64>,
    pub account: Arc<account::Account>,
    /// Velo's own audio engine (librespot, Connect device "Velo").
    pub audio: Arc<audio::AudioCtl>,
    pub covers: Arc<CoverArtCache>,
    pub lyrics_client: Arc<LrcLibClient>,
    pub viz: Arc<VizEngine>,
    /// User-facing errors (Spanish text) surfaced as `Player.error(message)`.
    pub errors: broadcast::Sender<String>,
}

#[cfg(test)]
pub(crate) fn test_env(rt: &tokio::runtime::Runtime) -> (tempfile::TempDir, Arc<Env>) {
    let dir = tempfile::tempdir().unwrap();
    let handle = rt.handle().clone();
    let prefs = prefs::Prefs::for_test(handle.clone(), dir.path().to_path_buf());
    let covers = Arc::new(CoverArtCache::with_cache_dir(dir.path().join("covers")).unwrap());
    let errors = broadcast::channel(16).0;
    let audio = audio::AudioCtl::new(handle.clone(), errors.clone(), crate::APP_NAME);
    let env = Arc::new(Env {
        rt: handle.clone(),
        account: account::Account::new(handle, prefs.clone(), covers.clone(), audio.clone()),
        audio,
        prefs,
        flags: watch::channel(Flags::default()).0,
        position: watch::channel(0).0,
        covers,
        lyrics_client: Arc::new(LrcLibClient::with_cache_dir(dir.path().join("lyrics")).unwrap()),
        viz: Arc::new(VizEngine::new()),
        errors,
    });
    (dir, env)
}

impl Env {
    pub fn report(&self, e: &VeloError) {
        tracing::warn!(error = %e, "backend error");
        // L1: with the single login the Web API is only a fallback; its quota is not something
        // the user can act on, so the section just stays hidden.
        if matches!(e, VeloError::RateLimited(_)) && self.account.session_mode() {
            return;
        }
        let _ = self.errors.send(user_message(e));
    }

    pub fn update_flags(&self, f: impl FnOnce(&mut Flags)) {
        self.flags.send_if_modified(|flags| {
            let before = *flags;
            f(flags);
            *flags != before
        });
    }
}

/// Spanish message for the UI toast.
pub fn user_message(e: &VeloError) -> String {
    match e {
        VeloError::NotAuthenticated => "Inicia sesión en Spotify para continuar.".into(),
        VeloError::PremiumRequired => "Esta acción requiere Spotify Premium.".into(),
        VeloError::NoActiveDevice => {
            "No hay ningún dispositivo de Spotify activo. Activa la reproducción en Velo o elige un dispositivo."
                .into()
        }
        VeloError::RateLimited(_) => {
            "Spotify pide ir más despacio; inténtalo en unos segundos.".into()
        }
        VeloError::NotFound => "Spotify no encontró lo que se pidió.".into(),
        VeloError::Forbidden(_) => "Spotify no permite esta acción sobre este elemento.".into(),
        VeloError::Network(_) => "No se pudo conectar con Spotify.".into(),
        VeloError::Other(m) => m.clone(),
    }
}

pub struct Core {
    pub outputs: audio_output::AudioOutputs,
    pub env: Arc<Env>,
    pub player: Arc<player::PlayerCtl>,
    pub library: Arc<library::LibraryCtl>,
    pub lyrics: Arc<lyrics::LyricsCtl>,
    pub actions: Arc<actions::ActionsCtl>,
    // Kept alive for the whole process; dropped never (the process exits with the event loop).
    _rt: tokio::runtime::Runtime,
}

static CORE: OnceLock<Core> = OnceLock::new();

/// The engine, created on first use (the first singleton QML touches).
pub fn core() -> &'static Core {
    CORE.get_or_init(Core::new)
}

/// Writes pending preferences and takes "Velo" out of Spotify Connect; call after the event
/// loop ends (blocks a few seconds at most).
pub fn shutdown() {
    if let Some(core) = CORE.get() {
        native_media::set_window_handle(0);
        core.outputs.shutdown();
        core.env.prefs.shutdown();
        core.env.audio.shutdown_blocking();
    }
}

impl Core {
    fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("velo-rt")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let prefs = prefs::Prefs::load(&rt);
        let covers = Arc::new(CoverArtCache::new().unwrap_or_else(|e| {
            tracing::warn!(%e, "no cache directory; covers go to the temp folder");
            CoverArtCache::with_cache_dir(std::env::temp_dir().join("velo-cache"))
                .expect("cover cache")
        }));
        let lyrics_client = Arc::new(LrcLibClient::new().unwrap_or_else(|e| {
            tracing::warn!(%e, "no cache directory; lyrics go to the temp folder");
            LrcLibClient::with_cache_dir(std::env::temp_dir().join("velo-cache")).expect("lyrics")
        }));
        let handle = rt.handle().clone();
        let _guard = handle.enter();
        let viz = Arc::new(VizEngine::new());
        let settings = prefs.settings();
        covers.set_limit_bytes(u64::from(settings.cache.cover_limit_mb) * 1024 * 1024);
        let flags = Flags {
            viz_enabled: viz::viz_switch(&settings),
            ..Flags::default()
        };
        let errors = broadcast::channel(16).0;
        let audio = audio::AudioCtl::new(handle.clone(), errors.clone(), crate::APP_NAME);
        let env = Arc::new(Env {
            rt: handle.clone(),
            account: account::Account::new(
                handle.clone(),
                prefs.clone(),
                covers.clone(),
                audio.clone(),
            ),
            audio,
            prefs,
            flags: watch::channel(flags).0,
            position: watch::channel(0).0,
            covers,
            lyrics_client,
            viz,
            errors,
        });
        cache::start(&env);
        env.account.start();
        // With a stored audio login the Connect device "Velo" comes up in the background.
        env.audio.start();
        let player = player::PlayerCtl::start(&env);
        viz::start(&env, &player);
        native_media::start(&env, &player);
        let library = library::LibraryCtl::start(&env, &player);
        // C3's local history feeds the sidebar recency and Home's local "recent"; Settings
        // clears it through the same service (a single store).
        library.install_history_reader(
            Arc::new(player::history::LocalHistoryReader(player.history.clone())),
            player.history.changes.subscribe(),
        );
        {
            let history = player.history.clone();
            env.audio
                .settings_runtime
                .set_history_clearer(Arc::new(move || {
                    let history = history.clone();
                    Box::pin(async move { history.clear_confirmed().await })
                }));
        }
        let lyrics = lyrics::LyricsCtl::start(&env, &player);
        let actions = actions::ActionsCtl::start(&env, &player);
        let outputs = audio_output::AudioOutputs::start(env.audio.clone(), env.errors.clone());
        Core {
            outputs,
            env,
            player,
            library,
            lyrics,
            actions,
            _rt: rt,
        }
    }
}
