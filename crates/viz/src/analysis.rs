use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use std::sync::Arc;
use std::time::Duration;
use velo_domain::{VIZ_BANDS, VIZ_MAX_FPS};

pub(crate) type Bands = [f32; VIZ_BANDS];
// ~5.4 Hz/bin at the engine's 44.1 kHz, with a 186 ms history. Reused on the
// worker at the display refresh rate; fractional bin overlap also covers sub-bin bass bands.
pub(crate) const FFT_SIZE: usize = 6144;
const SAMPLE_RATE: f32 = 48_000.0;
const MIN_HZ: f32 = 30.0;
const MAX_HZ: f32 = 16_000.0;
// Window shown below a band's reference peak. 60 dB made a 12 dB drop move a bar by 9 %.
const RANGE_DB: f32 = 24.0;
// An onset lifts its band by up to this much on top of the level (no ceiling).
const IMPULSE_LIFT: f32 = 0.5;
const EPSILON: f32 = 0.0001;
const ATTACK: f32 = 0.55;
const RELEASE: f32 = 0.2;
const BAND_FLOOR: f32 = 0.000001;
const ONSET_GROUPS: usize = 6;
const GROUP_BANDS: usize = VIZ_BANDS / ONSET_GROUPS;
/// Publication rate bounds and default: the UI reports the display's refresh rate.
pub const MIN_FPS: u32 = 30;
pub const MAX_FPS: u32 = 240;
pub const DEFAULT_FPS: u32 = 60;

/// Clamps a reported refresh rate into `MIN_FPS..=MAX_FPS`; unusable values give `DEFAULT_FPS`.
pub fn clamp_fps(hz: f64) -> u32 {
    if hz.is_finite() && hz > 0.0 {
        (hz.round() as u32).clamp(MIN_FPS, MAX_FPS)
    } else {
        DEFAULT_FPS
    }
}

fn frame_period(fps: u32) -> Duration {
    Duration::from_nanos(1_000_000_000_u64.div_ceil(u64::from(fps.clamp(MIN_FPS, MAX_FPS))))
}

pub(crate) struct Ring {
    samples: [f32; FFT_SIZE],
    next: usize,
    written: u64,
}
impl Default for Ring {
    fn default() -> Self {
        Self {
            samples: [0.0; FFT_SIZE],
            next: 0,
            written: 0,
        }
    }
}
impl Ring {
    pub(crate) fn push(&mut self, sample: f32) {
        self.samples[self.next] = if sample.is_finite() { sample } else { 0.0 };
        self.next = (self.next + 1) % FFT_SIZE;
        self.written = self.written.saturating_add(1);
    }
    #[cfg(any(windows, test))]
    pub(crate) fn clear(&mut self) {
        self.samples.fill(0.0);
    }
}

fn band_edge(index: f32, max_hz: f32) -> f32 {
    MIN_HZ * (max_hz / MIN_HZ).powf(index / VIZ_BANDS as f32)
}
pub(crate) fn sensitivity(factor: f32) -> f32 {
    if factor.is_finite() {
        factor.clamp(0.25, 2.0)
    } else {
        1.0
    }
}
fn normalized_db(relative: f32, factor: f32) -> f32 {
    // `relative` is the amplitude over its reference peak: 1.0 = the recent crest. No ceiling:
    // louder-than-recent moments grow past 1 and the UI draws them taller (behind the content).
    if relative <= 0.0 || !relative.is_finite() {
        return 0.0;
    }
    ((20.0 * relative.log10() + RANGE_DB) / RANGE_DB).max(0.0) * sensitivity(factor)
}

struct BandFilter {
    bins: Vec<(usize, f32)>,
    tilt: f32,
}

impl BandFilter {
    fn new(index: usize, sample_rate: u32) -> Self {
        let max_hz = MAX_HZ.min(sample_rate as f32 * 0.475);
        let hz_per_bin = sample_rate as f32 / FFT_SIZE as f32;
        let lo = band_edge(index as f32, max_hz) / hz_per_bin;
        let hi = band_edge(index as f32 + 1.0, max_hz) / hz_per_bin;
        let first = ((lo - 0.5).floor() as usize).max(1);
        let last = ((hi + 0.5).ceil() as usize).min(FFT_SIZE / 2);
        let mut bins: Vec<_> = (first..=last)
            .filter_map(|bin| {
                let overlap = (hi.min(bin as f32 + 0.5) - lo.max(bin as f32 - 0.5)).max(0.0);
                (overlap > 0.0).then_some((bin, overlap))
            })
            .collect();
        // Every frequency interval overlaps at least one bin cell, even when
        // several bass intervals share a bin. Normalize fractional weights.
        let total: f32 = bins.iter().map(|(_, weight)| weight).sum();
        for (_, weight) in &mut bins {
            *weight /= total;
        }
        Self {
            bins,
            // +3 dB/octave in amplitude balances pink-noise spectral density.
            tilt: (band_edge(index as f32 + 0.5, max_hz) / 1000.0).sqrt(),
        }
    }
}

pub(crate) struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    input: Vec<Complex32>,
    scratch: Vec<Complex32>,
    hann: [f32; FFT_SIZE],
    filters: [BandFilter; VIZ_BANDS],
    gain: f32,
    recent_peak: f32,
    silent_frames: u32,
    sample_rate: u32,
    last_written: Option<u64>,
    band_peak: Bands,
    band_mean: Bands,
    previous: Bands,
    flux_mean: Bands,
    flux_deviation: Bands,
    impulse: Bands,
    refractory: Bands,
    group_previous: [f32; ONSET_GROUPS],
    group_mean: [f32; ONSET_GROUPS],
    group_flux_mean: [f32; ONSET_GROUPS],
    group_flux_deviation: [f32; ONSET_GROUPS],
}
impl Analyzer {
    pub(crate) fn new() -> Self {
        Self::with_sample_rate(SAMPLE_RATE as u32)
    }

    pub(crate) fn with_sample_rate(sample_rate: u32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let hann = std::array::from_fn(|i| {
            0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos()
        });
        Self {
            scratch: vec![Complex32::default(); fft.get_inplace_scratch_len()],
            fft,
            input: vec![Complex32::default(); FFT_SIZE],
            gain: 2.0 / hann.iter().sum::<f32>(),
            hann,
            filters: std::array::from_fn(|i| BandFilter::new(i, sample_rate)),
            recent_peak: 0.35,
            silent_frames: 0,
            sample_rate,
            last_written: None,
            band_peak: [0.03; VIZ_BANDS],
            band_mean: [0.0; VIZ_BANDS],
            previous: [0.0; VIZ_BANDS],
            flux_mean: [0.0; VIZ_BANDS],
            flux_deviation: [0.0; VIZ_BANDS],
            impulse: [0.0; VIZ_BANDS],
            refractory: [0.0; VIZ_BANDS],
            group_previous: [0.0; ONSET_GROUPS],
            group_mean: [0.0; ONSET_GROUPS],
            group_flux_mean: [0.0; ONSET_GROUPS],
            group_flux_deviation: [0.0; ONSET_GROUPS],
        }
    }
    // Real mono input, positive-frequency half of a complex FFT. All allocations,
    // the plan, Hann window and bin mapping are reused between frames.
    pub(crate) fn analyze(&mut self, ring: &Ring, factor: f32) -> Bands {
        // Use the audio clock: PCM packet cadence need not match the display refresh rate.
        let elapsed = self.last_written.map_or(1.0 / VIZ_MAX_FPS as f32, |last| {
            let samples = ring.written.saturating_sub(last);
            if samples == 0 {
                1.0 / VIZ_MAX_FPS as f32
            } else {
                samples as f32 / self.sample_rate as f32
            }
        });
        let first = self.last_written.is_none();
        self.last_written = Some(ring.written);
        if ring.samples.iter().all(|sample| *sample == 0.0) {
            self.previous.fill(0.0);
            self.impulse.fill(0.0);
            self.refractory.fill(0.0);
            self.silent_frames = self.silent_frames.saturating_add(1);
            if self.silent_frames >= VIZ_MAX_FPS / 2 {
                self.recent_peak = 0.35;
                self.band_peak.fill(0.03);
                self.band_mean.fill(0.0);
                self.flux_mean.fill(0.0);
                self.flux_deviation.fill(0.0);
                self.group_previous.fill(0.0);
                self.group_mean.fill(0.0);
                self.group_flux_mean.fill(0.0);
                self.group_flux_deviation.fill(0.0);
                self.last_written = None;
            }
            return [0.0; VIZ_BANDS];
        }
        self.silent_frames = 0;
        for (i, value) in self.input.iter_mut().enumerate() {
            *value = Complex32::new(ring.samples[(ring.next + i) % FFT_SIZE] * self.hann[i], 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.input, &mut self.scratch);
        let peaks: Bands = std::array::from_fn(|i| {
            let filter = &self.filters[i];
            let power: f32 = filter
                .bins
                .iter()
                .map(|(bin, weight)| self.input[*bin].norm_sqr() * weight)
                .sum();
            power.sqrt() * self.gain * filter.tilt
        });
        let peak = peaks.iter().copied().fold(0.0_f32, f32::max);
        // Follow a transient in ~one frame; recover gain over ~8 seconds at
        // 30 Hz. Silence does not wind the gain up or amplify background noise.
        let coefficient = if peak > self.recent_peak {
            1.0 - (-elapsed / 0.032).exp()
        } else {
            1.0 - (-elapsed / 8.3).exp()
        };
        self.recent_peak += coefficient * (peak - self.recent_peak);
        let automatic_gain = (1.0 / self.recent_peak.max(0.0001)).clamp(0.15, 366.0);
        let release = 1.0 - (-elapsed / 4.0).exp();
        let mean_follow = 1.0 - (-elapsed / 0.8).exp();
        let noise_follow = 1.0 - (-elapsed / 1.5).exp();
        let audible = BAND_FLOOR.max(peak * 0.0001);
        // Pool roughly 1.5 octaves before accepting an onset. Random texture can
        // jump in one narrow FFT band without a musical entry in that register.
        let group_onset: [bool; ONSET_GROUPS] = std::array::from_fn(|group| {
            let power = peaks[group * GROUP_BANDS..(group + 1) * GROUP_BANDS]
                .iter()
                .map(|amplitude| amplitude * amplitude)
                .sum::<f32>();
            let amplitude = (power / GROUP_BANDS as f32).sqrt();
            let flux = (amplitude - self.group_previous[group]).max(0.0)
                / self.group_previous[group]
                    .max(self.group_mean[group] * 0.5)
                    .max(audible);
            let threshold =
                (self.group_flux_mean[group] + 4.0 * self.group_flux_deviation[group]).max(0.75);
            let onset = !first
                && ring.written >= FFT_SIZE as u64
                && amplitude > BAND_FLOOR * 10.0
                && amplitude > self.group_mean[group] * 1.4
                && flux > threshold;
            let bounded_flux = flux.min(3.0);
            self.group_flux_deviation[group] += noise_follow
                * ((bounded_flux - self.group_flux_mean[group]).abs()
                    - self.group_flux_deviation[group]);
            self.group_flux_mean[group] +=
                noise_follow * (bounded_flux - self.group_flux_mean[group]);
            self.group_previous[group] = amplitude;
            self.group_mean[group] += mean_follow * (amplitude - self.group_mean[group]);
            onset
        });
        std::array::from_fn(|i| {
            let amplitude = peaks[i];
            self.impulse[i] *= (-elapsed / 0.065).exp(); // 95% gone after 195 ms.
            self.refractory[i] = (self.refractory[i] - elapsed).max(0.0);
            // Positive spectral flux in each weighted band, before AGC. Adaptive
            // noise statistics reject steady texture; a 75% rise is also required.
            let flux = (amplitude - self.previous[i]).max(0.0)
                / self.previous[i].max(self.band_mean[i] * 0.5).max(audible);
            let threshold = (self.flux_mean[i] + 4.0 * self.flux_deviation[i]).max(0.75);
            if !first
                && group_onset[i / GROUP_BANDS]
                && amplitude > audible
                && amplitude > self.band_mean[i] * 1.3
                && flux > threshold
                && self.refractory[i] == 0.0
            {
                self.impulse[i] = ((flux - threshold) / (1.0 + flux - threshold)).min(1.0);
                self.refractory[i] = 0.12;
            }
            let bounded_flux = flux.min(3.0);
            self.flux_deviation[i] +=
                noise_follow * ((bounded_flux - self.flux_mean[i]).abs() - self.flux_deviation[i]);
            self.flux_mean[i] += noise_follow * (bounded_flux - self.flux_mean[i]);
            self.previous[i] = amplitude;
            self.band_mean[i] += mean_follow * (amplitude - self.band_mean[i]);
            let follow = if amplitude > self.band_peak[i] {
                0.65
            } else {
                release
            };
            self.band_peak[i] += follow * (amplitude - self.band_peak[i]);
            if amplitude <= audible {
                return 0.0;
            }
            // Mix local history with global AGC: quiet treble becomes visible,
            // while the global share preserves the song's spectral contour.
            let local_gain = (1.0 / self.band_peak[i].max(BAND_FLOOR)).clamp(0.4, 34_000.0);
            let base = 0.30 * normalized_db(amplitude * automatic_gain, factor)
                + 0.70 * normalized_db(amplitude * local_gain, factor);
            base + IMPULSE_LIFT * self.impulse[i] * sensitivity(factor)
        })
    }
}
pub(crate) fn smooth(previous: &mut Bands, target: &Bands) {
    // A silent FFT window must publish exact zero, without an artificial tail.
    if target.iter().all(|value| *value == 0.0) {
        previous.fill(0.0);
        return;
    }
    // Per 30 Hz frame: rises reach ~90% in 3 frames (100 ms) and falls fade over ~10 frames, so
    // onsets still read as hits but bars and waves glide instead of jumping (user, 2026-10-08:
    // "se sienten muy bruscas").
    for (old, new) in previous.iter_mut().zip(target) {
        let coefficient = if new > old { ATTACK } else { RELEASE };
        *old += coefficient * (new - *old);
        if *old < EPSILON {
            *old = 0.0;
        }
    }
}
pub(crate) fn changed(old: &Bands, new: &Bands) -> bool {
    old.iter()
        .zip(new)
        .any(|(a, b)| (a - b).abs() >= EPSILON || (*a != 0.0 && *b == 0.0))
}
pub(crate) struct FrameLimiter {
    last: Option<Duration>,
    period: Duration,
}
impl Default for FrameLimiter {
    fn default() -> Self {
        Self {
            last: None,
            period: frame_period(VIZ_MAX_FPS),
        }
    }
}
impl FrameLimiter {
    pub(crate) fn set_fps(&mut self, fps: u32) {
        self.period = frame_period(fps);
    }
    pub(crate) fn allow(&mut self, now: Duration) -> bool {
        if self.remaining(now).is_zero() {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
    pub(crate) fn remaining(&self, now: Duration) -> Duration {
        self.last.map_or(Duration::ZERO, |last| {
            self.period.saturating_sub(now.saturating_sub(last))
        })
    }
}
pub(crate) fn simulated(time: f32, factor: f32) -> Bands {
    std::array::from_fn(|i| {
        let x = i as f32 / (VIZ_BANDS - 1) as f32;
        let envelope = (1.0 - x).powf(0.65) * 0.7 + 0.06;
        let movement =
            0.52 + 0.22 * (time * 2.1 - x * 9.0).sin() + 0.13 * (time * 0.83 + x * 23.0).sin();
        let level = envelope * movement * sensitivity(factor);
        0.95 * -(-1.8 * level).exp_m1()
    })
}

#[cfg(test)]
mod tests {
    // No ceiling, but a steady signal must not run away: its own crest maps near 1 × sensitivity.
    const STEADY_MAX: f32 = 2.0 * 2.0 + IMPULSE_LIFT * 2.0;
    use super::*;
    fn sine(hz: f32, amplitude: f32, rate: u32) -> Ring {
        let mut ring = Ring::default();
        for i in 0..FFT_SIZE {
            ring.push((std::f32::consts::TAU * hz * i as f32 / rate as f32).sin() * amplitude);
        }
        ring
    }

    fn band_for_frequency(hz: f32) -> usize {
        (((hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln() * VIZ_BANDS as f32) as usize)
            .min(VIZ_BANDS - 1)
    }

    #[test]
    fn quiet_constant_treble_adapts_above_half_with_loud_bass() {
        let mut ring = sine(60.0, 0.6, 44_100);
        for i in 0..FFT_SIZE {
            ring.samples[i] +=
                (std::f32::consts::TAU * 8000.0 * i as f32 / 44_100.0).sin() * 0.0003;
        }
        let mut analyzer = Analyzer::with_sample_rate(44_100);
        let band = band_for_frequency(8000.0);
        let initial = analyzer.analyze(&ring, 1.0)[band];
        let mut adapted = initial;
        for _ in 0..900 {
            adapted = analyzer.analyze(&ring, 1.0)[band];
        }
        assert!(adapted > 0.5, "initial={initial} adapted={adapted}");
        assert!(adapted > initial + 0.1);
        assert_eq!(analyzer.impulse[band], 0.0);
    }

    #[test]
    fn periodic_kick_has_periodic_bass_onsets_and_short_decay() {
        let mut analyzer = Analyzer::with_sample_rate(44_100);
        let mut ring = Ring::default();
        let mut smoothed = [0.0; VIZ_BANDS];
        let band = band_for_frequency(60.0);
        let mut hits = Vec::new();
        let mut levels = Vec::new();
        for frame in 0..180 {
            for i in 0..1470 {
                let t = (frame * 1470 + i) as f32 / 44_100.0;
                let beat = (t - 0.5).max(0.0) % 0.5;
                let envelope = if t < 0.5 { 0.0 } else { (-beat * 40.0).exp() };
                ring.push(
                    0.002 * (std::f32::consts::TAU * 1000.0 * t).sin()
                        + 0.8 * envelope * (std::f32::consts::TAU * 60.0 * t).sin(),
                );
            }
            let before = analyzer.impulse[band];
            let bands = analyzer.analyze(&ring, 1.0);
            smooth(&mut smoothed, &bands);
            levels.push(smoothed[band]);
            if analyzer.impulse[band] > before + 0.1 {
                hits.push(frame);
            }
        }
        assert!(hits.len() >= 9, "hits={hits:?}");
        assert!(
            hits.windows(2)
                .all(|pair| (13..=17).contains(&(pair[1] - pair[0]))),
            "hits={hits:?}"
        );
        for hit in hits.iter().take(hits.len() - 1) {
            let peak = levels[*hit..*hit + 5]
                .iter()
                .copied()
                .fold(0.0_f32, f32::max);
            assert!(
                peak > levels[*hit + 12] + 0.15,
                "frame={hit} peak={peak} later={}",
                levels[*hit + 12]
            );
        }
        // Isolate the onset envelope from FFT history and the ordinary band release.
        let steady = sine(1000.0, 0.1, 44_100);
        for _ in 0..60 {
            analyzer.analyze(&steady, 1.0);
        }
        analyzer.impulse[band] = 1.0;
        for _ in 0..6 {
            analyzer.analyze(&steady, 1.0);
        }
        assert!(analyzer.impulse[band] < 0.05);
        let silence = analyzer.analyze(&Ring::default(), 1.0);
        smooth(&mut smoothed, &silence);
        assert_eq!(smoothed, [0.0; VIZ_BANDS]);
        assert_eq!(analyzer.impulse, [0.0; VIZ_BANDS]);
    }

    #[test]
    fn steady_streaming_pink_noise_has_no_onset_impulses_after_warmup() {
        let mut analyzer = Analyzer::with_sample_rate(44_100);
        let mut ring = Ring::default();
        let mut random = 17_u32;
        let mut pink = [0.0_f32; 6];
        let mut impulses = 0;
        for frame in 0..600 {
            for _ in 0..1470 {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                let white = random as f32 / u32::MAX as f32 * 2.0 - 1.0;
                let mut value = white * 0.1;
                for (octave, state) in pink.iter_mut().enumerate() {
                    let alpha = 0.5_f32.powi(octave as i32 + 1);
                    *state += alpha * (white - *state);
                    value += *state / alpha.sqrt();
                }
                ring.push(value * 0.1);
            }
            analyzer.analyze(&ring, 1.0);
            if frame > 90 && analyzer.impulse.iter().any(|v| *v > 0.01) {
                impulses += 1;
            }
        }
        assert_eq!(impulses, 0, "steady pink-noise frames with impulses");
    }

    #[test]
    fn sine_1khz_peaks_in_its_log_band() {
        let ring = sine(1000.0, 0.5, SAMPLE_RATE as u32);
        let bands = Analyzer::new().analyze(&ring, 1.0);
        let peak = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(peak, band_for_frequency(1000.0));
    }

    #[test]
    fn engine_pcm_uses_44100_hz_frequency_mapping() {
        let frequency = 10_000.0;
        let ring = sine(frequency, 0.5, 44_100);
        let bands = Analyzer::with_sample_rate(44_100).analyze(&ring, 1.0);
        let peak = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(peak, band_for_frequency(frequency));
    }

    #[test]
    fn every_band_including_first_four_responds_to_its_center_sine() {
        for rate in [44_100, 48_000, 192_000, 8_000] {
            let max_hz = MAX_HZ.min(rate as f32 * 0.475);
            for i in 0..VIZ_BANDS {
                let mut analyzer = Analyzer::with_sample_rate(rate);
                let filter = &analyzer.filters[i];
                assert!(!filter.bins.is_empty(), "rate={rate} band={i}");
                assert!((filter.bins.iter().map(|(_, w)| w).sum::<f32>() - 1.0).abs() < 1e-5);
                let hz = band_edge(i as f32 + 0.5, max_hz);
                let ring = sine(hz, 0.5, rate);
                let bands = analyzer.analyze(&ring, 1.0);
                assert!(
                    bands[i] > 0.3,
                    "rate={rate} band={i} hz={hz} value={}",
                    bands[i]
                );
                assert!(bands[i] < STEADY_MAX);
                // Wider bands resolve independently; the lowest may share bins.
                if rate <= 48_000 && i >= 12 {
                    let peak = bands
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1.total_cmp(b.1))
                        .unwrap()
                        .0;
                    assert_eq!(peak, i, "rate={rate} hz={hz}");
                }
            }
        }
    }

    #[test]
    fn pink_noise_lights_all_bands_without_clipping() {
        // Deterministic random-phase 1/f-power spectrum, transformed to PCM.
        let mut spectrum = vec![Complex32::default(); FFT_SIZE];
        let mut state = 17_u32;
        for i in 1..FFT_SIZE / 2 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let phase = state as f32 / u32::MAX as f32 * std::f32::consts::TAU;
            spectrum[i] = Complex32::from_polar(1.0 / (i as f32).sqrt(), phase);
            spectrum[FFT_SIZE - i] = spectrum[i].conj();
        }
        FftPlanner::new()
            .plan_fft_inverse(FFT_SIZE)
            .process(&mut spectrum);
        let peak = spectrum.iter().map(|v| v.re.abs()).fold(0.0_f32, f32::max);
        let mut ring = Ring::default();
        for value in spectrum {
            ring.push(value.re / peak * 0.8);
        }
        let mut analyzer = Analyzer::with_sample_rate(44_100);
        let mut bands = [0.0; VIZ_BANDS];
        for _ in 0..240 {
            bands = analyzer.analyze(&ring, 1.2);
        }
        assert!(bands.iter().all(|v| *v > 0.0 && *v < STEADY_MAX));
        let min = bands.iter().copied().fold(f32::INFINITY, f32::min);
        let max = bands.iter().copied().fold(0.0_f32, f32::max);
        // Linear (no knee): judge the tilt as a ratio, not an absolute spread.
        assert!(min > 0.6 * max, "pink noise tilt: min={min} max={max}");
    }

    #[test]
    fn agc_raises_quiet_signal_and_recovers_slowly_after_loud_signal() {
        let quiet = sine(1000.0, 0.002, 44_100);
        let loud = sine(1000.0, 0.9, 44_100);
        let band = band_for_frequency(1000.0);
        let mut analyzer = Analyzer::with_sample_rate(44_100);
        let initial = analyzer.analyze(&quiet, 1.0)[band];
        let mut settled = initial;
        for _ in 0..1200 {
            settled = analyzer.analyze(&quiet, 1.0)[band];
        }
        assert!(
            settled > initial + 0.15,
            "initial={initial} settled={settled}"
        );
        assert!(settled > 0.6, "settled={settled}");
        let quiet_peak = analyzer.recent_peak;
        analyzer.analyze(&loud, 1.0);
        let loud_peak = analyzer.recent_peak;
        assert!(loud_peak > quiet_peak * 10.0);
        analyzer.analyze(&quiet, 1.0);
        assert!(analyzer.recent_peak > loud_peak * 0.99);
        assert_eq!(analyzer.analyze(&Ring::default(), 2.0), [0.0; VIZ_BANDS]);
    }

    #[test]
    fn strong_sustained_signal_keeps_headroom_and_responds_to_level_changes() {
        let mut analyzer = Analyzer::with_sample_rate(44_100);
        let ring = sine(1000.0, 1.0, 44_100);
        let mut smoothed = [0.0; VIZ_BANDS];
        for _ in 0..600 {
            let bands = analyzer.analyze(&ring, 2.0);
            smooth(&mut smoothed, &bands);
            assert!(bands.iter().chain(&smoothed).all(|v| *v < STEADY_MAX));
        }
        let band = band_for_frequency(1000.0);
        let before = analyzer.analyze(&ring, 2.0)[band];
        let after = analyzer.analyze(&sine(1000.0, 0.8, 44_100), 2.0)[band];
        assert!(before - after > 0.001, "before={before} after={after}");
    }
    #[test]
    fn silence_is_exact_zero() {
        assert_eq!(
            Analyzer::new().analyze(&Ring::default(), 2.0),
            [0.0; VIZ_BANDS]
        );
        assert!(!changed(&[0.0; VIZ_BANDS], &[0.0; VIZ_BANDS]));
    }
    #[test]
    fn sensitivity_increases_with_a_soft_knee() {
        let amplitude = 10.0_f32.powf(-12.0 / 20.0);
        assert!(normalized_db(amplitude, 2.0) > normalized_db(amplitude, 1.0));
        let mut previous = 0.0;
        for amplitude in [0.1, 0.3, 1.0, 3.0, 10.0] {
            let level = normalized_db(amplitude, 2.0);
            assert!(level > previous && level.is_finite());
            previous = level;
        }
        assert_eq!(normalized_db(0.0, 2.0), 0.0);
        assert_eq!(normalized_db(f32::NAN, 2.0), 0.0);
        assert_eq!(sensitivity(0.0), 0.25);
        assert_eq!(sensitivity(3.0), 2.0);
        assert_eq!(sensitivity(f32::NAN), 1.0);
    }
    #[test]
    fn at_most_30_frames_in_any_simulated_second() {
        let mut limiter = FrameLimiter::default();
        let times: Vec<_> = (0..10_000)
            .filter(|ms| limiter.allow(Duration::from_millis(*ms)))
            .collect();
        for start in 0..9000 {
            assert!(
                times
                    .iter()
                    .filter(|t| **t >= start && **t < start + 1000)
                    .count()
                    <= 30
            );
        }
        let mut limiter = FrameLimiter::default();
        assert!(limiter.allow(Duration::ZERO));
        assert!(!limiter.allow(Duration::from_nanos(33_333_333)));
        assert!(limiter.allow(Duration::from_nanos(33_333_334)));
    }
    #[test]
    fn limiter_follows_the_reported_rate_and_clamps_it() {
        let mut limiter = FrameLimiter::default();
        limiter.set_fps(clamp_fps(144.0));
        let n = (0..1000)
            .filter(|ms| limiter.allow(Duration::from_micros(*ms * 1000)))
            .count();
        assert!((140..=145).contains(&n), "{n}");
        assert_eq!(clamp_fps(10.0), MIN_FPS);
        assert_eq!(clamp_fps(1000.0), MAX_FPS);
        assert_eq!(clamp_fps(f64::NAN), DEFAULT_FPS);
        assert_eq!(clamp_fps(0.0), DEFAULT_FPS);
        assert_eq!(clamp_fps(59.94), 60);
    }
    #[test]
    fn attack_is_quick_silence_settles_to_zero() {
        let mut bands = [0.0; VIZ_BANDS];
        smooth(&mut bands, &[1.0; VIZ_BANDS]);
        assert_eq!(bands, [ATTACK; VIZ_BANDS]);
        for _ in 0..2 {
            smooth(&mut bands, &[1.0; VIZ_BANDS]);
        }
        assert!(bands.iter().all(|v| *v > 0.9));
        smooth(&mut bands, &[0.0; VIZ_BANDS]);
        assert_eq!(bands, [0.0; VIZ_BANDS]);
        for _ in 0..100 {
            smooth(&mut bands, &[0.0; VIZ_BANDS]);
        }
        assert_eq!(bands, [0.0; VIZ_BANDS]);
    }
    #[test]
    fn ring_keeps_latest_samples_in_order() {
        let mut ring = Ring::default();
        for i in 0..FFT_SIZE + 5 {
            ring.push(i as f32);
        }
        assert_eq!(ring.samples[ring.next], 5.0);
        ring.clear();
        assert!(ring.samples.iter().all(|x| *x == 0.0));
    }
}
