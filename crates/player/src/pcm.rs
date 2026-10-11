use crate::eq::{EqControl, Equalizer};
use crate::mix::{self, MixControl, MixStatus};
use librespot_metadata::audio::item::AudioItem;
use librespot_playback::{
    NUM_CHANNELS, SAMPLE_RATE,
    audio_backend::{Sink, SinkError, SinkResult},
    convert::Converter,
    decoder::AudioPacket,
    mixer::VolumeGetter,
    player::{PlayerEvent, PlayerEventChannel},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc as std_mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, mpsc, watch};

const TAP_SAMPLES: usize = 2048;
/// Duraciones a 44,1 kHz: volumen 5 ms, pausa 120 ms, reanudación 150 ms,
/// discontinuidades manuales 8 ms por lado. El fin natural conserva el gapless.
const VOLUME_FRAMES: usize = SAMPLE_RATE as usize * 5 / 1000;
const PAUSE_FRAMES: usize = SAMPLE_RATE as usize * 120 / 1000;
const RESUME_FRAMES: usize = SAMPLE_RATE as usize * 150 / 1000;
const MICRO_FRAMES: usize = SAMPLE_RATE as usize * 8 / 1000;
/// E5b: cortar la voz (siguiente, otra carga) con un fundido de ~30 ms, sin clic.
const VOICE_STOP_FRAMES: usize = SAMPLE_RATE as usize * 30 / 1000;
const WAIT_STEP: Duration = Duration::from_millis(5);
/// Mixear: margen para que el decoder termine A y empiece B antes de que empiece el solape.
const MIX_MARGIN_MS: u64 = 1000;
/// Holgura de la cola sobre la ventana (paquetes en vuelo del decoder).
const MIX_SLACK_MS: u64 = 500;
/// Con menos de esto de A en cola al llegar B no hubo ventana: se concatena como siempre.
const MIX_MIN_TAIL_FRAMES: usize = SAMPLE_RATE as usize / 2;
const STALL_LIMIT: Duration = Duration::from_secs(1);

/// PCM normalizado, anterior al volumen y a todos los fades; solo en memoria.
pub struct PcmPacket {
    /// Instante del primer frame, nunca el de envío del paquete.
    pub captured_at: Instant,
    pub samples: Vec<f64>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Cumulative PCM consumption by librespot play request (not a playback position).
/// Request ids scope to one Engine. Join with transport events for URI/context metadata.
/// Voice, silence from underruns, discarded PCM, and seek positions never add frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaybackProgress {
    pub play_request_id: u64,
    pub consumed_frames: u64,
    pub played_ms: u64,
}
#[derive(Clone, Copy, Default)]
struct ProgressEntry {
    progress: PlaybackProgress,
    published_frames: u64,
    valid: bool,
}
struct PcmSpan {
    request_id: u64,
    samples: usize,
    /// Primer tramo de una pista que siguió a un fin natural: al oírse, su posición empieza.
    natural: bool,
}

/// Fundido cruzado en curso: `frames` es lo que quedaba de A al empezar.
#[derive(Clone, Copy)]
struct Fade {
    elapsed: usize,
    frames: usize,
    /// Progreso (0..1) con el que arranca este tramo: 0 al empezar; si B se atrasó, el punto
    /// congelado desde el que el fundido continúa sin salto de ganancia.
    t0: f32,
}
impl Fade {
    fn gains_at(&self, frame: usize) -> (f32, f32) {
        let u = (frame as f32 + 0.5) / self.frames.max(1) as f32;
        mix::gains(self.t0 + (1.0 - self.t0) * u)
    }
    /// Ganancias retenidas mientras B no tiene muestras: el progreso actual, sin avanzar.
    fn held_gains(&self) -> (f32, f32) {
        let u = self.elapsed as f32 / self.frames.max(1) as f32;
        mix::gains(self.t0 + (1.0 - self.t0) * u)
    }
    /// B se atrasó: congela el progreso y reparte lo que queda de él en lo que queda de A.
    fn rebase(&mut self, remaining: usize) {
        let u = self.elapsed as f32 / self.frames.max(1) as f32;
        self.t0 += (1.0 - self.t0) * u;
        self.frames = remaining;
        self.elapsed = 0;
    }
    fn remaining_ms(&self) -> u64 {
        self.frames.saturating_sub(self.elapsed) as u64 * 1000 / u64::from(SAMPLE_RATE)
    }
}

/// Local request wins over stale Spirc commands until its matching acknowledgement.
/// Packed gain + request are atomic, so the callback cannot see a torn target.
struct VolumeRequest {
    packed: AtomicU64,
    // Controller/forwarder only. Audio reads packed, never this acknowledgement queue.
    acknowledgements: Mutex<VecDeque<u16>>,
    origin: Instant,
    requested_us: AtomicU64,
}
/// Un override sin acknowledgement se descarta pasado este tiempo (Spirc pudo omitir el valor).
const VOLUME_OVERRIDE_TTL_US: u64 = 4_000_000;
#[derive(Clone)]
pub(crate) struct VolumeOverride(Arc<VolumeRequest>);
impl Default for VolumeOverride {
    fn default() -> Self {
        Self(Arc::new(VolumeRequest {
            packed: AtomicU64::new(0),
            acknowledgements: Mutex::new(VecDeque::new()),
            origin: Instant::now(),
            requested_us: AtomicU64::new(0),
        }))
    }
}
impl VolumeOverride {
    pub fn pending(&self) -> bool {
        self.expire();
        self.0.packed.load(Ordering::Acquire) >> 63 != 0
    }
    fn expire(&self) {
        let packed = self.0.packed.load(Ordering::Acquire);
        if packed >> 63 == 0 {
            return;
        }
        let age = (self.0.origin.elapsed().as_micros() as u64)
            .saturating_sub(self.0.requested_us.load(Ordering::Acquire));
        if age > VOLUME_OVERRIDE_TTL_US
            && self
                .0
                .packed
                .compare_exchange(packed, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            self.0
                .acknowledgements
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }
    }
    #[cfg(debug_assertions)]
    pub fn elapsed_us(&self) -> Option<u64> {
        self.pending().then(|| {
            (self.0.origin.elapsed().as_micros() as u64)
                .saturating_sub(self.0.requested_us.load(Ordering::Acquire))
        })
    }
    #[cfg(debug_assertions)]
    pub fn requested_volume(&self) -> Option<u16> {
        let packed = self.0.packed.load(Ordering::Acquire);
        (packed >> 63 != 0).then_some((packed >> 32) as u16)
    }
    pub fn request(&self, volume: u16, gain: f32) {
        let mut acknowledgements = self
            .0
            .acknowledgements
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if acknowledgements.back() != Some(&volume) {
            acknowledgements.push_back(volume);
        }
        self.0.requested_us.store(
            self.0.origin.elapsed().as_micros() as u64,
            Ordering::Release,
        );
        self.0.packed.store(
            (1 << 63) | (u64::from(volume) << 32) | u64::from(gain.to_bits()),
            Ordering::Release,
        );
    }
    pub fn acknowledge(&self, volume: u16) {
        let mut acknowledgements = self
            .0
            .acknowledgements
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Spirc puede omitir valores intermedios: descarta hasta la entrada coincidente.
        let Some(at) = acknowledgements.iter().position(|v| *v == volume) else {
            return;
        };
        acknowledgements.drain(..=at);
        if !acknowledgements.is_empty() {
            return;
        }
        let pending = self.0.packed.load(Ordering::Acquire);
        if pending >> 63 != 0 && ((pending >> 32) as u16) == volume {
            let _ = self
                .0
                .packed
                .compare_exchange(pending, 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }
    fn gain(&self, original: f32) -> f32 {
        self.expire();
        let pending = self.0.packed.load(Ordering::Acquire);
        if pending >> 63 != 0 {
            f32::from_bits(pending as u32)
        } else {
            original
        }
    }
}

/// E5b: voz de DJ Livi ya decodificada, a 44,1 kHz, mono o estéreo; solo en memoria.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceClip {
    samples: Vec<f32>,
    channels: usize,
}
impl VoiceClip {
    /// `samples` intercalados a 44,1 kHz; `None` si está vacío o no es mono/estéreo.
    pub fn new(samples: Vec<f32>, channels: usize) -> Option<Self> {
        (matches!(channels, 1 | 2)
            && samples.len() >= channels
            && samples.len() % channels == 0
            && samples.iter().all(|s| s.is_finite()))
        .then(|| Self {
            samples: samples.into_iter().map(|s| s.clamp(-1.0, 1.0)).collect(),
            channels,
        })
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn duration_ms(&self) -> u64 {
        self.frames() as u64 * 1000 / u64::from(SAMPLE_RATE)
    }
    pub(crate) fn frame(&self, index: usize) -> [f32; 2] {
        let at = index * self.channels;
        let left = self.samples.get(at).copied().unwrap_or(0.0);
        if self.channels == 1 {
            [left, left]
        } else {
            [left, self.samples.get(at + 1).copied().unwrap_or(0.0)]
        }
    }
}

/// Avance de la voz, publicado solo al cambiar (inicio, pausa, seek, fin), nunca por frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceProgress {
    pub position_ms: u64,
    pub at: Instant,
    pub playing: bool,
    pub finished: bool,
}

struct Voice {
    clip: Arc<VoiceClip>,
    frame: usize,
    paused: bool,
    stopping: bool,
    /// Ya llegó a cero en pausa y se avisó al worker para soltar WASAPI.
    idle: bool,
    ramp: Ramp,
    progress: watch::Sender<VoiceProgress>,
}
impl Voice {
    fn position_ms(&self) -> u64 {
        self.frame as u64 * 1000 / u64::from(SAMPLE_RATE)
    }
    fn report(&self) {
        self.progress.send_replace(VoiceProgress {
            position_ms: self.position_ms(),
            at: Instant::now(),
            playing: !self.paused && !self.stopping,
            finished: false,
        });
    }
    fn finish(self) {
        self.progress.send_replace(VoiceProgress {
            position_ms: self.position_ms(),
            at: Instant::now(),
            playing: false,
            finished: true,
        });
    }
}

/// Bit bajo: activo; bits restantes: generación. Incluso false→true sin callback se detecta.
#[derive(Clone, Default)]
pub(crate) struct TapActivation(Arc<AtomicU64>);
impl TapActivation {
    pub fn set(&self, enabled: bool) {
        let mut old = self.0.load(Ordering::Acquire);
        loop {
            if old & 1 == u64::from(enabled) {
                return;
            }
            let new = ((old >> 1).wrapping_add(1) << 1) | u64::from(enabled);
            match self
                .0
                .compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(current) => old = current,
            }
        }
    }
    fn state(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Clone)]
pub(crate) struct PcmTap {
    pub enabled: TapActivation,
    pub sender: mpsc::Sender<PcmPacket>,
}

/// Coseno: pendiente nula en ambos extremos, duración independiente del nivel.
struct Ramp {
    value: f32,
    from: f32,
    target: f32,
    elapsed: usize,
    frames: usize,
}
impl Ramp {
    fn new(value: f32) -> Self {
        Self {
            value,
            from: value,
            target: value,
            elapsed: 0,
            frames: 0,
        }
    }
    fn to(&mut self, target: f32, frames: usize) {
        if self.target != target {
            self.from = self.value;
            self.target = target;
            self.elapsed = 0;
            self.frames = frames;
        }
    }
    fn step(&mut self) -> f32 {
        if self.elapsed < self.frames {
            self.elapsed += 1;
            let t = self.elapsed as f32 / self.frames as f32;
            self.value = self.from
                + (self.target - self.from) * (0.5 - 0.5 * (std::f32::consts::PI * t).cos());
            if self.elapsed == self.frames {
                self.value = self.target;
            }
        }
        self.value
    }
}

/// Sobrevive al cierre de WASAPI, incluido el canal derecho pendiente.
struct Playback {
    queue: VecDeque<f32>,
    old: VecDeque<f32>,
    soft_volume: Box<dyn VolumeGetter + Send>,
    volume_override: VolumeOverride,
    eq: Equalizer,
    decoding_request: Option<u64>,
    spans: VecDeque<PcmSpan>,
    ledger: [ProgressEntry; 32],
    ledger_next: usize,
    ledger_active: Option<usize>,
    progress_tx: Option<broadcast::Sender<PlaybackProgress>>,
    volume: Ramp,
    transport: Ramp,
    micro: Ramp,
    playing: bool,
    pause_generation: u64,
    notified: bool,
    frame: [f32; 2],
    raw_frame: [f32; 2],
    channel: usize,
    consumed: u64,
    tap: PcmTap,
    tap_state: u64,
    pending: Vec<f64>,
    captured_at: Instant,
    events: Option<PlayerEventChannel>,
    natural_end: bool,
    /// Spirc terminó el contexto, pero todavía queda la cola final por oír.
    final_tail: bool,
    pause_request: mpsc::UnboundedSender<Option<std_mpsc::Sender<()>>>,
    timer_uri: Option<String>,
    timer_dj: bool,
    end_remaining: Option<usize>,
    timer_state: watch::Sender<TrackEndTimerState>,
    /// E5b: la voz de Livi tiene prioridad sobre la canción (que espera en pausa).
    voice: Option<Voice>,
    /// Comandos al worker de salida, para abrir el dispositivo solo para la voz.
    worker: Option<std_mpsc::Sender<OutputCommand>>,
    /// La voz se cortó desde fuera: al terminar, soltar la salida si la canción no suena.
    release_after_voice: bool,
    /// Mixear: ajuste en caliente (atómico) y estado del fundido.
    mix: MixControl,
    /// «Deck B»: PCM de la pista siguiente mientras A todavía suena (solo en memoria).
    deck_b: VecDeque<f32>,
    spans_b: VecDeque<PcmSpan>,
    ledger_active_b: Option<usize>,
    /// El decoder escribe en `deck_b` (decidido en TrackChanged(B), antes de sus muestras).
    route_b: bool,
    /// Solape planificado en frames.
    mix_window: usize,
    fade: Option<Fade>,
    /// Peso 0..1 del limitador de la mezcla (rampa de entrada/salida del solape).
    limiter_weight: f32,
    /// Metadatos de la pista que se decodifica (A; luego B al llegar su TrackChanged).
    item: Option<Box<AudioItem>>,
    /// Hubo un fin natural y aún no llegó el TrackChanged de la siguiente.
    after_natural: bool,
    natural_request: Option<u64>,
    /// Posición de decodificación de la pista actual: ancla + muestras escritas.
    decode_anchor_ms: u64,
    decoded_samples: u64,
    decode_duration_ms: u64,
    /// La cola de esta pista ya puede crecer hasta la ventana (fijo hasta la próxima pista).
    lookahead: bool,
    ahead_ms: u64,
    started: Option<(u64, Instant)>,
    mix_status: watch::Sender<MixStatus>,
}
impl Playback {
    fn new(
        tap: PcmTap,
        soft_volume: Box<dyn VolumeGetter + Send>,
        pause_request: mpsc::UnboundedSender<Option<std_mpsc::Sender<()>>>,
    ) -> Self {
        let gain = soft_volume.attenuation_factor() as f32;
        Self {
            queue: VecDeque::new(),
            old: VecDeque::new(),
            soft_volume,
            volume_override: VolumeOverride::default(),
            eq: Equalizer::new(EqControl::default(), SAMPLE_RATE),
            decoding_request: None,
            spans: VecDeque::with_capacity(32),
            ledger: [ProgressEntry::default(); 32],
            ledger_next: 0,
            ledger_active: None,
            progress_tx: None,
            volume: Ramp::new(gain),
            transport: Ramp::new(0.0),
            micro: Ramp::new(1.0),
            playing: false,
            pause_generation: 0,
            notified: false,
            frame: [0.0; 2],
            raw_frame: [0.0; 2],
            channel: 0,
            consumed: 0,
            tap_state: tap.enabled.state(),
            tap,
            pending: Vec::with_capacity(TAP_SAMPLES),
            captured_at: Instant::now(),
            events: None,
            natural_end: false,
            final_tail: false,
            pause_request,
            timer_uri: None,
            timer_dj: false,
            end_remaining: None,
            timer_state: watch::channel(TrackEndTimerState::Cancelled).0,
            voice: None,
            worker: None,
            release_after_voice: false,
            mix: MixControl::default(),
            deck_b: VecDeque::new(),
            spans_b: VecDeque::with_capacity(32),
            ledger_active_b: None,
            route_b: false,
            mix_window: 0,
            fade: None,
            limiter_weight: 0.0,
            item: None,
            after_natural: false,
            natural_request: None,
            decode_anchor_ms: 0,
            decoded_samples: 0,
            decode_duration_ms: 0,
            lookahead: false,
            ahead_ms: 0,
            started: None,
            mix_status: watch::channel(MixStatus::default()).0,
        }
    }
    fn transport(&mut self, playing: bool) {
        if playing {
            // La canción reanudada desde otro lado (Connect) gana a la voz.
            self.stop_voice();
        }
        if self.playing != playing {
            self.playing = playing;
            self.pause_generation = self.pause_generation.wrapping_add(1);
            self.notified = false;
            self.transport.to(
                if playing { 1.0 } else { 0.0 },
                if playing { RESUME_FRAMES } else { PAUSE_FRAMES },
            );
        }
    }
    fn discontinuity(&mut self) {
        self.final_tail = false;
        self.spans.clear(); // Discarded seek/manual-load PCM cannot count as listening.
        self.natural_request = None;
        self.after_natural = false;
        self.lookahead = false;
        if let Some(fade) = self.fade {
            // El micro-fade de salida debe sonar a la mezcla, no solo a A: premezclar ese tramo.
            let frames = (self.queue.len() / 2).min(MICRO_FRAMES);
            for frame in 0..frames {
                let (ga, gb) = fade.gains_at(fade.elapsed + frame);
                for channel in 0..2 {
                    let at = frame * 2 + channel;
                    let b = self.deck_b.get(at).copied().unwrap_or(0.0);
                    self.queue[at] = mix::limit(self.queue[at] * ga + b * gb);
                }
            }
        }
        self.end_mix();
        // Solo el corto resto anterior se usa para llegar a cero; lo demás se invalida.
        if !self.playing && self.transport.value == 0.0 {
            self.queue.clear();
            self.old.clear();
            self.pending.clear();
            self.frame = [0.0; 2];
            self.raw_frame = [0.0; 2];
            self.channel = 0;
            self.micro = Ramp::new(0.0);
            self.micro.to(1.0, MICRO_FRAMES);
            return;
        }
        self.old = std::mem::take(&mut self.queue);
        self.old.truncate(MICRO_FRAMES * 2);
        self.micro.to(0.0, MICRO_FRAMES);
        self.pending.clear();
    }
    fn events(&mut self) {
        while let Some(event) = self.events.as_mut().and_then(|rx| rx.try_recv().ok()) {
            match event {
                PlayerEvent::EndOfTrack { track_id, .. } => {
                    self.natural_end = true;
                    if track_id.to_uri().ok().as_ref() == self.timer_uri.as_ref()
                        && self.timer_uri.is_some()
                    {
                        self.end_remaining = Some(self.queue.len());
                        if !self.playing && self.transport.value > 0.0 {
                            self.transport.from = self.transport.value;
                            self.transport.elapsed = 0;
                            self.transport.frames = (self.queue.len() / 2).clamp(1, PAUSE_FRAMES);
                        }
                    }
                }
                PlayerEvent::PlayRequestIdChanged { play_request_id } => {
                    self.final_tail = false;
                    // Si la siguiente falló antes de su TrackChanged, la otra sigue siendo natural.
                    let natural = self.natural_end || self.after_natural;
                    if !natural {
                        self.cancel_timer();
                        self.discontinuity();
                    }
                    self.natural_end = false;
                    self.after_natural = natural;
                    // Solo si el decoder iba por delante (ventana de mezcla) la posición de la
                    // siguiente se reancla al oírse; sin ventana, librespot ya va a tiempo.
                    self.natural_request =
                        (natural && self.ahead_ms > 0).then_some(play_request_id);
                    self.decoding_request = Some(play_request_id);
                    self.anchor_decode(0);
                    self.decode_duration_ms = 0;
                    self.lookahead = false;
                }
                PlayerEvent::TrackChanged { audio_item } => {
                    self.decode_duration_ms = u64::from(audio_item.duration_ms);
                    // Mover la caja, sin copiar: este código también corre en el callback.
                    let previous = self.item.replace(audio_item);
                    if std::mem::take(&mut self.after_natural) {
                        self.plan_mix(previous.as_deref());
                    }
                }
                PlayerEvent::Playing {
                    play_request_id,
                    position_ms,
                    ..
                }
                | PlayerEvent::Loading {
                    play_request_id,
                    position_ms,
                    ..
                } if Some(play_request_id) == self.decoding_request => {
                    self.anchor_decode(position_ms);
                }
                PlayerEvent::Seeked {
                    play_request_id,
                    position_ms,
                    ..
                } => {
                    if self.end_remaining.is_some() {
                        self.cancel_timer();
                        self.transport(true);
                    }
                    self.discontinuity();
                    if Some(play_request_id) == self.decoding_request {
                        self.anchor_decode(position_ms);
                    }
                }
                PlayerEvent::Stopped { .. } if self.natural_end => {
                    // Fin del contexto: Spirc para el decoder, pero la cola final sigue sonando.
                    self.natural_end = false;
                    self.final_tail = true;
                    // librespot ya llamó sink.stop(): deshacer esa pausa para drenar A completa.
                    if self.voice.is_none() {
                        self.transport(true);
                    }
                }
                PlayerEvent::SessionDisconnected { .. } | PlayerEvent::Stopped { .. } => {
                    self.cancel_timer();
                    self.discontinuity();
                    self.transport(false);
                }
                _ => {}
            }
        }
    }
    /// Fundido corto y fin; en pausa (salida quizá cerrada) termina al instante.
    fn stop_voice(&mut self) {
        let Some(voice) = self.voice.as_mut() else {
            return;
        };
        self.release_after_voice = true;
        if voice.paused || voice.idle {
            if let Some(voice) = self.voice.take() {
                voice.finish();
            }
            if !self.playing {
                self.notified = false;
            }
            return;
        }
        if !voice.stopping {
            voice.stopping = true;
            voice.ramp.to(0.0, VOICE_STOP_FRAMES);
            voice.report();
        }
    }
    /// Un frame de voz en `raw_frame`/`frame`; false si no hay voz (o acaba de terminar).
    fn voice_frame(&mut self, volume: f32, output: &std_mpsc::Sender<OutputCommand>) -> bool {
        let playing = self.playing;
        let generation = self.pause_generation;
        let Some(voice) = self.voice.as_mut() else {
            return false;
        };
        let gain = voice.ramp.step();
        if voice.frame >= voice.clip.frames() || (voice.stopping && gain == 0.0) {
            if let Some(voice) = self.voice.take() {
                voice.finish();
            }
            if self.release_after_voice && !playing {
                // Nadie sigue tras un corte: la ruta de pausa avisa al worker en este frame.
                self.notified = false;
            }
            return false;
        }
        if voice.paused && gain == 0.0 {
            if !voice.idle {
                voice.idle = true;
                voice.report();
                if !playing {
                    let _ = output.send(OutputCommand::Faded(generation));
                }
            }
            self.raw_frame = [0.0; 2];
            self.frame = [0.0; 2];
            return true;
        }
        self.raw_frame = voice.clip.frame(voice.frame);
        voice.frame += 1;
        // El detector de bloqueo de write() mide consumo: la voz también avanza la salida.
        self.consumed = self.consumed.wrapping_add(2);
        self.frame = self
            .eq
            .process(self.raw_frame, SAMPLE_RATE)
            .map(|sample| sample * volume * gain);
        true
    }
    fn cancel_timer(&mut self) {
        self.timer_uri = None;
        self.timer_dj = false;
        self.end_remaining = None;
        self.timer_state.send_replace(TrackEndTimerState::Cancelled);
    }
    fn anchor_decode(&mut self, position_ms: u32) {
        self.decode_anchor_ms = u64::from(position_ms);
        self.decoded_samples = 0;
    }
    /// Segundos de mezcla si ahora se puede: activada, sin DJ, sin «al terminar la canción»
    /// y sin voz (la voz de Livi nunca se mezcla con la música).
    fn mix_seconds(&self) -> Option<u32> {
        self.mix
            .allowed()
            .filter(|_| self.timer_uri.is_none() && self.voice.is_none())
    }
    /// TrackChanged(B) tras el fin natural de A, antes de cualquier muestra de B: mezclar en
    /// el deck B o concatenar (gapless de álbum, sin ventana, mezcla apagada).
    fn plan_mix(&mut self, previous: Option<&AudioItem>) {
        let (Some(a), Some(b)) = (previous, self.item.as_deref()) else {
            return;
        };
        let Some(seconds) = self.mix_seconds() else {
            return;
        };
        if mix::same_album_gapless(a, b) || self.queue.len() / 2 < MIX_MIN_TAIL_FRAMES {
            return;
        }
        let shortest = a.duration_ms.min(b.duration_ms);
        let window = mix::window_frames(seconds, u64::from(shortest));
        if window > 0 {
            self.route_b = true;
            self.mix_window = window;
        }
    }
    /// Cierra cualquier mezcla (seek, otra carga, parada): ambos decks quedan vacíos.
    fn end_mix(&mut self) {
        self.fade = None;
        self.limiter_weight = 0.0;
        self.route_b = false;
        self.mix_window = 0;
        self.ahead_ms = 0;
        self.deck_b.clear();
        self.spans_b.clear();
        self.ledger_active_b = None;
        self.publish_mix();
    }
    /// A se vació: B pasa a ser la cola principal (intercambio O(1), sin reservar).
    fn promote(&mut self) {
        std::mem::swap(&mut self.queue, &mut self.deck_b);
        std::mem::swap(&mut self.spans, &mut self.spans_b);
        self.ledger_active = self.ledger_active_b.take();
        self.deck_b.clear();
        self.spans_b.clear();
        self.fade = None;
        self.route_b = false;
        self.mix_window = 0;
        self.ahead_ms = 0;
        self.publish_mix();
    }
    fn song_available(&self) -> bool {
        self.queue.len() >= 2 || (self.route_b && self.deck_b.len() >= 2)
    }
    /// Un frame de canción: A sola, o A·gA + B·gB con limitador durante el solape. O(1).
    fn deck_frame(&mut self) -> Option<[f32; 2]> {
        if self.route_b {
            let a_frames = self.queue.len() / 2;
            if self.fade.is_none()
                && a_frames > 0
                && a_frames <= self.mix_window
                && self.deck_b.len() / 2 >= mix::MIX_B_CUSHION_FRAMES
            {
                // Sin B a tiempo (con colchón), el solape es lo que quede de A cuando B llegue.
                self.fade = Some(Fade {
                    elapsed: 0,
                    frames: a_frames,
                    t0: 0.0,
                });
                self.publish_mix();
            }
            if a_frames == 0 {
                self.promote();
            }
        } else if self.queue.len() < 2 {
            // A terminó sin pista siguiente: nada corre por delante.
            self.after_natural = false;
            if self.ahead_ms != 0 {
                self.ahead_ms = 0;
                self.publish_mix();
            }
        }
        let a = pop_frame(&mut self.queue)?;
        self.end_remaining = self.end_remaining.map(|n| n.saturating_sub(2));
        if let Some(request) = count_frame(
            &mut self.spans,
            &mut self.ledger,
            &mut self.ledger_next,
            &mut self.ledger_active,
        ) {
            self.audible(request);
        }
        let Some(mut fade) = self.fade else {
            return Some(self.soften(a, false));
        };
        let mut stalled = false;
        let (ga, gb);
        let b = match pop_frame(&mut self.deck_b) {
            Some(b) => {
                (ga, gb) = fade.gains_at(fade.elapsed);
                fade.elapsed += 1;
                if let Some(request) = count_frame(
                    &mut self.spans_b,
                    &mut self.ledger,
                    &mut self.ledger_next,
                    &mut self.ledger_active_b,
                ) {
                    self.audible(request);
                }
                b
            }
            None => {
                // B se atrasó (red): el fundido se congela y A conserva la ganancia que tenía
                // (nunca silencio por falta de B); al volver B continúa sin salto.
                stalled = true;
                (ga, gb) = fade.held_gains();
                fade.rebase(self.queue.len() / 2);
                [0.0; 2]
            }
        };
        self.fade = Some(fade);
        let tick = if stalled {
            self.queue.len() / 2
        } else {
            fade.elapsed
        };
        if tick % SAMPLE_RATE as usize == 0 {
            self.publish_mix(); // ~1 Hz, solo durante el solape.
        }
        Some(self.soften([a[0] * ga + b[0] * gb, a[1] * ga + b[1] * gb], true))
    }
    /// Limitador continuo: su peso sube/baja con una rampa de pocos ms al entrar y salir del
    /// solape (incluida la cola de B ya sola), y con peso 0 la señal pasa idéntica.
    fn soften(&mut self, frame: [f32; 2], fading: bool) -> [f32; 2] {
        let step = 1.0 / mix::LIMITER_RAMP_FRAMES as f32;
        self.limiter_weight = if fading {
            (self.limiter_weight + step).min(1.0)
        } else {
            (self.limiter_weight - step).max(0.0)
        };
        let w = self.limiter_weight;
        [mix::limit_blend(frame[0], w), mix::limit_blend(frame[1], w)]
    }
    /// La pista siguiente a un fin natural empieza a oírse: su posición empieza aquí.
    fn audible(&mut self, request: u64) {
        self.started = Some((request, Instant::now()));
        self.ahead_ms = 0;
        self.publish_mix();
    }
    /// Sin reservar memoria: `watch` solo notifica si algo visible cambió.
    fn publish_mix(&mut self) {
        let next = MixStatus {
            active: self.fade.is_some(),
            remaining_ms: self.fade.map_or(0, |f| f.remaining_ms()),
            ahead_ms: self.ahead_ms,
            started: self.started,
        };
        self.mix_status.send_if_modified(|status| {
            let changed = status.active != next.active
                || status.ahead_ms != next.ahead_ms
                || status.started != next.started
                || status.remaining_ms.abs_diff(next.remaining_ms) >= 1000;
            if changed {
                *status = next;
            }
            changed
        });
    }
    /// Muestras extra que la cola principal admite: abre la ventana cuando al decoder le
    /// quedan ≤ N + margen de A, para retener los últimos N s y oír B encima.
    fn queue_extra(&mut self) -> usize {
        if self.route_b {
            // Mientras B espera su turno, su deck admite el colchón mínimo para iniciar el
            // fundido (si no, un buffer base corto lo dejaría sin empezar nunca).
            return if self.fade.is_none() {
                mix::MIX_B_CUSHION_FRAMES * NUM_CHANNELS as usize
            } else {
                0
            };
        }
        if !self.lookahead {
            let Some(seconds) = self.mix_seconds() else {
                return 0;
            };
            let window_ms = mix::window_frames(seconds, self.decode_duration_ms) as u64 * 1000
                / u64::from(SAMPLE_RATE);
            let decoded_ms = self.decode_anchor_ms
                + self.decoded_samples / NUM_CHANNELS as u64 * 1000 / u64::from(SAMPLE_RATE);
            if window_ms == 0
                || self.decode_duration_ms.saturating_sub(decoded_ms) > window_ms + MIX_MARGIN_MS
            {
                return 0;
            }
            self.lookahead = true;
            self.ahead_ms = window_ms + MIX_MARGIN_MS;
            self.publish_mix();
        }
        ((self.ahead_ms + MIX_SLACK_MS) * u64::from(SAMPLE_RATE) * NUM_CHANNELS as u64 / 1000)
            as usize
    }
    /// Capacidad para la ventana; `Some(n)` pide reservar fuera del lock (hilo del decoder).
    fn reserve_window(&mut self, slack: usize) -> Option<usize> {
        let extra = self.queue_extra();
        if extra == 0 {
            return None;
        }
        let needed = extra + slack;
        if self.queue.capacity() >= needed {
            return None;
        }
        if !self.route_b && self.deck_b.is_empty() && self.deck_b.capacity() >= needed {
            // Reutiliza el búfer grande que dejó la mezcla anterior (copia ~100 ms).
            self.deck_b.extend(self.queue.drain(..));
            std::mem::swap(&mut self.queue, &mut self.deck_b);
            return None;
        }
        Some(needed)
    }
    /// Adopta un búfer ya reservado; devuelve el viejo para soltarlo fuera del lock.
    fn adopt(&mut self, mut spare: VecDeque<f32>) -> VecDeque<f32> {
        if !self.route_b && spare.capacity() > self.queue.capacity() {
            spare.extend(self.queue.drain(..));
            std::mem::swap(&mut self.queue, &mut spare);
        }
        spare
    }
    /// Con la mezcla apagada no se retiene el búfer del deck B (~4 MB con 12 s).
    fn take_spare(&mut self) -> Option<VecDeque<f32>> {
        (self.mix.allowed().is_none()
            && !self.route_b
            && self.deck_b.is_empty()
            && self.deck_b.capacity() > 0)
            .then(|| std::mem::take(&mut self.deck_b))
    }
    /// PCM del decoder a su deck, con el tramo por request id (progreso de escucha).
    fn push_decoded(&mut self, samples: &[f32]) {
        let (deck, spans) = if self.route_b {
            (&mut self.deck_b, &mut self.spans_b)
        } else {
            (&mut self.queue, &mut self.spans)
        };
        deck.extend(samples);
        self.decoded_samples += samples.len() as u64;
        let Some(request_id) = self.decoding_request else {
            return;
        };
        if let Some(span) = spans
            .back_mut()
            .filter(|span| span.request_id == request_id)
        {
            span.samples += samples.len();
        } else {
            let natural = self.natural_request == Some(request_id);
            if natural {
                self.natural_request = None;
            }
            spans.push_back(PcmSpan {
                request_id,
                samples: samples.len(),
                natural,
            });
        }
    }
    fn next(&mut self, output: &std_mpsc::Sender<OutputCommand>) -> f32 {
        if self.channel == 0 {
            self.events();
            let target = self
                .volume_override
                .gain(self.soft_volume.attenuation_factor() as f32);
            if target != self.volume.target {
                #[cfg(debug_assertions)]
                tracing::debug!(
                    target_gain = target,
                    requested_volume = ?self.volume_override.requested_volume(),
                    request_to_pcm_us = ?self.volume_override.elapsed_us(),
                    "volume: PCM gain target applied (5 ms ramp starts)"
                );
            }
            self.volume.to(target, VOLUME_FRAMES);
            let volume = self.volume.step();
            if !self.voice_frame(volume, output) && !self.song_frame(volume, output) {
                return 0.0; // No consumir PCM en pausa; el worker suelta el dispositivo.
            }
        }
        self.emit()
    }
    /// El frame de la canción; false en pausa total (sin consumir PCM).
    fn song_frame(&mut self, volume: f32, output: &std_mpsc::Sender<OutputCommand>) -> bool {
        {
            if self.final_tail && !self.song_available() {
                self.final_tail = false;
                self.transport(false);
            }
            if let Some(remaining) = self.end_remaining {
                if remaining <= PAUSE_FRAMES * 2 && self.playing {
                    self.transport(false);
                    self.transport.frames = (remaining / 2).max(1);
                }
            }
            // Un fade-in espera PCM real: si el decoder carga/busca, no gastar la rampa en silencio.
            let available = if self.micro.target == 0.0 {
                self.old.len() >= 2
            } else {
                self.song_available()
            };
            let transport = if self.transport.target > self.transport.value && !available {
                self.transport.value
            } else {
                self.transport.step()
            };
            if !self.playing && transport == 0.0 {
                if let Some(remaining) = self.end_remaining.take() {
                    // Consumir el endpoint silencioso de A; B permanece íntegra para reanudar.
                    let count = remaining.min(self.queue.len());
                    self.queue.drain(..count);
                    self.discard_spans(count);
                    self.consumed = self.consumed.wrapping_add(count as u64);
                    self.timer_uri = None;
                    let _ = self.pause_request.send(None);
                    self.timer_state.send_replace(TrackEndTimerState::Finished);
                }
                if !self.notified {
                    let _ = output.send(OutputCommand::Faded(self.pause_generation));
                    self.notified = true;
                }
                return false;
            }
            if self.micro.target == 0.0 && self.micro.value == 0.0 {
                self.old.clear();
                self.micro.to(1.0, MICRO_FRAMES);
            }
            let micro = if self.micro.target == 1.0 && !self.song_available() {
                self.micro.value
            } else {
                self.micro.step()
            };
            let frame = if self.micro.target == 0.0 {
                pop_frame(&mut self.old)
            } else {
                self.deck_frame()
            };
            if let Some(frame) = frame {
                // El visualizador recibe la mezcla tal cual suena, antes del volumen.
                self.raw_frame = frame;
                self.consumed = self.consumed.wrapping_add(2);
            } else {
                self.raw_frame = [0.0; 2];
            }
            self.frame = self
                .eq
                .process(self.raw_frame, SAMPLE_RATE)
                .map(|sample| sample * volume * transport * micro);
        }
        true
    }
    fn discard_spans(&mut self, mut count: usize) {
        while count > 0 {
            let Some(span) = self.spans.front_mut() else {
                break;
            };
            let n = span.samples.min(count);
            span.samples -= n;
            count -= n;
            if span.samples == 0 {
                self.spans.pop_front();
            }
        }
    }
    /// Only called by decoder/output worker, never PlayedSource::next. Broadcasting may
    /// allocate; the callback only increments the fixed-size ledger under its existing lock.
    fn publish_progress(&mut self, force: bool) {
        let Some(sender) = &self.progress_tx else {
            return;
        };
        for (index, entry) in self.ledger.iter_mut().enumerate() {
            if entry.valid
                && entry.progress.consumed_frames != entry.published_frames
                && (force
                    || (Some(index) != self.ledger_active && Some(index) != self.ledger_active_b)
                    || entry.progress.consumed_frames - entry.published_frames
                        >= u64::from(SAMPLE_RATE) / 4)
            {
                let _ = sender.send(entry.progress);
                entry.published_frames = entry.progress.consumed_frames;
            }
        }
    }
    /// Tap del visualizador (antes del volumen y los fades) y muestra del canal actual.
    fn emit(&mut self) -> f32 {
        let activation = self.tap.enabled.state();
        if activation != self.tap_state {
            self.pending.clear();
            self.tap_state = activation;
        }
        if activation & 1 != 0 && !(self.pending.is_empty() && self.channel != 0) {
            if self.pending.is_empty() {
                self.captured_at = Instant::now();
            }
            self.pending.push(f64::from(self.raw_frame[self.channel]));
            if self.pending.len() == TAP_SAMPLES {
                let samples = std::mem::replace(&mut self.pending, Vec::with_capacity(TAP_SAMPLES));
                if let Ok(permit) = self.tap.sender.try_reserve() {
                    permit.send(PcmPacket {
                        captured_at: self.captured_at,
                        samples,
                        sample_rate: SAMPLE_RATE,
                        channels: NUM_CHANNELS as u16,
                    });
                }
            }
        }
        let sample = self.frame[self.channel];
        self.channel = (self.channel + 1) % 2;
        sample
    }
}
fn pop_frame(deck: &mut VecDeque<f32>) -> Option<[f32; 2]> {
    (deck.len() >= 2).then(|| {
        [
            deck.pop_front().unwrap_or(0.0),
            deck.pop_front().unwrap_or(0.0),
        ]
    })
}
/// Cuenta un frame oído para el request id de su tramo. Devuelve ese id si es el primer frame
/// de una pista que siguió a un fin natural (empieza a oírse).
fn count_frame(
    spans: &mut VecDeque<PcmSpan>,
    ledger: &mut [ProgressEntry; 32],
    ledger_next: &mut usize,
    active: &mut Option<usize>,
) -> Option<u64> {
    let span = spans.front_mut()?;
    let request_id = span.request_id;
    let started = std::mem::take(&mut span.natural).then_some(request_id);
    span.samples = span.samples.saturating_sub(2);
    if span.samples == 0 {
        spans.pop_front();
    }
    let index = active
        .filter(|i| ledger[*i].progress.play_request_id == request_id)
        .or_else(|| {
            ledger
                .iter()
                .position(|e| e.valid && e.progress.play_request_id == request_id)
        })
        .unwrap_or_else(|| {
            let index = *ledger_next;
            *ledger_next = (index + 1) % ledger.len();
            ledger[index] = ProgressEntry {
                valid: true,
                progress: PlaybackProgress {
                    play_request_id: request_id,
                    ..Default::default()
                },
                published_frames: 0,
            };
            index
        });
    *active = Some(index);
    let progress = &mut ledger[index].progress;
    progress.consumed_frames += 1;
    progress.played_ms = progress.consumed_frames * 1000 / u64::from(SAMPLE_RATE);
    started
}
/// Estado del temporizador local, impulsado por EOF/PCM; no usa reloj de pared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackEndTimerState {
    Armed,
    Finished,
    Cancelled,
}
#[derive(Clone)]
pub(crate) struct OutputControl(Arc<Mutex<Playback>>);
impl OutputControl {
    pub fn new(
        tap: PcmTap,
        volume: Box<dyn VolumeGetter + Send>,
        pause_request: mpsc::UnboundedSender<Option<std_mpsc::Sender<()>>>,
    ) -> Self {
        Self(Arc::new(Mutex::new(Playback::new(
            tap,
            volume,
            pause_request,
        ))))
    }
    pub fn attach_events(&self, events: PlayerEventChannel) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).events = Some(events);
    }
    pub fn configure(
        &self,
        eq: EqControl,
        volume: VolumeOverride,
        progress: broadcast::Sender<PlaybackProgress>,
        mix: MixControl,
    ) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.eq = Equalizer::new(eq, SAMPLE_RATE);
        state.volume_override = volume;
        state.progress_tx = Some(progress);
        state.mix = mix;
    }
    /// Estado del fundido, publicado sin sondeo.
    pub fn mix_status(&self) -> watch::Receiver<MixStatus> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mix_status
            .subscribe()
    }
    pub fn arm(&self, uri: String) -> watch::Receiver<TrackEndTimerState> {
        self.arm_for(uri, false)
    }
    pub fn arm_dj(&self, uri: String) -> watch::Receiver<TrackEndTimerState> {
        self.arm_for(uri, true)
    }
    fn arm_for(&self, uri: String, dj: bool) -> watch::Receiver<TrackEndTimerState> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.events();
        state.cancel_timer();
        // Canal nuevo por activación: cancelar/rearmar no puede despertar un temporizador antiguo.
        state.timer_state = watch::channel(TrackEndTimerState::Armed).0;
        state.timer_uri = Some(uri);
        state.timer_dj = dj;
        state.timer_state.subscribe()
    }
    pub fn cancel_dj_timer(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.timer_dj {
            let finishing = state.end_remaining.is_some();
            state.cancel_timer();
            if finishing {
                state.transport(true);
            }
        }
    }
    pub fn resume(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.end_remaining.is_some() || *state.timer_state.borrow() != TrackEndTimerState::Armed
        {
            state.cancel_timer();
        }
    }
    pub fn sync_events(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).events();
    }
    pub fn cancel(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let finishing = state.end_remaining.is_some();
        state.cancel_timer();
        if finishing {
            state.transport(true);
        }
    }

    /// E5b: hace sonar `clip` por la salida de las canciones (volumen, tap del visualizador)
    /// mientras la canción espera en pausa. Abre el dispositivo si estaba suelto. `None` si el
    /// sink todavía no existe.
    pub fn play_voice(&self, clip: Arc<VoiceClip>) -> Option<watch::Receiver<VoiceProgress>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let worker = state.worker.clone()?;
        if let Some(old) = state.voice.take() {
            old.finish();
        }
        state.release_after_voice = false;
        let (progress, receiver) = watch::channel(VoiceProgress {
            position_ms: 0,
            at: Instant::now(),
            playing: true,
            finished: false,
        });
        let mut ramp = Ramp::new(0.0);
        ramp.to(1.0, MICRO_FRAMES);
        state.voice = Some(Voice {
            clip,
            frame: 0,
            paused: false,
            stopping: false,
            idle: false,
            ramp,
            progress,
        });
        drop(state);
        if worker.send(OutputCommand::Open).is_err() {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(voice) = state.voice.take() {
                voice.finish();
            }
            return None;
        }
        Some(receiver)
    }
    /// Hay un «al terminar esta canción» esperando su límite.
    pub fn timer_armed(&self) -> bool {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *state.timer_state.borrow() == TrackEndTimerState::Armed
    }
    /// Hay una voz que no está terminando (play/pausa/seek/siguiente le corresponden).
    pub fn voice_active(&self) -> bool {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.voice.as_ref().is_some_and(|v| !v.stopping)
    }
    pub fn pause_voice(&self, paused: bool) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let worker = state.worker.clone();
        let Some(voice) = state.voice.as_mut().filter(|v| !v.stopping) else {
            return;
        };
        if voice.paused == paused {
            return;
        }
        voice.paused = paused;
        voice.idle = false;
        if paused {
            voice.ramp.to(0.0, PAUSE_FRAMES);
        } else {
            voice.ramp.to(1.0, RESUME_FRAMES);
        }
        voice.report();
        drop(state);
        if !paused {
            if let Some(worker) = worker {
                let _ = worker.send(OutputCommand::Open);
            }
        }
    }
    pub fn seek_voice(&self, position_ms: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(voice) = state.voice.as_mut().filter(|v| !v.stopping) {
            let frame = position_ms.saturating_mul(u64::from(SAMPLE_RATE)) / 1000;
            voice.frame = usize::try_from(frame)
                .unwrap_or(usize::MAX)
                .min(voice.clip.frames());
            voice.report();
        }
    }
    pub fn stop_voice(&self) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop_voice();
    }
    /// Sin voz ni canción sonando: suelta el dispositivo (CPU ~0) como tras un fade de pausa.
    pub fn release_if_idle(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.playing || state.voice.is_some() {
            return;
        }
        state.notified = true;
        let generation = state.pause_generation;
        if let Some(worker) = state.worker.clone() {
            drop(state);
            let _ = worker.send(OutputCommand::Faded(generation));
        }
    }
}
struct PlayedSource {
    shared: Arc<Mutex<Playback>>,
    output: std_mpsc::Sender<OutputCommand>,
}
impl Iterator for PlayedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        Some(
            self.shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .next(&self.output),
        )
    }
}
impl rodio::Source for PlayedSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        NUM_CHANNELS as rodio::ChannelCount
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        SAMPLE_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
enum OutputCommand {
    Start(bool, std_mpsc::Sender<SinkResult<()>>),
    /// E5b: abrir el dispositivo para la voz, sin reanudar la canción.
    Open,
    Faded(u64),
    Close,
    Quit,
}

/// Worker del dispositivo: espera eventos sin timer; stop nunca espera el fade.
pub(crate) struct TappedSink {
    shared: Arc<Mutex<Playback>>,
    output: std_mpsc::Sender<OutputCommand>,
    worker: Option<std::thread::JoinHandle<()>>,
    started: bool,
    switch: crate::output::OutputSwitch,
    applied: u64,
    faults: mpsc::UnboundedSender<()>,
}
impl TappedSink {
    pub fn new(
        control: OutputControl,
        faults: mpsc::UnboundedSender<()>,
        switch: crate::output::OutputSwitch,
    ) -> Self {
        let shared = control.0;
        let (output, commands) = std_mpsc::channel();
        shared.lock().unwrap_or_else(|e| e.into_inner()).worker = Some(output.clone());
        let worker_shared = shared.clone();
        let sender = output.clone();
        let output_faults = faults.clone();
        let worker_switch = switch.clone();
        let worker = std::thread::spawn(move || {
            let mut stream: Option<rodio::OutputStream> = None;
            while let Ok(command) = commands.recv() {
                let open = |stream: &mut Option<rodio::OutputStream>| {
                    if stream.is_some() {
                        return Ok(());
                    }
                    rodio::OutputStreamBuilder::from_default_device()
                        .and_then(|b| b.with_sample_rate(SAMPLE_RATE)
                            .with_buffer_size(rodio::cpal::BufferSize::Fixed(SAMPLE_RATE * worker_switch.buffer_ms() / 1000))
                            .open_stream_or_fallback())
                        .map(|mut opened| {
                            #[cfg(debug_assertions)]
                            tracing::debug!(requested_ms = worker_switch.buffer_ms(), actual_config = ?opened.config(), "output buffer opened (driver fallback may differ)");
                            opened.log_on_drop(false);
                            opened.mixer().add(PlayedSource {
                                shared: worker_shared.clone(),
                                output: sender.clone(),
                            });
                            *stream = Some(opened);
                        })
                        .map_err(|_| {
                            SinkError::NotConnected("Default audio output unavailable".into())
                        })
                };
                match command {
                    OutputCommand::Open => {
                        if open(&mut stream).is_err() {
                            let _ = output_faults.send(());
                            // Sin salida la voz nunca avanzaría: terminarla para seguir con música.
                            let mut state = worker_shared.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(voice) = state.voice.take() {
                                voice.finish();
                            }
                        }
                    }
                    OutputCommand::Start(reopen, reply) => {
                        if reopen {
                            stream = None;
                        }
                        let result = open(&mut stream);
                        if result.is_ok() {
                            let mut state = worker_shared.lock().unwrap_or_else(|e| e.into_inner());

                            state.events();
                            if *state.timer_state.borrow() == TrackEndTimerState::Finished {
                                state.cancel_timer();
                            }
                            state.transport(true);
                        }
                        let _ = reply.send(result);
                    }
                    OutputCommand::Faded(generation) => {
                        let mut state = worker_shared.lock().unwrap_or_else(|e| e.into_inner());
                        state.publish_progress(true);
                        let close = !state.playing
                            && state.pause_generation == generation
                            && state.voice.as_ref().is_none_or(|v| v.idle);
                        drop(state);
                        if close {
                            stream = None;
                        }
                    }
                    OutputCommand::Close => {
                        stream = None;
                    }
                    OutputCommand::Quit => break,
                }
            }
        });
        Self {
            shared,
            output,
            worker: Some(worker),
            started: false,
            applied: switch.generation(),
            switch,
            faults,
        }
    }
    fn fault<T>(&self, result: SinkResult<T>) -> SinkResult<T> {
        if result.is_err() {
            let _ = self.faults.send(());
        }
        result
    }
}
impl Drop for TappedSink {
    fn drop(&mut self) {
        let _ = self.output.send(OutputCommand::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Sink for TappedSink {
    fn start(&mut self) -> SinkResult<()> {
        let generation = self.switch.generation();
        let (reply, result) = std_mpsc::channel();
        self.output
            .send(OutputCommand::Start(generation != self.applied, reply))
            .map_err(|_| SinkError::NotConnected("Audio worker closed".into()))?;
        self.fault(
            result
                .recv()
                .unwrap_or_else(|_| Err(SinkError::NotConnected("Audio worker closed".into()))),
        )?;
        self.applied = generation;
        self.started = true;
        Ok(())
    }
    fn stop(&mut self) -> SinkResult<()> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .transport(false);
        self.started = false;
        Ok(())
    }
    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        if !self.started {
            return self.fault(Err(SinkError::NotConnected(
                "Audio output is closed".into(),
            )));
        }
        if self.switch.generation() != self.applied {
            self.start()?;
        }
        let AudioPacket::Samples(samples) = packet else {
            return Ok(());
        };
        let decoded = converter.f64_to_f32(&samples);
        let base_limit =
            SAMPLE_RATE as usize * NUM_CHANNELS as usize * self.switch.buffer_ms() as usize / 1000;
        // Mixear: la ampliación de la ventana se reserva aquí, en el hilo del decoder y
        // fuera del lock; mezclar/promover los decks no requiere ampliar sus búferes.
        let reserve = {
            let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            state.events();
            state.reserve_window(base_limit + decoded.len())
        };
        let spare = reserve.map(VecDeque::with_capacity);
        let released = {
            let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            state.events();
            state.publish_progress(false);
            let old = spare.map(|spare| state.adopt(spare));
            state.push_decoded(&decoded);
            state.publish_mix();
            (old, state.take_spare())
        };
        drop(released);
        let mut progress = Progress::new(Instant::now());
        loop {
            let (len, consumed, limit) = {
                let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
                state.publish_progress(false);
                state.publish_mix();
                let extra = state.queue_extra();
                let len = if state.route_b {
                    state.deck_b.len()
                } else {
                    state.queue.len()
                };
                (len, state.consumed, base_limit + extra)
            };
            let pause_request = {
                let state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
                (*state.timer_state.borrow() == TrackEndTimerState::Finished)
                    .then(|| state.pause_request.clone())
            };
            if let Some(pause_request) = pause_request {
                // El EOF puede haber llegado antes de cargar B. Encolar pausa antes de devolver
                // write garantiza que el poll del decoder la procese sin acumular PCM ilimitado.
                let (reply, received) = std_mpsc::channel();
                let _ = pause_request.send(Some(reply));
                if received.recv_timeout(STALL_LIMIT).is_err() {
                    return self.fault(Err(SinkError::NotConnected(
                        "Playback pause was not acknowledged".into(),
                    )));
                }
                return Ok(());
            }
            if len <= limit {
                return Ok(());
            }
            if progress.stalled(consumed, Instant::now()) {
                let _ = self.output.send(OutputCommand::Close);
                self.started = false;
                return self.fault(Err(SinkError::NotConnected(
                    "Audio output stopped consuming".into(),
                )));
            }
            std::thread::sleep(WAIT_STEP);
        }
    }
}
struct Progress {
    consumed: u64,
    last: Instant,
}
impl Progress {
    fn new(now: Instant) -> Self {
        Self {
            consumed: 0,
            last: now,
        }
    }
    fn stalled(&mut self, consumed: u64, now: Instant) -> bool {
        if consumed != self.consumed {
            self.consumed = consumed;
            self.last = now;
        }
        now.saturating_duration_since(self.last) > STALL_LIMIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_request_reaches_pcm_without_spirc_or_decoder_and_old_acks_do_not_win() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        state.volume = Ramp::new(0.);
        state.queue.extend(vec![0.5; VOLUME_FRAMES * 8]);
        state.volume_override.request(1000, 0.2);
        state.volume_override.request(2000, 0.8);
        state.volume_override.acknowledge(1000); // Spirc still handling the earlier drag.
        let out = frames(&mut state, &output, VOLUME_FRAMES + 1);
        assert!(out[0] > 0.); // Next callback frame starts the ramp, no network acknowledgement.
        assert_eq!(out[VOLUME_FRAMES * 2], 0.4);
        assert!(state.volume_override.pending());
        state.volume_override.acknowledge(2000);
        assert!(!state.volume_override.pending());
        // With no local request pending, the original mixer handles external Connect changes.
        assert_eq!(state.volume_override.gain(0.3), 0.3);
    }
    #[test]
    fn volume_drag_back_to_an_earlier_value_waits_for_its_final_ack() {
        let volume = VolumeOverride::default();
        volume.request(1000, 0.2);
        volume.request(2000, 0.8);
        volume.request(1000, 0.2);
        volume.acknowledge(1000); // First request has the same value as the final one.
        assert!(volume.pending());
        volume.acknowledge(2000);
        assert!(volume.pending());
        volume.acknowledge(1000);
        assert!(!volume.pending());
    }
    #[test]
    fn volume_ack_skipping_intermediate_values_clears_override() {
        let volume = VolumeOverride::default();
        volume.request(1000, 0.2);
        volume.request(2000, 0.4);
        volume.request(3000, 0.6);
        volume.acknowledge(3000); // Spirc omitted 1000 and 2000.
        assert!(!volume.pending());
    }
    #[test]
    fn volume_override_expires_without_ack() {
        let Some(origin) = Instant::now().checked_sub(std::time::Duration::from_secs(60)) else {
            return;
        };
        let volume = VolumeOverride(Arc::new(VolumeRequest {
            packed: AtomicU64::new(0),
            acknowledgements: Mutex::new(VecDeque::new()),
            origin,
            requested_us: AtomicU64::new(0),
        }));
        let now_us = origin.elapsed().as_micros() as u64;
        volume.request(1000, 0.2); // Stamped "now" (60 s after origin).
        assert!(volume.pending());
        volume.0.requested_us.store(
            now_us - VOLUME_OVERRIDE_TTL_US - 1_000_000,
            Ordering::Release,
        );
        assert!(!volume.pending());
        assert_eq!(volume.gain(0.7), 0.7);
    }
    fn observed_pcm(state: &mut Playback, request_id: u64, frame_count: usize) {
        state
            .queue
            .extend(std::iter::repeat_n(0.1, frame_count * 2));
        state.spans.push_back(PcmSpan {
            request_id,
            samples: frame_count * 2,
            natural: false,
        });
    }
    fn heard(state: &Playback, request_id: u64) -> u64 {
        state
            .ledger
            .iter()
            .find(|e| e.valid && e.progress.play_request_id == request_id)
            .map_or(0, |e| e.progress.consumed_frames)
    }
    #[test]
    fn listening_counts_actual_pcm_by_occurrence_even_when_decoder_has_advanced() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        observed_pcm(&mut state, 1, 100);
        observed_pcm(&mut state, 2, 100); // Same URI may repeat; request identities stay separate.
        state.decoding_request = Some(2);
        frames(&mut state, &output, 125);
        assert_eq!(heard(&state, 1), 100);
        assert_eq!(heard(&state, 2), 25);
        frames(&mut state, &output, 1000); // Buffer underrun does not count synthetic silence.
        assert_eq!(heard(&state, 2), 100);
        observed_pcm(&mut state, 2, 20000);
        state.transport(false);
        frames(&mut state, &output, PAUSE_FRAMES + 1);
        let paused = heard(&state, 2);
        frames(&mut state, &output, 1000);
        assert_eq!(heard(&state, 2), paused);
        state.discontinuity(); // Seek while paused discards buffered samples without listening.
        assert_eq!(heard(&state, 2), paused);
        observed_pcm(&mut state, 2, 100);
        state.transport(true);
        frames(&mut state, &output, 100);
        assert_eq!(heard(&state, 2), paused + 100);
    }
    #[test]
    fn listening_excludes_dj_voice_and_seek_tail_and_flushes_without_a_timer() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        let (tx, mut rx) = broadcast::channel(32);
        state.progress_tx = Some(tx);
        observed_pcm(&mut state, 9, 2000);
        frames(&mut state, &output, 100);
        state.discontinuity();
        observed_pcm(&mut state, 9, 1000);
        frames(&mut state, &output, MICRO_FRAMES + 100);
        assert_eq!(heard(&state, 9), 200);
        state.voice = Some(Voice {
            clip: Arc::new(VoiceClip::new(vec![0.2; 400], 2).unwrap()),
            frame: 0,
            paused: false,
            stopping: false,
            idle: false,
            ramp: Ramp::new(1.),
            progress: watch::channel(VoiceProgress {
                position_ms: 0,
                at: Instant::now(),
                playing: true,
                finished: false,
            })
            .0,
        });
        frames(&mut state, &output, 100);
        assert_eq!(heard(&state, 9), 200);
        state.publish_progress(false);
        assert!(rx.try_recv().is_err()); // Active occurrence publishes at most 4 Hz.
        state.publish_progress(true); // Worker flush on fade/EOF; no callback allocation.
        let report = rx.try_recv().unwrap();
        assert_eq!(report.consumed_frames, 200);
        assert_eq!(report.played_ms, 200 * 1000 / u64::from(SAMPLE_RATE));
        state.publish_progress(true);
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn eq_is_after_raw_visualizer_tap_and_before_volume_and_fades() {
        let (mut state, mut pcm, output) = playback(true);
        settled(&mut state);
        let eq = EqControl::default();
        eq.set(crate::EqConfig {
            enabled: true,
            preamp_db: -6.,
            ..Default::default()
        });
        state.eq = Equalizer::new(eq, SAMPLE_RATE);
        state.volume_override.request(1, 0.5);
        state.queue.extend(vec![0.5; TAP_SAMPLES * 2]);
        let out = frames(&mut state, &output, TAP_SAMPLES / 2);
        assert!((out[out.len() - 1] - 0.5 * 0.5 * 10_f32.powf(-6. / 20.)).abs() < 1e-6);
        assert!(pcm.try_recv().unwrap().samples.iter().all(|v| *v == 0.5));
    }
    use librespot_playback::mixer::{Mixer, MixerConfig, softmixer::SoftMixer};
    fn voice_output(
        enabled: bool,
    ) -> (
        OutputControl,
        mpsc::Receiver<PcmPacket>,
        std_mpsc::Receiver<OutputCommand>,
    ) {
        let (mut state, pcm, _) = playback(enabled);
        let (worker, commands) = std_mpsc::channel();
        state.worker = Some(worker);
        (OutputControl(Arc::new(Mutex::new(state))), pcm, commands)
    }

    #[test]
    fn voice_clip_validates_frames_channels_and_finite_samples() {
        for (samples, channels) in [
            (vec![], 1),
            (vec![0.5], 0),
            (vec![0.5], 2),
            (vec![0.5; 3], 2),
            (vec![f32::NAN], 1),
            (vec![f32::INFINITY], 1),
        ] {
            assert!(VoiceClip::new(samples, channels).is_none());
        }
        let clip = VoiceClip::new(vec![2.0, -2.0], 2).unwrap();
        assert_eq!(clip.frame(0), [1.0, -1.0]);
        assert_eq!(clip.frame(1), [0.0; 2]);
    }

    #[test]
    fn voice_has_priority_preserves_song_pcm_and_tap_precedes_volume_and_fades() {
        for channels in [1, 2] {
            let (control, mut pcm, commands) = voice_output(true);
            let clip =
                Arc::new(VoiceClip::new(vec![0.5; TAP_SAMPLES * channels], channels).unwrap());
            let progress = control.play_voice(clip).unwrap();
            assert!(matches!(commands.try_recv(), Ok(OutputCommand::Open)));
            let worker = control.0.lock().unwrap().worker.clone().unwrap();
            {
                let mut state = control.0.lock().unwrap();
                state.queue.extend([0.75, -0.75]);
                state.volume = Ramp::new(0.0);
                let out = frames(&mut state, &worker, TAP_SAMPLES);
                assert!(out[0] < 0.001 && out[MICRO_FRAMES * 2] > 0.49);
                assert!(out.chunks_exact(2).all(|f| f[0] == f[1]));
                assert_eq!(
                    state.queue.iter().copied().collect::<Vec<_>>(),
                    [0.75, -0.75]
                );
                frames(&mut state, &worker, 1);
            }
            assert!(pcm.try_recv().unwrap().samples.iter().all(|s| *s == 0.5));
            assert!(progress.borrow().finished);
            assert!(!control.voice_active());
        }
    }

    #[test]
    fn paused_voice_fades_releases_output_and_resumes_without_losing_frames() {
        let (control, _, commands) = voice_output(false);
        let progress = control
            .play_voice(Arc::new(
                VoiceClip::new(vec![0.5; SAMPLE_RATE as usize], 1).unwrap(),
            ))
            .unwrap();
        let worker = control.0.lock().unwrap().worker.clone().unwrap();
        commands.try_recv().unwrap();
        frames(&mut control.0.lock().unwrap(), &worker, MICRO_FRAMES);
        control.pause_voice(true);
        frames(&mut control.0.lock().unwrap(), &worker, PAUSE_FRAMES + 1);
        assert!(matches!(commands.try_recv(), Ok(OutputCommand::Faded(_))));
        let paused = *progress.borrow();
        assert!(!paused.playing && !paused.finished && paused.position_ms >= 120);
        let frame = control.0.lock().unwrap().voice.as_ref().unwrap().frame;
        let silent = frames(&mut control.0.lock().unwrap(), &worker, 500);
        assert!(silent.iter().all(|s| *s == 0.0));
        assert_eq!(
            control.0.lock().unwrap().voice.as_ref().unwrap().frame,
            frame
        );
        control.pause_voice(false);
        assert!(matches!(commands.try_recv(), Ok(OutputCommand::Open)));
        assert!(progress.borrow().playing);
        frames(&mut control.0.lock().unwrap(), &worker, 1);
        assert_eq!(
            control.0.lock().unwrap().voice.as_ref().unwrap().frame,
            frame + 1
        );
        control.seek_voice(200);
        assert_eq!(progress.borrow().position_ms, 200);
        control.stop_voice();
        let out = frames(
            &mut control.0.lock().unwrap(),
            &worker,
            VOICE_STOP_FRAMES + 1,
        );
        assert_eq!(*out.last().unwrap(), 0.0);
        assert!(progress.borrow().finished);
    }

    #[test]
    fn stopping_paused_voice_finishes_without_a_callback_and_dead_worker_is_rejected() {
        let (control, _, commands) = voice_output(false);
        let clip = Arc::new(VoiceClip::new(vec![0.5; 4410], 1).unwrap());
        let progress = control.play_voice(clip.clone()).unwrap();
        control.pause_voice(true);
        control.stop_voice();
        assert!(progress.borrow().finished);
        drop(commands);
        assert!(control.play_voice(clip).is_none());
        assert!(!control.voice_active());
    }

    #[test]
    fn stopping_dj_cancels_its_boundary_but_keeps_the_users_sleep_timer() {
        let (control, _, _) = voice_output(false);
        let dj = control.arm_dj("spotify:track:4uLU6hMCjMI75M1A2tKUQC".into());
        control.cancel_dj_timer();
        assert_eq!(*dj.borrow(), TrackEndTimerState::Cancelled);
        let user = control.arm("spotify:track:4uLU6hMCjMI75M1A2tKUQC".into());
        control.cancel_dj_timer();
        assert_eq!(*user.borrow(), TrackEndTimerState::Armed);
        let dj = control.arm_dj("spotify:track:4uLU6hMCjMI75M1A2tKUQC".into());
        let user = control.arm("spotify:track:4uLU6hMCjMI75M1A2tKUQC".into());
        assert_eq!(*dj.borrow(), TrackEndTimerState::Cancelled);
        control.cancel_dj_timer();
        assert_eq!(*user.borrow(), TrackEndTimerState::Armed);
    }
    fn playback(
        enabled: bool,
    ) -> (
        Playback,
        mpsc::Receiver<PcmPacket>,
        std_mpsc::Sender<OutputCommand>,
    ) {
        let (sender, receiver) = mpsc::channel(2);
        let activation = TapActivation::default();
        activation.set(enabled);
        let state = Playback::new(
            PcmTap {
                enabled: activation,
                sender,
            },
            Box::new(librespot_playback::mixer::NoOpVolume),
            mpsc::unbounded_channel().0,
        );
        let (output, _) = std_mpsc::channel();
        (state, receiver, output)
    }
    fn settled(state: &mut Playback) {
        state.playing = true;
        state.transport = Ramp::new(1.0);
    }
    fn frames(
        state: &mut Playback,
        output: &std_mpsc::Sender<OutputCommand>,
        count: usize,
    ) -> Vec<f32> {
        (0..count * 2).map(|_| state.next(output)).collect()
    }
    #[test]
    fn v1_real_five_ms_at_low_volume_mute_and_retarget_preserves_stereo() {
        for percent in [20, 40, 50, 100] {
            let mixer = SoftMixer::open(MixerConfig::default()).unwrap();
            mixer.set_volume(crate::percent_to_volume(percent));
            let gain = mixer.get_soft_volume().attenuation_factor() as f32;
            let mut ramp = Ramp::new(gain);
            ramp.to(0.0, VOLUME_FRAMES);
            for frame in 1..=VOLUME_FRAMES {
                let value = ramp.step();
                if frame < VOLUME_FRAMES {
                    assert!(value > 0.0, "{percent}% ended at {frame}");
                } else {
                    assert_eq!(value, 0.0);
                }
            }
            ramp.to(gain, VOLUME_FRAMES);
            for _ in 0..VOLUME_FRAMES / 2 {
                ramp.step();
            }
            let interpolated = ramp.value;
            ramp.to(gain * 0.25, VOLUME_FRAMES);
            assert_eq!(ramp.from, interpolated);
            assert!((ramp.step() - interpolated).abs() < gain / 1000.0);
            for _ in 1..VOLUME_FRAMES {
                ramp.step();
            }
            assert_eq!(ramp.value, gain * 0.25);
        }
    }
    #[test]
    fn v2_output_reads_original_softmixer_without_any_decoder_write_and_tap_is_pre_gain() {
        let mixer = SoftMixer::open(MixerConfig::default()).unwrap();
        mixer.set_volume(crate::percent_to_volume(100));
        let (mut state, mut pcm, output) = playback(true);
        settled(&mut state);
        state.soft_volume = mixer.get_soft_volume();
        state.queue.extend(vec![0.5; TAP_SAMPLES * 3]);
        mixer.set_volume(0); // Decoder no escribe, pero la salida ve este cambio.
        let out = frames(&mut state, &output, TAP_SAMPLES / 2);
        assert!(out[0] > 0.49);
        assert_eq!(out[2 * VOLUME_FRAMES], 0.0);
        assert!(out.chunks_exact(2).all(|stereo| stereo[0] == stereo[1]));
        assert!(pcm.try_recv().unwrap().samples.iter().all(|v| *v == 0.5));
        state.transport(false);
        frames(&mut state, &output, TAP_SAMPLES / 2);
        assert!(pcm.try_recv().unwrap().samples.iter().all(|v| *v == 0.5));
    }
    #[test]
    fn fade_pause_consumes_only_what_sounds_resume_keeps_every_sample_and_can_reverse() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        state.queue.extend((1..=40_000).map(|v| v as f32));
        frames(&mut state, &output, 17);
        state.transport(false);
        let first = state.next(&output);
        assert!(first > 34.9);
        let right = state.next(&output);
        assert!(right > first);
        frames(&mut state, &output, PAUSE_FRAMES);
        let consumed = state.consumed;
        let next = *state.queue.front().unwrap();
        assert_eq!(next, consumed as f32 + 1.0);
        frames(&mut state, &output, 500);
        assert_eq!(state.consumed, consumed);
        state.transport(true);
        frames(&mut state, &output, 1);
        assert_eq!(state.consumed, consumed + 2);
        assert_eq!(state.raw_frame[0], next);
        frames(&mut state, &output, RESUME_FRAMES / 3);
        let gain = state.transport.value;
        state.transport(false);
        assert_eq!(state.transport.from, gain);
        frames(&mut state, &output, PAUSE_FRAMES / 3);
        let gain = state.transport.value;
        state.transport(true);
        assert_eq!(state.transport.from, gain);
        assert!((state.transport.step() - gain).abs() < 0.001);
    }
    #[test]
    fn a1_drop_output_mid_stereo_preserves_pending_right_and_entire_queue() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        state.queue.extend((1..=1200).map(|v| v as f32));
        let shared = Arc::new(Mutex::new(state));
        let mut source = PlayedSource {
            shared: shared.clone(),
            output: output.clone(),
        };
        assert_eq!(source.next(), Some(1.0));
        drop(source);
        let source = PlayedSource { shared, output };
        assert_eq!(
            source.take(1199).collect::<Vec<_>>(),
            (2..=1200).map(|v| v as f32).collect::<Vec<_>>()
        );
    }
    #[test]
    fn a1_seek_and_manual_load_invalidate_old_pcm_but_natural_transition_is_gapless() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        let (tx, rx) = mpsc::unbounded_channel();
        state.events = Some(rx);
        state.queue.extend(vec![1.0; MICRO_FRAMES * 4]);
        tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 1 })
            .unwrap();
        state.events();
        state.queue.extend(vec![2.0; MICRO_FRAMES * 4]);
        let out = frames(&mut state, &output, MICRO_FRAMES * 2);
        assert!(out[0] > 0.99);
        assert_eq!(out[MICRO_FRAMES * 2 - 1], 0.0);
        assert_eq!(*out.last().unwrap(), 2.0);
        assert!(state.old.is_empty());
        let track_id = crate::parse_track_uri("spotify:track:4uLU6hMCjMI75M1A2tKUQC").unwrap();
        tx.send(PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: track_id.clone(),
        })
        .unwrap();
        tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 2 })
            .unwrap();
        state.events();
        let queued = state.queue.len();
        assert!(queued > 0);
        assert_eq!(state.micro.value, 1.0);
        tx.send(PlayerEvent::Seeked {
            play_request_id: 2,
            track_id,
            position_ms: 1000,
        })
        .unwrap();
        state.events();
        assert!(state.queue.is_empty());
    }
    #[test]
    fn a2_reactivation_discards_partial_packet_even_without_callback_while_disabled() {
        let (mut state, mut pcm, output) = playback(true);
        settled(&mut state);
        state.queue.extend(vec![0.25; TAP_SAMPLES / 2]);
        frames(&mut state, &output, TAP_SAMPLES / 4);
        assert_eq!(state.pending.len(), TAP_SAMPLES / 2);
        state.tap.enabled.set(false);
        state.tap.enabled.set(true);
        let activated = Instant::now();
        state.queue.extend(vec![0.75; TAP_SAMPLES]);
        frames(&mut state, &output, TAP_SAMPLES / 2);
        let packet = pcm.try_recv().unwrap();
        assert!(packet.captured_at >= activated);
        assert!(packet.samples.iter().all(|v| *v == 0.75));
    }
    #[test]
    fn a3_stall_uses_last_consumption_not_age_of_write() {
        let now = Instant::now();
        let mut progress = Progress::new(now);
        assert!(!progress.stalled(1, now + Duration::from_millis(900)));
        assert!(!progress.stalled(1, now + Duration::from_millis(1005)));
        assert!(progress.stalled(1, now + Duration::from_millis(1901)));
    }
    #[test]
    fn underrun_preserves_stereo_and_pcm_stays_bounded() {
        let (mut state, mut pcm, output) = playback(true);
        settled(&mut state);
        assert_eq!(state.next(&output), 0.0);
        state.queue.extend([0.25, -0.5]);
        assert_eq!(state.next(&output), 0.0);
        assert_eq!(state.next(&output), 0.25);
        assert_eq!(state.next(&output), -0.5);
        frames(&mut state, &output, TAP_SAMPLES * 4);
        assert!(pcm.try_recv().is_ok());
        assert!(pcm.try_recv().is_ok());
        assert!(pcm.try_recv().is_err());
    }
    #[test]
    fn a1_load_or_seek_while_fully_paused_never_replays_old_queue_on_resume() {
        let (mut state, _, output) = playback(false);
        state.queue.extend(vec![1.0; 2000]);
        state.discontinuity();
        assert!(state.queue.is_empty());
        assert!(state.old.is_empty());
        state.queue.extend(vec![2.0; RESUME_FRAMES * 2]);
        state.transport(true);
        frames(&mut state, &output, RESUME_FRAMES);
        assert_eq!(state.raw_frame, [2.0; 2]);
        assert_eq!(state.frame, [2.0; 2]);
    }
    #[test]
    fn fade_completion_is_generation_scoped_and_only_emitted_at_zero() {
        let (mut state, _, _) = playback(false);
        settled(&mut state);
        state.queue.extend(vec![1.0; PAUSE_FRAMES * 2]);
        let (output, received) = std_mpsc::channel();
        state.transport(false);
        let pause = state.pause_generation;
        frames(&mut state, &output, PAUSE_FRAMES - 1);
        assert!(received.try_recv().is_err());
        frames(&mut state, &output, 1);
        assert!(matches!(received.try_recv(), Ok(OutputCommand::Faded(g)) if g == pause));
        state.transport(true);
        assert_ne!(state.pause_generation, pause);
        assert!(state.playing);
        frames(&mut state, &output, 1);
        assert!(received.try_recv().is_err());
    }
    #[test]
    fn m1_local_eof_fades_only_a_tail_and_preserves_b_without_wall_clock() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        let uri = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
        state.timer_uri = Some(uri.into());
        state.timer_state.send_replace(TrackEndTimerState::Armed);
        let (pause, mut requests) = mpsc::unbounded_channel();
        state.pause_request = pause;
        let (tx, rx) = mpsc::unbounded_channel();
        state.events = Some(rx);
        state.queue.extend(vec![1.0; PAUSE_FRAMES * 2]);
        tx.send(PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: crate::parse_track_uri(uri).unwrap(),
        })
        .unwrap();
        state.events(); // Límite A queda fijado antes de añadir el primer PCM de B.
        tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 2 })
            .unwrap();
        state.events();
        state.queue.extend(vec![2.0; 500]);
        let out = frames(&mut state, &output, PAUSE_FRAMES);
        assert!(out[0] > 0.99);
        assert!(out.iter().all(|v| *v <= 1.0));
        assert_eq!(*state.timer_state.borrow(), TrackEndTimerState::Finished);
        assert!(requests.try_recv().is_ok());
        assert!(state.queue.iter().all(|v| *v == 2.0));
        assert_eq!(state.queue.len(), 500);
        frames(&mut state, &output, 100);
        assert_eq!(state.queue.len(), 500);
        state.cancel_timer();
        state.transport(true);
        frames(&mut state, &output, 1);
        assert_eq!(state.raw_frame, [2.0; 2]);
    }
    #[test]
    fn m1_local_pause_seek_keep_armed_manual_load_cancels_and_rearm_is_isolated() {
        let (state, _, output) = playback(false);
        let control = OutputControl(Arc::new(Mutex::new(state)));
        let uri = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
        let first = control.arm(uri.into());
        let (tx, rx) = mpsc::unbounded_channel();
        {
            let mut state = control.0.lock().unwrap();
            state.events = Some(rx);
            settled(&mut state);
            state.transport(false);
            frames(&mut state, &output, PAUSE_FRAMES);
        }
        control.resume();
        assert_eq!(*first.borrow(), TrackEndTimerState::Armed);
        tx.send(PlayerEvent::Seeked {
            play_request_id: 1,
            track_id: crate::parse_track_uri(uri).unwrap(),
            position_ms: 5000,
        })
        .unwrap();
        control.sync_events();
        assert_eq!(*first.borrow(), TrackEndTimerState::Armed);
        let second = control.arm(uri.into());
        assert_eq!(*first.borrow(), TrackEndTimerState::Cancelled);
        tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 2 })
            .unwrap();
        control.sync_events();
        assert_eq!(*second.borrow(), TrackEndTimerState::Cancelled);
    }
    #[test]
    fn fades_in_wait_for_pcm_after_decoder_stall_and_manual_seek() {
        let (mut state, _, output) = playback(false);
        state.transport(true);
        frames(&mut state, &output, RESUME_FRAMES * 2);
        assert_eq!(state.transport.value, 0.0);
        state.queue.extend(vec![1.0; RESUME_FRAMES * 2]);
        let first = frames(&mut state, &output, 1);
        assert!(first[0] < 0.001);
        frames(&mut state, &output, RESUME_FRAMES - 1);
        assert_eq!(state.transport.value, 1.0);
        state.discontinuity();
        frames(&mut state, &output, MICRO_FRAMES * 4);
        assert_eq!(state.micro.value, 0.0);
        state.queue.extend(vec![1.0; MICRO_FRAMES * 2]);
        let first = frames(&mut state, &output, 1);
        assert!(first[0] < 0.001);
        frames(&mut state, &output, MICRO_FRAMES - 1);
        assert_eq!(state.micro.value, 1.0);
    }
    // ---- Mixear ----

    fn mix_sine(frame: usize, hz: f64) -> f32 {
        (0.3 * (std::f64::consts::TAU * hz * frame as f64 / f64::from(SAMPLE_RATE)).sin()) as f32
    }

    /// Reproduce el tope y el backpressure de write(), con paquetes de 20 ms y sin dispositivo.
    fn mix_synthetic_stream(a_seconds: u32, b_seconds: u32, seconds: u32) -> Vec<f32> {
        let (mut state, tx, output, _) = mixing(false);
        state.mix.set(crate::MixConfig {
            enabled: true,
            seconds,
        });
        begin(
            &mut state,
            &tx,
            crate::mix::tests::item(MIX_A, "Atlas", &["Lumen"], 1, 1, a_seconds * 1000),
        );
        let mut out = Vec::new();
        let base_limit = SECOND * 2 / 10;
        let chunk_frames = SECOND / 50;
        for (track, duration, hz) in [(0, a_seconds, 220.0), (1, b_seconds, 330.0)] {
            if track == 1 {
                natural_next(
                    &mut state,
                    &tx,
                    crate::mix::tests::item(MIX_B, "Mapa", &["Lumen"], 1, 1, duration * 1000),
                );
            }
            for start in (0..duration as usize * SECOND).step_by(chunk_frames) {
                let end = (start + chunk_frames).min(duration as usize * SECOND);
                let samples: Vec<f32> = (start..end)
                    .flat_map(|frame| [mix_sine(frame, hz); 2])
                    .collect();
                if let Some(needed) = state.reserve_window(base_limit + samples.len()) {
                    drop(state.adopt(VecDeque::with_capacity(needed)));
                }
                state.push_decoded(&samples);
                loop {
                    let limit = base_limit + state.queue_extra();
                    let len = if state.route_b {
                        state.deck_b.len()
                    } else {
                        state.queue.len()
                    };
                    if len <= limit {
                        break;
                    }
                    out.extend([state.next(&output), state.next(&output)]);
                }
            }
        }
        while state.song_available() {
            out.extend([state.next(&output), state.next(&output)]);
        }
        assert_eq!(
            heard(&state, 1),
            u64::from(a_seconds) * u64::from(SAMPLE_RATE)
        );
        assert_eq!(
            heard(&state, 2),
            u64::from(b_seconds) * u64::from(SAMPLE_RATE)
        );
        out
    }

    #[test]
    fn mix_synthetic_stream_has_final_window_stable_rms_and_smooth_edges() {
        for (a, b, seconds, expected_ms) in [
            (40, 40, 6, 6000),
            (20, 20, 6, 6000),
            (20, 20, 12, 6666),
            (10, 9, 12, 3000),
            (20, 6, 12, 2000),
        ] {
            let out = mix_synthetic_stream(a, b, seconds);
            let window = expected_ms * SECOND / 1000;
            let start = a as usize * SECOND - window;
            assert_eq!(out.len() / 2, (a + b) as usize * SECOND - window);
            for (frame, pair) in out.chunks_exact(2).enumerate() {
                let expected = if frame < start {
                    mix_sine(frame, 220.0)
                } else if frame < a as usize * SECOND {
                    let k = frame - start;
                    let (ga, gb) = crate::mix::gains((k as f32 + 0.5) / window as f32);
                    ga * mix_sine(frame, 220.0) + gb * mix_sine(k, 330.0)
                } else {
                    mix_sine(frame - start, 330.0)
                };
                assert!(
                    (pair[0] - expected).abs() < 2e-6,
                    "{a}/{b}/{seconds}: {frame}"
                );
                assert_eq!(pair[0], pair[1]);
            }
            let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
            let max_step = left
                .windows(2)
                .map(|f| (f[1] - f[0]).abs())
                .fold(0.0, f32::max);
            assert!(max_step < 0.021, "{a}/{b}/{seconds}: {max_step}");
            let target_rms = 0.3 / 2_f32.sqrt();
            for block in left[start..start + window].chunks_exact(SECOND / 10) {
                let rms = (block.iter().map(|v| v * v).sum::<f32>() / block.len() as f32).sqrt();
                assert!(
                    (rms / target_rms - 1.0).abs() < 0.03,
                    "{a}/{b}/{seconds}: {rms}"
                );
            }
        }
    }

    #[test]
    fn mix_output_recreation_mid_stereo_preserves_the_fade_and_pcm_exactly() {
        let (mut reference, _, output, _) = ready_to_mix(false, [0.5; 2], [0.25; 2]);
        let (mut changed, _, _, _) = ready_to_mix(false, [0.5; 2], [0.25; 2]);
        frames(&mut reference, &output, SECOND / 2 + 1000);
        frames(&mut changed, &output, SECOND / 2 + 1000);
        let expected: Vec<f32> = (0..2001).map(|_| reference.next(&output)).collect();
        let shared = Arc::new(Mutex::new(changed));
        let mut source = PlayedSource {
            shared: shared.clone(),
            output: output.clone(),
        };
        let mut actual = vec![source.next().unwrap()];
        drop(source); // Cierre/reapertura del worker: la cola y el canal pendiente sobreviven.
        let mut source = PlayedSource {
            shared: shared.clone(),
            output,
        };
        actual.extend(source.by_ref().take(2000));
        assert_eq!(actual, expected);
        assert_eq!(
            shared.lock().unwrap().fade.unwrap().elapsed,
            reference.fade.unwrap().elapsed
        );
    }

    #[test]
    fn mix_next_previous_and_seek_microfade_the_current_sum_without_clicks() {
        for action in 0..3 {
            let (mut state, tx, output, _) = ready_to_mix(false, [0.5; 2], [0.25; 2]);
            let mut out = frames(&mut state, &output, SECOND);
            assert!(state.fade.is_some());
            if action == 2 {
                tx.send(PlayerEvent::Seeked {
                    play_request_id: 2,
                    track_id: crate::parse_track_uri(MIX_B).unwrap(),
                    position_ms: 2000,
                })
                .unwrap();
            } else {
                // Siguiente/anterior generan la misma invalidación por una ocurrencia nueva.
                tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 3 })
                    .unwrap();
            }
            state.events();
            assert!(state.deck_b.is_empty() && state.fade.is_none());
            decode(&mut state, MICRO_FRAMES * 3, [-0.3; 2]);
            out.extend(frames(&mut state, &output, MICRO_FRAMES * 3));
            let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
            let max_step = left
                .windows(2)
                .map(|f| (f[1] - f[0]).abs())
                .fold(0.0, f32::max);
            assert!(max_step < 0.004, "{action}: {max_step}");
        }
    }

    #[test]
    fn mix_status_counts_down_freezes_in_pause_and_finishes_at_promotion() {
        let (mut state, _, output, _) = mixing(false);
        state.mix.set(crate::MixConfig {
            enabled: true,
            seconds: 3,
        });
        // Construir una ventana de tres segundos para observar varias publicaciones.
        state.queue.extend(std::iter::repeat_n(0.3, SECOND * 6));
        state.deck_b.extend(std::iter::repeat_n(0.2, SECOND * 8));
        state.route_b = true;
        state.mix_window = SECOND * 3;
        let status = state.mix_status.subscribe();
        frames(&mut state, &output, 1);
        assert!(status.borrow().active);
        assert_eq!(status.borrow().remaining_ms, 3000);
        frames(&mut state, &output, SECOND - 1);
        assert_eq!(status.borrow().remaining_ms, 2000);
        state.transport(false);
        frames(&mut state, &output, PAUSE_FRAMES + 1);
        let elapsed = state.fade.unwrap().elapsed;
        let paused_status = *status.borrow();
        frames(&mut state, &output, SECOND);
        assert_eq!(state.fade.unwrap().elapsed, elapsed);
        assert_eq!(*status.borrow(), paused_status);
        state.transport(true);
        frames(&mut state, &output, SECOND * 3 - elapsed + 1);
        assert!(!status.borrow().active);
        assert_eq!(status.borrow().remaining_ms, 0);
    }

    #[test]
    fn mix_volume_and_active_eq_process_one_sum_while_tap_stays_raw() {
        let (mut state, _, output, mut pcm) = ready_to_mix(true, [0.3; 2], [0.2; 2]);
        let eq = EqControl::default();
        eq.set(crate::EqConfig {
            enabled: true,
            preamp_db: -6.0,
            ..Default::default()
        });
        state.eq = Equalizer::new(eq, SAMPLE_RATE);
        state.volume_override.request(1, 0.5);
        frames(&mut state, &output, SECOND / 2);
        while pcm.try_recv().is_ok() {}
        state.pending.clear();
        let out = frames(&mut state, &output, TAP_SAMPLES / 2);
        let raw = pcm.try_recv().unwrap();
        for (k, pair) in raw.samples.chunks_exact(2).enumerate() {
            let (ga, gb) = crate::mix::gains((k as f32 + 0.5) / SECOND as f32);
            let mixed = 0.3 * ga + 0.2 * gb;
            assert!((pair[0] as f32 - mixed).abs() < 1e-6);
            assert!((out[k * 2] - mixed * 0.5 * 10_f32.powf(-6.0 / 20.0)).abs() < 1e-6);
        }
    }

    #[test]
    fn mix_limiter_enters_and_leaves_continuously_and_is_transparent_outside() {
        let (mut state, _, output, _) = ready_to_mix(false, [1.0; 2], [1.0; 2]);
        let out = frames(&mut state, &output, SECOND * 2);
        let max_step = out
            .chunks_exact(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|f| (f[1] - f[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.005, "{max_step}");
        // Fuera del solape (y fuera de las rampas) la señal es idéntica bit a bit.
        let start = SECOND / 2;
        let ramp = crate::mix::LIMITER_RAMP_FRAMES;
        assert!(out[..start * 2].iter().all(|v| *v == 1.0));
        assert!(out[(start + SECOND + ramp) * 2..].iter().all(|v| *v == 1.0));
        assert!(out.iter().all(|v| v.abs() <= 1.0));
    }

    #[test]
    fn mix_below_the_knee_passes_identically() {
        let knee = crate::mix::LIMITER_KNEE;
        for k in 0..=1000 {
            let x = knee * (k as f32 / 500.0 - 1.0);
            for w in [0.0, 0.3, 1.0] {
                assert_eq!(crate::mix::limit_blend(x, w), x);
            }
        }
    }

    /// B con `b_frames` de colchón al llegar y A de 1,5 s (ventana 1 s): suena A sola.
    #[test]
    fn mix_b_with_one_frame_never_starts_the_fade_and_a_stays_whole() {
        let (mut state, tx, output, _) = mixing(false);
        begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
        decode(&mut state, SECOND, [0.5; 2]);
        natural_next(&mut state, &tx, album_item(MIX_B, "Mapa", 1));
        decode(&mut state, 1, [0.25; 2]);
        let out = frames(&mut state, &output, SECOND);
        assert!(out.iter().all(|v| *v == 0.5));
        assert!(state.fade.is_none());
        assert_eq!(heard(&state, 1), SECOND as u64);
        assert_eq!(heard(&state, 2), 0);
        // B llega sin colchón suficiente: tras A suena a ganancia plena.
        let out = frames(&mut state, &output, 1);
        assert_eq!(out[0], 0.25);
        assert!(!state.route_b);
    }

    #[test]
    fn mix_b_that_never_arrives_leaves_a_untouched_and_b_starts_at_full_gain() {
        let (mut state, tx, output, _) = mixing(false);
        begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
        decode(&mut state, SECOND, [0.5; 2]);
        natural_next(&mut state, &tx, album_item(MIX_B, "Mapa", 1));
        let out = frames(&mut state, &output, SECOND);
        assert!(out.iter().all(|v| *v == 0.5));
        assert!(state.fade.is_none());
        decode(&mut state, SECOND, [0.25; 2]);
        let out = frames(&mut state, &output, 10);
        assert!(out.iter().all(|v| *v == 0.25));
    }

    #[test]
    fn mix_b_that_empties_midway_freezes_the_fade_and_resumes_without_a_jump() {
        let (mut state, _tx, output, _) = ready_to_mix(false, [0.5; 2], [0.25; 2]);
        // Solo el colchón mínimo de B (menos que la ventana entera).
        state.deck_b.truncate(crate::mix::MIX_B_CUSHION_FRAMES * 2);
        let mut out = frames(
            &mut state,
            &output,
            SECOND / 2 + crate::mix::MIX_B_CUSHION_FRAMES - 1,
        );
        assert!(state.fade.is_some() && !state.deck_b.is_empty());
        out.extend(frames(&mut state, &output, 1));
        // B se vació: A conserva la ganancia retenida, sin silencio ni cambio.
        let before = state.fade.unwrap();
        let held = frames(&mut state, &output, SECOND / 4);
        let after = state.fade.unwrap();
        assert_eq!(after.elapsed, 0);
        let (ga, gb) = after.held_gains();
        assert_eq!(held[0], 0.5 * ga);
        assert!(held.iter().all(|v| *v == held[0]));
        assert!(held[0] > 0.1 && held[0] < 0.5, "{}", held[0]);
        assert!(after.t0 > before.t0 && gb > 0.0);
        assert_eq!(state.deck_b.len(), 0);
        let remaining = state.queue.len() / 2;
        assert!(state.mix_status.borrow().active);
        // B vuelve: el fundido continúa con ganancias continuas y termina justo con A.
        decode(&mut state, SECOND, [0.25; 2]);
        let mut rest = frames(&mut state, &output, remaining + 1);
        assert!(!state.route_b && state.fade.is_none());
        let after_end = frames(&mut state, &output, SECOND / 10 - 1);
        rest.extend(&after_end);
        // Las ganancias no saltan en la reanudación (lo que cambia es la señal de B misma).
        assert!(
            (rest[0] - (0.5 * ga + 0.25 * gb)).abs() < 0.005,
            "{}",
            rest[0]
        );
        for segment in [&out, &held, &rest] {
            let left: Vec<f32> = segment.chunks_exact(2).map(|f| f[0]).collect();
            let max_step = left
                .windows(2)
                .map(|f| (f[1] - f[0]).abs())
                .fold(0.0, f32::max);
            assert!(max_step < 0.005, "{max_step}");
        }
        assert!(after_end.iter().all(|v| (*v - 0.25).abs() < 1e-6));
        assert_eq!(heard(&state, 1), SECOND as u64 * 3 / 2);
    }

    #[test]
    fn mix_repeat_and_shuffle_use_the_actual_natural_successor() {
        for (uri, album, number, skip_mix) in [
            (MIX_A, "Atlas", 4, false), // Repetir pista no es la pista siguiente del álbum.
            (MIX_B, "Atlas", 5, true),  // Shuffle también respeta adyacencia si la elige.
            (MIX_B, "Atlas", 8, false),
        ] {
            let (mut state, tx, output, _) = mixing(false);
            begin(&mut state, &tx, album_item(MIX_A, "Atlas", 4));
            decode(&mut state, SECOND, [0.3; 2]);
            natural_next(&mut state, &tx, album_item(uri, album, number));
            assert_eq!(state.route_b, !skip_mix);
            decode(&mut state, SECOND * 2, [0.2; 2]);
            frames(&mut state, &output, SECOND + 1);
            assert_eq!(heard(&state, 1), SECOND as u64);
            assert_eq!(
                heard(&state, 2),
                if skip_mix { 1 } else { SECOND as u64 + 1 }
            );
        }
    }

    #[test]
    fn mix_empty_queue_drains_the_entire_final_tail_after_the_real_sink_stop() {
        let (mut state, tx, output, _) = mixing(false);
        begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
        state.anchor_decode(8_500);
        decode(&mut state, SECOND * 3 / 2, [0.3; 2]);
        assert!(state.queue_extra() > 0);
        tx.send(PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: crate::parse_track_uri(MIX_A).unwrap(),
        })
        .unwrap();
        let shared = Arc::new(Mutex::new(state));
        // librespot llama stop() ANTES de enviar Stopped al terminar un contexto.
        let mut sink = TappedSink {
            shared: shared.clone(),
            output: output.clone(),
            worker: None,
            started: true,
            switch: crate::output::OutputSwitch::default(),
            applied: 0,
            faults: mpsc::unbounded_channel().0,
        };
        sink.stop().unwrap();
        tx.send(PlayerEvent::Stopped {
            play_request_id: 1,
            track_id: crate::parse_track_uri(MIX_A).unwrap(),
        })
        .unwrap();
        let mut state = shared.lock().unwrap();
        let out = frames(&mut state, &output, SECOND * 3 / 2);
        assert!(
            out.iter().all(|v| *v == 0.3),
            "no cortar A al acabar el contexto"
        );
        assert_eq!(heard(&state, 1), SECOND as u64 * 3 / 2);
        assert!(state.queue.is_empty());
        frames(&mut state, &output, PAUSE_FRAMES + 1);
        assert!(!state.playing && state.notified);
    }
    const MIX_A: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
    const MIX_B: &str = "spotify:track:0000000000000000000001";
    const SECOND: usize = SAMPLE_RATE as usize;
    fn mixing(
        tap: bool,
    ) -> (
        Playback,
        mpsc::UnboundedSender<PlayerEvent>,
        std_mpsc::Sender<OutputCommand>,
        mpsc::Receiver<PcmPacket>,
    ) {
        let (mut state, pcm, output) = playback(tap);
        settled(&mut state);
        let (tx, rx) = mpsc::unbounded_channel();
        state.events = Some(rx);
        state.mix.set(crate::MixConfig {
            enabled: true,
            seconds: 1,
        });
        (state, tx, output, pcm)
    }
    fn album_item(uri: &str, album: &str, number: u32) -> Box<AudioItem> {
        crate::mix::tests::item(uri, album, &["Lumen"], 1, number, 10_000)
    }
    fn begin(state: &mut Playback, tx: &mpsc::UnboundedSender<PlayerEvent>, item: Box<AudioItem>) {
        let track_id = item.track_id.clone();
        tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 1 })
            .unwrap();
        tx.send(PlayerEvent::TrackChanged { audio_item: item })
            .unwrap();
        tx.send(PlayerEvent::Playing {
            play_request_id: 1,
            track_id,
            position_ms: 0,
        })
        .unwrap();
        state.events();
        // La carga manual inicial no tenía nada sonando: sin micro-fade pendiente.
        state.micro = Ramp::new(1.0);
        state.old.clear();
    }
    fn natural_next(
        state: &mut Playback,
        tx: &mpsc::UnboundedSender<PlayerEvent>,
        item: Box<AudioItem>,
    ) {
        tx.send(PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: crate::parse_track_uri(MIX_A).unwrap(),
        })
        .unwrap();
        tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 2 })
            .unwrap();
        tx.send(PlayerEvent::TrackChanged { audio_item: item })
            .unwrap();
        state.events();
    }
    fn decode(state: &mut Playback, frames: usize, frame: [f32; 2]) {
        let samples: Vec<f32> = std::iter::repeat_n(frame, frames).flatten().collect();
        state.push_decoded(&samples);
    }
    /// A (1,5 s en cola) y B (2 s en el deck B) listos para un solape de 1 s.
    fn ready_to_mix(
        tap: bool,
        a: [f32; 2],
        b: [f32; 2],
    ) -> (
        Playback,
        mpsc::UnboundedSender<PlayerEvent>,
        std_mpsc::Sender<OutputCommand>,
        mpsc::Receiver<PcmPacket>,
    ) {
        let (mut state, tx, output, pcm) = mixing(tap);
        begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
        state.anchor_decode(8_500);
        decode(&mut state, SECOND * 3 / 2, a);
        assert!(state.queue_extra() > 0, "la ventana se abrió para A");
        natural_next(&mut state, &tx, album_item(MIX_B, "Mapa", 1));
        assert!(state.route_b);
        assert_eq!(state.mix_window, SECOND);
        decode(&mut state, SECOND * 2, b);
        (state, tx, output, pcm)
    }

    #[test]
    fn mix_queue_limit_only_grows_inside_the_window() {
        let (mut state, tx, _, _) = mixing(false);
        let status = state.mix_status.subscribe();
        begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
        assert_eq!(state.queue_extra(), 0);
        decode(&mut state, SECOND * 6, [0.1; 2]);
        // 10 s de pista, 6 s decodificados: faltan 4 s > 1 s de ventana + 1 s de margen.
        assert_eq!(state.queue_extra(), 0);
        assert_eq!(status.borrow().ahead_ms, 0);
        decode(&mut state, SECOND * 21 / 10, [0.1; 2]);
        let extra = state.queue_extra();
        assert_eq!(
            extra,
            ((1000 + MIX_MARGIN_MS + MIX_SLACK_MS) * u64::from(SAMPLE_RATE) * 2 / 1000) as usize
        );
        assert_eq!(status.borrow().ahead_ms, 1000 + MIX_MARGIN_MS);
        // Reservado fuera del callback: la ventana entera cabe sin crecer en el decoder.
        state.queue = (0..1000).map(|v| v as f32).collect();
        let needed = state.reserve_window(1000).unwrap();
        assert_eq!(needed, extra + 1000);
        let old = state.adopt(VecDeque::with_capacity(needed));
        assert!(state.queue.capacity() >= needed && state.queue.len() == 1000);
        assert_eq!(state.queue[999], 999.0);
        drop(old);
        assert!(state.reserve_window(1000).is_none());
        // Pista nueva: el tope vuelve a la base hasta su propia ventana.
        natural_next(&mut state, &tx, album_item(MIX_B, "Atlas", 2));
        assert_eq!(state.queue_extra(), 0);
        // Mezcla apagada: nunca sube.
        let (mut off, tx, _, _) = mixing(false);
        off.mix.set(crate::MixConfig::default());
        begin(&mut off, &tx, album_item(MIX_A, "Atlas", 1));
        decode(&mut off, SECOND * 10, [0.1; 2]);
        assert_eq!(off.queue_extra(), 0);
        assert!(off.reserve_window(1000).is_none());
    }

    #[test]
    fn mix_is_a_cos_plus_b_sin_with_unit_power_then_b_takes_over() {
        let (mut state, _tx, output, _) = ready_to_mix(false, [0.6, 0.0], [0.0, 0.6]);
        let status = state.mix_status.subscribe();
        let before = frames(&mut state, &output, SECOND / 2);
        assert!(before.chunks_exact(2).all(|f| f == [0.6, 0.0]));
        assert!(!status.borrow().active);
        let out = frames(&mut state, &output, SECOND);
        for (k, f) in out.chunks_exact(2).enumerate() {
            let (ga, gb) = crate::mix::gains((k as f32 + 0.5) / SECOND as f32);
            assert!((f[0] - 0.6 * ga).abs() < 1e-5, "{k}");
            assert!((f[1] - 0.6 * gb).abs() < 1e-5, "{k}");
            let power = (f[0] * f[0] + f[1] * f[1]) / 0.36;
            assert!((power - 1.0).abs() < 1e-4, "{k}");
        }
        let after = frames(&mut state, &output, 10);
        assert!(after.chunks_exact(2).all(|f| f == [0.0, 0.6]));
        assert!(!state.route_b && state.fade.is_none() && state.deck_b.is_empty());
        assert_eq!(state.queue.len(), (SECOND - 10) * 2);
        assert!(!status.borrow().active);
    }

    #[test]
    fn same_album_next_track_concatenates_sample_exact() {
        let (mut state, tx, output, _) = mixing(false);
        begin(&mut state, &tx, album_item(MIX_A, "Atlas", 4));
        state.anchor_decode(8_000);
        let a: Vec<f32> = (0..SECOND * 2).map(|v| (v % 997) as f32 / 2000.0).collect();
        state.push_decoded(&a);
        assert!(state.queue_extra() > 0);
        natural_next(&mut state, &tx, album_item(MIX_B, "Atlas", 5));
        assert!(!state.route_b && state.mix_window == 0);
        let b: Vec<f32> = (0..SECOND).map(|v| -((v % 991) as f32) / 2000.0).collect();
        state.push_decoded(&b);
        let out = frames(&mut state, &output, (a.len() + b.len()) / 2);
        assert_eq!(out, [a, b].concat());
        // El decoder iba por delante: la posición de B empieza cuando se oye.
        assert_eq!(state.mix_status.borrow().started.map(|s| s.0), Some(2));
        assert_eq!(state.mix_status.borrow().ahead_ms, 0);
    }

    #[test]
    fn without_next_track_a_plays_at_full_gain_to_the_end() {
        let (mut state, tx, output, _) = mixing(false);
        begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
        decode(&mut state, SECOND * 3 / 2, [0.5; 2]);
        tx.send(PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: crate::parse_track_uri(MIX_A).unwrap(),
        })
        .unwrap();
        tx.send(PlayerEvent::Stopped {
            play_request_id: 1,
            track_id: crate::parse_track_uri(MIX_A).unwrap(),
        })
        .unwrap();
        let out = frames(&mut state, &output, SECOND * 3 / 2);
        assert!(out.iter().all(|v| *v == 0.5));
        assert!(!state.route_b && state.fade.is_none());
    }

    #[test]
    fn pause_keeps_both_decks_and_seek_or_next_empties_them() {
        let (mut state, tx, output, _) = ready_to_mix(false, [0.5; 2], [0.25; 2]);
        frames(&mut state, &output, SECOND / 2 + 1000);
        assert!(state.fade.is_some());
        state.transport(false);
        frames(&mut state, &output, PAUSE_FRAMES + 1);
        let (a, b, fade) = (
            state.queue.len(),
            state.deck_b.len(),
            state.fade.unwrap().elapsed,
        );
        frames(&mut state, &output, 2000);
        assert_eq!(
            (state.queue.len(), state.deck_b.len()),
            (a, b),
            "en pausa nada se consume"
        );
        assert_eq!(state.fade.unwrap().elapsed, fade);
        state.transport(true);
        frames(&mut state, &output, 100);
        assert!(state.fade.unwrap().elapsed > fade && state.route_b);
        // Siguiente durante el fundido: carga manual, ambos decks vacíos.
        tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 3 })
            .unwrap();
        state.events();
        assert!(state.deck_b.is_empty() && state.spans_b.is_empty() && state.queue.is_empty());
        assert!(state.fade.is_none() && !state.route_b);
        assert!(state.old.len() <= MICRO_FRAMES * 2);
        assert!(!state.mix_status.borrow().active);
        // Seek durante el fundido, en pausa total: nada viejo vuelve al reanudar.
        let (mut state, tx, output, _) = ready_to_mix(false, [0.5; 2], [0.25; 2]);
        frames(&mut state, &output, SECOND / 2 + 1000);
        state.transport(false);
        frames(&mut state, &output, PAUSE_FRAMES + 1);
        tx.send(PlayerEvent::Seeked {
            play_request_id: 2,
            track_id: crate::parse_track_uri(MIX_B).unwrap(),
            position_ms: 30_000,
        })
        .unwrap();
        state.events();
        assert!(state.deck_b.is_empty() && state.queue.is_empty() && state.old.is_empty());
        assert!(state.fade.is_none() && !state.route_b);
        assert_eq!(state.decode_anchor_ms, 30_000);
    }

    #[test]
    fn visualizer_gets_the_mix_before_volume_and_the_limiter_holds_full_scale() {
        let (mut state, _tx, output, mut pcm) = ready_to_mix(true, [1.0; 2], [1.0; 2]);
        state.volume_override.request(1, 0.5);
        state.volume = Ramp::new(0.5);
        frames(&mut state, &output, SECOND / 2);
        while pcm.try_recv().is_ok() {}
        state.pending.clear();
        let out = frames(&mut state, &output, TAP_SAMPLES / 2);
        let packet = pcm.try_recv().unwrap();
        for (k, pair) in packet.samples.chunks_exact(2).enumerate() {
            let (ga, gb) = crate::mix::gains((k as f32 + 0.5) / SECOND as f32);
            let weight = ((k + 1) as f32 / crate::mix::LIMITER_RAMP_FRAMES as f32).min(1.0);
            let mixed = crate::mix::limit_blend(ga + gb, weight);
            assert!((pair[0] as f32 - mixed).abs() < 1e-6, "{k}");
            assert!((out[k * 2] - mixed * 0.5).abs() < 1e-6, "{k}");
        }
        let all = frames(&mut state, &output, SECOND);
        assert!(all.iter().all(|v| v.abs() <= 0.5 + 1e-6));
        assert!(
            packet
                .samples
                .iter()
                .all(|v| v.abs() <= 1.0 && *v > f64::from(crate::mix::LIMITER_KNEE) * 0.99)
        );
    }

    #[test]
    fn listening_progress_is_counted_per_request_during_the_overlap() {
        let (mut state, _tx, output, _) = ready_to_mix(false, [0.5; 2], [0.25; 2]);
        let status = state.mix_status.subscribe();
        frames(&mut state, &output, SECOND / 2);
        assert_eq!((heard(&state, 1), heard(&state, 2)), (SECOND as u64 / 2, 0));
        assert!(status.borrow().started.is_none());
        frames(&mut state, &output, 300);
        assert_eq!(heard(&state, 1), SECOND as u64 / 2 + 300);
        assert_eq!(heard(&state, 2), 300);
        // B empezó a oírse con el fundido: su posición se ancla a ese instante.
        assert_eq!(status.borrow().started.map(|s| s.0), Some(2));
        frames(&mut state, &output, SECOND);
        assert_eq!(heard(&state, 1), SECOND as u64 * 3 / 2);
        assert_eq!(heard(&state, 2), SECOND as u64 + 300);
    }

    #[test]
    fn late_next_track_shortens_the_overlap_or_follows_without_a_gap() {
        for late in [true, false] {
            let (mut state, tx, output, _) = mixing(false);
            begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
            decode(&mut state, SECOND * 3 / 2, [0.5; 2]);
            natural_next(&mut state, &tx, album_item(MIX_B, "Mapa", 1));
            assert!(state.route_b);
            if late {
                // B no llega a tiempo: A suena sola y entera, y B sigue sin hueco.
                let out = frames(&mut state, &output, SECOND * 3 / 2);
                assert!(out.iter().all(|v| *v == 0.5));
                decode(&mut state, 100, [0.25; 2]);
                let out = frames(&mut state, &output, 100);
                assert!(out.iter().all(|v| *v == 0.25));
                assert!(!state.route_b && state.fade.is_none());
            } else {
                // B llega con 0,3 s de A: el solape dura lo que queda de A.
                frames(&mut state, &output, SECOND * 12 / 10);
                decode(&mut state, SECOND, [0.25; 2]);
                frames(&mut state, &output, 1);
                assert_eq!(state.fade.unwrap().frames, SECOND * 3 / 10);
                frames(&mut state, &output, SECOND * 3 / 10);
                assert!(!state.route_b && state.fade.is_none());
                // B sonó en todo el solape y en el frame siguiente, ya sola.
                assert_eq!(state.queue.len(), (SECOND - SECOND * 3 / 10 - 1) * 2);
            }
            assert_eq!(heard(&state, 1), SECOND as u64 * 3 / 2);
        }
    }

    #[test]
    fn sleep_timer_dj_or_voice_never_mix() {
        for blocker in 0..3 {
            let (mut state, tx, _, _) = mixing(false);
            begin(&mut state, &tx, album_item(MIX_A, "Atlas", 1));
            match blocker {
                0 => state.timer_uri = Some(MIX_A.into()),
                1 => state.mix.set_blocked(true),
                _ => {
                    state.voice = Some(Voice {
                        clip: Arc::new(VoiceClip::new(vec![0.0; 2], 2).unwrap()),
                        frame: 0,
                        paused: true,
                        stopping: false,
                        idle: true,
                        ramp: Ramp::new(0.0),
                        progress: watch::channel(VoiceProgress {
                            position_ms: 0,
                            at: Instant::now(),
                            playing: false,
                            finished: false,
                        })
                        .0,
                    })
                }
            }
            decode(&mut state, SECOND * 9, [0.5; 2]);
            assert_eq!(state.queue_extra(), 0, "{blocker}");
            decode(&mut state, SECOND, [0.5; 2]);
            natural_next(&mut state, &tx, album_item(MIX_B, "Mapa", 1));
            assert!(!state.route_b && state.mix_window == 0, "{blocker}");
        }
    }

    #[test]
    fn m1_natural_end_of_last_track_keeps_tail_even_if_spirc_stops_before_output() {
        let (mut state, _, output) = playback(false);
        settled(&mut state);
        let uri = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
        let track_id = crate::parse_track_uri(uri).unwrap();
        state.timer_uri = Some(uri.into());
        state.timer_state.send_replace(TrackEndTimerState::Armed);
        state.queue.extend(vec![1.0; 100]);
        state.transport(false);
        let (tx, rx) = mpsc::unbounded_channel();
        state.events = Some(rx);
        tx.send(PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: track_id.clone(),
        })
        .unwrap();
        tx.send(PlayerEvent::Stopped {
            play_request_id: 1,
            track_id,
        })
        .unwrap();
        let out = frames(&mut state, &output, 50);
        assert!(out[0] > 0.99);
        assert_eq!(*state.timer_state.borrow(), TrackEndTimerState::Finished);
        assert_eq!(state.consumed, 100);
        assert!(state.queue.is_empty());
    }
}
