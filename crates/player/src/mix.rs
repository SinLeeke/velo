//! «Mixear» (como el botón de Spotify): fundido cruzado en tiempo real entre canciones
//! consecutivas, hecho por el sink propio (pcm.rs) sin tocar librespot ni las pistas.
//! Aquí solo hay el buzón atómico de control y funciones puras; el audio vive en memoria.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use librespot_metadata::audio::item::{AudioItem, UniqueFields};
use librespot_playback::SAMPLE_RATE;
pub use velo_domain::MixConfig;

const ENABLED: u64 = 1;
/// El DJ está activo: su voz tiene prioridad y la canción espera en pausa, nunca se mezcla.
const BLOCKED: u64 = 2;
const SECONDS_SHIFT: u32 = 8;

/// Buzón atómico (como `EqControl`): el hilo de audio lo lee sin esperar ni reservar.
#[derive(Clone)]
pub(crate) struct MixControl(Arc<AtomicU64>);
impl Default for MixControl {
    fn default() -> Self {
        Self(Arc::new(AtomicU64::new(pack(MixConfig::default(), false))))
    }
}
fn pack(config: MixConfig, blocked: bool) -> u64 {
    let config = config.clamped();
    (u64::from(config.seconds) << SECONDS_SHIFT)
        | if config.enabled { ENABLED } else { 0 }
        | if blocked { BLOCKED } else { 0 }
}
impl MixControl {
    pub fn set(&self, config: MixConfig) {
        let mut old = self.0.load(Ordering::Acquire);
        loop {
            let new = pack(config, old & BLOCKED != 0);
            match self
                .0
                .compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(current) => old = current,
            }
        }
    }
    pub fn set_blocked(&self, blocked: bool) {
        if blocked {
            self.0.fetch_or(BLOCKED, Ordering::AcqRel);
        } else {
            self.0.fetch_and(!BLOCKED, Ordering::AcqRel);
        }
    }
    pub fn config(&self) -> MixConfig {
        let packed = self.0.load(Ordering::Acquire);
        MixConfig {
            enabled: packed & ENABLED != 0,
            seconds: (packed >> SECONDS_SHIFT) as u32,
        }
        .clamped()
    }
    /// Segundos pedidos si se puede mezclar ahora (activada y sin DJ).
    pub fn allowed(&self) -> Option<u32> {
        let packed = self.0.load(Ordering::Acquire);
        (packed & (ENABLED | BLOCKED) == ENABLED).then(|| self.config().seconds)
    }
}

/// Lo que el sink publica (desde el hilo del decoder, o en transiciones desde el callback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MixStatus {
    /// Hay un fundido sonando.
    pub active: bool,
    /// Lo que le queda al fundido (granularidad ~1 s entre publicaciones).
    pub remaining_ms: u64,
    /// El decoder va por delante del audio (cola de la ventana): los eventos de librespot de
    /// fin y de la pista siguiente llegan hasta esto antes de oírse.
    pub ahead_ms: u64,
    /// La pista siguiente empezó a oírse (request id, instante), tras un fin natural.
    pub started: Option<(u64, std::time::Instant)>,
}

/// El fundido visto por la interfaz (`EngineState.mix`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MixState {
    pub active: bool,
    pub remaining_ms: u64,
    /// Interno: tolerancia extra para no tomar el fin adelantado de la pista como un corte.
    pub ahead_ms: u64,
}

/// Curvas de igual potencia: `gA = cos(πt/2)`, `gB = sin(πt/2)`, con `t` en 0..=1.
pub fn gains(t: f32) -> (f32, f32) {
    let (sin, cos) = (std::f32::consts::FRAC_PI_2 * t.clamp(0.0, 1.0)).sin_cos();
    (cos, sin)
}

/// Rodilla del limitador suave, −1 dBFS.
pub const LIMITER_KNEE: f32 = 0.891_250_9;
/// Limitador suave de la suma: lineal hasta la rodilla y saturación tanh hasta 1 (nunca la
/// supera). Solo durante el solape, antes del ecualizador.
pub fn limit(x: f32) -> f32 {
    let magnitude = x.abs();
    if magnitude <= LIMITER_KNEE || !x.is_finite() {
        return if x.is_finite() { x } else { 0.0 };
    }
    let room = 1.0 - LIMITER_KNEE;
    (LIMITER_KNEE + room * ((magnitude - LIMITER_KNEE) / room).tanh()).copysign(x)
}

/// Rampa (≈5 ms) con la que el limitador entra al solape y sale tras él: así el paso de señal
/// sin limitar a limitada nunca es un salto, y fuera del solape el PCM queda intacto.
pub const LIMITER_RAMP_FRAMES: usize = SAMPLE_RATE as usize / 200;

/// Colchón mínimo de B (≈250 ms decodificados) para iniciar el fundido; con menos, A sigue sola.
pub const MIX_B_CUSHION_FRAMES: usize = SAMPLE_RATE as usize / 4;

/// Mezcla continua sin limitar → limitada con peso `weight` (0 = señal intacta, bit a bit).
/// Con peso > 0 el resultado nunca supera 1 (la rampa de entrada puede recibir sumas > 1).
pub fn limit_blend(x: f32, weight: f32) -> f32 {
    if weight <= 0.0 {
        return x;
    }
    (x + weight * (limit(x) - x)).clamp(-1.0, 1.0)
}

/// Frames del solape: `seconds` (1..=12) recortado a 1/3 de la duración de la pista.
/// Duración desconocida (0): sin mezcla.
pub fn window_frames(seconds: u32, duration_ms: u64) -> usize {
    let wanted = u64::from(seconds.clamp(MixConfig::MIN_SECONDS, MixConfig::MAX_SECONDS)) * 1000;
    let ms = wanted.min(duration_ms / 3);
    (ms * u64::from(SAMPLE_RATE) / 1000) as usize
}

/// Gapless de álbum: B es la pista siguiente del mismo álbum (nombre y artistas del álbum),
/// en el mismo disco o la primera del disco siguiente. Entonces se concatena sin mezclar.
pub fn same_album_gapless(a: &AudioItem, b: &AudioItem) -> bool {
    let (
        UniqueFields::Track {
            album: album_a,
            album_artists: artists_a,
            number: number_a,
            disc_number: disc_a,
            ..
        },
        UniqueFields::Track {
            album: album_b,
            album_artists: artists_b,
            number: number_b,
            disc_number: disc_b,
            ..
        },
    ) = (&a.unique_fields, &b.unique_fields)
    else {
        return false;
    };
    !album_a.is_empty()
        && album_a == album_b
        && artists_a == artists_b
        && ((disc_a == disc_b && *number_b == number_a.saturating_add(1))
            || (*disc_b == disc_a.saturating_add(1) && *number_b == 1))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use librespot_metadata::artist::ArtistsWithRole;

    pub(crate) fn item(
        uri: &str,
        album: &str,
        artists: &[&str],
        disc: u32,
        number: u32,
        duration_ms: u32,
    ) -> Box<AudioItem> {
        let id = crate::parse_track_uri(uri).unwrap();
        Box::new(AudioItem {
            track_id: id,
            uri: uri.into(),
            files: Default::default(),
            name: "Fixture".into(),
            covers: vec![],
            language: vec![],
            duration_ms,
            is_explicit: false,
            availability: Ok(()),
            alternatives: None,
            unique_fields: UniqueFields::Track {
                artists: ArtistsWithRole(vec![]),
                album: album.into(),
                album_artists: artists.iter().map(|a| (*a).to_owned()).collect(),
                popularity: 0,
                number,
                disc_number: disc,
            },
        })
    }
    const A: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
    const B: &str = "spotify:track:0000000000000000000001";

    #[test]
    fn equal_power_curves_keep_unit_energy_and_cross_at_the_middle() {
        assert_eq!(gains(0.0), (1.0, 0.0));
        let (a, b) = gains(1.0);
        assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
        for step in 0..=100 {
            let (a, b) = gains(step as f32 / 100.0);
            assert!((a * a + b * b - 1.0).abs() < 1e-5);
        }
        let (a, b) = gains(0.5);
        assert!((a - b).abs() < 1e-6);
        assert_eq!(gains(-1.0), gains(0.0));
        assert_eq!(gains(2.0), gains(1.0));
    }

    #[test]
    fn equal_power_does_not_guarantee_constant_level_for_correlated_tracks() {
        let (a, b) = gains(0.5);
        let sample = 0.25;
        let in_phase = limit(sample * a + sample * b);
        let opposite_phase = limit(sample * a - sample * b);
        assert!((in_phase * in_phase / (sample * sample) - 2.0).abs() < 1e-5);
        assert!(opposite_phase.abs() < 1e-6);
    }

    #[test]
    fn limiter_is_transparent_below_the_knee_and_never_reaches_full_scale() {
        for x in [0.0, 0.25, -0.5, 0.89, -LIMITER_KNEE] {
            assert_eq!(limit(x), x);
        }
        let mut previous = LIMITER_KNEE;
        for step in 1..400 {
            let x = LIMITER_KNEE + step as f32 * 0.01;
            let y = limit(x);
            assert!(y > previous - 1e-6 && y <= 1.0, "{x} -> {y}");
            assert_eq!(limit(-x), -y);
            previous = y;
        }
        assert_eq!(limit(f32::NAN), 0.0);
        assert_eq!(limit(f32::INFINITY), 0.0);
    }

    #[test]
    fn window_is_clamped_to_a_third_of_the_track_and_to_the_supported_seconds() {
        let frames = |s: u64| (s * u64::from(SAMPLE_RATE) / 1000) as usize;
        assert_eq!(window_frames(6, 200_000), frames(6_000));
        assert_eq!(window_frames(12, 30_000), frames(10_000));
        assert_eq!(window_frames(40, 600_000), frames(12_000));
        assert_eq!(window_frames(0, 600_000), frames(1_000));
        assert_eq!(window_frames(6, 0), 0);
    }

    #[test]
    fn album_gapless_needs_same_album_artists_and_the_next_number_or_disc() {
        let a = item(A, "Atlas", &["Lumen"], 1, 4, 200_000);
        assert!(same_album_gapless(
            &a,
            &item(B, "Atlas", &["Lumen"], 1, 5, 1)
        ));
        assert!(same_album_gapless(
            &a,
            &item(B, "Atlas", &["Lumen"], 2, 1, 1)
        ));
        assert!(!same_album_gapless(
            &a,
            &item(B, "Atlas", &["Lumen"], 1, 6, 1)
        ));
        assert!(!same_album_gapless(
            &a,
            &item(B, "Atlas", &["Lumen"], 1, 3, 1)
        ));
        assert!(!same_album_gapless(
            &a,
            &item(B, "Atlas", &["Otro"], 1, 5, 1)
        ));
        assert!(!same_album_gapless(
            &a,
            &item(B, "Mapa", &["Lumen"], 1, 5, 1)
        ));
        assert!(!same_album_gapless(
            &a,
            &item(B, "Atlas", &["Lumen"], 2, 2, 1)
        ));
        let untitled = item(A, "", &[], 1, 1, 1);
        assert!(!same_album_gapless(&untitled, &item(B, "", &[], 1, 2, 1)));
    }

    #[test]
    fn control_mailbox_clamps_and_dj_blocks_without_losing_the_choice() {
        let control = MixControl::default();
        assert_eq!(control.config(), MixConfig::default());
        assert_eq!(control.allowed(), None);
        control.set(MixConfig {
            enabled: true,
            seconds: 30,
        });
        assert_eq!(control.allowed(), Some(12));
        control.set_blocked(true);
        assert_eq!(control.allowed(), None);
        control.set(MixConfig {
            enabled: true,
            seconds: 4,
        });
        assert_eq!(control.allowed(), None);
        control.set_blocked(false);
        assert_eq!(control.allowed(), Some(4));
        control.set(MixConfig {
            enabled: false,
            seconds: 4,
        });
        assert_eq!(control.allowed(), None);
    }
}
