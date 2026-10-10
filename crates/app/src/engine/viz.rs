//! Visualizer gate: connects the window/playback flags and the user's settings to the FFT engine.
//! Event-driven (no timers). With `running` false the engine holds no capture stream.

use std::sync::Arc;
use std::time::{Duration, Instant};

use velo_domain::Visualizer;

use super::Env;
use super::clock::running;

/// Also throttle Qt delivery, so frames queued during a busy UI cannot arrive in a burst.
pub struct FrameGate {
    last: Option<Instant>,
    hz: u32,
}

impl Default for FrameGate {
    fn default() -> Self {
        Self {
            last: None,
            hz: velo_viz::DEFAULT_FPS,
        }
    }
}

impl FrameGate {
    /// Sets the display's refresh rate (clamped to 30..=240 Hz).
    pub fn set_hz(&mut self, hz: f64) {
        self.hz = velo_viz::clamp_fps(hz);
    }

    pub fn accept(&mut self, flags: &super::clock::Flags, now: Instant) -> bool {
        // 90 % of the frame period: delivery jitter must not drop every other frame.
        let period = Duration::from_nanos(900_000_000_u64 / u64::from(self.hz));
        if !running(flags)
            || flags.remote
            || self
                .last
                .is_some_and(|last| now.saturating_duration_since(last) < period)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

pub fn start(env: &Arc<Env>, player: &Arc<super::player::PlayerCtl>) {
    // enabled ∧ playing ∧ visible ∧ focused → capture + FFT on/off (and which process to target).
    super::viz_pcm::start(env, player);

    // User settings: sensitivity goes to the engine, the switch into the flags.
    let env = env.clone();
    let mut settings = env.prefs.watch_settings();
    env.rt.clone().spawn(async move {
        loop {
            let (sensitivity, enabled) = {
                let s = settings.borrow_and_update();
                (s.visualizer.sensitivity, viz_switch(&s))
            };
            env.viz.set_sensitivity(sensitivity);
            env.update_flags(|f| {
                f.viz_enabled = enabled;
                let s = settings.borrow();
                f.performance_mode = s.appearance.performance_mode;
                f.performance_keep_visualizer = s.appearance.performance_keep_visualizer;
                f.pause_in_background = s.appearance.pause_in_background;
                // C1 owns the schema; read via the existing Settings API during integration.
                f.library_bar_enabled = serde_json::to_value(&*s)
                    .ok()
                    .and_then(|v| {
                        v["visualizer"]["player_bar_mode"]
                            .as_str()
                            .map(|m| matches!(m, "bars" | "wave"))
                    })
                    .unwrap_or(false);
            });
            if settings.changed().await.is_err() {
                break;
            }
        }
    });
}

/// The visualizer's effective switch: the user's toggle, forced off by Modo rendimiento unless
/// `performance_keep_visualizer` is set (the toggle itself is kept, so leaving the mode brings
/// the visualizer back).
pub fn viz_switch(settings: &velo_domain::Settings) -> bool {
    settings.visualizer.enabled
        && (!settings.appearance.performance_mode
            || settings.appearance.performance_keep_visualizer)
}

#[cfg(test)]
mod tests {
    #[test]
    fn surface_demand_truth_table() {
        for bits in 0_u32..2048 {
            let f = super::super::clock::Flags {
                local_source_available: bits & 1024 != 0,
                playing: bits & 1 != 0,
                visible: bits & 2 != 0,
                focused: bits & 4 != 0,
                viz_enabled: bits & 8 != 0,
                now_playing_visible: bits & 16 != 0,
                library_bar_visible: bits & 32 != 0,
                library_bar_enabled: bits & 64 != 0,
                performance_mode: bits & 128 != 0,
                pause_in_background: bits & 256 != 0,
                remote: bits & 512 != 0,
                ..Default::default()
            };
            let demand = (bits & 24 == 24) || (bits & 96 == 96);
            assert_eq!(
                super::running(&f),
                demand
                    && bits & 1024 != 0
                    && bits & 3 == 3
                    && bits & 128 == 0
                    && bits & 512 == 0
                    && (bits & 4 != 0 || bits & 256 == 0)
            );
        }
    }

    use super::*;

    #[test]
    fn performance_mode_turns_the_visualizer_off_without_changing_the_toggle() {
        let mut s = velo_domain::Settings::default();
        assert!(viz_switch(&s));
        s.appearance.performance_mode = true;
        assert!(!viz_switch(&s));
        assert!(s.visualizer.enabled);
        s.appearance.performance_keep_visualizer = true;
        assert!(viz_switch(&s));
        s.appearance.performance_keep_visualizer = false;
        s.appearance.performance_mode = false;
        s.visualizer.enabled = false;
        assert!(!viz_switch(&s));
    }

    #[test]
    fn queued_frames_obey_the_display_rate_and_stop_for_hidden_paused_or_remote_playback() {
        let t = Instant::now();
        let flags = super::super::clock::Flags {
            playing: true,
            visible: true,
            focused: true,
            viz_enabled: true,
            local_source_available: true,
            now_playing_visible: true,
            pause_in_background: true,
            ..Default::default()
        };
        let mut gate = FrameGate::default();
        gate.set_hz(30.0);
        let accepted = (0..1000)
            .filter(|ms| gate.accept(&flags, t + Duration::from_millis(*ms)))
            .count();
        assert!(accepted <= 34, "{accepted}");
        gate.set_hz(144.0);
        gate.last = None;
        let accepted = (0..1000)
            .filter(|ms| gate.accept(&flags, t + Duration::from_millis(*ms)))
            .count();
        assert!((130..=145).contains(&accepted), "{accepted}");
        gate.set_hz(f64::NAN);
        assert_eq!(gate.hz, 60);
        gate.set_hz(30.0);
        gate.last = None;
        // Back at 30 Hz the stop conditions and burst rejection are the same.
        for stopped in [
            super::super::clock::Flags {
                playing: false,
                ..flags
            },
            super::super::clock::Flags {
                visible: false,
                ..flags
            },
            super::super::clock::Flags {
                focused: false,
                ..flags
            },
            super::super::clock::Flags {
                viz_enabled: false,
                ..flags
            },
            super::super::clock::Flags {
                remote: true,
                ..flags
            },
        ] {
            assert!(!gate.accept(&stopped, t + Duration::from_secs(2)));
        }
        assert!(gate.accept(&flags, t + Duration::from_secs(2)));
        assert!(!gate.accept(&flags, t + Duration::from_secs(2)));
    }
}
