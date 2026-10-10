//! Velo's window-owned SMTC session. Event-driven; all WinRT calls stay off Qt.
use tokio::sync::{mpsc, watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Play,
    Pause,
    Next,
    Previous,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub enabled: bool,
    pub controllable: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Spotify image URI or an existing cached file URI; never downloads on the UI thread.
    pub cover_url: String,
    pub position_ms: u64,
    pub duration_ms: u64,
}

impl Snapshot {
    pub fn allows(&self, button: Button) -> bool {
        self.enabled
            && self.controllable
            && match button {
                Button::Play => !self.playing,
                Button::Pause => self.playing,
                Button::Next | Button::Previous => true,
            }
    }

    /// Windows uses signed 100 ns units.
    pub fn timeline(&self) -> (i64, i64) {
        let ticks = |ms: u64| ms.saturating_mul(10_000).min(i64::MAX as u64) as i64;
        (
            ticks(self.position_ms.min(self.duration_ms)),
            ticks(self.duration_ms),
        )
    }
}

pub struct Publisher {
    state: watch::Sender<Option<Snapshot>>,
}

impl Publisher {
    /// `hwnd` identifies the live main window. Drop requests asynchronous session cleanup.
    pub fn new(hwnd: usize, buttons: mpsc::UnboundedSender<Button>) -> std::io::Result<Self> {
        let (state, rx) = watch::channel(Some(Snapshot::default()));
        #[cfg(windows)]
        {
            let runtime = tokio::runtime::Handle::current();
            std::thread::Builder::new()
                .name("velo-smtc".into())
                .spawn(move || {
                    if let Err(error) = platform::run(hwnd, buttons, rx, runtime) {
                        tracing::warn!(%error, "Velo SMTC unavailable");
                    }
                })?;
        }
        #[cfg(not(windows))]
        let _ = (hwnd, buttons, rx);
        Ok(Self { state })
    }

    pub fn update(&self, snapshot: Snapshot) {
        self.state.send_if_modified(|old| {
            if old.as_ref() == Some(&snapshot) {
                false
            } else {
                *old = Some(snapshot);
                true
            }
        });
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.state.send_replace(None);
    }
}

#[cfg(windows)]
#[path = "publisher_windows.rs"]
mod platform;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buttons_follow_session_capability_and_playback_state() {
        let mut s = Snapshot {
            enabled: true,
            controllable: true,
            ..Default::default()
        };
        assert!(s.allows(Button::Play) && s.allows(Button::Next) && s.allows(Button::Previous));
        assert!(!s.allows(Button::Pause));
        s.playing = true;
        assert!(!s.allows(Button::Play) && s.allows(Button::Pause));
        for (enabled, controllable) in [(false, true), (true, false), (false, false)] {
            s.enabled = enabled;
            s.controllable = controllable;
            for button in [Button::Play, Button::Pause, Button::Next, Button::Previous] {
                assert!(!s.allows(button));
            }
        }
    }
    #[test]
    fn timeline_clamps_position_and_saturates_windows_units() {
        let mut s = Snapshot {
            position_ms: 3000,
            duration_ms: 2000,
            ..Default::default()
        };
        assert_eq!(s.timeline(), (20_000_000, 20_000_000));
        s.duration_ms = u64::MAX;
        s.position_ms = u64::MAX;
        assert_eq!(s.timeline(), (i64::MAX, i64::MAX));
        assert_eq!(Snapshot::default().timeline(), (0, 0));
    }
}
