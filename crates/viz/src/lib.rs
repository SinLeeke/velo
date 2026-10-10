//! Spotify-only decorative visualizer. No system/default-device capture path.
//! The bridge passes enabled && playing && visible && focused to `set_active`.
//! An inactive worker releases WASAPI and waits without a timer.

mod analysis;
#[cfg(windows)]
mod capture;
mod external;

use analysis::{Analyzer, Bands, FrameLimiter, Ring, sensitivity, simulated, smooth};
pub use analysis::{DEFAULT_FPS, MAX_FPS, MIN_FPS, clamp_fps};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use velo_domain::{VIZ_BANDS, Visualizer};

struct Control {
    active: bool,
    pid: Option<u32>,
    generation: u64,
    sensitivity: f32,
    simulated: bool,
    shutdown: bool,
    external: bool,
    samples: std::collections::VecDeque<external::Samples>,
}

struct Shared {
    control: Mutex<Control>,
    changed: Condvar,
    /// Target publication rate (the display's refresh rate), read by the worker each loop.
    fps: AtomicU32,
    bands: watch::Sender<Bands>,
    #[cfg(windows)]
    event: Option<capture::Event>,
}

impl Shared {
    fn current(&self, generation: u64) -> bool {
        let c = self.control.lock().unwrap();
        c.active && !c.shutdown && c.generation == generation
    }

    fn wake(&self) {
        self.changed.notify_one();
        #[cfg(windows)]
        if let Some(event) = &self.event {
            event.signal();
        }
    }

    fn publish(&self, generation: u64, bands: Bands) {
        let c = self.control.lock().unwrap();
        if c.active && !c.shutdown && c.generation == generation {
            self.bands.send_if_modified(|old| {
                if analysis::changed(old, &bands) {
                    *old = bands;
                    true
                } else {
                    false
                }
            });
        }
    }
}

/// A background worker owns WASAPI, its circular buffer and reusable FFT plan.
pub struct VizEngine {
    shared: Arc<Shared>,
    finished: Mutex<mpsc::Receiver<()>>,
}

impl Default for VizEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl VizEngine {
    /// Starts idle. Missing/unusable Spotify PIDs select simulation only while
    /// active. Capture initialization runs asynchronously on the worker.
    pub fn new() -> Self {
        Self::create(false)
    }

    fn create(force_simulated: bool) -> Self {
        let (bands, _) = watch::channel([0.0; VIZ_BANDS]);
        let shared = Arc::new(Shared {
            control: Mutex::new(Control {
                active: false,
                pid: None,
                generation: 0,
                sensitivity: 1.0,
                simulated: true,
                shutdown: false,
                external: false,
                samples: Default::default(),
            }),
            changed: Condvar::new(),
            fps: AtomicU32::new(DEFAULT_FPS),
            bands,
            #[cfg(windows)]
            event: capture::Event::new().ok(),
        });
        let (done, finished) = mpsc::channel();
        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("velo-viz".into())
            .spawn(move || {
                worker(worker_shared, force_simulated);
                let _ = done.send(());
            })
            .expect("could not create visualizer worker");
        Self {
            shared,
            finished: Mutex::new(finished),
        }
    }

    /// Publication rate in Hz (clamped to 30..=240). The worker picks it up on its next frame.
    pub fn set_target_fps(&self, hz: f64) {
        if self.shared.fps.swap(clamp_fps(hz), Ordering::Relaxed) != clamp_fps(hz) {
            self.shared.wake();
        }
    }

    pub fn target_fps(&self) -> u32 {
        self.shared.fps.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn new_simulated() -> Self {
        Self::create(true)
    }
}

impl Visualizer for VizEngine {
    /// Clears publication immediately on stop; stream teardown is asynchronous.
    /// The audio event interrupts capture waits, and initialization checks for
    /// cancellation every 20 ms. Repeating the same state/PID is a no-op.
    fn set_active(&self, active: bool, target_pid: Option<u32>) {
        let mut c = self.shared.control.lock().unwrap();
        if !c.external && c.active == active && (!active || c.pid == target_pid) {
            return;
        }
        c.active = active;
        c.external = false;
        c.samples.clear();
        c.pid = target_pid;
        c.generation = c.generation.wrapping_add(1);
        if !active {
            // Same lock gates publication: no stale frame can follow zero.
            self.shared.bands.send_if_modified(|bands| {
                let changed = *bands != [0.0; VIZ_BANDS];
                *bands = [0.0; VIZ_BANDS];
                changed
            });
        }
        self.shared.wake();
    }

    fn set_sensitivity(&self, factor: f32) {
        self.shared.control.lock().unwrap().sensitivity = sensitivity(factor);
    }
    fn bands(&self) -> watch::Receiver<Bands> {
        self.shared.bands.subscribe()
    }
    fn is_simulated(&self) -> bool {
        self.shared.control.lock().unwrap().simulated
    }
}

impl Drop for VizEngine {
    fn drop(&mut self) {
        self.set_active(false, None);
        self.shared.control.lock().unwrap().shutdown = true;
        self.shared.wake();
        // A broken driver must not hold the UI indefinitely. The worker owns its
        // resources and completes cleanup even if this bounded wait expires.
        let _ = self
            .finished
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_millis(150));
    }
}

fn worker(shared: Arc<Shared>, force_simulated: bool) {
    #[cfg(windows)]
    let com = capture::Com::new().ok();
    let mut limiter = FrameLimiter::default();
    let epoch = Instant::now();
    loop {
        let (generation, pid, external) = {
            let mut c = shared.control.lock().unwrap();
            while !c.active && !c.shutdown {
                c = shared.changed.wait(c).unwrap();
            }
            if c.shutdown {
                return;
            }
            (c.generation, c.pid, c.external)
        };
        if external {
            external::run(&shared, generation, &mut limiter, epoch);
            continue;
        }
        let mut analyzer = Analyzer::new();
        #[cfg(windows)]
        let mut capture = if !force_simulated && com.is_some() {
            pid.and_then(|pid| {
                capture::Capture::open(pid, &shared, generation)
                    .inspect_err(|error| {
                        if shared.current(generation) {
                            tracing::warn!(%error, pid, "Spotify capture unavailable; using simulated bands");
                        }
                    })
                    .ok()
            })
        } else {
            None
        };
        #[cfg(not(windows))]
        let _ = (force_simulated, pid);
        #[cfg(windows)]
        let mut is_simulated = capture.is_none();
        #[cfg(not(windows))]
        let is_simulated = true;
        #[cfg(windows)]
        let mut ring = Ring::default();
        #[cfg(not(windows))]
        let ring = Ring::default();
        let mut smoothed = [0.0; VIZ_BANDS];
        #[cfg(windows)]
        let mut last_audio = Instant::now();
        while shared.current(generation) {
            #[cfg(windows)]
            if let Some(stream) = capture.as_mut() {
                match stream.read(&mut ring, &shared, generation) {
                    Ok(true) => last_audio = Instant::now(),
                    Ok(false) => {
                        if last_audio.elapsed() >= Duration::from_millis(100) {
                            ring.clear();
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Spotify capture lost; using simulated bands");
                        capture = None;
                        is_simulated = true;
                    }
                }
            }
            let factor = {
                let mut c = shared.control.lock().unwrap();
                if c.generation != generation || !c.active || c.shutdown {
                    break;
                }
                c.simulated = is_simulated;
                c.sensitivity
            };
            limiter.set_fps(shared.fps.load(Ordering::Relaxed));
            let now = epoch.elapsed();
            if limiter.allow(now) {
                let target = if is_simulated {
                    simulated(now.as_secs_f32(), factor)
                } else {
                    analyzer.analyze(&ring, factor)
                };
                smooth(&mut smoothed, &target);
                shared.publish(generation, smoothed);
            }
            if is_simulated {
                let c = shared.control.lock().unwrap();
                if c.active && !c.shutdown && c.generation == generation {
                    let _ = shared
                        .changed
                        .wait_timeout(c, limiter.remaining(epoch.elapsed()))
                        .unwrap();
                }
            }
        }
        // Capture is dropped before entering the idle wait.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inactive_stops_publishing_and_can_restart() {
        let engine = VizEngine::new_simulated();
        let mut bands = engine.bands();
        engine.set_active(true, None);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !bands.has_changed().unwrap() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(bands.has_changed().unwrap());
        assert!(engine.is_simulated());
        engine.set_active(false, None);
        assert_eq!(*bands.borrow_and_update(), [0.0; VIZ_BANDS]);
        std::thread::sleep(Duration::from_millis(150));
        assert!(!bands.has_changed().unwrap());
        engine.set_active(true, Some(123));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !bands.has_changed().unwrap() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(bands.has_changed().unwrap());
        let start = Instant::now();
        drop(engine);
        assert!(start.elapsed() < Duration::from_millis(200));
        assert_eq!(*bands.borrow(), [0.0; VIZ_BANDS]);
    }
}
