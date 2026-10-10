//! Ten RBJ peaking filters, with independent stereo histories and a short bank crossfade.
//! The mailbox uses atomics: the audio reader never waits for the control writer.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub const EQ_FREQUENCIES: [f64; 10] = [
    60., 170., 310., 600., 1000., 3000., 6000., 12000., 14000., 16000.,
];

pub use velo_domain::EqConfig;

/// Clamps to the audible range and replaces non-finite values.
fn checked(config: EqConfig) -> EqConfig {
    let db = |v: f32| {
        if v.is_finite() {
            v.clamp(-12., 12.)
        } else {
            0.
        }
    };
    EqConfig {
        enabled: config.enabled,
        preamp_db: db(config.preamp_db),
        gains_db: config.gains_db.map(db),
    }
}

struct Mailbox {
    // Only control writers serialize. SeqCst establishes a coherent bounded snapshot without
    // unsafe memory, spinning, allocating or taking a mutex in the audio reader.
    writer: Mutex<()>,
    version: AtomicU64,
    values: [AtomicU64; 12],
}

#[derive(Clone)]
pub(crate) struct EqControl(Arc<Mailbox>);
impl Default for EqControl {
    fn default() -> Self {
        Self(Arc::new(Mailbox {
            writer: Mutex::new(()),
            version: AtomicU64::new(0),
            values: std::array::from_fn(|_| AtomicU64::new(0)),
        }))
    }
}
impl EqControl {
    pub fn set(&self, config: EqConfig) {
        let config = checked(config);
        let _writer = self.0.writer.lock().unwrap_or_else(|e| e.into_inner());
        self.0.version.fetch_add(1, Ordering::SeqCst);
        self.0.values[0].store(u64::from(config.enabled), Ordering::SeqCst);
        self.0.values[1].store(u64::from(config.preamp_db.to_bits()), Ordering::SeqCst);
        for (value, gain) in self.0.values[2..].iter().zip(config.gains_db) {
            value.store(u64::from(gain.to_bits()), Ordering::SeqCst);
        }
        self.0.version.fetch_add(1, Ordering::SeqCst);
    }
    fn read(&self, previous: u64) -> Option<(u64, EqConfig)> {
        let version = self.0.version.load(Ordering::SeqCst);
        if version == previous || version & 1 != 0 {
            return None;
        }
        let config = EqConfig {
            enabled: self.0.values[0].load(Ordering::SeqCst) != 0,
            preamp_db: f32::from_bits(self.0.values[1].load(Ordering::SeqCst) as u32),
            gains_db: std::array::from_fn(|i| {
                f32::from_bits(self.0.values[i + 2].load(Ordering::SeqCst) as u32)
            }),
        };
        (self.0.version.load(Ordering::SeqCst) == version).then_some((version, config))
    }
}

#[derive(Clone, Copy)]
struct Coefficients {
    b: [f64; 3],
    a: [f64; 2],
}
impl Coefficients {
    const IDENTITY: Self = Self {
        b: [1., 0., 0.],
        a: [0., 0.],
    };
    fn peaking(hz: f64, db: f32, rate: u32) -> Self {
        if db == 0. {
            return Self::IDENTITY;
        }
        let w = std::f64::consts::TAU * hz.min(f64::from(rate) * 0.45) / f64::from(rate.max(1));
        let amplitude = 10_f64.powf(f64::from(db) / 40.);
        let alpha = w.sin() / 2.; // Q = 1; stable narrow peaking bands.
        let a0 = 1. + alpha / amplitude;
        Self {
            b: [
                (1. + alpha * amplitude) / a0,
                -2. * w.cos() / a0,
                (1. - alpha * amplitude) / a0,
            ],
            a: [-2. * w.cos() / a0, (1. - alpha / amplitude) / a0],
        }
    }
    fn magnitude(self, hz: f64, rate: u32) -> f64 {
        let w = std::f64::consts::TAU * hz / f64::from(rate);
        let real = |c: [f64; 3]| c[0] + c[1] * w.cos() + c[2] * (2. * w).cos();
        let imag = |c: [f64; 3]| -c[1] * w.sin() - c[2] * (2. * w).sin();
        let a = [1., self.a[0], self.a[1]];
        (real(self.b).hypot(imag(self.b))) / real(a).hypot(imag(a))
    }
}

#[derive(Clone, Copy)]
struct Bank {
    enabled: bool,
    coefficients: [Coefficients; 10],
    state: [[[f64; 2]; 10]; 2],
    gain: f64,
}
impl Bank {
    fn new(config: EqConfig, rate: u32) -> Self {
        if !config.enabled {
            return Self {
                enabled: false,
                coefficients: [Coefficients::IDENTITY; 10],
                state: [[[0.; 2]; 10]; 2],
                gain: 1.,
            };
        }
        let coefficients = std::array::from_fn(|i| {
            Coefficients::peaking(EQ_FREQUENCIES[i], config.gains_db[i], rate)
        });
        // Measure the cascade's peak (including overlapping treble bands), only on changes.
        // Summing individual boosts would unnecessarily mute a ten-band +6 dB preset by
        // 60 dB. Negative preamp supplies headroom first; automatic attenuation supplies
        // any missing part. Leave 0.5 dB for grid error/transients; clamp remaining overshoot.
        let mut peak = 0_f64;
        if config.enabled && config.gains_db.iter().any(|g| *g > 0.) {
            let top = f64::from(rate.max(1)) * 0.49;
            for hz in (0..384)
                .map(|i| top * ((i + 1) as f64 / 384.).powi(3))
                .chain(EQ_FREQUENCIES.map(|hz| hz.min(top)))
            {
                let db: f64 = coefficients
                    .iter()
                    .map(|c| 20. * c.magnitude(hz, rate.max(1)).log10())
                    .sum();
                peak = peak.max(db);
            }
            peak += 0.5;
        }
        let preamp = if peak > 0. {
            f64::from(config.preamp_db).min(-peak)
        } else {
            f64::from(config.preamp_db)
        };
        Self {
            enabled: config.enabled,
            coefficients,
            state: [[[0.; 2]; 10]; 2],
            gain: 10_f64.powf(preamp / 20.),
        }
    }
    fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        if !self.enabled {
            return input;
        } // Also preserves NaN payloads and signed zero.
        std::array::from_fn(|channel| {
            // A non-finite sample must not enter the recursive state: it would stay there
            // forever (NaN * anything = NaN) and silence the stream until restart.
            let sample = f64::from(input[channel]);
            let mut value = if sample.is_finite() { sample } else { 0. } * self.gain;
            for (c, z) in self.coefficients.iter().zip(&mut self.state[channel]) {
                let out = c.b[0] * value + z[0];
                z[0] = c.b[1] * value - c.a[0] * out + z[1];
                z[1] = c.b[2] * value - c.a[1] * out;
                // Kill denormals during silent filter tails.
                for v in z {
                    if v.abs() < 1e-24 {
                        *v = 0.;
                    }
                }
                value = out;
            }
            if value.is_finite() {
                value.clamp(-1., 1.) as f32
            } else {
                self.state[channel] = [[0.; 2]; 10];
                0.
            }
        })
    }
}

pub(crate) struct Equalizer {
    control: EqControl,
    version: u64,
    config: EqConfig,
    rate: u32,
    current: Bank,
    old: Bank,
    remaining: usize,
    transition_frames: usize,
}
impl Equalizer {
    pub fn new(control: EqControl, rate: u32) -> Self {
        let config = EqConfig::default();
        let bank = Bank::new(config, rate);
        Self {
            control,
            version: 0,
            config,
            rate,
            current: bank,
            old: bank,
            remaining: 0,
            transition_frames: (rate as usize / 100).max(1),
        }
    }
    pub fn process(&mut self, input: [f32; 2], rate: u32) -> [f32; 2] {
        // Finish a bank transition before retargeting: bursts coalesce to the latest config,
        // keeping the instantaneous waveform continuous without a third filter bank.
        if self.remaining == 0 {
            let update = self.control.read(self.version);
            if update.is_some() || rate != self.rate {
                let config = update.map_or(self.config, |(version, config)| {
                    self.version = version;
                    config
                });
                if config != self.config || rate != self.rate {
                    if !config.enabled && !self.current.enabled {
                        // Editing a saved preset while disabled must remain a true bypass,
                        // including signed zero/NaN; never crossfade identical dry samples.
                        self.config = config;
                        self.rate = rate;
                        return input;
                    }
                    self.old = self.current;
                    self.current = Bank::new(config, rate);
                    self.config = config;
                    self.rate = rate;
                    self.transition_frames = (rate as usize / 100).max(1);
                    self.remaining = self.transition_frames;
                }
            }
        }
        let next = self.current.process(input);
        if self.remaining == 0 {
            return next;
        }
        let old = self.old.process(input);
        let t = 1. - self.remaining as f32 / self.transition_frames as f32;
        let mix = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
        self.remaining -= 1;
        std::array::from_fn(|i| old[i] + (next[i] - old[i]) * mix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nan_input_does_not_poison_the_filter_state() {
        let mut gains = [0.; 10];
        gains[4] = 6.;
        let mut eq = Equalizer::new(EqControl::default(), 44_100);
        eq.control.set(EqConfig {
            enabled: true,
            preamp_db: 0.,
            gains_db: gains,
        });
        for _ in 0..1000 {
            eq.process([0.2, 0.2], 44_100);
        }
        eq.process([f32::NAN, f32::INFINITY], 44_100);
        let out = (0..2000).fold([0.; 2], |_, _| eq.process([0.2, 0.2], 44_100));
        assert!(out[0].abs() > 0.05 && out[1].abs() > 0.05, "{out:?}");
    }
    #[test]
    fn bypass_is_bit_identical_even_for_signed_zero_and_nan() {
        let control = EqControl::default();
        let mut eq = Equalizer::new(control.clone(), 44100);
        for (i, bits) in [0, 0x80000000, 0x7fc01234, 0x3effffff, 0xbf800000]
            .into_iter()
            .enumerate()
        {
            control.set(EqConfig {
                enabled: false,
                preamp_db: i as f32,
                gains_db: [i as f32; 10],
            });
            let input = [f32::from_bits(bits), f32::from_bits(bits ^ 0x80000000)];
            assert_eq!(
                eq.process(input, 44100).map(f32::to_bits),
                input.map(f32::to_bits)
            );
        }
    }
    #[test]
    fn all_bands_peak_at_six_db_and_are_flat_far_away() {
        for rate in [44100, 48000, 96000] {
            for hz in EQ_FREQUENCIES {
                let c = Coefficients::peaking(hz, 6., rate);
                let db = 20. * c.magnitude(hz, rate).log10();
                assert!((db - 6.).abs() < 1., "{hz}/{rate}: {db}");
                let far = if hz < 1000. { 20000. } else { 10. };
                assert!((20. * c.magnitude(far, rate).log10()).abs() < 0.1);
            }
        }
    }
    #[test]
    fn actual_sine_response_matches_each_band_relative_to_preamp() {
        for (index, hz) in EQ_FREQUENCIES.into_iter().enumerate() {
            let mut gains_db = [0.; 10];
            gains_db[index] = 6.;
            let mut bank = Bank::new(
                EqConfig {
                    enabled: true,
                    preamp_db: -12.,
                    gains_db,
                },
                44100,
            );
            let mut input_power = 0.;
            let mut output_power = 0.;
            for i in 0..44100 {
                let sample = (0.05 * (std::f64::consts::TAU * hz * i as f64 / 44100.).sin()) as f32;
                let output = bank.process([sample, 0.]);
                if i >= 22050 {
                    input_power += f64::from(sample).powi(2);
                    output_power += f64::from(output[0]).powi(2);
                }
            }
            let db = 10. * (output_power / input_power).log10() + 12.;
            assert!((db - 6.).abs() < 1., "band {hz}: {db}");
        }
    }
    #[test]
    fn flat_preamp_respects_both_positive_and_negative_gain_for_quiet_pcm() {
        for preamp_db in [-12., -6., 0., 6., 12.] {
            let mut bank = Bank::new(
                EqConfig {
                    enabled: true,
                    preamp_db,
                    ..Default::default()
                },
                44100,
            );
            let output = bank.process([0.05, -0.05]);
            let expected = 0.05 * 10_f32.powf(preamp_db / 20.);
            assert!((output[0] - expected).abs() < 1e-6);
            assert_eq!(output[1], -output[0]);
        }
    }
    #[test]
    fn identical_config_does_not_restart_filters_and_sample_rate_changes_rebuild() {
        let control = EqControl::default();
        let config = EqConfig {
            enabled: true,
            gains_db: [3.; 10],
            ..Default::default()
        };
        let mut eq = Equalizer::new(control.clone(), 44100);
        control.set(config);
        for _ in 0..500 {
            eq.process([0.; 2], 44100);
        }
        control.set(config);
        eq.process([0.; 2], 44100);
        assert_eq!(eq.remaining, 0);
        eq.process([0.; 2], 22050);
        assert_eq!(eq.rate, 22050);
        assert_eq!(eq.remaining, 219);
        assert!(
            (20. * eq.current.coefficients[9]
                .magnitude(22050. * 0.45, 22050)
                .log10()
                - 3.)
                .abs()
                < 1e-6
        );
    }
    #[test]
    fn atomic_mailbox_is_coherent_under_concurrent_preset_writes() {
        let control = EqControl::default();
        let writer = control.clone();
        let task = std::thread::spawn(move || {
            for i in 0..10000 {
                let db = (i % 25) as f32 - 12.;
                writer.set(EqConfig {
                    enabled: true,
                    preamp_db: db,
                    gains_db: [db; 10],
                });
            }
        });
        let mut previous = 0;
        for _ in 0..10000 {
            if let Some((version, config)) = control.read(previous) {
                assert_eq!(config.gains_db, [config.preamp_db; 10]);
                previous = version;
            }
        }
        task.join().unwrap();
    }
    #[test]
    fn stereo_histories_do_not_leak_and_low_rates_remain_stable() {
        for rate in [8000, 22050, 44100] {
            let mut bank = Bank::new(
                EqConfig {
                    enabled: true,
                    gains_db: [6.; 10],
                    ..Default::default()
                },
                rate,
            );
            for i in 0..10000 {
                let output = bank.process([if i == 0 { 1. } else { 0. }, 0.]);
                assert_eq!(output[1], 0.);
                assert!(output[0].is_finite() && output[0].abs() <= 1.);
            }
        }
    }
    #[test]
    fn preset_bursts_and_disabling_have_no_large_discontinuity() {
        let control = EqControl::default();
        let mut eq = Equalizer::new(control.clone(), 44100);
        let mut previous = 0.;
        let mut jump = 0_f32;
        for i in 0..20000 {
            if i % 173 == 0 {
                control.set(EqConfig {
                    enabled: i < 15000,
                    gains_db: [if i % 346 == 0 { 12. } else { -12. }; 10],
                    preamp_db: -6.,
                });
            }
            let input = 0.2 * (std::f32::consts::TAU * 60. * i as f32 / 44100.).sin();
            let out = eq.process([input; 2], 44100)[0];
            jump = jump.max((out - previous).abs());
            previous = out;
            if i > 16000 {
                assert_eq!(out.to_bits(), input.to_bits());
            }
        }
        assert!(jump < 0.01, "sample jump {jump}");
    }
    #[test]
    fn positive_presets_with_preamp_never_clip() {
        for preamp_db in [-12., -6., 0., 12.] {
            let mut bank = Bank::new(
                EqConfig {
                    enabled: true,
                    preamp_db,
                    gains_db: [6.; 10],
                },
                44100,
            );
            for i in 0..44100 {
                let sample = (std::f32::consts::TAU * 170. * i as f32 / 44100.).sin();
                let output = bank.process([sample, -sample]);
                assert!(output.iter().all(|s| s.is_finite() && s.abs() < 1.));
            }
        }
    }
    #[test]
    fn mailbox_never_accepts_partial_or_invalid_configuration() {
        let control = EqControl::default();
        control.set(EqConfig {
            enabled: true,
            preamp_db: f32::NAN,
            gains_db: [99.; 10],
        });
        let (_, config) = control.read(0).unwrap();
        assert_eq!(config.preamp_db, 0.);
        assert_eq!(config.gains_db, [12.; 10]);
        control.0.version.fetch_add(1, Ordering::SeqCst);
        assert!(control.read(0).is_none());
    }
}
