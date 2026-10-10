//! Event-driven Spotify system media session source.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use velo_domain::{NowPlayingEvent, NowPlayingSource};
#[cfg(any(windows, test))]
mod logic;
pub mod publisher;
#[cfg(windows)]
mod smtc;

#[derive(Default)]
struct State {
    session: Option<NowPlayingEvent>,
    track: Option<NowPlayingEvent>,
    playback: Option<NowPlayingEvent>,
    position: Option<NowPlayingEvent>,
}
impl State {
    fn current(&self) -> Vec<NowPlayingEvent> {
        [&self.session, &self.track, &self.playback, &self.position]
            .into_iter()
            .filter_map(Clone::clone)
            .collect()
    }
    #[cfg(any(windows, test))]
    fn update(&mut self, event: NowPlayingEvent) -> bool {
        match &event {
            NowPlayingEvent::SessionStarted { .. } => {
                // A PID correction belongs to the existing session and must not
                // discard its media/playback snapshot.
                if !matches!(self.session, Some(NowPlayingEvent::SessionStarted { .. })) {
                    *self = Self::default();
                }
                let changed = self.session.as_ref() != Some(&event);
                self.session = Some(event);
                return changed;
            }
            NowPlayingEvent::SessionEnded => {
                *self = Self::default();
                self.session = Some(event);
            }
            NowPlayingEvent::TrackChanged { .. } => {
                let changed = !logic::same_track(self.track.as_ref(), &event);
                self.track = Some(event);
                return changed;
            }
            NowPlayingEvent::PlaybackChanged { .. } => {
                let changed = self.playback.as_ref() != Some(&event);
                self.playback = Some(event);
                return changed;
            }
            NowPlayingEvent::Position { .. } => self.position = Some(event),
        }
        true
    }
}

/// Watches Spotify SMTC without timers or polling while the manager is available.
/// Drop waits at most one second for its MTA worker, then detaches it if necessary.
///
/// Subscribe first, then immediately consume [`Self::current`] before receiving live
/// events. Tokio broadcast cannot privately replay a snapshot to one subscriber;
/// this explicit snapshot is the concrete source's initial-state protocol. Events
/// queued during the snapshot may overlap it and should be applied as state updates.
/// Outside Windows this source stays empty and creates no worker.
pub struct SmtcSource {
    sender: broadcast::Sender<NowPlayingEvent>,
    state: Arc<Mutex<State>>,
    available: Arc<AtomicBool>,
    #[cfg(windows)]
    worker: smtc::Worker,
}
impl SmtcSource {
    /// Starts the worker without blocking on WinRT initialization.
    pub fn new() -> std::io::Result<Self> {
        let (sender, _) = broadcast::channel(64);
        let state = Arc::new(Mutex::new(State::default()));
        let available = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        let worker = smtc::Worker::start(sender.clone(), state.clone(), available.clone())?;
        Ok(Self {
            sender,
            state,
            available,
            #[cfg(windows)]
            worker,
        })
    }
    /// Latest state, ordered session, track, playback, position.
    pub fn current(&self) -> Vec<NowPlayingEvent> {
        self.state.lock().unwrap().current()
    }
    /// Whether the worker has acquired the Windows media-session manager and
    /// subscribed to session changes. This does not require a Spotify session.
    /// False during startup/recovery, after worker exit, and outside Windows.
    /// Startup failures retry after 1, 2, 4, 8, 16, then 30 seconds until available.
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }
}
impl NowPlayingSource for SmtcSource {
    fn subscribe(&self) -> broadcast::Receiver<NowPlayingEvent> {
        self.sender.subscribe()
    }
    fn session_pid(&self) -> Option<u32> {
        match self.state.lock().unwrap().session {
            Some(NowPlayingEvent::SessionStarted { pid }) => pid,
            _ => None,
        }
    }
}
#[cfg(windows)]
impl Drop for SmtcSource {
    fn drop(&mut self) {
        self.worker.stop();
    }
}
