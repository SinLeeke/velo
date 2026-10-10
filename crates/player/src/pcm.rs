use crate::eq::{EqControl, Equalizer};
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
            pause_request,
            timer_uri: None,
            timer_dj: false,
            end_remaining: None,
            timer_state: watch::channel(TrackEndTimerState::Cancelled).0,
            voice: None,
            worker: None,
            release_after_voice: false,
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
        self.spans.clear(); // Discarded seek/manual-load PCM cannot count as listening.
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
                    if !self.natural_end {
                        self.cancel_timer();
                        self.discontinuity();
                    }
                    self.natural_end = false;
                    self.decoding_request = Some(play_request_id);
                }
                PlayerEvent::Seeked { .. } => {
                    if self.end_remaining.is_some() {
                        self.cancel_timer();
                        self.transport(true);
                    }
                    self.discontinuity();
                }
                PlayerEvent::Stopped { .. } if self.natural_end => {
                    // Fin del contexto: Spirc para el decoder, pero la cola final sigue sonando.
                    self.natural_end = false;
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
                self.queue.len() >= 2
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
            let micro = if self.micro.target == 1.0 && self.queue.len() < 2 {
                self.micro.value
            } else {
                self.micro.step()
            };
            let queue = if self.micro.target == 0.0 {
                &mut self.old
            } else {
                &mut self.queue
            };
            if queue.len() >= 2 {
                self.raw_frame = [
                    queue.pop_front().unwrap_or(0.0),
                    queue.pop_front().unwrap_or(0.0),
                ];
                self.consumed = self.consumed.wrapping_add(2);
                if self.micro.target != 0.0 {
                    self.end_remaining = self.end_remaining.map(|n| n.saturating_sub(2));
                    self.count_frame();
                }
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
    fn count_frame(&mut self) {
        let Some(span) = self.spans.front_mut() else {
            return;
        };
        let request_id = span.request_id;
        span.samples = span.samples.saturating_sub(2);
        if span.samples == 0 {
            self.spans.pop_front();
        }
        let index = self
            .ledger_active
            .filter(|i| self.ledger[*i].progress.play_request_id == request_id)
            .or_else(|| {
                self.ledger
                    .iter()
                    .position(|e| e.valid && e.progress.play_request_id == request_id)
            })
            .unwrap_or_else(|| {
                let index = self.ledger_next;
                self.ledger_next = (index + 1) % self.ledger.len();
                self.ledger[index] = ProgressEntry {
                    valid: true,
                    progress: PlaybackProgress {
                        play_request_id: request_id,
                        ..Default::default()
                    },
                    published_frames: 0,
                };
                index
            });
        self.ledger_active = Some(index);
        let progress = &mut self.ledger[index].progress;
        progress.consumed_frames += 1;
        progress.played_ms = progress.consumed_frames * 1000 / u64::from(SAMPLE_RATE);
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
                    || Some(index) != self.ledger_active
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
    ) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.eq = Equalizer::new(eq, SAMPLE_RATE);
        state.volume_override = volume;
        state.progress_tx = Some(progress);
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
        {
            let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            state.events();
            state.publish_progress(false);
            state.queue.extend(converter.f64_to_f32(&samples));
            if let Some(request_id) = state.decoding_request {
                if let Some(span) = state
                    .spans
                    .back_mut()
                    .filter(|span| span.request_id == request_id)
                {
                    span.samples += samples.len();
                } else {
                    state.spans.push_back(PcmSpan {
                        request_id,
                        samples: samples.len(),
                    });
                }
            }
        }
        let mut progress = Progress::new(Instant::now());
        loop {
            let (len, consumed) = {
                let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
                state.publish_progress(false);
                (state.queue.len(), state.consumed)
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
            let queue_limit =
                SAMPLE_RATE as usize * NUM_CHANNELS as usize * self.switch.buffer_ms() as usize
                    / 1000;
            if len <= queue_limit {
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
