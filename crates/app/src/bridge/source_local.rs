//! Source (a): the official Spotify app on this PC.
//!
//! Track, play/pause and position come from the Windows media session (SMTC, push events, no
//! polling). The Web API adds what SMTC does not carry (track URI, shuffle, repeat, volume,
//! device) and executes the actions; it is queried on demand: when the track changes, after an
//! action, when the window becomes visible and after signing in. Never on a timer.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use velo_domain::{
    NowPlayingEvent, NowPlayingSource, PlaybackState, RepeatMode, Result, pick_image,
};
use velo_nowplaying::SmtcSource;

use super::source::{
    PlaybackSource, Refresh, SourceAction, SourceState, SourceTrack, norm, perform_api,
};
use crate::engine::Env;
use crate::engine::clock::Clock;

/// Spotify's `/me/player` can lag a moment behind SMTC; wait before asking.
const SYNC_DEBOUNCE: Duration = Duration::from_millis(350);
const SYNC_RETRY: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone)]
struct ApiSnapshot {
    uri: Option<String>,
    cover_url: Option<String>,
    volume: Option<u8>,
    shuffle: bool,
    repeat: RepeatMode,
    device_name: String,
}

struct Inner {
    session: bool,
    pid: Option<u32>,
    track: Option<SourceTrack>,
    playing: bool,
    clock: Clock,
    api: Option<ApiSnapshot>,
    /// Web API reported no device although a session exists.
    api_no_device: bool,
    api_remote: bool,
}

pub struct LocalSource {
    env: Arc<Env>,
    tx: watch::Sender<SourceState>,
    inner: Mutex<Inner>,
    sync: mpsc::UnboundedSender<Refresh>,
    // Keeps the SMTC worker alive.
    _smtc: Option<Arc<SmtcSource>>,
}

impl LocalSource {
    pub fn start(env: &Arc<Env>) -> Arc<Self> {
        let smtc = match SmtcSource::new() {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                tracing::warn!(%e, "SMTC unavailable; no local Spotify session");
                None
            }
        };
        let (sync, sync_rx) = mpsc::unbounded_channel();
        let this = Arc::new(Self {
            env: env.clone(),
            tx: watch::channel(SourceState::default()).0,
            inner: Mutex::new(Inner {
                session: false,
                pid: None,
                track: None,
                playing: false,
                clock: Clock::new(Instant::now()),
                api: None,
                api_no_device: false,
                api_remote: false,
            }),
            sync,
            _smtc: smtc.clone(),
        });
        if let Some(smtc) = smtc {
            let me = this.clone();
            env.rt.spawn(async move { me.watch_smtc(smtc).await });
        }
        let me = this.clone();
        env.rt.spawn(async move { me.sync_loop(sync_rx).await });
        // Sign-in changes what the Web API can add.
        let me = this.clone();
        let mut account = env.account.view.subscribe();
        env.rt.spawn(async move {
            let mut was = account.borrow_and_update().signed_in;
            while account.changed().await.is_ok() {
                let now = account.borrow_and_update().signed_in;
                if now != was {
                    was = now;
                    if now {
                        me.refresh(Refresh::SignedIn);
                    } else {
                        me.inner().api = None;
                        me.publish();
                    }
                }
            }
        });
        this
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn watch_smtc(self: Arc<Self>, smtc: Arc<SmtcSource>) {
        let mut events = smtc.subscribe();
        for event in smtc.current() {
            self.on_event(event);
        }
        loop {
            match events.recv().await {
                Ok(event) => self.on_event(event),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    for event in smtc.current() {
                        self.on_event(event);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    fn on_event(&self, event: NowPlayingEvent) {
        let now = Instant::now();
        let mut want_sync = None;
        {
            let mut s = self.inner();
            match event {
                NowPlayingEvent::SessionStarted { pid } => {
                    if !s.session {
                        want_sync = Some(Refresh::Startup);
                    }
                    s.session = true;
                    s.pid = pid;
                }
                NowPlayingEvent::SessionEnded => {
                    s.session = false;
                    s.pid = None;
                    s.track = None;
                    s.playing = false;
                    s.api = None;
                    s.clock = Clock::new(now);
                }
                NowPlayingEvent::TrackChanged {
                    title,
                    artist,
                    album,
                    duration_ms,
                    thumbnail,
                } => {
                    // A session that sends metadata exists even if the start event was missed.
                    s.session = true;
                    let duration = duration_ms.unwrap_or(0);
                    let incoming = SourceTrack {
                        title,
                        artist,
                        album,
                        duration_ms: duration,
                        uri: None,
                        thumbnail: thumbnail.map(Arc::new),
                        cover_url: None,
                    };
                    let changed = !s.track.as_ref().is_some_and(|t| t.same_track(&incoming));
                    if changed {
                        s.api = None;
                        s.api_no_device = false;
                        let playing = s.playing;
                        s.clock.set(0, now, playing);
                        want_sync = Some(Refresh::TrackChanged);
                    }
                    s.clock.set_duration(duration);
                    s.track = Some(incoming);
                }
                NowPlayingEvent::PlaybackChanged { is_playing } => {
                    s.playing = is_playing;
                    if is_playing {
                        s.api_remote = false;
                    }
                    s.clock.set_playing(is_playing, now);
                    want_sync = Some(Refresh::AfterAction);
                }
                NowPlayingEvent::Position {
                    position_ms,
                    sampled_at,
                } => {
                    let playing = s.playing;
                    s.clock.set(position_ms, sampled_at, playing);
                }
            }
        }
        self.publish();
        if let Some(why) = want_sync {
            self.refresh(why);
        }
    }

    fn publish(&self) {
        let now = Instant::now();
        let s = self.inner();
        let track = s.track.clone().map(|mut t| {
            if let Some(api) = &s.api {
                t.uri = api.uri.clone();
                t.cover_url = api.cover_url.clone();
            }
            t
        });
        let state = SourceState {
            active: s.session && !s.api_remote,
            track,
            is_playing: s.playing,
            position_ms: s.clock.now_ms(now),
            position_at: now,
            volume: s.api.as_ref().and_then(|a| a.volume),
            shuffle: s.api.as_ref().is_some_and(|a| a.shuffle),
            repeat: s.api.as_ref().map_or(RepeatMode::Off, |a| a.repeat),
            device_name: s
                .api
                .as_ref()
                .map_or_else(|| "Spotify".to_string(), |a| a.device_name.clone()),
            controllable: s.session && s.api.is_some() && !s.api_no_device && !s.api_remote,
            device_available: s.session,
            pid: s.pid,
            remote: false,
            own_engine: false,
            dj: velo_player::DjState::default(),
        };
        drop(s);
        self.tx.send_replace(state);
    }

    async fn sync_loop(self: Arc<Self>, mut requests: mpsc::UnboundedReceiver<Refresh>) {
        while requests.recv().await.is_some() {
            tokio::time::sleep(SYNC_DEBOUNCE).await;
            while requests.try_recv().is_ok() {}
            self.sync().await;
        }
    }

    /// One Web API read, retried while it still reports the previous track.
    async fn sync(&self) {
        let Some(api) = self.env.account.api() else {
            return;
        };
        for attempt in 0..3 {
            match api.playback().await {
                Ok(Some(p)) => {
                    if self.accept(&p) {
                        return;
                    }
                }
                Ok(None) => {
                    let mut s = self.inner();
                    s.api_no_device = s.session;
                    drop(s);
                    self.publish();
                    return;
                }
                Err(e) => {
                    tracing::debug!(error = %e, "playback sync failed");
                    return;
                }
            }
            if attempt < 2 {
                tokio::time::sleep(SYNC_RETRY).await;
            }
        }
    }

    /// Merges a Web API snapshot when it describes the track SMTC shows.
    fn accept(&self, p: &PlaybackState) -> bool {
        let Some(track) = &p.track else {
            return false;
        };
        {
            let mut s = self.inner();
            let device = p.device.as_ref();
            s.api_remote = device.is_some_and(|d| {
                !d.kind.eq_ignore_ascii_case("computer")
                    || (!s.playing && s.api.as_ref().is_some_and(|a| a.device_name != d.name))
            });
            if s.api_remote {
                drop(s);
                self.publish();
                return true;
            }
            let matches = s.track.as_ref().is_none_or(|t| {
                norm(&t.title) == norm(&track.title)
                    && norm(&t.artist) == norm(&track.artist_line())
            });
            if !matches {
                return false;
            }
            s.api_no_device = device.is_none_or(|d| !d.is_active);
            s.api = Some(ApiSnapshot {
                uri: Some(track.uri.clone()),
                cover_url: pick_image(&track.images, 600).map(|i| i.url.clone()),
                volume: device.and_then(|d| d.volume_percent),
                shuffle: p.shuffle,
                repeat: p.repeat,
                device_name: device.map_or_else(|| "Spotify".into(), |d| d.name.clone()),
            });
            // The newest measurement wins: SMTC may never send timeline updates.
            if p.sampled_at >= s.clock.anchor_at() {
                let playing = s.playing;
                s.clock.set(p.progress_ms, p.sampled_at, playing);
            }
        }
        self.publish();
        true
    }
}

#[async_trait]
impl PlaybackSource for LocalSource {
    fn name(&self) -> &'static str {
        "local-spotify"
    }

    fn state(&self) -> watch::Receiver<SourceState> {
        self.tx.subscribe()
    }

    async fn act(&self, action: SourceAction) -> Result<()> {
        perform_api(&self.env, action).await
    }

    fn refresh(&self, why: Refresh) {
        let _ = self.sync.send(why);
    }
}
