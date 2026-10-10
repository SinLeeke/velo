//! Playback sources: where `Player` gets its state from and who executes its actions.
//!
//! `Player` (engine::player) never talks to SMTC or to the Web API directly; it merges the state
//! of the registered [`PlaybackSource`]s and sends them [`SourceAction`]s. In priority order:
//! - `source_velo::VeloSource`: Velo's own librespot engine, the Connect device "Velo"
//!   (librespot events + Spirc commands; no Web API needed).
//! - `source_local::LocalSource`: the official Spotify app on this PC (SMTC events + Web API).
//! - `source_remote::RemoteSource`: another Spotify Connect device, Web API only.
//!
//! The policy deciding where new playback goes (Velo by default) is [`play_route`] and
//! [`pick_device`]; both are pure and tested below.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::watch;
use velo_domain::{Device, MusicProvider, RepeatMode, Result, VeloError};

use crate::engine::Env;

/// The track a source reports.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    /// Playable URI when the source knows it (needed for "like").
    pub uri: Option<String>,
    /// Small cover bytes from the OS media session.
    pub thumbnail: Option<Arc<Vec<u8>>>,
    /// Remote cover URL (preferred: higher resolution).
    pub cover_url: Option<String>,
}

impl SourceTrack {
    /// Same song? URIs decide when both sides have one, else title, artist and album.
    pub fn same_track(&self, other: &SourceTrack) -> bool {
        match (&self.uri, &other.uri) {
            (Some(a), Some(b)) => a == b,
            _ => {
                norm(&self.title) == norm(&other.title)
                    && norm(&self.artist) == norm(&other.artist)
                    && norm(&self.album) == norm(&other.album)
            }
        }
    }
}

pub fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Latest state of a source. Position is a measurement: `position_ms` at `position_at`.
#[derive(Debug, Clone)]
pub struct SourceState {
    /// An active device exists behind this source (`Player.hasSession`).
    pub active: bool,
    pub track: Option<SourceTrack>,
    pub is_playing: bool,
    pub position_ms: u64,
    pub position_at: Instant,
    pub volume: Option<u8>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub device_name: String,
    /// The device accepts remote commands (Web API).
    pub controllable: bool,
    /// A usable Connect device exists even though none is active yet.
    pub device_available: bool,
    /// Local Spotify process, target of the visualizer loopback.
    pub pid: Option<u32>,
    /// Remote Connect device, without a local PCM/loopback stream.
    pub remote: bool,
    /// Velo's own engine: commands go straight to Spirc (no Web API, no sign-in needed).
    pub own_engine: bool,
    /// Narración y metadatos viajan en el mismo snapshot: nunca mezclar voz/canción de eventos distintos.
    pub dj: velo_player::DjState,
}

impl Default for SourceState {
    fn default() -> Self {
        Self {
            active: false,
            track: None,
            is_playing: false,
            position_ms: 0,
            position_at: Instant::now(),
            volume: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            device_name: String::new(),
            controllable: false,
            device_available: false,
            pid: None,
            remote: false,
            own_engine: false,
            dj: velo_player::DjState::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAction {
    Play,
    Pause,
    Next,
    Previous,
    Seek(u64),
    Volume(u8),
    Shuffle(bool),
    Repeat(RepeatMode),
}

/// Why a source is asked to re-read its authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Startup,
    SignedIn,
    /// The window became visible again.
    Visible,
    AfterAction,
    TrackChanged,
}

#[async_trait]
pub trait PlaybackSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Latest state; changes are event-driven.
    fn state(&self) -> watch::Receiver<SourceState>;
    async fn act(&self, action: SourceAction) -> Result<()>;
    /// Asks the source to re-sync; returns immediately.
    fn refresh(&self, why: Refresh);
}

// ---------------------------------------------------------------------------- Web API actions

/// Executes an action through the Web API, activating a device first when none is active.
pub async fn perform_api(env: &Env, action: SourceAction) -> Result<()> {
    let api = env.account.api().ok_or(VeloError::NotAuthenticated)?;
    match run_action(&*api, action).await {
        Err(VeloError::NoActiveDevice) => {
            activate_device(env, &*api, starts_playback(action)).await?;
            // The transfer already started playback; a second "play" would be refused.
            if action == SourceAction::Play {
                tokio::time::sleep(Duration::from_millis(800)).await;
                return Ok(());
            }
            retry_device_action(|| run_action(&*api, action)).await
        }
        other => other,
    }
}

fn starts_playback(action: SourceAction) -> bool {
    matches!(
        action,
        SourceAction::Play | SourceAction::Next | SourceAction::Previous
    )
}

/// Bounded retries after a transfer: Connect takes time to expose the active device.
pub async fn retry_device_action<F, Fut>(mut request: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    for ms in [300, 600, 1000] {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        match request().await {
            Err(VeloError::NoActiveDevice) => {}
            result => return result,
        }
    }
    Err(VeloError::NoActiveDevice)
}

async fn run_action(api: &dyn MusicProvider, action: SourceAction) -> Result<()> {
    match action {
        SourceAction::Play => api.play().await,
        SourceAction::Pause => api.pause().await,
        SourceAction::Next => api.next().await,
        SourceAction::Previous => api.previous().await,
        SourceAction::Seek(ms) => api.seek(ms).await,
        SourceAction::Volume(v) => api.set_volume(v).await,
        SourceAction::Shuffle(on) => api.set_shuffle(on).await,
        SourceAction::Repeat(mode) => api.set_repeat(mode).await,
    }
}

/// Transfers playback to the preferred (or most sensible) device.
pub async fn activate_device(env: &Env, api: &dyn MusicProvider, play: bool) -> Result<()> {
    let devices = api.devices().await?;
    let preferred = env.prefs.settings().playback.preferred_device;
    let chosen = env.audio.chosen_device();
    let velo = env.audio.device_id();
    let id = pick_device(
        &devices,
        preferred.as_deref(),
        chosen.as_deref(),
        velo.as_deref(),
    )
    .ok_or(VeloError::NoActiveDevice)?;
    api.transfer(&id, play).await
}

/// Where playback starts when nothing plays: the device in the settings file (by id or name),
/// else the one the user picked in the device selector this session, else Velo's own engine,
/// else this computer, else the first usable one.
pub fn pick_device(
    devices: &[Device],
    preferred: Option<&str>,
    chosen: Option<&str>,
    velo: Option<&str>,
) -> Option<String> {
    let usable = || devices.iter().filter(|d| d.id.is_some());
    let named = |want: &str| usable().find(|d| d.id.as_deref() == Some(want) || d.name == want);
    preferred
        .and_then(named)
        .or_else(|| chosen.and_then(named))
        .or_else(|| velo.and_then(|v| usable().find(|d| d.id.as_deref() == Some(v))))
        .or_else(|| usable().find(|d| d.kind.eq_ignore_ascii_case("computer")))
        .or_else(|| usable().next())
        .and_then(|d| d.id.clone())
}

/// Who starts new playback (a playlist, an album, a list of tracks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayRoute {
    /// Velo's engine, through Spirc.
    Velo,
    /// The Web API, on the active device (or the one `activate_device` picks).
    WebApi,
}

/// Velo is the default device: new playback goes to its engine when it runs and nothing else
/// plays, unless the user picked another device in the selector. An active device is respected.
pub fn play_route(
    engine_ready: bool,
    velo_active: bool,
    other_active: bool,
    chosen_other: bool,
) -> PlayRoute {
    if engine_ready && (velo_active || (!other_active && !chosen_other)) {
        PlayRoute::Velo
    } else {
        PlayRoute::WebApi
    }
}

/// `Player.canControl` for a source state. An active source needs a controllable device and,
/// for the Web API, a Premium sign-in (Velo's engine takes Spirc commands directly). With no
/// active device the controls stay usable when Velo's engine can take over (play transfers the
/// account's playback to it).
pub fn can_control(st: &SourceState, signed_in: bool, premium: bool, engine_ready: bool) -> bool {
    let web = signed_in && premium;
    if st.active {
        st.controllable && (st.own_engine || web)
    } else {
        (engine_ready || st.device_available) && web
    }
}

/// `true` while either source reports an active device (a lower-priority source stays silent).
pub fn either_active(
    rt: &tokio::runtime::Handle,
    mut a: watch::Receiver<SourceState>,
    mut b: watch::Receiver<SourceState>,
) -> watch::Receiver<bool> {
    let now = a.borrow_and_update().active || b.borrow_and_update().active;
    let (tx, rx) = watch::channel(now);
    rt.spawn(async move {
        loop {
            tokio::select! {
                changed = a.changed() => if changed.is_err() { break },
                changed = b.changed() => if changed.is_err() { break },
                () = tx.closed() => break,
            }
            let active = a.borrow_and_update().active || b.borrow_and_update().active;
            tx.send_if_modified(|v| std::mem::replace(v, active) != active);
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_play_and_track_navigation_start_playback_on_transfer() {
        for action in [
            SourceAction::Play,
            SourceAction::Next,
            SourceAction::Previous,
        ] {
            assert!(starts_playback(action));
        }
        for action in [
            SourceAction::Pause,
            SourceAction::Seek(42),
            SourceAction::Volume(50),
            SourceAction::Shuffle(true),
            SourceAction::Repeat(RepeatMode::Context),
        ] {
            assert!(!starts_playback(action));
        }
    }

    #[tokio::test]
    async fn activation_retries_are_delayed_bounded_and_stop_at_success() {
        let started = Instant::now();
        let mut tries = 0;
        retry_device_action(|| {
            tries += 1;
            std::future::ready(if tries == 2 {
                Ok(())
            } else {
                Err(VeloError::NoActiveDevice)
            })
        })
        .await
        .unwrap();
        assert_eq!(tries, 2);
        assert!(started.elapsed() >= Duration::from_millis(900));
        let mut tries = 0;
        let result = retry_device_action(|| {
            tries += 1;
            std::future::ready(Err(VeloError::NoActiveDevice))
        })
        .await;
        assert!(matches!(result, Err(VeloError::NoActiveDevice)));
        assert_eq!(tries, 3);
    }

    #[tokio::test]
    async fn activation_does_not_retry_rate_limits_or_other_errors() {
        let mut tries = 0;
        let result = retry_device_action(|| {
            tries += 1;
            std::future::ready(Err(VeloError::RateLimited(Duration::from_secs(30))))
        })
        .await;
        assert_eq!(tries, 1);
        assert!(matches!(result, Err(VeloError::RateLimited(_))));
    }

    fn dev(id: Option<&str>, name: &str, kind: &str) -> Device {
        Device {
            id: id.map(Into::into),
            name: name.into(),
            kind: kind.into(),
            is_active: false,
            volume_percent: None,
            supports_volume: true,
        }
    }

    #[test]
    fn device_choice_prefers_setting_then_computer() {
        let devices = vec![
            dev(Some("p"), "Pixel", "Smartphone"),
            dev(Some("c"), "PC", "Computer"),
            dev(None, "Restricted", "Speaker"),
        ];
        let pick = |d: &[Device], p| pick_device(d, p, None, None);
        assert_eq!(pick(&devices, Some("Pixel")).as_deref(), Some("p"));
        assert_eq!(pick(&devices, Some("c")).as_deref(), Some("c"));
        assert_eq!(pick(&devices, Some("gone")).as_deref(), Some("c"));
        assert_eq!(pick(&devices, None).as_deref(), Some("c"));
        assert_eq!(pick(&devices[..1], None).as_deref(), Some("p"));
        assert_eq!(pick(&devices[2..], None), None);
    }

    #[test]
    fn velo_is_the_default_device_unless_the_user_chose_another() {
        let devices = vec![
            dev(Some("p"), "Pixel", "Smartphone"),
            dev(Some("c"), "PC", "Computer"),
            dev(Some("v"), "Velo", "Computer"),
        ];
        // Velo's engine wins over the other computer...
        assert_eq!(
            pick_device(&devices, None, None, Some("v")).as_deref(),
            Some("v")
        );
        // ...but the device picked in the selector and the settings file come first.
        assert_eq!(
            pick_device(&devices, None, Some("p"), Some("v")).as_deref(),
            Some("p")
        );
        assert_eq!(
            pick_device(&devices, Some("PC"), Some("p"), Some("v")).as_deref(),
            Some("c")
        );
        // Velo not listed yet (engine just started): the old order applies.
        assert_eq!(
            pick_device(&devices[..2], None, None, Some("v")).as_deref(),
            Some("c")
        );
    }

    #[test]
    fn new_playback_goes_to_velo_only_when_nothing_else_is_in_charge() {
        use PlayRoute::*;
        // (engine ready, velo active, other active, chosen other) -> route
        for (ready, velo, other, chosen, route) in [
            (true, false, false, false, Velo),
            (true, true, false, false, Velo),
            (true, true, true, true, Velo),
            (true, false, true, false, WebApi),
            (true, false, false, true, WebApi),
            (false, false, false, false, WebApi),
            (false, true, false, false, WebApi),
        ] {
            assert_eq!(
                play_route(ready, velo, other, chosen),
                route,
                "{ready} {velo} {other} {chosen}"
            );
        }
    }

    #[test]
    fn controls_follow_the_active_source_or_velo_taking_over() {
        let idle = SourceState::default();
        assert!(!can_control(&idle, true, true, false));
        assert!(can_control(&idle, true, true, true));
        assert!(!can_control(&idle, true, false, true));
        let available = SourceState {
            device_available: true,
            ..idle
        };
        assert!(can_control(&available, true, true, false));
        assert!(!can_control(&available, true, false, false));
        assert!(!can_control(&available, false, true, false));
        let web = SourceState {
            active: true,
            controllable: true,
            ..SourceState::default()
        };
        assert!(can_control(&web, true, true, false));
        assert!(!can_control(&web, false, false, true));
        let velo = SourceState {
            own_engine: true,
            ..web.clone()
        };
        assert!(can_control(&velo, false, false, true));
        let busy = SourceState {
            controllable: false,
            ..velo
        };
        assert!(!can_control(&busy, true, true, true));
    }

    #[test]
    fn either_active_follows_both_sources() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let a = watch::channel(SourceState::default()).0;
        let b = watch::channel(SourceState::default()).0;
        let mut any = either_active(rt.handle(), a.subscribe(), b.subscribe());
        assert!(!*any.borrow());
        rt.block_on(async {
            b.send_modify(|s| s.active = true);
            any.changed().await.unwrap();
            assert!(*any.borrow_and_update());
            a.send_modify(|s| s.active = true);
            b.send_modify(|s| s.active = false);
            tokio::task::yield_now().await;
            assert!(*any.borrow());
            a.send_modify(|s| s.active = false);
            any.changed().await.unwrap();
            assert!(!*any.borrow_and_update());
        });
    }

    #[test]
    fn same_track_uses_uri_then_text() {
        let t = |uri: Option<&str>, title: &str| SourceTrack {
            title: title.into(),
            artist: "A".into(),
            album: "B".into(),
            duration_ms: 1,
            uri: uri.map(Into::into),
            thumbnail: None,
            cover_url: None,
        };
        assert!(t(Some("u1"), "x").same_track(&t(Some("u1"), "y")));
        assert!(!t(Some("u1"), "x").same_track(&t(Some("u2"), "x")));
        assert!(t(None, " Song ").same_track(&t(Some("u"), "song")));
        assert!(!t(None, "a").same_track(&t(None, "b")));
    }
}
