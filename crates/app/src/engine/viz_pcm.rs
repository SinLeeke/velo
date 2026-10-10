//! Event-driven Spotify-only input routing and opt-in PCM lifecycle.
use std::sync::Arc;
use std::time::Instant;

use velo_domain::Visualizer;
use velo_player::{Engine, PcmPacket};

use super::clock::{Flags, running};
use super::{Env, player::PlayerCtl};
use crate::bridge::source::SourceState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    Off,
    Engine,
    Spotify(u32),
}

fn input(flags: &Flags, source: &SourceState) -> Input {
    if !running(flags) || !source.active || !source.is_playing || source.remote {
        Input::Off
    } else if source.own_engine {
        Input::Engine
    } else {
        source
            .pid
            .filter(|pid| *pid != 0)
            .map_or(Input::Off, Input::Spotify)
    }
}

#[derive(Default)]
struct TapGate {
    since: Option<Instant>,
}

impl TapGate {
    fn set(&mut self, active: bool, now: Instant) {
        self.since = active.then_some(now);
    }

    fn accepts(&self, captured_at: Instant) -> bool {
        self.since.is_some_and(|since| captured_at >= since)
    }
}

fn drain(rx: &mut Option<tokio::sync::mpsc::Receiver<PcmPacket>>) {
    if let Some(rx) = rx {
        while rx.try_recv().is_ok() {}
    }
}

trait Tap {
    fn enable(&self, enabled: bool);
}

impl Tap for Engine {
    fn enable(&self, enabled: bool) {
        self.set_pcm_enabled(enabled);
    }
}

fn configure(
    next: Input,
    tap: Option<&dyn Tap>,
    pcm: &mut Option<tokio::sync::mpsc::Receiver<PcmPacket>>,
    gate: &mut TapGate,
    viz: &velo_viz::VizEngine,
) {
    // Disable before draining; timestamps also reject any in-flight old packet.
    if let Some(tap) = tap {
        tap.enable(false);
    }
    gate.set(false, Instant::now());
    drain(pcm);
    match next {
        Input::Engine => {
            let available = tap.is_some() && pcm.is_some();
            viz.set_external_active(available);
            gate.set(available, Instant::now());
            if let Some(tap) = tap {
                tap.enable(available);
            }
        }
        Input::Spotify(pid) => viz.set_active(true, Some(pid)),
        Input::Off => viz.set_active(false, None),
    }
}

pub fn start(env: &Arc<Env>, player: &Arc<PlayerCtl>) {
    let env = env.clone();
    let mut flags = env.flags.subscribe();
    let mut source = player.media_state.subscribe();
    let mut audio = env.audio.view.subscribe();
    env.rt.clone().spawn(async move {
        let mut engine: Option<Arc<Engine>> = None;
        let mut pcm = None;
        let mut selected = Input::Off;
        let mut gate = TapGate::default();
        loop {
            let next_engine = env.audio.engine();
            let replaced = match (&engine, &next_engine) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if replaced {
                if let Some(old) = &engine {
                    old.set_pcm_enabled(false);
                }
                gate.set(false, Instant::now());
                drain(&mut pcm);
                env.viz.set_active(false, None);
                engine = next_engine;
                pcm = engine.as_ref().and_then(|e| e.take_pcm());
            }
            let local_source = {
                let s = source.borrow();
                s.active
                    && !s.remote
                    && if s.own_engine {
                        engine.is_some() && pcm.is_some()
                    } else {
                        s.pid.is_some_and(|pid| pid != 0)
                    }
            };
            env.update_flags(|f| f.local_source_available = local_source);
            let next = input(&flags.borrow_and_update(), &source.borrow_and_update());
            audio.borrow_and_update();
            if replaced || selected != next {
                configure(
                    next,
                    engine.as_ref().map(|e| e.as_ref() as &dyn Tap),
                    &mut pcm,
                    &mut gate,
                    &env.viz,
                );
                selected = next;
            }
            tokio::select! {
                biased;
                changed = flags.changed() => if changed.is_err() { break },
                changed = source.changed() => if changed.is_err() { break },
                changed = audio.changed() => if changed.is_err() { break },
                packet = async {
                    match &mut pcm {
                        Some(rx) if gate.since.is_some() => rx.recv().await,
                        _ => std::future::pending().await,
                    }
                } => {
                    match packet {
                        Some(p) if gate.accepts(p.captured_at) => {
                            env.viz.submit_pcm(p.samples, p.sample_rate, p.channels);
                        }
                        Some(_) => {},
                        None => {
                            if let Some(engine) = &engine { engine.set_pcm_enabled(false); }
                            gate.set(false, Instant::now());
                            env.viz.set_external_active(false);
                        }
                    }
                }
            }
        }
        if let Some(engine) = engine {
            engine.set_pcm_enabled(false);
        }
        drain(&mut pcm);
        env.viz.set_active(false, None);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_disables_the_tap_drains_old_audio_and_requires_a_receiver() {
        struct FakeTap(std::sync::atomic::AtomicBool);
        impl Tap for FakeTap {
            fn enable(&self, enabled: bool) {
                self.0.store(enabled, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let tap = FakeTap(Default::default());
        let viz = velo_viz::VizEngine::new();
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        let mut pcm = Some(rx);
        let mut gate = TapGate::default();
        configure(Input::Engine, Some(&tap), &mut pcm, &mut gate, &viz);
        assert!(tap.0.load(std::sync::atomic::Ordering::SeqCst));
        tx.try_send(PcmPacket {
            captured_at: Instant::now(),
            samples: vec![0.5; 4],
            sample_rate: 44_100,
            channels: 2,
        })
        .unwrap_or_else(|_| panic!("empty channel"));
        configure(Input::Off, Some(&tap), &mut pcm, &mut gate, &viz);
        assert!(!tap.0.load(std::sync::atomic::Ordering::SeqCst));
        assert!(gate.since.is_none());
        assert!(pcm.as_mut().unwrap().try_recv().is_err());
        configure(Input::Engine, Some(&tap), &mut pcm, &mut gate, &viz);
        assert!(tap.0.load(std::sync::atomic::Ordering::SeqCst));
        configure(Input::Engine, Some(&tap), &mut None, &mut gate, &viz);
        assert!(!tap.0.load(std::sync::atomic::Ordering::SeqCst));
        assert!(gate.since.is_none());
    }

    #[test]
    fn only_own_pcm_or_the_official_spotify_process_can_be_selected() {
        let flags = Flags {
            playing: true,
            visible: true,
            focused: true,
            viz_enabled: true,
            local_source_available: true,
            now_playing_visible: true,
            pause_in_background: true,
            ..Default::default()
        };
        let own = SourceState {
            active: true,
            is_playing: true,
            own_engine: true,
            pid: Some(7),
            ..Default::default()
        };
        assert_eq!(input(&flags, &own), Input::Engine);
        let official = SourceState {
            own_engine: false,
            ..own.clone()
        };
        assert_eq!(input(&flags, &official), Input::Spotify(7));
        for source in [
            SourceState {
                remote: true,
                ..own.clone()
            },
            SourceState {
                active: false,
                ..own.clone()
            },
            SourceState {
                is_playing: false,
                ..own.clone()
            },
            SourceState {
                pid: None,
                ..official.clone()
            },
            SourceState {
                pid: Some(0),
                ..official.clone()
            },
        ] {
            assert_eq!(input(&flags, &source), Input::Off);
        }
        for flags in [
            Flags {
                visible: false,
                ..flags
            },
            Flags {
                focused: false,
                ..flags
            },
            Flags {
                playing: false,
                ..flags
            },
            Flags {
                viz_enabled: false,
                ..flags
            },
        ] {
            assert_eq!(input(&flags, &own), Input::Off);
            assert_eq!(input(&flags, &official), Input::Off);
        }
    }

    #[test]
    fn tap_off_and_reactivation_reject_old_and_in_flight_packets() {
        let now = Instant::now();
        let mut gate = TapGate::default();
        assert!(!gate.accepts(now));
        gate.set(true, now);
        assert!(gate.accepts(now));
        gate.set(false, now);
        assert!(!gate.accepts(now));
        gate.set(true, now + std::time::Duration::from_secs(1));
        assert!(!gate.accepts(now));
        assert!(gate.accepts(now + std::time::Duration::from_secs(1)));
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(PcmPacket {
            captured_at: now,
            samples: vec![0.5; 4],
            sample_rate: 44_100,
            channels: 2,
        })
        .unwrap_or_else(|_| panic!("empty channel"));
        let mut rx = Some(rx);
        drain(&mut rx);
        assert!(rx.as_mut().unwrap().try_recv().is_err());
    }
}
