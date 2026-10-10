use crate::{State, logic};
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use velo_domain::NowPlayingEvent;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::*;
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::core::Result;

type Session = GlobalSystemMediaTransportControlsSession;
type Manager = GlobalSystemMediaTransportControlsSessionManager;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(3);

// Type inference avoids an additional windows-future dependency. Results (including
// non-Send streams) are retrieved only on this worker's apartment.
macro_rules! wait_winrt {
    ($operation:expr) => {
        wait_winrt!($operation, OPERATION_TIMEOUT)
    };
    ($operation:expr, $timeout:expr) => {{
        let operation = $operation;
        match wait_future(
            std::future::IntoFuture::into_future(operation.clone()),
            $timeout,
        ) {
            Some(result) => result,
            None => {
                let _ = operation.Cancel();
                Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x800705B4_u32 as i32,
                )))
            }
        }
    }};
}
struct Completion(mpsc::Sender<()>);
impl std::task::Wake for Completion {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.0.send(());
    }
}
fn wait_future<F: std::future::Future>(future: F, timeout: Duration) -> Option<F::Output> {
    let deadline = Instant::now() + timeout;
    let (tx, rx) = mpsc::channel();
    let waker = std::task::Waker::from(Arc::new(Completion(tx)));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return Some(result);
        }
        // No polling timer: only completion wakes us, bounded by one deadline.
        if rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_err()
            || Instant::now() >= deadline
        {
            return None;
        }
    }
}
#[derive(Clone, Copy)]
enum Notice {
    Sessions,
    Media(u64),
    Playback(u64),
    Timeline(u64),
    Close,
}

pub(crate) struct Worker {
    tx: mpsc::Sender<Notice>,
    thread: Option<JoinHandle<()>>,
    closing: Arc<AtomicBool>,
}
impl Worker {
    pub(super) fn start(
        sender: broadcast::Sender<NowPlayingEvent>,
        state: Arc<Mutex<State>>,
        available: Arc<AtomicBool>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let callback_tx = tx.clone();
        let closing = Arc::new(AtomicBool::new(false));
        let worker_closing = closing.clone();
        let thread = thread::Builder::new()
            .name("velo-smtc".into())
            .spawn(move || {
                let _availability = Availability(available.clone());
                if let Err(error) = run(callback_tx, rx, sender, state, worker_closing, &available)
                {
                    tracing::warn!(%error, "SMTC worker stopped");
                }
            })?;
        Ok(Self {
            tx,
            thread: Some(thread),
            closing,
        })
    }
    pub(crate) fn stop(&mut self) {
        self.closing.store(true, Ordering::Release);
        let _ = self.tx.send(Notice::Close);
        if let Some(thread) = self.thread.take() {
            // Wait for actual thread termination, including COM/handler cleanup.
            // Dropping a still-running JoinHandle safely detaches the worker.
            let finished = unsafe {
                WaitForSingleObject(HANDLE(thread.as_raw_handle()), 1000) == WAIT_OBJECT_0
            };
            if finished {
                let _ = thread.join();
            }
        }
    }
}
struct Availability(Arc<AtomicBool>);
impl Drop for Availability {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
struct ManagerHandler {
    manager: Manager,
    token: i64,
}
impl Drop for ManagerHandler {
    fn drop(&mut self) {
        let _ = self.manager.RemoveSessionsChanged(self.token);
    }
}
struct SessionHandlers {
    session: Session,
    media: Option<i64>,
    playback: Option<i64>,
    timeline: Option<i64>,
    generation: u64,
}
impl SessionHandlers {
    fn attach(session: Session, tx: &mpsc::Sender<Notice>, generation: u64) -> Result<Self> {
        let mut handlers = Self {
            session,
            media: None,
            playback: None,
            timeline: None,
            generation,
        };
        let notify = tx.clone();
        handlers.media = Some(
            handlers
                .session
                .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = notify.send(Notice::Media(generation));
                    Ok(())
                }))?,
        );
        let notify = tx.clone();
        handlers.playback = Some(
            handlers
                .session
                .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = notify.send(Notice::Playback(generation));
                    Ok(())
                }))?,
        );
        let notify = tx.clone();
        handlers.timeline = Some(handlers.session.TimelinePropertiesChanged(
            &TypedEventHandler::new(move |_, _| {
                let _ = notify.send(Notice::Timeline(generation));
                Ok(())
            }),
        )?);
        Ok(handlers)
    }
}
impl Drop for SessionHandlers {
    fn drop(&mut self) {
        if let Some(token) = self.media {
            let _ = self.session.RemoveMediaPropertiesChanged(token);
        }
        if let Some(token) = self.playback {
            let _ = self.session.RemovePlaybackInfoChanged(token);
        }
        if let Some(token) = self.timeline {
            let _ = self.session.RemoveTimelinePropertiesChanged(token);
        }
    }
}
fn emit(state: &Mutex<State>, sender: &broadcast::Sender<NowPlayingEvent>, event: NowPlayingEvent) {
    emit_with_processes(state, sender, event, spotify_processes);
}
fn emit_with_processes(
    state: &Mutex<State>,
    sender: &broadcast::Sender<NowPlayingEvent>,
    event: NowPlayingEvent,
    processes: impl FnOnce() -> Option<Vec<(u32, u32, String)>>,
) {
    let refresh_pid = match &event {
        NowPlayingEvent::TrackChanged { .. } => {
            !logic::same_track(state.lock().unwrap().track.as_ref(), &event)
        }
        NowPlayingEvent::PlaybackChanged { is_playing: true } => true,
        _ => false,
    };
    if refresh_pid {
        update_pid(state, sender, processes);
    }
    let mut state = state.lock().unwrap();
    if state.update(event.clone()) {
        let _ = sender.send(event);
    }
}
fn update_pid(
    state: &Mutex<State>,
    sender: &broadcast::Sender<NowPlayingEvent>,
    processes: impl FnOnce() -> Option<Vec<(u32, u32, String)>>,
) {
    let Some(NowPlayingEvent::SessionStarted { pid: current }) = state.lock().unwrap().session
    else {
        return;
    };
    if let Some(processes) = processes() {
        let pid = logic::refreshed_pid(current, &processes);
        if pid != current {
            emit(state, sender, NowPlayingEvent::SessionStarted { pid });
        }
    }
}
fn select(manager: &Manager) -> Result<Option<Session>> {
    let sessions = manager.GetSessions()?;
    for index in 0..sessions.Size()? {
        let session = sessions.GetAt(index)?;
        if session
            .SourceAppUserModelId()
            .is_ok_and(|id| logic::spotify_session(&id.to_string_lossy()))
        {
            return Ok(Some(session));
        }
    }
    Ok(None)
}
fn run(
    tx: mpsc::Sender<Notice>,
    rx: mpsc::Receiver<Notice>,
    sender: broadcast::Sender<NowPlayingEvent>,
    state: Arc<Mutex<State>>,
    closing: Arc<AtomicBool>,
    available: &AtomicBool,
) -> Result<()> {
    let Some(connection) = retry_startup(
        &closing,
        || connect(&tx),
        |delay| wait_for_retry(&rx, &closing, delay),
    ) else {
        return Ok(());
    };
    if closing.load(Ordering::Acquire) {
        return Ok(());
    }
    available.store(true, Ordering::Release);
    let manager = &connection.handler.manager;
    let mut active: Option<SessionHandlers> = None;
    let mut generation = 0_u64;
    // Subscribe before enumerating so startup cannot miss a session change.
    let _ = tx.send(Notice::Sessions);
    while let Ok(notice) = rx.recv() {
        // Do not drain old event work when drop requests shutdown.
        if closing.load(Ordering::Acquire) {
            break;
        }
        match notice {
            Notice::Close => break,
            Notice::Sessions => {
                let next = match select(manager) {
                    Ok(next) => next,
                    Err(error) => {
                        tracing::warn!(%error, "Cannot enumerate SMTC sessions");
                        continue;
                    }
                };
                if active.as_ref().map(|a| &a.session) == next.as_ref() {
                    continue;
                }
                if active.take().is_some() {
                    emit(&state, &sender, NowPlayingEvent::SessionEnded);
                }
                generation = generation.wrapping_add(1);
                if let Some(session) = next {
                    match SessionHandlers::attach(session, &tx, generation) {
                        Ok(handlers) => {
                            emit(
                                &state,
                                &sender,
                                NowPlayingEvent::SessionStarted {
                                    pid: spotify_processes().and_then(|p| logic::root_pid(&p)),
                                },
                            );
                            refresh(&handlers.session, &state, &sender, true);
                            active = Some(handlers);
                        }
                        Err(error) => tracing::warn!(%error, "Cannot subscribe to Spotify SMTC"),
                    }
                }
            }
            Notice::Media(g) | Notice::Playback(g) | Notice::Timeline(g) => {
                if let Some(handlers) = active.as_ref().filter(|a| a.generation == g) {
                    refresh(
                        &handlers.session,
                        &state,
                        &sender,
                        matches!(notice, Notice::Media(_)),
                    );
                }
            }
        }
    }
    drop(active);
    Ok(())
}
// Fields drop in declaration order: release the manager before uninitializing COM.
struct Connection {
    handler: ManagerHandler,
    _apartment: Apartment,
}
fn connect(tx: &mpsc::Sender<Notice>) -> Result<Connection> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let apartment = Apartment;
    let manager = wait_winrt!(Manager::RequestAsync()?)?;
    let notify = tx.clone();
    let token = manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        let _ = notify.send(Notice::Sessions);
        Ok(())
    }))?;
    Ok(Connection {
        handler: ManagerHandler { manager, token },
        _apartment: apartment,
    })
}
fn retry_startup<T>(
    closing: &AtomicBool,
    mut attempt: impl FnMut() -> Result<T>,
    mut wait: impl FnMut(Duration) -> bool,
) -> Option<T> {
    let mut delay = Duration::from_secs(1);
    while !closing.load(Ordering::Acquire) {
        match attempt() {
            Ok(value) => return Some(value),
            Err(error) => tracing::warn!(%error, ?delay, "Cannot initialize SMTC; retrying"),
        }
        if !wait(delay) {
            break;
        }
        delay = (delay * 2).min(Duration::from_secs(30));
    }
    None
}
fn wait_for_retry(rx: &mpsc::Receiver<Notice>, closing: &AtomicBool, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        if closing.load(Ordering::Acquire) {
            return false;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Notice::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            Err(mpsc::RecvTimeoutError::Timeout) => return !closing.load(Ordering::Acquire),
            Ok(_) => {}
        }
    }
}
fn refresh(
    session: &Session,
    state: &Mutex<State>,
    sender: &broadcast::Sender<NowPlayingEvent>,
    media: bool,
) {
    let playing = session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus());
    if let Ok(status) = playing {
        emit(
            state,
            sender,
            NowPlayingEvent::PlaybackChanged {
                is_playing: status
                    == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing,
            },
        );
    }
    let timeline = session.GetTimelineProperties().ok();
    let duration = timeline
        .as_ref()
        .and_then(|t| {
            Some(logic::duration_ms(
                t.StartTime().ok()?.Duration,
                t.EndTime().ok()?.Duration,
            ))
        })
        .flatten();
    if media {
        let properties = session
            .TryGetMediaPropertiesAsync()
            .and_then(|op| wait_winrt!(op));
        match properties.and_then(|p| {
            Ok(NowPlayingEvent::TrackChanged {
                title: p.Title()?.to_string_lossy(),
                artist: p.Artist()?.to_string_lossy(),
                album: p.AlbumTitle()?.to_string_lossy(),
                duration_ms: duration,
                thumbnail: p
                    .Thumbnail()
                    .ok()
                    .and_then(|reference| thumbnail(&reference).ok()),
            })
        }) {
            Ok(event) => emit(state, sender, event),
            Err(error) => tracing::debug!(%error, "Cannot read SMTC media properties"),
        }
    } else {
        emit_duration(state, sender, duration);
    }
    if let Some(timeline) = timeline {
        let sample = (|| -> Result<NowPlayingEvent> {
            let position = timeline.Position()?.Duration;
            let updated = timeline.LastUpdatedTime()?.UniversalTime;
            let sampled_at = Instant::now();
            let now = windows_ticks();
            let is_playing = matches!(
                state.lock().unwrap().playback,
                Some(NowPlayingEvent::PlaybackChanged { is_playing: true })
            );
            Ok(NowPlayingEvent::Position {
                position_ms: logic::position_ms(position, updated, now, is_playing),
                sampled_at,
            })
        })();
        if let Ok(event) = sample {
            emit(state, sender, event);
        }
    }
}
fn emit_duration(
    state: &Mutex<State>,
    sender: &broadcast::Sender<NowPlayingEvent>,
    duration: Option<u64>,
) {
    let track = state.lock().unwrap().track.clone();
    if let Some(mut event @ NowPlayingEvent::TrackChanged { .. }) = track {
        if let NowPlayingEvent::TrackChanged { duration_ms, .. } = &mut event {
            *duration_ms = duration;
        }
        emit(state, sender, event);
    }
}
fn windows_ticks() -> i64 {
    const EPOCH_OFFSET: u64 = 116_444_736_000_000_000;
    let ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        / 100;
    (ticks + u128::from(EPOCH_OFFSET)).min(i64::MAX as u128) as i64
}
fn thumbnail(reference: &IRandomAccessStreamReference) -> Result<Vec<u8>> {
    const LIMIT: u64 = 2 * 1024 * 1024;
    let stream = wait_winrt!(reference.OpenReadAsync()?)?;
    let result = (|| {
        let size = stream.Size()?;
        if size > LIMIT {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x800700DF_u32 as i32,
            )));
        }
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        let result = (|| {
            let loaded = wait_winrt!(reader.LoadAsync(size as u32)?)?;
            if u64::from(loaded) != size {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004005_u32 as i32,
                )));
            }
            let mut bytes = vec![0; loaded as usize];
            reader.ReadBytes(&mut bytes)?;
            Ok(bytes)
        })();
        let _ = reader.Close();
        result
    })();
    let _ = stream.Close();
    result
}
fn spotify_processes() -> Option<Vec<(u32, u32, String)>> {
    // Snapshot handle ownership is local and always closed after enumeration.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut processes = Vec::new();
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                processes.push((
                    entry.th32ProcessID,
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..end]),
                ));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        Some(processes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Storage::Streams::DataReaderLoadOperation;

    fn track() -> NowPlayingEvent {
        NowPlayingEvent::TrackChanged {
            title: "title".into(),
            artist: "artist".into(),
            album: "album".into(),
            duration_ms: None,
            thumbnail: Some(vec![1, 2]),
        }
    }
    fn failure() -> windows::core::Error {
        windows::core::Error::from_hresult(windows::core::HRESULT(0x80004005_u32 as i32))
    }
    #[test]
    fn timeline_emits_late_and_changed_duration_once() {
        let state = Mutex::new(State::default());
        let (sender, mut receiver) = broadcast::channel(16);
        emit(&state, &sender, track());
        assert_eq!(receiver.try_recv().unwrap(), track());
        for duration in [Some(123), Some(456), None] {
            emit_duration(&state, &sender, duration);
            let mut expected = track();
            if let NowPlayingEvent::TrackChanged { duration_ms, .. } = &mut expected {
                *duration_ms = duration;
            }
            assert_eq!(receiver.try_recv().unwrap(), expected);
            emit_duration(&state, &sender, duration);
            assert!(receiver.try_recv().is_err());
        }
    }
    #[test]
    fn track_and_playing_events_recover_pid_before_emission() {
        let state = Mutex::new(State::default());
        let (sender, mut receiver) = broadcast::channel(16);
        emit(
            &state,
            &sender,
            NowPlayingEvent::SessionStarted { pid: None },
        );
        receiver.try_recv().unwrap();
        emit_with_processes(&state, &sender, track(), || {
            Some(vec![(42, 0, "Spotify.exe".into())])
        });
        assert_eq!(
            receiver.try_recv().unwrap(),
            NowPlayingEvent::SessionStarted { pid: Some(42) }
        );
        assert_eq!(receiver.try_recv().unwrap(), track());
        emit_with_processes(&state, &sender, track(), || panic!("duplicate track"));
        let playing = NowPlayingEvent::PlaybackChanged { is_playing: true };
        emit_with_processes(&state, &sender, playing.clone(), || {
            Some(vec![(43, 0, "SpotifyStore.exe".into())])
        });
        assert_eq!(
            receiver.try_recv().unwrap(),
            NowPlayingEvent::SessionStarted { pid: Some(43) }
        );
        assert_eq!(receiver.try_recv().unwrap(), playing);
        assert_eq!(state.lock().unwrap().track, Some(track()));
        emit_with_processes(&state, &sender, playing, || {
            Some(vec![(43, 0, "SpotifyStore.exe".into())])
        });
        assert!(receiver.try_recv().is_err());
        emit_with_processes(
            &state,
            &sender,
            NowPlayingEvent::PlaybackChanged { is_playing: false },
            || panic!("paused"),
        );
    }
    #[test]
    fn winrt_wait_returns_results_and_errors() -> Result<()> {
        assert_eq!(wait_winrt!(DataReaderLoadOperation::ready(Ok(7)))?, 7);
        assert_eq!(
            wait_winrt!(DataReaderLoadOperation::ready(Err(failure())))
                .unwrap_err()
                .code(),
            failure().code()
        );
        assert_eq!(wait_winrt!(DataReaderLoadOperation::spawn(|| Ok(9)))?, 9);
        Ok(())
    }
    #[test]
    fn winrt_timeout_tolerates_operation_ignoring_cancellation() {
        let (release, blocked) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let operation = DataReaderLoadOperation::spawn(move || {
            let _ = blocked.recv();
            let _ = finished.send(());
            Ok(1)
        });
        let started = Instant::now();
        let error = wait_winrt!(operation.clone(), Duration::from_millis(20)).unwrap_err();
        assert_eq!(error.code().0, 0x800705B4_u32 as i32);
        assert!(started.elapsed() < Duration::from_secs(1));
        // windows-future's spawned operations ignore Cancel. The caller must
        // still return by the deadline, with the late callback remaining safe.
        assert_eq!(operation.Status().unwrap().0, 0);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn drop_detaches_blocked_worker_within_one_second() {
        let (tx, _rx) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let closing = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn(move || {
            let _ = blocked.recv();
            let _ = finished.send(());
        });
        let source = crate::SmtcSource {
            sender: broadcast::channel(16).0,
            state: Arc::new(Mutex::new(State::default())),
            available: Arc::new(AtomicBool::new(false)),
            worker: Worker {
                tx,
                thread: Some(thread),
                closing: closing.clone(),
            },
        };
        let started = Instant::now();
        drop(source);
        let elapsed = started.elapsed();
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(closing.load(Ordering::Acquire));
        assert!(elapsed >= Duration::from_millis(900));
        assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
    }
    #[test]
    fn stop_joins_cooperative_worker() {
        let (tx, rx) = mpsc::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let mut worker = Worker {
            tx,
            closing: Arc::new(AtomicBool::new(false)),
            thread: Some(thread::spawn(move || {
                assert!(matches!(rx.recv(), Ok(Notice::Close)));
                flag.store(true, Ordering::Release);
            })),
        };
        worker.stop();
        assert!(finished.load(Ordering::Acquire));
        worker.stop();
    }
    #[test]
    fn startup_retries_failures_with_capped_backoff_then_stops_retrying() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let result = retry_startup(
            &AtomicBool::new(false),
            || {
                attempts += 1;
                if attempts <= 8 {
                    Err(failure())
                } else {
                    Ok(42)
                }
            },
            |delay| {
                delays.push(delay.as_secs());
                true
            },
        );
        assert_eq!(result, Some(42));
        assert_eq!(attempts, 9);
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
    }
    #[test]
    fn startup_backoff_is_interruptible_and_does_not_retry_after_close() {
        let (tx, rx) = mpsc::channel();
        let closing = AtomicBool::new(false);
        let mut attempts = 0;
        tx.send(Notice::Sessions).unwrap();
        tx.send(Notice::Close).unwrap();
        let started = Instant::now();
        let result: Option<()> = retry_startup(
            &closing,
            || {
                attempts += 1;
                Err(failure())
            },
            |_| wait_for_retry(&rx, &closing, Duration::from_secs(30)),
        );
        assert!(result.is_none());
        assert_eq!(attempts, 1);
        assert!(started.elapsed() < Duration::from_secs(1));
        closing.store(true, Ordering::Release);
        let result: Option<()> = retry_startup(&closing, || panic!("closed"), |_| panic!("closed"));
        assert!(result.is_none());
    }
    #[test]
    fn availability_reports_recovery_and_worker_exit_without_spotify() {
        let (tx, _rx) = mpsc::channel();
        let available = Arc::new(AtomicBool::new(false));
        let source = crate::SmtcSource {
            sender: broadcast::channel(16).0,
            state: Arc::new(Mutex::new(State::default())),
            available: available.clone(),
            worker: Worker {
                tx,
                thread: None,
                closing: Arc::new(AtomicBool::new(false)),
            },
        };
        let guard = Availability(available.clone());
        let mut attempts = 0;
        let manager = retry_startup(
            &AtomicBool::new(false),
            || {
                assert!(!source.is_available());
                attempts += 1;
                if attempts <= 2 {
                    Err(failure())
                } else {
                    Ok(())
                }
            },
            |_| true,
        );
        assert!(manager.is_some());
        available.store(true, Ordering::Release);
        assert!(source.is_available());
        assert!(source.current().is_empty());
        drop(guard);
        assert!(!source.is_available());
    }
}
