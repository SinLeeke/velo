//! Pure policy for the playback clock and the visualizer gate (no Qt, no async).

use std::time::{Duration, Instant};

/// How often `Player.positionMs` may change while it is allowed to tick (4 Hz).
pub const TICK: Duration = Duration::from_millis(250);

/// Everything that decides whether periodic work may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    pub playing: bool,
    /// Window shown and not minimized.
    pub visible: bool,
    pub focused: bool,
    /// The user's visualizer switch.
    pub viz_enabled: bool,
    pub now_playing_visible: bool,
    pub library_bar_visible: bool,
    pub library_bar_enabled: bool,
    pub performance_mode: bool,
    /// Opción "mantener el visualizador en Modo rendimiento".
    pub performance_keep_visualizer: bool,
    pub pause_in_background: bool,
    /// Process of the local Spotify app, target of the per-process loopback.
    pub pid: Option<u32>,
    /// Remote Connect playback has no local audio to visualize.
    pub remote: bool,
    pub local_source_available: bool,
}

/// `Visualizer.running`: a visible consumer, playing, visible window and local audio;
/// background/performance preferences gate it. Publication follows the display refresh rate.
pub fn running(f: &Flags) -> bool {
    ((f.now_playing_visible && f.viz_enabled) || (f.library_bar_visible && f.library_bar_enabled))
        && f.playing
        && f.visible
        && !f.remote
        && f.local_source_available
        && (!f.performance_mode || f.performance_keep_visualizer)
        && (f.focused || !f.pause_in_background)
}

/// The position clock only ticks while the music plays and the window is visible.
pub fn should_tick(f: &Flags) -> bool {
    f.playing && f.visible
}

/// Gates periodic publication across visibility changes; events publish directly.
#[derive(Default)]
pub struct PositionGate {
    last: Option<Instant>,
}

impl PositionGate {
    pub fn accept(&mut self, flags: &Flags, now: Instant) -> bool {
        if !should_tick(flags)
            || self.last.is_some_and(|last| {
                now.saturating_duration_since(last) < TICK - Duration::from_millis(50)
            })
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// Local interpolation of the playback position from the last measurement.
#[derive(Debug, Clone)]
pub struct Clock {
    anchor_ms: u64,
    anchor_at: Instant,
    playing: bool,
    duration_ms: u64,
}

impl Clock {
    pub fn new(now: Instant) -> Self {
        Self {
            anchor_ms: 0,
            anchor_at: now,
            playing: false,
            duration_ms: 0,
        }
    }

    /// A measurement: the position was `position_ms` at `at`.
    pub fn set(&mut self, position_ms: u64, at: Instant, playing: bool) {
        self.anchor_ms = position_ms;
        self.anchor_at = at;
        self.playing = playing;
    }

    pub fn set_duration(&mut self, duration_ms: u64) {
        self.duration_ms = duration_ms;
    }

    /// Switches play/pause without moving the position.
    pub fn set_playing(&mut self, playing: bool, now: Instant) {
        if playing != self.playing {
            self.anchor_ms = self.now_ms(now);
            self.anchor_at = now;
            self.playing = playing;
        }
    }

    /// Instant of the latest measurement.
    pub fn anchor_at(&self) -> Instant {
        self.anchor_at
    }

    pub fn now_ms(&self, now: Instant) -> u64 {
        let mut p = self.anchor_ms;
        if self.playing {
            p = p.saturating_add(
                u64::try_from(now.saturating_duration_since(self.anchor_at).as_millis())
                    .unwrap_or(u64::MAX),
            );
        }
        if self.duration_ms > 0 {
            p = p.min(self.duration_ms);
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags() -> Flags {
        Flags {
            playing: true,
            visible: true,
            focused: true,
            viz_enabled: true,
            pid: None,
            remote: false,
            local_source_available: true,
            now_playing_visible: true,
            pause_in_background: true,
            ..Default::default()
        }
    }

    #[test]
    fn periodic_publication_rejects_bursts_and_stops_when_idle() {
        let now = Instant::now();
        let mut gate = PositionGate::default();
        assert!(gate.accept(&flags(), now));
        assert!(!gate.accept(&flags(), now + Duration::from_millis(199)));
        assert!(!gate.accept(
            &Flags {
                playing: false,
                ..flags()
            },
            now + TICK
        ));
        assert!(!gate.accept(
            &Flags {
                visible: false,
                ..flags()
            },
            now + TICK
        ));
        assert!(gate.accept(&flags(), now + TICK));
        assert!(!gate.accept(&flags(), now + TICK));
    }

    #[test]
    fn ticker_jitter_does_not_drop_every_other_position() {
        let now = Instant::now();
        let mut gate = PositionGate::default();
        for ms in [0, 251, 499, 753, 999, 1250] {
            assert!(gate.accept(&flags(), now + Duration::from_millis(ms)));
        }
    }

    #[test]
    fn viz_runs_only_when_everything_allows() {
        assert!(running(&flags()));
        assert!(!running(&Flags {
            playing: false,
            ..flags()
        }));
        assert!(!running(&Flags {
            visible: false,
            ..flags()
        }));
        assert!(!running(&Flags {
            focused: false,
            ..flags()
        }));
        assert!(!running(&Flags {
            viz_enabled: false,
            ..flags()
        }));
    }

    #[test]
    fn clock_ticks_only_playing_and_visible() {
        assert!(should_tick(&flags()));
        assert!(!should_tick(&Flags {
            playing: false,
            ..flags()
        }));
        assert!(!should_tick(&Flags {
            visible: false,
            ..flags()
        }));
        // Focus does not matter for the clock.
        assert!(should_tick(&Flags {
            focused: false,
            ..flags()
        }));
        assert!(TICK >= Duration::from_millis(250));
    }

    #[test]
    fn clock_interpolates_and_pauses() {
        let t0 = Instant::now();
        let mut c = Clock::new(t0);
        c.set(10_000, t0, true);
        assert_eq!(c.now_ms(t0 + Duration::from_millis(1500)), 11_500);
        c.set_playing(false, t0 + Duration::from_secs(2));
        assert_eq!(c.now_ms(t0 + Duration::from_secs(60)), 12_000);
        c.set_playing(true, t0 + Duration::from_secs(60));
        assert_eq!(c.now_ms(t0 + Duration::from_secs(61)), 13_000);
    }

    #[test]
    fn clock_clamps_to_duration_and_ignores_past_instants() {
        let t0 = Instant::now();
        let mut c = Clock::new(t0);
        c.set_duration(5_000);
        c.set(4_000, t0, true);
        assert_eq!(c.now_ms(t0 + Duration::from_secs(10)), 5_000);
        assert_eq!(c.now_ms(t0), 4_000);
    }

    #[test]
    fn clock_reanchors_after_seek_and_saturates_unknown_duration() {
        let t0 = Instant::now();
        let mut c = Clock::new(t0);
        c.set(30_000, t0, true);
        c.set(5_000, t0 + Duration::from_secs(2), true);
        assert_eq!(c.now_ms(t0 + Duration::from_secs(3)), 6_000);
        c.set(u64::MAX, t0, true);
        assert_eq!(c.now_ms(t0 + Duration::from_secs(1)), u64::MAX);
    }

    #[test]
    fn every_window_and_playback_combination_obeys_the_gates() {
        for bits in 0..16 {
            let f = Flags {
                playing: bits & 1 != 0,
                visible: bits & 2 != 0,
                focused: bits & 4 != 0,
                viz_enabled: bits & 8 != 0,
                now_playing_visible: true,
                pause_in_background: true,
                local_source_available: true,
                ..Flags::default()
            };
            assert_eq!(should_tick(&f), bits & 3 == 3);
            assert_eq!(running(&f), bits == 15);
        }
    }

    #[test]
    fn visualizer_demand_table() {
        let base = Flags {
            playing: true,
            visible: true,
            focused: true,
            viz_enabled: true,
            now_playing_visible: true,
            library_bar_enabled: true,
            local_source_available: true,
            ..Flags::default()
        };
        assert!(running(&base));
        // No consumer visible.
        assert!(!running(&Flags {
            now_playing_visible: false,
            ..base
        }));
        // Library bar alone is a consumer when enabled.
        assert!(running(&Flags {
            now_playing_visible: false,
            library_bar_visible: true,
            ..base
        }));
        assert!(!running(&Flags {
            playing: false,
            ..base
        }));
        assert!(!running(&Flags {
            visible: false,
            ..base
        }));
        assert!(!running(&Flags {
            remote: true,
            ..base
        }));
        assert!(!running(&Flags {
            local_source_available: false,
            ..base
        }));
        assert!(!running(&Flags {
            performance_mode: true,
            ..base
        }));
        // Modo rendimiento + "mantener el visualizador" sí corre; la opción sola no cambia nada.
        assert!(running(&Flags {
            performance_mode: true,
            performance_keep_visualizer: true,
            ..base
        }));
        assert!(running(&Flags {
            performance_keep_visualizer: true,
            ..base
        }));
        assert!(!running(&Flags {
            performance_mode: true,
            performance_keep_visualizer: true,
            playing: false,
            ..base
        }));
        // Without focus it only stops when "pause in background" is on.
        assert!(running(&Flags {
            focused: false,
            ..base
        }));
        assert!(!running(&Flags {
            focused: false,
            pause_in_background: true,
            ..base
        }));
    }
}
