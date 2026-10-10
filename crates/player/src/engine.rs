//! The engine: one librespot Session + Player published as the Spotify Connect device "Velo"
//! (librespot-connect `Spirc`). The phone, the official apps and the Web API see it and can
//! transfer playback to it; Velo drives it directly through Spirc commands.
//!
//! State is pushed by librespot's player events into a `watch` channel (no polling). Commands
//! return immediately: Spirc queues them and the result shows up as events.

use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use librespot_connect::{
    ConnectConfig, LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack,
    Spirc,
};
use librespot_core::{
    Session, SessionConfig, authentication::Credentials, cache::Cache, config::DeviceType,
};
use librespot_playback::{
    config::{Bitrate, PlayerConfig, VolumeCtrl},
    mixer::{Mixer, MixerConfig, NoOpVolume, mappings::MappedCtrl, softmixer::SoftMixer},
    player::Player,
};
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};
use velo_domain::{EqConfig, RepeatMode};
use velo_spotify::TokenStore;

use crate::{
    PlayerError,
    dj::{self, DjCommand, DjError, DjHost, DjState},
    eq::EqControl,
    login,
    pcm::{
        OutputControl, PcmPacket, PcmTap, PlaybackProgress, TapActivation, TappedSink,
        TrackEndTimerState, VoiceClip, VoiceProgress, VolumeOverride,
    },
    state::{self, EngineState},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

pub use velo_domain::EngineOptions;

const MIB: u64 = 1024 * 1024;

/// Checks the discrete choices and the cache path of the shared domain schema.
pub fn validate_options(options: &EngineOptions) -> Result<(), PlayerError> {
    if !matches!(options.quality, 96 | 160 | 320)
        || !matches!(options.output_buffer_ms, 50 | 100 | 200)
        || options.device_name.trim().is_empty()
        || options.device_name.chars().count() > 100
        || (options.audio_cache_dir.is_some()
            && !matches!(options.audio_cache_limit_bytes / MIB, 512 | 1024 | 4096))
        || options
            .audio_cache_dir
            .as_ref()
            .is_some_and(|p| p.as_os_str().is_empty() || !p.is_absolute())
    {
        return Err(PlayerError::InvalidOptions);
    }
    Ok(())
}
fn legacy_options(config: &EngineConfig) -> EngineOptions {
    EngineOptions {
        device_name: config.name.clone(),
        audio_cache_dir: None,
        ..Default::default()
    }
}
fn player_config(options: &EngineOptions) -> PlayerConfig {
    PlayerConfig {
        bitrate: match options.quality {
            96 => Bitrate::Bitrate96,
            160 => Bitrate::Bitrate160,
            _ => Bitrate::Bitrate320,
        },
        gapless: options.gapless,
        normalisation: options.normalize_volume,
        position_update_interval: None,
        ..PlayerConfig::default()
    }
}
fn open_cache(options: &EngineOptions) -> Result<Option<Cache>, PlayerError> {
    validate_options(options)?;
    let Some(dir) = options.audio_cache_dir.as_ref() else {
        return Ok(None);
    };
    // Upstream caches AudioFile's encrypted compressed bytes. Neither credentials.json
    // nor volume gets a location: login remains exclusively in the system keyring.
    Cache::new(
        None::<&PathBuf>,
        None::<&PathBuf>,
        Some(dir),
        Some(options.audio_cache_limit_bytes),
    )
    .map(Some)
    .map_err(|_| PlayerError::AudioCache)
}

/// How the device shows up in Spotify Connect.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Device name in every Spotify device list.
    pub name: String,
    /// Stable id of this device (see [`stable_device_id`]).
    pub device_id: String,
    /// 0–100, until a client sets another one.
    pub initial_volume: u8,
}

impl EngineConfig {
    /// "Velo" on this computer, with an id that survives restarts.
    pub fn velo(name: &str) -> Self {
        let host = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_default();
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default();
        Self {
            name: name.to_string(),
            device_id: stable_device_id(&format!("Velo|{host}|{user}")),
            initial_volume: 50,
        }
    }
}

/// 40 hex characters derived from `seed` (FNV-1a), so the device keeps its id across restarts
/// and Spotify does not list a new "Velo" every time the app starts. Not a secret.
pub fn stable_device_id(seed: &str) -> String {
    let mut out = String::with_capacity(48);
    for salt in 0u8..3 {
        let mut h: u64 =
            0xcbf2_9ce4_8422_2325 ^ u64::from(salt).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        for b in seed.bytes().chain([salt]) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        out.push_str(&format!("{h:016x}"));
    }
    out.truncate(40);
    out
}

/// What to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadTarget {
    /// A playlist, album or artist, optionally starting at a row.
    Context {
        uri: String,
        index: Option<u32>,
        shuffle: bool,
    },
    /// An explicit list of tracks (liked songs, search results), starting at `index`.
    Tracks {
        uris: Vec<String>,
        index: u32,
        shuffle: bool,
    },
}

impl LoadTarget {
    fn checked(&self) -> Result<(), PlayerError> {
        let playable = |uri: &str| {
            ["spotify:track:", "spotify:episode:"]
                .iter()
                .any(|p| uri.strip_prefix(p).is_some_and(|id| !id.is_empty()))
        };
        match self {
            LoadTarget::Context { uri, .. } => {
                let ok = uri
                    .strip_prefix("spotify:")
                    .and_then(|rest| rest.split_once(':'))
                    .is_some_and(|(kind, id)| {
                        !id.is_empty()
                            && matches!(
                                kind,
                                "playlist" | "album" | "artist" | "show" | "user" | "station"
                            )
                    });
                ok.then_some(()).ok_or(PlayerError::InvalidUri)
            }
            LoadTarget::Tracks { uris, index, .. } => {
                if uris.is_empty()
                    || *index as usize >= uris.len()
                    || !uris.iter().all(|u| playable(u))
                {
                    Err(PlayerError::InvalidUri)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// The Spirc options: start playing at once, at the requested row, shuffled on request.
    pub fn options(&self) -> LoadRequestOptions {
        let (index, shuffle) = match self {
            LoadTarget::Context { index, shuffle, .. } => (*index, *shuffle),
            LoadTarget::Tracks { index, shuffle, .. } => (Some(*index), *shuffle),
        };
        LoadRequestOptions {
            start_playing: true,
            seek_to: 0,
            context_options: Some(LoadContextOptions::Options(Options {
                shuffle,
                repeat: false,
                repeat_track: false,
            })),
            // Shuffle without a row: librespot picks a random first track.
            playing_track: index.map(PlayingTrack::Index),
        }
    }

    fn request(self) -> Result<LoadRequest, PlayerError> {
        self.checked()?;
        let options = self.options();
        Ok(match self {
            LoadTarget::Context { uri, .. } => LoadRequest::from_context_uri(uri, options),
            LoadTarget::Tracks { uris, .. } => LoadRequest::from_tracks(uris, options),
        })
    }
}

/// A running engine. Drop or [`Engine::shutdown`] removes the device from Spotify Connect.
pub struct Engine {
    mixer: Arc<dyn Mixer>,
    volume_mapping: VolumeCtrl,
    volume_commands: Mutex<()>,
    volume_override: VolumeOverride,
    eq_control: EqControl,
    progress: broadcast::Sender<PlaybackProgress>,
    transport_events: broadcast::Sender<librespot_playback::player::PlayerEvent>,
    output_switch: crate::output::OutputSwitch,
    output_control: OutputControl,
    spirc: Spirc,
    session: Session,
    player: Option<Arc<Player>>,
    device_id: String,
    state: watch::Receiver<EngineState>,
    /// Compartido con el reenviador de eventos; el DJ publica aquí su estado.
    state_tx: Arc<watch::Sender<EngineState>>,
    stopped: watch::Receiver<bool>,
    pcm_enabled: TapActivation,
    pcm: Mutex<Option<mpsc::Receiver<PcmPacket>>>,
    forwarder: JoinHandle<()>,
    /// The explicit track list Velo loaded last (track lists, queue edits, retries); `None`
    /// after loading a context URI or taking over another device's playback.
    known: Arc<Mutex<KnownQueue>>,
    /// E5b: el DJ que suena en este motor, si hay uno.
    dj: Mutex<Option<DjHandle>>,
    /// Sube con cada arranque/parada del DJ: un controlador viejo ya no publica ni actúa.
    dj_generation: AtomicU64,
}

struct DjHandle {
    commands: mpsc::UnboundedSender<DjCommand>,
    task: tokio::task::AbortHandle,
}

impl Engine {
    /// Shared authenticated session; discovery never creates a second audio login.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Starts with the stored audio login (no browser). [`PlayerError::NotSignedIn`] when there
    /// is none: call [`Engine::start_login`].
    pub async fn from_keyring(config: EngineConfig) -> Result<Self, PlayerError> {
        Self::from_store(config, login::keyring_store()).await
    }

    /// Like [`Engine::from_keyring`] with another token store.
    pub async fn from_store(
        config: EngineConfig,
        store: Arc<dyn TokenStore>,
    ) -> Result<Self, PlayerError> {
        let options = legacy_options(&config);
        Self::from_store_with_options(config, store, options).await
    }

    pub async fn from_keyring_with_options(
        config: EngineConfig,
        options: EngineOptions,
    ) -> Result<Self, PlayerError> {
        Self::from_store_with_options(config, login::keyring_store(), options).await
    }
    pub async fn from_store_with_options(
        config: EngineConfig,
        store: Arc<dyn TokenStore>,
        options: EngineOptions,
    ) -> Result<Self, PlayerError> {
        validate_options(&options)?;
        let token = login::access_token_from_store(store).await?;
        Self::start(config, token, options).await
    }

    /// Opens the browser for the one-time audio login, stores its refresh token in the keyring
    /// and starts the engine.
    pub async fn start_login(config: EngineConfig) -> Result<Self, PlayerError> {
        Self::start_login_with_store(config, login::keyring_store()).await
    }

    /// Permite preparar la autorización en memoria antes de publicarla en el keyring.
    pub async fn start_login_with_store(
        config: EngineConfig,
        store: Arc<dyn TokenStore>,
    ) -> Result<Self, PlayerError> {
        let options = legacy_options(&config);
        Self::start_login_with_store_and_options(config, store, options).await
    }
    pub async fn start_login_with_options(
        config: EngineConfig,
        options: EngineOptions,
    ) -> Result<Self, PlayerError> {
        Self::start_login_with_store_and_options(config, login::keyring_store(), options).await
    }
    pub async fn start_login_with_store_and_options(
        config: EngineConfig,
        store: Arc<dyn TokenStore>,
        options: EngineOptions,
    ) -> Result<Self, PlayerError> {
        validate_options(&options)?;
        let token = login::login_in_browser(store).await?;
        Self::start(config, token, options).await
    }

    async fn start(
        mut config: EngineConfig,
        access_token: String,
        options: EngineOptions,
    ) -> Result<Self, PlayerError> {
        validate_options(&options)?;
        config.name = options.device_name.clone(); // Never derive a new id when renaming.
        let cache_options = options.clone();
        let cache = tokio::task::spawn_blocking(move || open_cache(&cache_options))
            .await
            .map_err(|_| PlayerError::AudioCache)??;
        crate::install_crypto_provider();
        let mixer: Arc<dyn Mixer> = Arc::new(
            SoftMixer::open(MixerConfig::default()).map_err(|_| PlayerError::AudioBackend)?,
        );
        mixer.set_volume(state::percent_to_volume(config.initial_volume));
        let volume_mapping = VolumeCtrl::default();
        // librespot's own client id (SessionConfig::default) is what Spotify streams audio to.
        let session = Session::new(
            SessionConfig {
                device_id: config.device_id.clone(),
                autoplay: Some(options.autoplay),
                proxy: options
                    .proxy_url
                    .as_deref()
                    .and_then(|u| url::Url::parse(u).ok()),
                ..SessionConfig::default()
            },
            cache,
        );
        let (faults, mut fault_rx) = mpsc::unbounded_channel();
        let (pcm_tx, pcm_rx) = mpsc::channel(2);
        let pcm_enabled = TapActivation::default();

        let tap = PcmTap {
            enabled: pcm_enabled.clone(),
            sender: pcm_tx,
        };
        let output_switch = crate::output::OutputSwitch::default();
        output_switch.set_buffer_ms(options.output_buffer_ms)?;
        let sink_switch = output_switch.clone();
        let soft_volume = mixer.get_soft_volume(); // V1b: applied after the PCM tap.
        let (pause_request, mut pause_rx) = mpsc::unbounded_channel();
        let output_control = OutputControl::new(tap, soft_volume, pause_request);
        let eq_control = EqControl::default();
        eq_control.set(options.eq);
        let volume_override = VolumeOverride::default();
        let (progress, _) = broadcast::channel(256);
        let (transport_events, _) = broadcast::channel(256);
        output_control.configure(
            eq_control.clone(),
            volume_override.clone(),
            progress.clone(),
        );
        let sink_control = output_control.clone();
        let player = Player::new(
            player_config(&options),
            session.clone(),
            Box::new(NoOpVolume), // V1b: normalized Spotify PCM reaches the tap before volume.
            move || {
                // Sink propio (pcm.rs): cola ~120 ms, fade desde el siguiente frame y volumen/
                // tap en la salida, antes de cualquier ganancia.
                Box::new(TappedSink::new(sink_control, faults, sink_switch))
            },
        );
        output_control.attach_events(player.get_player_event_channel());
        // Subscribed before Spirc starts, so the first SessionConnected is not missed.
        let mut events = player.get_player_event_channel();
        let connect = ConnectConfig {
            name: config.name.clone(),
            device_type: DeviceType::Computer,
            initial_volume: state::percent_to_volume(config.initial_volume),
            ..ConnectConfig::default()
        };
        let started = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Spirc::new(
                connect,
                session.clone(),
                Credentials::with_access_token(access_token),
                player.clone(),
                mixer.clone(),
            ),
        )
        .await;
        let (spirc, task) = match started {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => {
                tracing::warn!(kind = ?e.kind, "Spotify Connect could not start");
                close_off_thread(session, Some(player));
                return Err(PlayerError::Session);
            }
            Err(_) => {
                close_off_thread(session, Some(player));
                return Err(PlayerError::SessionTimeout);
            }
        };

        let (stopped_tx, stopped) = watch::channel(false);
        tokio::spawn(async move {
            task.await;
            let _ = stopped_tx.send(true);
        });

        let (state_tx, state) =
            watch::channel(EngineState::new(config.initial_volume, Instant::now()));
        let state_tx = Arc::new(state_tx);
        let shared_state = state_tx.clone();
        let known = Arc::new(Mutex::new(KnownQueue::default()));
        let event_known = known.clone();
        let pause_player = Arc::downgrade(&player);
        let event_volume = volume_override.clone();
        let event_transport = transport_events.clone();
        let forwarder = tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = events.recv() => match event {
                        Some(event) => {
                            if let librespot_playback::player::PlayerEvent::VolumeChanged { volume } = &event {
                                #[cfg(debug_assertions)]
                                tracing::debug!(volume, latest_requested_volume = ?event_volume.requested_volume(), request_to_spirc_ack_us = ?event_volume.elapsed_us(), "volume: Spirc acknowledgement received");
                                event_volume.acknowledge(*volume);
                            }
                            let _ = event_transport.send(event.clone());
                            event_known.lock().unwrap_or_else(|e| e.into_inner()).event(&event, state_tx.borrow().repeat);
                            state_tx.send_if_modified(|s| state::apply_event(s, &event, Instant::now()));
                        }
                        None => break,
                    },
                    Some(reply) = pause_rx.recv() => {
                        if let Some(player) = pause_player.upgrade() {
                            player.pause();
                            if let Some(reply) = reply { let _ = reply.send(()); }
                            // El último Arc jamás cae en el callback de audio ni en Qt.
                            std::thread::spawn(move || drop(player));
                        }
                    },
                    Some(()) = fault_rx.recv() => {
                        state_tx.send_modify(|s| s.audio_errors = s.audio_errors.wrapping_add(1));
                    }
                }
            }
        });

        tracing::info!(name = %config.name, "Spotify Connect device ready");
        Ok(Self {
            mixer,
            volume_mapping,
            volume_commands: Mutex::new(()),
            volume_override,
            eq_control,
            progress,
            transport_events,
            output_switch,
            output_control,
            spirc,
            session,
            player: Some(player),
            device_id: config.device_id,
            state,
            state_tx: shared_state,
            stopped,
            pcm_enabled,
            pcm: Mutex::new(Some(pcm_rx)),
            forwarder,
            known,
            dj: Mutex::new(None),
            dj_generation: AtomicU64::new(0),
        })
    }

    /// The Connect device id (what the Web API lists in `/me/player/devices`).
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Reopen only the output sink on the next PCM packet (or next resume if paused).
    /// Keeps the decoder, track position and Spotify Connect session intact.
    pub fn reopen_output(&self) {
        self.output_switch.request();
    }
    /// Changes only WASAPI/CPAL output and the bounded decoder queue, keeping Session/Spirc.
    pub fn set_output_buffer_ms(&self, ms: u32) -> Result<(), PlayerError> {
        self.output_switch.set_buffer_ms(ms)
    }
    pub fn set_eq(&self, config: EqConfig) {
        self.eq_control.set(config);
    }
    /// Consumers must handle Lagged. Values are cumulative, so dropped intermediate values
    /// do not lose listening time. Subscribe before issuing load; IDs scope to this Engine.
    pub fn playback_progress(&self) -> broadcast::Receiver<PlaybackProgress> {
        self.progress.subscribe()
    }
    /// Original librespot transport events, including play request IDs, URI and seek/pause.
    pub fn playback_events(&self) -> broadcast::Receiver<librespot_playback::player::PlayerEvent> {
        self.transport_events.subscribe()
    }

    /// Latest state, pushed by librespot events.
    pub fn state(&self) -> watch::Receiver<EngineState> {
        self.state.clone()
    }

    /// Becomes `true` once the Connect task ended (shutdown, network loss, session closed).
    pub fn stopped(&self) -> watch::Receiver<bool> {
        self.stopped.clone()
    }

    fn send(&self, result: Result<(), librespot_core::Error>) -> Result<(), PlayerError> {
        result.map_err(|_| PlayerError::Stopped)
    }

    /// Reanuda; mientras habla Livi, reanuda su voz.
    pub fn play(&self) -> Result<(), PlayerError> {
        if self.output_control.voice_active() {
            self.output_control.pause_voice(false);
            return Ok(());
        }
        self.play_song()
    }
    fn play_song(&self) -> Result<(), PlayerError> {
        self.output_control.resume();
        self.send(self.spirc.play())
    }
    /// Pausa; mientras habla Livi, pausa su voz (la canción ya espera en pausa).
    pub fn pause(&self) -> Result<(), PlayerError> {
        if self.output_control.voice_active() {
            self.output_control.pause_voice(true);
            return Ok(());
        }
        self.send(self.spirc.pause())
    }
    /// Siguiente; con DJ pasa por el controlador (bloque nuevo al final). El diálogo de Livi
    /// siempre suena: mientras habla, «siguiente» no lo corta.
    pub fn next(&self) -> Result<(), PlayerError> {
        if self.dj_active() && self.output_control.voice_active() {
            return Ok(());
        }
        if self.output_control.voice_active() {
            self.output_control.stop_voice();
            return Ok(());
        }
        if let Some(dj) = self.dj.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if !dj.task.is_finished() && dj.commands.send(DjCommand::Next).is_ok() {
                return Ok(());
            }
        }
        self.next_plain()
    }
    fn next_plain(&self) -> Result<(), PlayerError> {
        self.output_control.cancel();
        self.known.lock().unwrap_or_else(|e| e.into_inner()).step(1);
        self.send(self.spirc.next())?;
        Ok(())
    }
    /// Restarts the track after 3 s, else goes to the previous one (Spotify's behaviour).
    pub fn previous(&self) -> Result<(), PlayerError> {
        if self.output_control.voice_active() {
            self.output_control.seek_voice(0);
            return Ok(());
        }
        self.send(self.spirc.prev())?;
        if crate::queue::position_now(&self.state.borrow(), Instant::now()) <= 3000 {
            self.known
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .step(-1);
        }
        Ok(())
    }
    pub fn seek(&self, position_ms: u64) -> Result<(), PlayerError> {
        if self.output_control.voice_active() {
            self.output_control.seek_voice(position_ms);
            return Ok(());
        }
        let ms = u32::try_from(position_ms).unwrap_or(u32::MAX);
        self.send(self.spirc.set_position_ms(ms))
    }
    /// 0–100 (clamped).
    pub fn set_volume(&self, percent: u8) -> Result<(), PlayerError> {
        let _commands = self
            .volume_commands
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let volume = state::percent_to_volume(percent);
        #[cfg(debug_assertions)]
        tracing::debug!(
            percent = percent.min(100),
            volume,
            "volume: engine request -> Spirc enqueue"
        );
        // Compute the same pure mapping as SoftMixer, so a concurrent Spirc write cannot
        // corrupt the preview. Keep original SoftMixer authoritative for remote Connect.
        if self.volume_override.pending() || self.mixer.volume() != volume {
            self.volume_override
                .request(volume, self.volume_mapping.to_mapped(volume) as f32);
        }
        let result = self.send(self.spirc.set_volume(volume));
        if result.is_err() {
            self.volume_override.acknowledge(volume);
        }
        result
    }
    pub fn set_shuffle(&self, on: bool) -> Result<(), PlayerError> {
        self.send(self.spirc.shuffle(on))
    }
    pub fn set_repeat(&self, mode: RepeatMode) -> Result<(), PlayerError> {
        let (context, track) = state::repeat_switches(mode);
        if let Some(context) = context {
            self.send(self.spirc.repeat(context))?;
        }
        self.send(self.spirc.repeat_track(track))
    }

    /// Loads a context or a track list and plays it here. Spirc ignores loads while the device
    /// is not the active one, so it is activated first (a no-op when it already is).
    pub fn load(&self, target: LoadTarget) -> Result<(), PlayerError> {
        let known = match &target {
            LoadTarget::Tracks { uris, .. } => Some(uris.clone()),
            LoadTarget::Context { .. } => None,
        };
        let index = match &target {
            LoadTarget::Tracks {
                index,
                shuffle: false,
                ..
            } => Some(*index as usize),
            _ => None,
        };
        let request = target.request()?;
        self.stop_dj();
        self.send(self.spirc.activate())?;
        self.output_control.cancel();
        self.remember(known, index);
        self.send(self.spirc.load(request))?;
        Ok(())
    }

    /// Pulls the account's current playback (from the phone, the official app...) to Velo.
    pub fn transfer_here(&self) -> Result<(), PlayerError> {
        self.stop_dj();
        self.remember(None, None);
        self.send(self.spirc.transfer(None))
    }

    fn remember(&self, tracks: Option<Vec<String>>, index: Option<usize>) {
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = KnownQueue {
            tracks,
            index,
            pending: true,
            natural_end: false,
        };
    }
    pub fn known_tracks(&self) -> Option<Vec<String>> {
        self.known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tracks
            .clone()
    }
    /// Índice de la ocurrencia, no la primera URI coincidente; None si Connect no permite saberlo.
    pub fn known_occurrence(&self, current: &str) -> Option<usize> {
        let known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        known.index.filter(|at| {
            known
                .tracks
                .as_ref()
                .and_then(|t| t.get(*at))
                .is_some_and(|u| u == current)
        })
    }
    /// Q1: replaces what plays after the current song with `upcoming` (then `tail`, the part of
    /// the context the queue view did not show), keeping the song, its position and play/pause.
    /// Refused with [`PlayerError::QueueChanged`] when the song is no longer `expected_uri` or
    /// Velo is not the active device, so a stale view never pulls an old song back.
    pub fn replace_queue(
        &self,
        expected_uri: &str,
        upcoming: &[String],
        tail: &[String],
    ) -> Result<(), PlayerError> {
        let state = self.state.borrow().clone();
        if !state.active || state.track.as_ref().map(|t| t.uri.as_str()) != Some(expected_uri) {
            return Err(PlayerError::QueueChanged);
        }
        let position = crate::queue::position_now(&state, Instant::now());
        let tracks = crate::queue::rebuilt_list(expected_uri, upcoming, tail);
        self.reload(tracks, position, state.playing, state.repeat)
    }

    /// Skips to `target` (a row of "A continuación"): plays it from the start, followed by
    /// `upcoming` and `tail`. Same refusal rules as [`Self::replace_queue`].
    pub fn skip_to_queue(
        &self,
        expected_uri: &str,
        target: &str,
        upcoming: &[String],
        tail: &[String],
    ) -> Result<(), PlayerError> {
        let state = self.state.borrow().clone();
        if !state.active || state.track.as_ref().map(|t| t.uri.as_str()) != Some(expected_uri) {
            return Err(PlayerError::QueueChanged);
        }
        if !crate::queue::playable(target) {
            return Err(PlayerError::InvalidUri);
        }
        let tracks = crate::queue::rebuilt_list(target, upcoming, tail);
        self.reload(tracks, 0, true, state.repeat)
    }

    /// E2b: plays `failure.uri` again from where it stopped, followed by `upcoming` and `tail`
    /// (Spirc had already skipped it, so the song now playing is the first of `upcoming`).
    pub fn retry_track(
        &self,
        failure: &crate::TrackFailure,
        upcoming: &[String],
        tail: &[String],
    ) -> Result<(), PlayerError> {
        let state = self.state.borrow().clone();
        if !state.active {
            return Err(PlayerError::QueueChanged);
        }
        let tracks = crate::queue::rebuilt_list(&failure.uri, upcoming, tail);
        self.reload(tracks, failure.position_ms, true, state.repeat)
    }

    fn reload(
        &self,
        tracks: Vec<String>,
        position_ms: u64,
        playing: bool,
        repeat: RepeatMode,
    ) -> Result<(), PlayerError> {
        let request = crate::queue::rebuild_request(tracks.clone(), position_ms, playing, repeat)
            .ok_or(PlayerError::InvalidUri)?;
        self.remember(Some(tracks), Some(0));
        self.send(self.spirc.load(request))?;
        Ok(())
    }

    /// M1 local: parar en el límite audible de esta ocurrencia. Seek/pausa mantienen el armado;
    /// una carga manual lo cancela. EOF natural inicia el fade del resto de PCM de esa pista.
    pub fn sleep_at_track_end(
        &self,
        uri: &str,
    ) -> Result<watch::Receiver<TrackEndTimerState>, PlayerError> {
        let state = self.state.borrow();
        if !state.active || state.track.as_ref().is_none_or(|t| t.uri != uri) {
            return Err(PlayerError::QueueChanged);
        }
        Ok(self.output_control.arm(uri.to_string()))
    }
    pub fn refresh_track_end_sleep(&self) {
        self.output_control.sync_events();
    }
    pub fn cancel_track_end_sleep(&self) {
        self.output_control.cancel();
    }

    /// Consulta puntual de disponibilidad, sin reproducir ni publicar un éxito supuesto.
    pub async fn check_dj_available(&self) -> bool {
        let available = dj::fetch_block(&self.session, None).await.is_ok();
        self.state_tx.send_if_modified(|s| {
            if s.dj_available == available {
                false
            } else {
                s.dj_available = available;
                true
            }
        });
        available
    }

    /// E5b: resuelve lexicon y deja el controlador voz → canción → cierre, bloque tras bloque.
    /// Cargar otra cosa, transferir o parar el motor lo terminan.
    pub async fn start_dj(self: &Arc<Self>) -> Result<(), PlayerError> {
        self.stop_dj();
        let generation = self.dj_generation.load(Ordering::SeqCst);
        self.publish_dj(DjState {
            active: true,
            ..DjState::default()
        });
        let first = match dj::fetch_block(&self.session, None).await {
            Ok(block) => block,
            Err(error) => {
                tracing::debug!(%error, "DJ: Spotify no entregó la sesión");
                if self.dj_generation.load(Ordering::SeqCst) == generation {
                    self.publish_dj(DjState::default());
                    self.state_tx.send_modify(|s| s.dj_available = false);
                }
                return Err(PlayerError::DjUnavailable);
            }
        };
        if self.dj_generation.load(Ordering::SeqCst) != generation {
            return Ok(()); // otra acción del usuario llegó mientras tanto
        }
        if let Err(error) = self.send(self.spirc.activate()) {
            self.publish_dj(DjState::default());
            return Err(error);
        }
        let mut slot = self.dj.lock().unwrap_or_else(|e| e.into_inner());
        if self.dj_generation.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        self.state_tx.send_modify(|s| s.dj_available = true);
        let (commands, rx) = mpsc::unbounded_channel();
        let host = Arc::new(EngineDj {
            engine: Arc::downgrade(self),
            generation,
        });
        let task = tokio::spawn(dj::run(host, first, rx));
        *slot = Some(DjHandle {
            commands,
            task: task.abort_handle(),
        });
        Ok(())
    }

    /// «Elige tú, Livi»: el bloque siguiente de la sesión, con su presentación.
    pub fn dj_next_segment(&self) -> Result<(), PlayerError> {
        let dj = self.dj.lock().unwrap_or_else(|e| e.into_inner());
        match dj.as_ref() {
            Some(dj) if !dj.task.is_finished() && dj.commands.send(DjCommand::Jump).is_ok() => {
                Ok(())
            }
            _ => Err(PlayerError::DjInactive),
        }
    }

    pub fn dj_active(&self) -> bool {
        self.dj
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|dj| !dj.task.is_finished())
    }

    /// Termina el DJ (la canción que suena sigue); corta la voz con un fundido corto.
    pub fn stop_dj(&self) {
        self.dj_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(dj) = self.dj.lock().unwrap_or_else(|e| e.into_inner()).take() {
            dj.task.abort();
        }
        self.output_control.stop_voice();
        self.output_control.cancel_dj_timer();
        self.publish_dj(DjState::default());
    }

    fn publish_dj(&self, dj: DjState) {
        self.state_tx.send_if_modified(|s| {
            if s.dj == dj {
                false
            } else {
                s.dj = dj;
                true
            }
        });
    }

    /// El bloque del DJ como lista explícita, en pausa si antes habla Livi.
    fn load_dj(&self, uris: Vec<String>, play: bool) -> Result<(), PlayerError> {
        let target = LoadTarget::Tracks {
            uris: uris.clone(),
            index: 0,
            shuffle: false,
        };
        target.checked()?;
        let mut options = target.options();
        options.start_playing = play;
        self.send(self.spirc.activate())?;
        self.output_control.cancel();
        self.remember(Some(uris.clone()), Some(0));
        self.send(self.spirc.load(LoadRequest::from_tracks(uris, options)))
    }

    /// PCM hook for the visualizer: off by default. While off, nothing is copied.
    pub fn set_pcm_enabled(&self, enabled: bool) {
        self.pcm_enabled.set(enabled);
    }

    /// The PCM receiver (interleaved stereo f64, 44.1 kHz, bounded and lossy). Only the first
    /// call gets it.
    pub fn take_pcm(&self) -> Option<mpsc::Receiver<PcmPacket>> {
        self.pcm.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// Pauses, removes the device from Spotify Connect and closes the session. Waits a few
    /// seconds at most.
    pub async fn shutdown(&self) {
        self.stop_dj();
        self.set_pcm_enabled(false);
        let _ = self.spirc.shutdown();
        let mut stopped = self.stopped.clone();
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, stopped.wait_for(|s| *s)).await;
        self.session.shutdown();
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(dj) = self.dj.lock().unwrap_or_else(|e| e.into_inner()).take() {
            dj.task.abort();
        }
        self.forwarder.abort();
        self.pcm_enabled.set(false);
        let _ = self.spirc.shutdown();
        close_off_thread(self.session.clone(), self.player.take());
    }
}

/// El motor visto por el controlador del DJ. Débil: el DJ nunca mantiene vivo el motor.
struct EngineDj {
    engine: Weak<Engine>,
    generation: u64,
}

impl EngineDj {
    fn engine(&self) -> Result<Arc<Engine>, DjError> {
        self.engine
            .upgrade()
            .filter(|e| e.dj_generation.load(Ordering::SeqCst) == self.generation)
            .ok_or(DjError::Stopped)
    }
    fn session(&self) -> Option<Session> {
        self.engine().ok().map(|e| e.session.clone())
    }
}

impl DjHost for EngineDj {
    fn fetch_block(
        &self,
        url: Option<String>,
    ) -> impl Future<Output = Result<dj::DjBlock, DjError>> + Send {
        let session = self.session();
        async move { dj::fetch_block(&session.ok_or(DjError::Stopped)?, url.as_deref()).await }
    }
    fn fetch_voice(
        &self,
        narration: dj::Narration,
    ) -> impl Future<Output = Result<VoiceClip, DjError>> + Send {
        let session = self.session();
        async move { dj::fetch_voice(&session.ok_or(DjError::Stopped)?, &narration).await }
    }
    fn load(&self, uris: Vec<String>, play: bool) -> Result<(), DjError> {
        self.engine()?
            .load_dj(uris, play)
            .map_err(|_| DjError::Stopped)
    }
    fn resume(&self) -> Result<(), DjError> {
        self.engine()?.play_song().map_err(|_| DjError::Stopped)
    }
    fn pause(&self) {
        if let Ok(engine) = self.engine() {
            let _ = engine.spirc.pause();
        }
    }
    fn skip(&self) -> Result<(), DjError> {
        self.engine()?.next_plain().map_err(|_| DjError::Stopped)
    }
    fn arm(&self, uri: &str) -> Option<watch::Receiver<TrackEndTimerState>> {
        let engine = self.engine().ok()?;
        // «Al terminar esta canción» del usuario manda: esa vez no habla Livi.
        if engine.output_control.timer_armed() {
            return None;
        }
        let state = engine.state.borrow();
        if !state.active || state.track.as_ref().is_none_or(|t| t.uri != uri) {
            return None;
        }
        Some(engine.output_control.arm_dj(uri.to_owned()))
    }
    fn speak(&self, clip: Arc<VoiceClip>) -> Option<watch::Receiver<VoiceProgress>> {
        let engine = self.engine().ok()?;
        if engine.state.borrow().playing {
            let _ = engine.spirc.pause();
        }
        engine.output_control.play_voice(clip)
    }
    fn stop_voice(&self) {
        if let Ok(engine) = self.engine() {
            engine.output_control.stop_voice();
        }
    }
    fn release_output(&self) {
        if let Ok(engine) = self.engine() {
            engine.output_control.cancel_dj_timer();
            engine.output_control.release_if_idle();
        }
    }
    fn state(&self) -> watch::Receiver<EngineState> {
        match self.engine() {
            Ok(engine) => engine.state(),
            Err(_) => watch::channel(EngineState::default()).1,
        }
    }
    fn publish(&self, dj: DjState) {
        if let Ok(engine) = self.engine() {
            engine.publish_dj(dj);
        }
    }
    fn error(&self, error: DjError) {
        if let Ok(engine) = self.engine() {
            engine.state_tx.send_modify(|s| {
                s.dj_errors = s.dj_errors.wrapping_add(1);
                s.dj_error = Some(error);
            });
        }
    }
}

/// librespot's Player joins its decoder thread on drop; never do that on a runtime worker or
/// on the UI thread.
fn close_off_thread(session: Session, player: Option<Arc<Player>>) {
    let _ = std::thread::Builder::new()
        .name("velo-player-close".into())
        .spawn(move || {
            session.shutdown();
            drop(player);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_options_validate_discrete_choices_and_require_an_explicit_cache_path() {
        let base = EngineOptions::default();
        let ok = |o: &EngineOptions| validate_options(o).is_ok();
        assert!(ok(&base));
        for quality in [96, 160, 320] {
            for output_buffer_ms in [50, 100, 200] {
                for mb in [512u64, 1024, 4096] {
                    assert!(ok(&EngineOptions {
                        quality,
                        output_buffer_ms,
                        audio_cache_dir: Some(std::env::temp_dir()),
                        audio_cache_limit_bytes: mb * MIB,
                        ..base.clone()
                    }));
                }
            }
        }
        for invalid in [
            EngineOptions {
                quality: 128,
                ..base.clone()
            },
            EngineOptions {
                output_buffer_ms: 120,
                ..base.clone()
            },
            EngineOptions {
                audio_cache_dir: Some(std::env::temp_dir()),
                audio_cache_limit_bytes: 0,
                ..base.clone()
            },
            EngineOptions {
                device_name: " ".into(),
                ..base.clone()
            },
            EngineOptions {
                device_name: "x".repeat(101),
                ..base.clone()
            },
            EngineOptions {
                audio_cache_dir: Some(PathBuf::from("relative")),
                ..base.clone()
            },
        ] {
            assert!(!ok(&invalid));
        }
        for quality in [96, 160, 320] {
            let options = EngineOptions {
                quality,
                normalize_volume: false,
                gapless: false,
                ..base.clone()
            };
            let player = player_config(&options);
            assert!(!player.gapless && !player.normalisation);
            assert!(player.position_update_interval.is_none());
            assert!(matches!(
                (quality, player.bitrate),
                (96, Bitrate::Bitrate96) | (160, Bitrate::Bitrate160) | (320, Bitrate::Bitrate320)
            ));
        }
        assert_eq!(
            EngineConfig::velo("Velo").device_id,
            EngineConfig::velo("Renamed device").device_id
        );
    }
    #[test]
    fn audio_cache_has_no_credentials_or_volume_locations() {
        let path = std::env::temp_dir().join(format!(
            "velo-c2-cache-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let options = EngineOptions {
            audio_cache_dir: Some(path.clone()),
            ..Default::default()
        };
        let cache = open_cache(&options).unwrap().unwrap();
        cache.save_volume(12345);
        cache.save_credentials(&Credentials::with_access_token("offline-fixture"));
        assert!(cache.volume().is_none());
        assert!(cache.credentials().is_none());
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        drop(cache);
        std::fs::remove_dir(path).unwrap();
        assert!(
            open_cache(&legacy_options(&EngineConfig::velo("Velo")))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn device_id_is_stable_hex_and_depends_on_the_seed() {
        let a = stable_device_id("Velo|PC|juan");
        assert_eq!(a, stable_device_id("Velo|PC|juan"));
        assert_eq!(a.len(), 40);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, stable_device_id("Velo|PC|ana"));
        assert_eq!(EngineConfig::velo("Velo").name, "Velo");
    }

    #[test]
    fn load_options_start_at_the_row_and_honour_shuffle() {
        let ctx = |index, shuffle| LoadTarget::Context {
            uri: "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M".into(),
            index,
            shuffle,
        };
        let o = ctx(Some(3), false).options();
        assert!(o.start_playing);
        assert!(matches!(o.playing_track, Some(PlayingTrack::Index(3))));
        assert!(matches!(
            o.context_options,
            Some(LoadContextOptions::Options(Options { shuffle: false, .. }))
        ));
        let o = ctx(None, true).options();
        assert!(o.playing_track.is_none());
        assert!(matches!(
            o.context_options,
            Some(LoadContextOptions::Options(Options { shuffle: true, .. }))
        ));
        let tracks = LoadTarget::Tracks {
            uris: vec!["spotify:track:a".into(), "spotify:track:b".into()],
            index: 1,
            shuffle: false,
        };
        assert!(matches!(
            tracks.options().playing_track,
            Some(PlayingTrack::Index(1))
        ));
        assert!(tracks.request().is_ok());
    }

    #[test]
    fn load_targets_are_validated_before_reaching_spirc() {
        for uri in [
            "spotify:playlist:x",
            "spotify:album:x",
            "spotify:artist:x",
            "spotify:show:x",
        ] {
            let t = LoadTarget::Context {
                uri: uri.into(),
                index: None,
                shuffle: false,
            };
            assert!(t.checked().is_ok(), "{uri}");
        }
        for uri in [
            "",
            "spotify:track:x",
            "spotify:playlist:",
            "https://open.spotify.com/x",
        ] {
            let t = LoadTarget::Context {
                uri: uri.into(),
                index: None,
                shuffle: false,
            };
            assert!(matches!(t.checked(), Err(PlayerError::InvalidUri)), "{uri}");
        }
        let tracks = |uris: &[&str], index| LoadTarget::Tracks {
            uris: uris.iter().map(|u| (*u).to_string()).collect(),
            index,
            shuffle: false,
        };
        assert!(
            tracks(&["spotify:track:a", "spotify:episode:b"], 0)
                .checked()
                .is_ok()
        );
        assert!(tracks(&[], 0).checked().is_err());
        assert!(tracks(&["spotify:track:a"], 1).checked().is_err());
        assert!(tracks(&["spotify:album:a"], 0).checked().is_err());
        assert!(tracks(&["spotify:local:a:b:c:1"], 0).checked().is_err());
    }
}

#[derive(Default)]
struct KnownQueue {
    tracks: Option<Vec<String>>,
    index: Option<usize>,
    pending: bool,
    natural_end: bool,
}
impl KnownQueue {
    fn step(&mut self, direction: isize) {
        self.index = self.index.and_then(|at| at.checked_add_signed(direction));
        self.pending = true;
    }
    fn event(&mut self, event: &librespot_playback::player::PlayerEvent, repeat: RepeatMode) {
        use librespot_playback::player::PlayerEvent;
        match event {
            PlayerEvent::EndOfTrack { .. } => self.natural_end = true,
            PlayerEvent::TrackChanged { audio_item } => self.track_changed(&audio_item.uri, repeat),
            _ => {}
        }
    }
    fn track_changed(&mut self, uri: &str, repeat: RepeatMode) {
        let Some(tracks) = &self.tracks else { return };
        if self.natural_end && !self.pending && repeat != RepeatMode::Track {
            self.index = self.index.map(|at| (at + 1) % tracks.len().max(1));
        } else if !self.pending && !self.natural_end {
            self.index = None;
        }
        self.index = self
            .index
            .filter(|at| tracks.get(*at).is_some_and(|u| u == uri));
        if self.index.is_none() {
            let mut matches = tracks.iter().enumerate().filter(|(_, u)| u.as_str() == uri);
            self.index = matches
                .next()
                .map(|(at, _)| at)
                .filter(|_| matches.next().is_none());
        }
        self.pending = false;
        self.natural_end = false;
    }
}

#[cfg(test)]
mod occurrence_tests {
    use super::*;
    #[test]
    fn q1b_loaded_index_natural_duplicate_next_previous_and_external_ambiguity() {
        let mut queue = KnownQueue {
            tracks: Some(["a", "b", "b", "c"].map(str::to_owned).to_vec()),
            index: Some(1),
            pending: true,
            natural_end: false,
        };
        queue.track_changed("b", RepeatMode::Off);
        assert_eq!(queue.index, Some(1));
        queue.natural_end = true;
        queue.track_changed("b", RepeatMode::Off);
        assert_eq!(queue.index, Some(2));
        queue.natural_end = true;
        queue.track_changed("b", RepeatMode::Track);
        assert_eq!(queue.index, Some(2));
        queue.step(-1);
        queue.track_changed("b", RepeatMode::Off);
        assert_eq!(queue.index, Some(1));
        queue.step(1);
        queue.track_changed("b", RepeatMode::Off);
        assert_eq!(queue.index, Some(2));
        queue.track_changed("b", RepeatMode::Off);
        assert_eq!(queue.index, None);
        queue.track_changed("c", RepeatMode::Off);
        assert_eq!(queue.index, Some(3));
    }
}
