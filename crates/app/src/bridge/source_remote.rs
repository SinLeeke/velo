//! Source (b): another Spotify Connect device (phone, speaker) controlled with the Web API only.
//! Used when there is no Spotify media session on this PC.
//!
//! State is read with `GET /me/player` at startup, after signing in, when the window becomes
//! visible again, after every action and when the track should end. While the music plays on
//! a remote device and the window is visible it is also refreshed every 5 s (the only polling
//! in the app). Paused or minimized: no timer, no requests.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use velo_domain::{PlaybackState, Result, VeloError, pick_image};

use super::source::{PlaybackSource, Refresh, SourceAction, SourceState, SourceTrack, perform_api};
use crate::engine::Env;
use crate::engine::clock::{Flags, should_tick};
use crate::engine::{
    Bus,
    player::history::{ProgressSample, RemoteListeningClock},
};

/// Longest wait between refreshes while playing with the window visible.
const REFRESH_EVERY: Duration = Duration::from_secs(5);
/// A little after the computed end of the track, once Spotify has moved on.
const END_GRACE: Duration = Duration::from_millis(400);
const MIN_DELAY: Duration = Duration::from_millis(500);
const SYNC_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Default)]
struct Backoff {
    failures: u32,
    until: Option<Instant>,
}

impl Backoff {
    fn failed(&mut self, error: &VeloError, now: Instant) {
        let exponential =
            Duration::from_secs((1_u64 << self.failures.min(4)).saturating_mul(2).min(30));
        let delay = match error {
            VeloError::RateLimited(retry_after) => exponential.max(*retry_after),
            _ => exponential,
        };
        self.failures = self.failures.saturating_add(1);
        self.until = now.checked_add(delay);
    }

    fn remaining(&self, now: Instant) -> Duration {
        self.until
            .map_or(Duration::ZERO, |at| at.saturating_duration_since(now))
    }
}

fn refresh_delay(outranked_now: bool, flags: &Flags, state: &SourceState) -> Option<Duration> {
    (!outranked_now && flags.remote && should_tick(flags) && state.active && state.is_playing)
        .then(|| next_delay(state, Instant::now()))
}

pub struct RemoteSource {
    pub listening: Bus<ProgressSample>,
    listening_clock: Mutex<RemoteListeningClock>,
    env: Arc<Env>,
    tx: watch::Sender<SourceState>,
    trigger: mpsc::UnboundedSender<Refresh>,
    // Avoids two overlapping reads.
    reading: tokio::sync::Mutex<()>,
    backoff: Mutex<Backoff>,
}

/// When to refresh next while playing on a remote device: at most `REFRESH_EVERY`, sooner when
/// the track is about to end.
pub fn next_delay(state: &SourceState, now: Instant) -> Duration {
    let duration = state.track.as_ref().map_or(0, |t| t.duration_ms);
    if duration == 0 {
        return REFRESH_EVERY;
    }
    let position =
        state.position_ms + now.saturating_duration_since(state.position_at).as_millis() as u64;
    let remaining = Duration::from_millis(duration.saturating_sub(position));
    (remaining + END_GRACE).clamp(MIN_DELAY, REFRESH_EVERY)
}

impl RemoteSource {
    /// `outranked` is true while a higher-priority source (Velo's engine, the local app) is
    /// active: meanwhile this source stays silent (no requests, no polling).
    pub fn start(env: &Arc<Env>, outranked: watch::Receiver<bool>) -> Arc<Self> {
        let (trigger, requests) = mpsc::unbounded_channel();
        let this = Arc::new(Self {
            listening: Bus::new(),
            listening_clock: Mutex::new(RemoteListeningClock::default()),
            env: env.clone(),
            tx: watch::channel(SourceState::default()).0,
            trigger,
            reading: tokio::sync::Mutex::new(()),
            backoff: Mutex::new(Backoff::default()),
        });
        let me = this.clone();
        env.rt
            .spawn(async move { me.run(requests, outranked).await });
        this.refresh(Refresh::Startup);
        this
    }

    async fn run(
        self: Arc<Self>,
        mut requests: mpsc::UnboundedReceiver<Refresh>,
        mut outranked: watch::Receiver<bool>,
    ) {
        let mut flags = self.env.flags.subscribe();
        let mut account = self.env.account.view.subscribe();
        let mut identity = {
            let a = account.borrow_and_update();
            (a.signed_in, a.user_id.clone())
        };
        let mut outranked_now = *outranked.borrow_and_update();
        loop {
            let current_flags = *flags.borrow_and_update();
            let state = self.tx.borrow().clone();
            let delay = refresh_delay(outranked_now, &current_flags, &state).map(|d| {
                d.max(
                    self.backoff
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remaining(Instant::now()),
                )
            });
            tokio::select! {
                request = requests.recv() => {
                    if request.is_none() {
                        break;
                    }
                    tokio::time::sleep(SYNC_DEBOUNCE).await;
                    while requests.try_recv().is_ok() {}
                    if !outranked_now {
                        self.sync().await;
                    }
                }
                changed = outranked.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let active = *outranked.borrow_and_update();
                    if active != outranked_now {
                        outranked_now = active;
                        if active {
                            self.clear();
                        } else {
                            self.sync().await;
                        }
                    }
                }
                changed = account.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let now = {
                        let a = account.borrow_and_update();
                        (a.signed_in, a.user_id.clone())
                    };
                    if now != identity {
                        identity = now;
                        self.listening_clock.lock().unwrap_or_else(|e| e.into_inner()).reset();
                        if identity.0 && !outranked_now {
                            self.sync().await;
                        } else if !identity.0 {
                            *self.backoff.lock().unwrap_or_else(|e| e.into_inner()) = Backoff::default();
                            self.clear();
                        }
                    }
                }
                // Visibility only changes the schedule above.
                changed = flags.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    self.listening_clock.lock().unwrap_or_else(|e| e.into_inner()).discontinuity();
                }
                () = sleep_for(delay) => {
                    self.sync().await;
                }
            }
        }
    }

    fn clear(&self) {
        self.listening_clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset();
        self.tx.send_replace(SourceState::default());
    }

    async fn sync(&self) {
        let Some((api, ticket)) = self.env.account.api_ticket() else {
            self.clear();
            return;
        };
        let _one_at_a_time = self.reading.lock().await;
        if !self
            .backoff
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remaining(Instant::now())
            .is_zero()
        {
            return;
        }
        let result = async {
            let playback = api.playback().await?;
            let mut state = playback
                .as_ref()
                .map_or_else(SourceState::default, state_of);
            if !state.active {
                state.device_available = api.devices().await?.iter().any(|d| d.id.is_some());
            }
            Ok::<_, VeloError>((state, playback))
        }
        .await;
        match result {
            Ok((state, playback)) => {
                self.env.account.commit_if_current(ticket, || {
                    let mut clock = self
                        .listening_clock
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if state.active {
                        if let Some(p) = playback.as_ref() {
                            let sample = clock.observe(p);
                            if let Some(sample) = sample {
                                self.listening.send(sample);
                            }
                        }
                    } else {
                        clock.reset();
                    }
                    *self.backoff.lock().unwrap_or_else(|e| e.into_inner()) = Backoff::default();
                    self.tx.send_replace(state);
                });
            }
            Err(e) => {
                self.listening_clock
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .discontinuity();
                self.backoff
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .failed(&e, Instant::now());
                tracing::debug!(error = %e, "remote playback read failed");
            }
        }
    }
}

async fn sleep_for(delay: Option<Duration>) {
    match delay {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

fn state_of(p: &PlaybackState) -> SourceState {
    let device = p.device.as_ref();
    if device.is_none_or(|d| !d.is_active) {
        return SourceState {
            remote: true,
            ..SourceState::default()
        };
    }
    SourceState {
        active: device.is_some(),
        track: p.track.as_ref().map(|t| SourceTrack {
            title: t.title.clone(),
            artist: t.artist_line(),
            album: t.album.clone(),
            duration_ms: t.duration_ms,
            uri: Some(t.uri.clone()),
            thumbnail: None,
            cover_url: pick_image(&t.images, 600).map(|i| i.url.clone()),
        }),
        is_playing: p.is_playing,
        position_ms: p.progress_ms,
        position_at: p.sampled_at,
        volume: device.and_then(|d| d.volume_percent),
        shuffle: p.shuffle,
        repeat: p.repeat,
        device_name: device.map(|d| d.name.clone()).unwrap_or_default(),
        controllable: device.is_some(),
        device_available: device.is_some_and(|d| d.id.is_some()),
        pid: None,
        remote: true,
        own_engine: false,
        dj: velo_player::DjState::default(),
    }
}

#[async_trait]
impl PlaybackSource for RemoteSource {
    fn name(&self) -> &'static str {
        "spotify-connect-remote"
    }

    fn state(&self) -> watch::Receiver<SourceState> {
        self.tx.subscribe()
    }

    async fn act(&self, action: SourceAction) -> Result<()> {
        // A seek/action boundary cannot be mistaken for effective listening.
        self.listening_clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .discontinuity();
        perform_api(&self.env, action).await
    }

    fn refresh(&self, why: Refresh) {
        let _ = self.trigger.send(why);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_back_off_exponentially_and_success_resets_the_deadline() {
        let now = Instant::now();
        let mut backoff = Backoff::default();
        for seconds in [2, 4, 8, 16, 30, 30, 30] {
            backoff.failed(&VeloError::Network("offline".into()), now);
            assert_eq!(backoff.remaining(now), Duration::from_secs(seconds));
            assert_eq!(
                backoff.remaining(now + Duration::from_secs(seconds)),
                Duration::ZERO
            );
        }
        backoff = Backoff::default();
        assert_eq!(backoff.remaining(now), Duration::ZERO);
        backoff.failed(&VeloError::Network("offline".into()), now);
        assert_eq!(backoff.remaining(now), Duration::from_secs(2));
    }

    #[test]
    fn retry_after_is_respected_even_when_longer_than_the_backoff_cap() {
        let now = Instant::now();
        let mut backoff = Backoff::default();
        backoff.failed(&VeloError::RateLimited(Duration::from_secs(90)), now);
        assert_eq!(
            backoff.remaining(now + Duration::from_secs(30)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn api_snapshot_requires_an_active_device_and_preserves_controls() {
        let mut p = PlaybackState {
            track: None,
            is_playing: true,
            progress_ms: 1_234,
            sampled_at: Instant::now(),
            shuffle: true,
            repeat: velo_domain::RepeatMode::Track,
            device: None,
            context_uri: None,
        };
        assert!(!state_of(&p).active);
        assert!(!state_of(&p).is_playing);
        p.device = Some(velo_domain::Device {
            id: Some("speaker".into()),
            name: "Living room".into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: Some(72),
            supports_volume: true,
        });
        assert!(!state_of(&p).active);
        p.device.as_mut().unwrap().is_active = true;
        let s = state_of(&p);
        assert!(s.active && s.controllable && s.remote && s.is_playing);
        assert!(s.shuffle);
        assert_eq!(s.repeat, velo_domain::RepeatMode::Track);
        assert_eq!(s.position_ms, 1_234);
        assert_eq!(s.position_at, p.sampled_at);
        assert_eq!(s.volume, Some(72));
        assert_eq!(s.device_name, "Living room");
        assert_eq!(s.pid, None);
    }

    #[test]
    fn polling_has_no_deadline_when_paused_hidden_or_local() {
        let mut s = state(240_000, 10_000);
        s.active = true;
        let flags = Flags {
            visible: true,
            playing: true,
            remote: true,
            ..Flags::default()
        };
        assert!(refresh_delay(false, &flags, &s).is_some());
        assert!(refresh_delay(true, &flags, &s).is_none());
        assert!(
            refresh_delay(
                false,
                &Flags {
                    playing: false,
                    ..flags
                },
                &s
            )
            .is_none()
        );
        assert!(
            refresh_delay(
                false,
                &Flags {
                    visible: false,
                    ..flags
                },
                &s
            )
            .is_none()
        );
        s.is_playing = false;
        assert!(refresh_delay(false, &flags, &s).is_none());
    }

    fn state(duration: u64, position: u64) -> SourceState {
        SourceState {
            track: Some(SourceTrack {
                title: "t".into(),
                artist: "a".into(),
                album: "b".into(),
                duration_ms: duration,
                uri: None,
                thumbnail: None,
                cover_url: None,
            }),
            position_ms: position,
            is_playing: true,
            ..SourceState::default()
        }
    }

    #[test]
    fn refresh_is_every_five_seconds_at_most() {
        let s = state(240_000, 10_000);
        assert_eq!(next_delay(&s, s.position_at), REFRESH_EVERY);
        assert_eq!(next_delay(&state(0, 0), Instant::now()), REFRESH_EVERY);
    }

    #[test]
    fn refresh_lands_just_after_the_track_ends() {
        let s = state(240_000, 238_000);
        // 2 s left: the regular 5 s refresh would be late, so wait 2.4 s.
        assert_eq!(next_delay(&s, s.position_at), Duration::from_millis(2_400));
        // Already past the end: do not spin.
        let late = state(240_000, 240_000);
        assert_eq!(next_delay(&late, late.position_at), MIN_DELAY);
    }
}
