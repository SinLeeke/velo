//! Lossy, memory-only external PCM input. FFT and mono conversion stay on the viz worker.
use std::time::Instant;

use crate::analysis::{Analyzer, FrameLimiter, Ring, smooth};
use crate::{Shared, VizEngine};
use velo_domain::VIZ_BANDS;

pub(super) struct Samples {
    values: Vec<f64>,
    sample_rate: u32,
    channels: u16,
}

impl VizEngine {
    /// Select Velo's PCM instead of process loopback. Stopping drops all queued samples.
    pub fn set_external_active(&self, active: bool) {
        let mut c = self.shared.control.lock().unwrap();
        if c.external && c.active == active {
            return;
        }
        c.active = active;
        c.external = true;
        c.pid = None;
        c.simulated = false;
        c.samples.clear();
        c.generation = c.generation.wrapping_add(1);
        self.shared.bands.send_replace([0.0; VIZ_BANDS]);
        self.shared.wake();
    }

    /// Bounded to two decoder packets; never waits for the FFT or an audio device.
    pub fn submit_pcm(&self, samples: Vec<f64>, sample_rate: u32, channels: u16) {
        if channels == 0 || !(8_000..=192_000).contains(&sample_rate) {
            return;
        }
        let mut c = self.shared.control.lock().unwrap();
        if !c.active || !c.external || c.shutdown {
            return;
        }
        if c.samples.len() == 2 {
            c.samples.pop_front();
        }
        c.samples.push_back(Samples {
            values: samples,
            sample_rate,
            channels,
        });
        self.shared.changed.notify_one();
    }
}

pub(super) fn run(shared: &Shared, generation: u64, limiter: &mut FrameLimiter, epoch: Instant) {
    let mut ring = Ring::default();
    let mut analyzer = Analyzer::with_sample_rate(44_100);
    let mut rate = 44_100;
    let mut smoothed = [0.0; VIZ_BANDS];
    loop {
        let (packet, factor) = {
            let mut c = shared.control.lock().unwrap();
            while c.active && !c.shutdown && c.generation == generation && c.samples.is_empty() {
                c = shared.changed.wait(c).unwrap();
            }
            if !c.active || c.shutdown || c.generation != generation {
                return;
            }
            (c.samples.pop_front().unwrap(), c.sensitivity)
        };
        if rate != packet.sample_rate {
            rate = packet.sample_rate;
            analyzer = Analyzer::with_sample_rate(rate);
            ring = Ring::default();
        }
        for frame in packet.values.chunks_exact(usize::from(packet.channels)) {
            ring.push((frame.iter().sum::<f64>() / f64::from(packet.channels)) as f32);
        }
        limiter.set_fps(shared.fps.load(std::sync::atomic::Ordering::Relaxed));
        if limiter.allow(epoch.elapsed()) {
            smooth(&mut smoothed, &analyzer.analyze(&ring, factor));
            shared.publish(generation, smoothed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velo_domain::Visualizer;

    #[test]
    fn external_pcm_is_real_and_stop_rejects_and_clears_samples() {
        let engine = VizEngine::new_simulated();
        engine.set_external_active(true);
        assert!(!engine.is_simulated());
        let samples: Vec<_> = (0..4096)
            .flat_map(|i| {
                let s = (std::f64::consts::TAU * 1000.0 * f64::from(i) / 44_100.0).sin() * 0.5;
                [s, s]
            })
            .collect();
        let mut bands = engine.bands();
        engine.submit_pcm(samples, 44_100, 2);
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while !bands.has_changed().unwrap() && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(bands.borrow_and_update().iter().any(|b| *b > 0.0));
        engine.set_external_active(false);
        engine.submit_pcm(vec![1.0; 8192], 44_100, 2);
        assert!(engine.shared.control.lock().unwrap().samples.is_empty());
        assert_eq!(*bands.borrow_and_update(), [0.0; VIZ_BANDS]);
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert!(!bands.has_changed().unwrap());
        engine.set_external_active(true);
        assert_eq!(*engine.bands().borrow(), [0.0; VIZ_BANDS]);
    }
}
