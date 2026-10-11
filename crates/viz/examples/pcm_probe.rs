//! Offline PCM worker probe. With no argument, generates then reads a temporary
//! mono PCM16 WAV (bass tones, quiet/loud 1 kHz, pink noise and two song patterns). An optional path
//! reads another local PCM16 WAV. No network, playback device or capture.
//! cargo run -p velo-viz --example pcm_probe -- [local.wav]
use std::io::{self, ErrorKind};
use std::path::Path;
use std::time::{Duration, Instant};
use velo_domain::Visualizer;
use velo_viz::VizEngine;

const RATE: u32 = 44_100;

fn song(t: f64, bpm: f64, white: f64) -> f64 {
    let beat = 60.0 / bpm;
    let kick_t = t % beat;
    let kick = 0.65
        * (-kick_t * 38.0).exp()
        * (std::f64::consts::TAU * (55.0 * kick_t + 0.8 * (1.0 - (-kick_t * 30.0).exp()))).sin();
    let snare_t = (t + beat) % (2.0 * beat);
    let snare = 0.22 * (-snare_t * 28.0).exp() * white;
    let syllable = t % (beat * 1.5);
    let voice_envelope = (syllable * 25.0).min(1.0) * (-syllable * 5.0).exp();
    let note = if ((t / (beat * 4.0)) as u32).is_multiple_of(2) {
        220.0
    } else {
        277.18
    };
    let voice = (1..=8)
        .map(|harmonic| {
            let h = f64::from(harmonic);
            (std::f64::consts::TAU * note * h * t).sin() / h
        })
        .sum::<f64>()
        * voice_envelope
        * 0.09;
    kick + snare + voice
}

fn generated_wav(path: &Path) -> io::Result<()> {
    let mut samples = Vec::new();
    let mut random = 17_u32;
    let mut pink = [0.0_f64; 6];
    for second in 0..18 {
        for i in 0..RATE {
            let t = f64::from(i) / f64::from(RATE);
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let white = f64::from(random) / f64::from(u32::MAX) * 2.0 - 1.0;
            let value = match second {
                0..=3 => {
                    let hz = 30.0 * (16_000.0_f64 / 30.0).powf((f64::from(second) + 0.5) / 48.0);
                    (std::f64::consts::TAU * hz * t).sin() * 0.5
                }
                4 => (std::f64::consts::TAU * 1000.0 * t).sin() * 0.002,
                5 => (std::f64::consts::TAU * 1000.0 * t).sin() * 0.9,
                6..=7 => {
                    // Six octave-spaced low-pass noise streams approximate pink noise.
                    let mut value = white * 0.1;
                    for (octave, state) in pink.iter_mut().enumerate() {
                        let alpha = 0.5_f64.powi(octave as i32 + 1);
                        *state += alpha * (white - *state);
                        value += *state / alpha.sqrt();
                    }
                    value * 0.1
                }
                8..=11 => song(f64::from(second - 8) + t, 120.0, white),
                12..=15 => song(f64::from(second - 12) + t, 90.0, white),
                _ => 0.0,
            };
            samples.extend_from_slice(&((value.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    let size = samples.len() as u32;
    let mut wav = Vec::with_capacity(samples.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(size + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes()); // block alignment
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&size.to_le_bytes());
    wav.extend_from_slice(&samples);
    std::fs::write(path, wav)
}

fn read_wav(path: &Path) -> io::Result<(Vec<f64>, u32, u16)> {
    let bytes = std::fs::read(path)?;
    let invalid = || {
        io::Error::new(
            ErrorKind::InvalidData,
            "expected RIFF/WAVE PCM16, 8–192 kHz",
        )
    };
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(invalid());
    }
    let mut format = None;
    let mut data = None;
    let mut offset = 12;
    while let Some(header) = bytes.get(offset..offset + 8) {
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or_else(invalid)?;
        let chunk = bytes.get(start..end).ok_or_else(invalid)?;
        match &header[..4] {
            b"fmt " if size >= 16 => {
                let pcm = u16::from_le_bytes(chunk[..2].try_into().unwrap());
                let channels = u16::from_le_bytes(chunk[2..4].try_into().unwrap());
                let rate = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes(chunk[14..16].try_into().unwrap());
                if pcm != 1 || bits != 16 || channels == 0 || !(8_000..=192_000).contains(&rate) {
                    return Err(invalid());
                }
                format = Some((rate, channels));
            }
            b"data" => data = Some(chunk),
            _ => {}
        }
        offset = end + (size % 2);
    }
    let (rate, channels) = format.ok_or_else(invalid)?;
    let data = data.ok_or_else(invalid)?;
    if data.len() % (usize::from(channels) * 2) != 0 {
        return Err(invalid());
    }
    let samples = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|s| f64::from(i16::from_le_bytes(*s)) / 32768.0)
        .collect();
    Ok((samples, rate, channels))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let generated = input.is_none();
    let path = input.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("velo-pcm-probe-{}.wav", std::process::id()))
    });
    if generated {
        generated_wav(&path)?;
    }
    let (samples, rate, channels) = read_wav(&path)?;
    if generated {
        std::fs::remove_file(&path)?;
    }
    println!("WAV: {} ({rate} Hz, {channels} channels)", path.display());
    if generated {
        println!(
            "0-4s: first four bass bands; 4-5s: quiet 1kHz; 5-6s: loud 1kHz; 6-8s: pink noise; \
             8-12s: kick + snare + voice at 120 BPM; 12-16s: 90 BPM; 16-18s: silence"
        );
    }
    let engine = VizEngine::new();
    engine.set_external_active(true);
    let bands = engine.bands();
    let start = Instant::now();
    // Slightly >33.334ms so every packet is eligible under the <=30Hz limiter.
    let frames = (rate as usize).div_ceil(29);
    let mut sent = 0;
    for (index, packet) in samples.chunks(frames * usize::from(channels)).enumerate() {
        engine.submit_pcm(packet.to_vec(), rate, channels);
        sent += packet.len() / usize::from(channels);
        let deadline = start + Duration::from_secs_f64(sent as f64 / f64::from(rate));
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        // Feed this real analyzed frame to VisualizerRingProbe.qml -- --bands <csv>.
        if generated && index == 230 {
            let csv = bands
                .borrow()
                .iter()
                .map(|v| format!("{v:.6}"))
                .collect::<Vec<_>>()
                .join(",");
            println!("ring-fixture: {csv}");
        }
        if index % 3 == 0 {
            let values = *bands.borrow();
            let line: String = values
                .iter()
                .map(|v| {
                    const LEVELS: &[u8] = b" .:-=+*#%@";
                    LEVELS[(v * 9.0).round() as usize] as char
                })
                .collect();
            println!(
                "{:5.2}s |{line}| bass={:.2?} treble={:.3} max={:.3}",
                start.elapsed().as_secs_f32(),
                &values[..4],
                values[24..].iter().sum::<f32>() / 24.0,
                values.iter().copied().fold(0.0_f32, f32::max)
            );
        }
    }
    engine.set_external_active(false);
    assert!(bands.borrow().iter().all(|v| *v == 0.0));
    Ok(())
}
