//! Native media publication follows authoritative source events, never the position timer.
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use velo_nowplaying::publisher::{Button, Publisher, Snapshot};

use super::{
    Env,
    player::{PlayerCtl, PlayerView},
};
use crate::bridge::source::{Refresh, SourceAction, SourceState};

static WINDOW: OnceLock<watch::Sender<usize>> = OnceLock::new();
fn window() -> &'static watch::Sender<usize> {
    WINDOW.get_or_init(|| watch::channel(0).0)
}

pub fn set_window_handle(hwnd: usize) {
    window().send_replace(hwnd);
}

/// Called by the Qt chrome when the main HWND becomes available or is destroyed.
#[unsafe(no_mangle)]
pub extern "C" fn velo_media_window_changed(hwnd: usize) {
    set_window_handle(hwnd);
}

fn snapshot(source: &SourceState, view: &PlayerView, now: Instant) -> Snapshot {
    if !source.own_engine || !source.active || source.remote {
        return Snapshot::default();
    }
    let Some(track) = &source.track else {
        return Snapshot::default();
    };
    let cached =
        view.title == track.title && view.artist == track.artist && view.album == track.album;
    let cover_url = if cached && !view.cover_url.is_empty() {
        view.cover_url.clone()
    } else {
        track.cover_url.clone().unwrap_or_default()
    };
    let elapsed = if source.is_playing {
        u64::try_from(
            now.saturating_duration_since(source.position_at)
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
    } else {
        0
    };
    Snapshot {
        enabled: true,
        controllable: source.controllable,
        playing: source.is_playing,
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        cover_url,
        position_ms: source
            .position_ms
            .saturating_add(elapsed)
            .min(track.duration_ms),
        duration_ms: track.duration_ms,
    }
}

fn action(button: Button, state: &Snapshot) -> Option<SourceAction> {
    state.allows(button).then_some(match button {
        Button::Play => SourceAction::Play,
        Button::Pause => SourceAction::Pause,
        Button::Next => SourceAction::Next,
        Button::Previous => SourceAction::Previous,
    })
}

pub fn start(env: &Arc<Env>, player: &Arc<PlayerCtl>) {
    let env = env.clone();
    let mut source = player.media_state.subscribe();
    let mut view = player.view.subscribe();
    let mut hwnd = window().subscribe();
    let player = player.clone();
    env.rt.clone().spawn(async move {
        let (buttons, mut presses) = mpsc::unbounded_channel();
        let mut publisher = None;
        let mut current_hwnd = 0;
        loop {
            let handle = *hwnd.borrow_and_update();
            if current_hwnd != handle {
                publisher = None;
                current_hwnd = handle;
                if handle != 0 {
                    publisher = Publisher::new(handle, buttons.clone()).inspect_err(|error| {
                        tracing::warn!(%error, "could not start Velo SMTC worker");
                    }).ok();
                }
            }
            if let Some(publisher) = &publisher {
                publisher.update(snapshot(&source.borrow_and_update(), &view.borrow_and_update(), Instant::now()));
            }
            tokio::select! {
                biased;
                changed = hwnd.changed() => if changed.is_err() { break },
                changed = source.changed() => if changed.is_err() { break },
                changed = view.changed() => if changed.is_err() { break },
                Some(button) = presses.recv() => {
                    // Revalidate against current authority: late buttons cannot control another device.
                    let target = player.active_source();
                    let state = snapshot(&target.state().borrow(), &player.view.borrow(), Instant::now());
                    if let Some(action) = action(button, &state) {
                        let env = env.clone();
                        env.rt.clone().spawn(async move {
                            let current = target.state().borrow().clone();
                            if !current.active || !current.own_engine || !current.controllable { return; }
                            if let Err(error) = target.act(action).await { env.report(&error); }
                            target.refresh(Refresh::AfterAction);
                        });
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::source::SourceTrack;

    #[test]
    fn windows_voice_snapshot_uses_livi_timeline_and_returns_to_song_metadata() {
        let at = Instant::now();
        let mut engine = velo_player::EngineState {
            active: true,
            ..Default::default()
        };
        engine.dj.active = true;
        engine.dj.speech = Some(velo_player::DjSpeech {
            title: "Siguiente bloque".into(),
            artist: "DJ Livi".into(),
            album: "DJ".into(),
            image_url: None,
            line: "Hola".into(),
            duration_ms: 5000,
            position_ms: 1000,
            position_at: at,
            playing: true,
        });
        let view = PlayerView {
            title: "Siguiente bloque".into(),
            artist: "DJ Livi".into(),
            album: "DJ".into(),
            cover_url: "file:///C:/cache/livi.png".into(),
            ..Default::default()
        };
        let source = crate::bridge::source_velo::state_of(&engine, "Velo");
        let voice = snapshot(&source, &view, at + std::time::Duration::from_secs(1));
        assert_eq!(
            (
                voice.title.as_str(),
                voice.artist.as_str(),
                voice.duration_ms,
                voice.position_ms
            ),
            ("Siguiente bloque", "DJ Livi", 5000, 2000)
        );
        assert_eq!(voice.cover_url, view.cover_url);
        assert_eq!(action(Button::Pause, &voice), Some(SourceAction::Pause));
        engine.dj.speech.as_mut().unwrap().playing = false;
        let source = crate::bridge::source_velo::state_of(&engine, "Velo");
        assert_eq!(
            action(Button::Play, &snapshot(&source, &view, at)),
            Some(SourceAction::Play)
        );
        engine.dj.speech = None;
        engine.track = Some(velo_player::EngineTrack {
            uri: "spotify:track:x".into(),
            title: "Canción".into(),
            artist: "Artista".into(),
            album: "Álbum".into(),
            cover_url: Some("https://i.scdn.co/image/song".into()),
            duration_ms: 200_000,
        });
        let source = crate::bridge::source_velo::state_of(&engine, "Velo");
        let song = snapshot(&source, &view, at);
        assert_eq!(
            (
                song.title.as_str(),
                song.artist.as_str(),
                song.cover_url.as_str()
            ),
            ("Canción", "Artista", "https://i.scdn.co/image/song")
        );
    }

    #[test]
    fn publishes_only_the_own_active_engine_and_uses_cached_cover() {
        let now = Instant::now();
        let source = SourceState {
            active: true,
            own_engine: true,
            controllable: true,
            is_playing: true,
            position_ms: 1000,
            position_at: now,
            track: Some(SourceTrack {
                title: "Marea".into(),
                artist: "Lumen".into(),
                album: "Atlas".into(),
                duration_ms: 9000,
                uri: None,
                thumbnail: None,
                cover_url: Some("https://i.scdn.co/image/x".into()),
            }),
            ..Default::default()
        };
        let view = PlayerView {
            title: "Marea".into(),
            artist: "Lumen".into(),
            album: "Atlas".into(),
            cover_url: "file:///C:/cache/cover.jpg".into(),
            ..Default::default()
        };
        let state = snapshot(&source, &view, now + std::time::Duration::from_secs(1));
        assert_eq!(state.cover_url, view.cover_url);
        assert_eq!(state.position_ms, 2000);
        assert_eq!(action(Button::Pause, &state), Some(SourceAction::Pause));
        assert_eq!(action(Button::Play, &state), None);
        assert_eq!(action(Button::Next, &state), Some(SourceAction::Next));
        assert_eq!(
            action(Button::Previous, &state),
            Some(SourceAction::Previous)
        );
        let paused = snapshot(
            &SourceState {
                is_playing: false,
                ..source.clone()
            },
            &view,
            now,
        );
        assert_eq!(action(Button::Play, &paused), Some(SourceAction::Play));
        assert_eq!(paused.position_ms, 1000);
        assert_eq!(
            snapshot(&source, &PlayerView::default(), now).cover_url,
            "https://i.scdn.co/image/x"
        );
        for source in [
            SourceState {
                own_engine: false,
                ..source.clone()
            },
            SourceState {
                active: false,
                ..source.clone()
            },
            SourceState {
                remote: true,
                ..source.clone()
            },
            SourceState {
                track: None,
                ..source
            },
        ] {
            let state = snapshot(&source, &view, now);
            assert_eq!(state, Snapshot::default());
            assert_eq!(action(Button::Next, &state), None);
        }
    }
}
