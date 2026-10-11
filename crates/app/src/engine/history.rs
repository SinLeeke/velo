//! Listening history consumes cumulative *effective* audio, never the UI timeline.
//! Integration: C2 implements ProgressSource with consumed PCM + occurrence transport events.
#[path = "../../../services/src/history.rs"]
pub mod storage;
use super::super::{Bus, Env};
use crate::model::ModelOp;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
pub use storage::{Entry, HistoryStore};
use tokio::sync::{mpsc, watch};

pub const ROLES: &str =
    "entryId,uri,title,artist,album,imageUrl,artistId,albumId,contextUri,durationMs,playedAtMs";

/// occurrence includes playback request/session identity; seek/resume must keep it stable.
/// effective_ms is cumulative consumed PCM, excluding buffering/paused/seek and DJ speech.
/// Emit on transport transitions and threshold crossings, never per PCM sample/frame.
#[derive(Clone, Default)]
pub struct ProgressSample {
    pub occurrence: String,
    pub effective_ms: u64,
    pub entry: Entry,
    pub dj_voice: bool,
}
pub trait ProgressSource: Send + Sync {
    fn subscribe(&self) -> mpsc::UnboundedReceiver<ProgressSample>;
}
impl ProgressSource for Bus<ProgressSample> {
    fn subscribe(&self) -> mpsc::UnboundedReceiver<ProgressSample> {
        self.take().expect("one history consumer per source")
    }
}

/// Consumed audio (ms) after which a song enters Recientes: any real playback.
pub const RECORD_AFTER_MS: u64 = 1;

/// Conservative listening from existing Connect snapshots. No clock/timer is added:
/// only continuous position advances between successful observations count.
pub struct RemoteListeningClock {
    session: u128,
    sequence: u64,
    identity: Option<(String, String, String)>,
    previous: Option<(std::time::Instant, u64, bool)>,
    effective_ms: u64,
}
impl Default for RemoteListeningClock {
    fn default() -> Self {
        Self {
            session: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |t| t.as_nanos()),
            sequence: 0,
            identity: None,
            previous: None,
            effective_ms: 0,
        }
    }
}
impl RemoteListeningClock {
    pub fn discontinuity(&mut self) {
        self.previous = None;
    }
    pub fn reset(&mut self) {
        self.identity = None;
        self.discontinuity();
    }
    pub fn observe(&mut self, state: &velo_domain::PlaybackState) -> Option<ProgressSample> {
        let Some(track) = state.track.as_ref() else {
            self.reset();
            return None;
        };
        if !track.uri.starts_with("spotify:track:") {
            self.reset();
            return None;
        }
        let context = state.context_uri.clone().unwrap_or_default();
        let identity = (
            track.uri.clone(),
            context.clone(),
            state
                .device
                .as_ref()
                .and_then(|d| d.id.clone())
                .unwrap_or_default(),
        );
        let repeat = self.previous.is_some_and(|(_, position, playing)| {
            playing
                && state.is_playing
                && state.repeat != velo_domain::RepeatMode::Off
                && position >= track.duration_ms.saturating_sub(6_000)
                && state.progress_ms < 6_000
                && state.progress_ms < position
        });
        if self.identity.as_ref() != Some(&identity) || repeat {
            self.sequence = self.sequence.wrapping_add(1);
            self.identity = Some(identity);
            self.previous = None;
            self.effective_ms = 0;
        }
        if let Some((at, position, playing)) = self.previous {
            let elapsed = state.sampled_at.saturating_duration_since(at).as_millis() as u64;
            let delta = state.progress_ms.saturating_sub(position);
            // No credit for seeks, buffering, or long gaps while minimized/offline.
            if playing
                && state.progress_ms >= position
                && elapsed <= 10_000
                && delta.abs_diff(elapsed) <= 1_500
            {
                self.effective_ms = self.effective_ms.saturating_add(delta.min(elapsed));
            }
        }
        self.previous = Some((state.sampled_at, state.progress_ms, state.is_playing));
        Some(ProgressSample {
            occurrence: format!("connect-{:x}-{}", self.session, self.sequence),
            effective_ms: self.effective_ms,
            dj_voice: false,
            entry: Entry {
                uri: track.uri.clone(),
                title: track.title.clone(),
                artist: track.artist_line(),
                album: track.album.clone(),
                duration_ms: track.duration_ms,
                context_uri: context,
                remote_image_url: velo_domain::pick_image(&track.images, 64)
                    .map(|i| i.url.clone())
                    .unwrap_or_default(),
                artist_id: track
                    .artists
                    .first()
                    .and_then(|a| a.id.clone())
                    .unwrap_or_default(),
                album_id: track.album_id.clone().unwrap_or_default(),
                ..Entry::default()
            },
        })
    }
}

#[derive(Default)]
pub struct ListeningClock {
    current: String,
    recorded: bool,
    private_session: bool,
    maximum: u64,
    seen: std::collections::HashSet<String>,
    order: std::collections::VecDeque<String>,
}
impl ListeningClock {
    pub fn observe(&mut self, sample: &ProgressSample) -> Option<Entry> {
        if sample.occurrence != self.current {
            self.current = sample.occurrence.clone();
            self.recorded = false;
            self.maximum = 0;
        }
        if self.private_session {
            // Remember this occurrence so toggling privacy off cannot record it later.
            self.suppress_current();
            return None;
        }
        if self.seen.contains(&sample.occurrence) {
            return None;
        }
        if !sample.dj_voice {
            self.maximum = self.maximum.max(sample.effective_ms);
        }
        // v0.9.2 (user: "debería registrar todo, hasta las que skipeo rápido"): a song counts
        // as soon as real audio of it has played, not after 30 s.
        let threshold = RECORD_AFTER_MS;
        if self.recorded
            || self.current.is_empty()
            || sample.entry.uri.is_empty()
            || sample.dj_voice
            || threshold == 0
            || self.maximum < threshold
        {
            return None;
        }
        self.recorded = true;
        self.remember_current();
        let mut entry = sample.entry.clone();
        entry.entry_id = sample.occurrence.clone();
        if entry.played_at_ms == 0 {
            entry.played_at_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |t| t.as_millis() as u64);
        }
        Some(entry)
    }
    fn remember_current(&mut self) {
        if !self.current.is_empty() && self.seen.insert(self.current.clone()) {
            self.order.push_back(self.current.clone());
        }
        while self.order.len() > storage::MAX_ENTRIES {
            if let Some(id) = self.order.pop_front() {
                self.seen.remove(&id);
            }
        }
    }
    pub fn suppress_current(&mut self) {
        self.recorded = true;
        self.remember_current();
    }
}

#[derive(Clone, Default)]
pub struct HistoryView {
    pub loading: bool,
    pub has_more: bool,
    pub error: String,
    pub count: i32,
}
pub struct HistoryUi {
    pub op: ModelOp,
    pub view: HistoryView,
    pub cleared: bool,
}
enum Command {
    Account(String),
    Load(bool),
    Append(Box<Entry>, u64),
    Clear(Option<tokio::sync::oneshot::Sender<()>>),
}

pub struct HistoryCtl {
    pub view: watch::Sender<HistoryView>,
    pub ui: Bus<HistoryUi>,
    pub context_times: watch::Sender<std::collections::HashMap<String, u64>>,
    /// Bumped after every publication (append, account switch, clear): event-driven readers.
    pub changes: watch::Sender<u64>,
    commands: mpsc::UnboundedSender<Command>,
    settings: watch::Receiver<velo_domain::Settings>,
    clock: Mutex<ListeningClock>,
    epoch: AtomicU64,
    paging: AtomicBool,
    pub entries: Mutex<Vec<Entry>>,
}
impl HistoryCtl {
    pub fn start(env: &Arc<Env>) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let this = Arc::new(Self {
            view: watch::channel(HistoryView::default()).0,
            ui: Bus::new(),
            context_times: watch::channel(std::collections::HashMap::new()).0,
            changes: watch::channel(0).0,
            commands: tx,
            settings: env.prefs.watch_settings(),
            clock: Mutex::new(ListeningClock::default()),
            epoch: AtomicU64::new(0),
            paging: AtomicBool::new(false),
            entries: Mutex::new(Vec::new()),
        });
        let mut accounts = env.account.view.subscribe();
        let me = this.clone();
        env.rt.spawn(async move {
            let mut previous = None;
            loop {
                let v = accounts.borrow_and_update().clone();
                let account = if v.signed_in {
                    v.user_id
                } else {
                    String::new()
                };
                if previous.as_ref() != Some(&account) {
                    me.epoch.fetch_add(1, Ordering::SeqCst);
                    me.clock
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .suppress_current();
                    let _ = me.commands.send(Command::Account(account.clone()));
                    previous = Some(account);
                }
                if accounts.changed().await.is_err() {
                    break;
                }
            }
        });
        let me = this.clone();
        #[cfg(not(test))]
        let data = directories::ProjectDirs::from("app", "Velo", "Velo")
            .map(|d| d.data_dir().to_path_buf());
        #[cfg(test)]
        let data = Some(env.prefs.config_dir().to_path_buf());
        env.rt.spawn(async move {
            let mut store: Option<Arc<HistoryStore>> = None;
            let mut loaded = 0;
            while let Some(command) = rx.recv().await {
                let loading_command = matches!(command, Command::Load(_));
                let mut cleared = false;
                let mut append = false;
                let mut inserted = false;
                match command {
                    Command::Account(account) => {
                        me.context_times
                            .send_replace(std::collections::HashMap::new());
                        store = data
                            .as_ref()
                            .filter(|_| !account.is_empty())
                            .map(|d| HistoryStore::for_account(d, &account));
                        loaded = 0;
                        me.entries.lock().unwrap_or_else(|e| e.into_inner()).clear();
                        me.publish(Vec::new(), 0, false, false, String::new(), false);
                    }
                    Command::Load(more) => {
                        append = more && loaded > 0;
                        if !append {
                            loaded = 0;
                        }
                    }
                    Command::Append(entry, epoch) => {
                        if epoch != me.epoch.load(Ordering::SeqCst)
                            || me.settings.borrow().playback.private_session
                        {
                            continue;
                        }
                        let Some(s) = &store else { continue };
                        // The app-level epoch above already guards stale entries; commands are
                        // serialized with Clear, so the store epoch cannot change in between.
                        match s.append(*entry, s.epoch()).await {
                            // Loaded list: insert just the new row so the open list keeps
                            // its scroll position and its already-paged rows.
                            Ok(true) if loaded > 0 => inserted = true,
                            Ok(true) => loaded = 0,
                            Ok(false) => continue,
                            Err(_) => {
                                me.publish_error();
                                continue;
                            }
                        }
                    }
                    Command::Clear(done) => {
                        if let Some(s) = &store {
                            if s.clear().await.is_err() {
                                me.publish_error();
                                continue;
                            }
                        }
                        if let Some(done) = done {
                            let _ = done.send(());
                        }
                        loaded = 0;
                        cleared = true;
                    }
                }
                let Some(s) = &store else {
                    me.publish(Vec::new(), 0, false, false, String::new(), cleared);
                    if loading_command {
                        me.paging.store(false, Ordering::SeqCst);
                    }
                    continue;
                };
                me.view.send_modify(|v| v.loading = true);
                match s.page(if inserted { 0 } else { loaded }).await {
                    Ok((rows, count)) => {
                        if !append {
                            if let Ok(times) = s.context_times().await {
                                me.context_times.send_replace(times);
                            }
                        }
                        if inserted {
                            if let Some(first) = rows.into_iter().next() {
                                loaded = (loaded + 1).min(count);
                                me.publish_insert(first, count, loaded < count);
                            }
                        } else {
                            loaded += rows.len();
                            me.publish(rows, count, loaded < count, append, String::new(), cleared);
                        }
                    }
                    Err(_) => me.publish_error(),
                }
                if loading_command {
                    me.paging.store(false, Ordering::SeqCst);
                }
            }
        });
        this
    }

    /// A new newest entry on an already loaded list: `Insert(0)`, no page reload.
    fn publish_insert(&self, entry: Entry, count: usize, has_more: bool) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.insert(0, entry.clone());
        let overflow = entries.len() > count;
        entries.truncate(count);
        drop(entries);
        let view = HistoryView {
            count: count as i32,
            has_more,
            loading: false,
            error: String::new(),
        };
        self.view.send_replace(view.clone());
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
        self.ui.send(HistoryUi {
            op: ModelOp::Insert(0, vec![ui_row(entry)]),
            view: view.clone(),
            cleared: false,
        });
        if overflow {
            self.ui.send(HistoryUi {
                op: ModelOp::Remove(count),
                view,
                cleared: false,
            });
        }
    }
    fn publish_error(&self) {
        self.publish(
            Vec::new(),
            0,
            false,
            false,
            "No se pudo leer o guardar el historial local.".into(),
            false,
        );
    }
    fn publish(
        &self,
        rows: Vec<Entry>,
        count: usize,
        has_more: bool,
        append: bool,
        error: String,
        cleared: bool,
    ) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if !append {
            entries.clear();
        }
        entries.extend(rows.clone());
        let view = HistoryView {
            count: count as i32,
            has_more,
            loading: false,
            error,
        };
        self.view.send_replace(view.clone());
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
        let rows = rows.into_iter().map(ui_row).collect();
        self.ui.send(HistoryUi {
            op: if append {
                ModelOp::Append(rows)
            } else {
                ModelOp::Replace(rows)
            },
            view,
            cleared,
        });
    }
    fn request_page(&self, more: bool) {
        if !self.paging.swap(true, Ordering::SeqCst) {
            self.view.send_modify(|v| {
                v.loading = true;
                v.error.clear();
            });
            let _ = self.commands.send(Command::Load(more));
        }
    }
    pub fn load(&self) {
        self.request_page(false);
    }
    pub fn load_more(&self) {
        self.request_page(true);
    }
    pub fn clear(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .suppress_current();
        let _ = self.commands.send(Command::Clear(None));
    }
    /// Clears the account's history and resolves once the store confirmed it (Settings).
    pub async fn clear_confirmed(&self) -> velo_domain::Result<()> {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .suppress_current();
        let (done, confirmed) = tokio::sync::oneshot::channel();
        let failed = || velo_domain::VeloError::Other("No se pudo borrar el historial".into());
        self.commands
            .send(Command::Clear(Some(done)))
            .map_err(|_| failed())?;
        confirmed.await.map_err(|_| failed())
    }
    pub fn progress(&self, sample: ProgressSample) {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        clock.private_session = self.settings.borrow().playback.private_session;
        let epoch = self.epoch.load(Ordering::SeqCst);
        if let Some(entry) = clock.observe(&sample) {
            let _ = self.commands.send(Command::Append(Box::new(entry), epoch));
        }
    }
    pub fn attach(self: &Arc<Self>, rt: &tokio::runtime::Handle, source: &dyn ProgressSource) {
        let mut rx = source.subscribe();
        let me = self.clone();
        rt.spawn(async move {
            while let Some(sample) = rx.recv().await {
                me.progress(sample);
            }
        });
    }
    pub fn entry(&self, id: &str) -> Option<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|e| e.entry_id == id)
            .cloned()
    }
}

/// Cached file cover when it still exists, else the remote one (the file cache is pruned).
fn display_image(e: &Entry) -> String {
    let local = e.image_url.starts_with("qrc:")
        || url::Url::parse(&e.image_url)
            .ok()
            .and_then(|u| u.to_file_path().ok())
            .is_some_and(|p| p.exists());
    if local {
        e.image_url.clone()
    } else if e.remote_image_url.starts_with("https://") {
        e.remote_image_url.clone()
    } else {
        String::new()
    }
}

fn ui_row(mut e: Entry) -> serde_json::Value {
    e.image_url = display_image(&e);
    e.duration_ms = e.duration_ms.min(i32::MAX as u64);
    serde_json::to_value(e).expect("history entry serializes")
}

/// What the sidebar and the home read from the local history (first loaded page, newest first).
pub struct LocalHistoryReader(pub Arc<HistoryCtl>);
fn entry_track(e: &Entry) -> velo_domain::Track {
    velo_domain::Track {
        id: e.uri.rsplit(':').next().map(str::to_string),
        uri: e.uri.clone(),
        title: e.title.clone(),
        artists: vec![velo_domain::ArtistRef {
            id: Some(e.artist_id.clone()).filter(|id| !id.is_empty()),
            name: e.artist.clone(),
        }],
        album: e.album.clone(),
        album_id: Some(e.album_id.clone()).filter(|id| !id.is_empty()),
        duration_ms: e.duration_ms,
        explicit: false,
        images: Some(e.remote_image_url.clone())
            .filter(|u| u.starts_with("https://"))
            .map(|url| {
                vec![velo_domain::ImageRef {
                    url,
                    width: None,
                    height: None,
                }]
            })
            .unwrap_or_default(),
        added_at: None,
    }
}

/// Tracks ranked by number of plays (ties: the most recent play first). `entries` is newest first.
pub fn top_by_plays(entries: &[Entry], limit: usize) -> Vec<velo_domain::Track> {
    let mut plays = std::collections::HashMap::<&str, (usize, usize)>::new();
    for (i, e) in entries.iter().enumerate() {
        if e.uri.starts_with("spotify:track:") {
            plays.entry(e.uri.as_str()).or_insert((0, i)).0 += 1;
        }
    }
    let mut ranked: Vec<_> = plays.into_iter().collect();
    ranked.sort_by_key(|(_, (count, first))| (std::cmp::Reverse(*count), *first));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, (_, first))| entry_track(&entries[first]))
        .collect()
}

impl crate::engine::sidebar::HistoryReader for LocalHistoryReader {
    fn recent(&self, limit: usize) -> Vec<velo_domain::Track> {
        let entries = self.0.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.iter().take(limit).map(entry_track).collect()
    }
    fn top(&self, limit: usize) -> Vec<velo_domain::Track> {
        let entries = self.0.entries.lock().unwrap_or_else(|e| e.into_inner());
        top_by_plays(&entries, limit)
    }
    fn last_played_contexts(&self) -> std::collections::HashMap<String, i64> {
        self.0
            .context_times
            .borrow()
            .iter()
            .map(|(uri, ms)| (uri.clone(), (*ms).min(i64::MAX as u64) as i64))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn remote(at: std::time::Instant, position: u64) -> velo_domain::PlaybackState {
        velo_domain::PlaybackState {
            track: Some(velo_domain::Track {
                id: Some("a".into()),
                uri: "spotify:track:a".into(),
                title: "A".into(),
                artists: vec![],
                album: "Album".into(),
                album_id: Some("album".into()),
                duration_ms: 60_000,
                explicit: false,
                images: vec![],
                added_at: None,
            }),
            sampled_at: at,
            progress_ms: position,
            is_playing: true,
            context_uri: Some("spotify:playlist:p".into()),
            device: None,
            shuffle: false,
            repeat: velo_domain::RepeatMode::Track,
        }
    }
    #[test]
    fn v92_connect_records_once_at_threshold_and_records_real_repeat() {
        let mut remote_clock = RemoteListeningClock::default();
        let mut clock = ListeningClock::default();
        let at = std::time::Instant::now();
        for i in 0..=6 {
            let s = remote_clock
                .observe(&remote(
                    at + std::time::Duration::from_secs(i * 5),
                    i * 5_000,
                ))
                .unwrap();
            let entry = clock.observe(&s);
            // v0.9.2: the first observation with real progress records it (quick skips count).
            assert_eq!(entry.is_some(), i == 1);
            if let Some(e) = entry {
                assert_eq!(e.context_uri, "spotify:playlist:p");
            }
        }
        let first = remote_clock
            .observe(&remote(at + std::time::Duration::from_secs(35), 35_000))
            .unwrap();
        assert!(clock.observe(&first).is_none());
        remote_clock.observe(&remote(at + std::time::Duration::from_secs(59), 59_000));
        let second = remote_clock
            .observe(&remote(at + std::time::Duration::from_secs(61), 1_000))
            .unwrap();
        assert_ne!(first.occurrence, second.occurrence);
        assert_eq!(second.effective_ms, 0);
        for i in 1..=6 {
            let s = remote_clock
                .observe(&remote(
                    at + std::time::Duration::from_secs(61 + i * 5),
                    1_000 + i * 5_000,
                ))
                .unwrap();
            assert_eq!(clock.observe(&s).is_some(), i == 1);
        }
    }
    #[test]
    fn v92_connect_excludes_seek_pause_buffering_and_unobserved_gaps() {
        let mut clock = RemoteListeningClock::default();
        let at = std::time::Instant::now();
        let mut p = remote(at, 0);
        let id = clock.observe(&p).unwrap().occurrence;
        for (seconds, position, playing) in [
            (5, 5_000, true),
            (10, 45_000, true),
            (15, 45_000, true),
            (20, 46_000, false),
            (100, 46_000, false),
            (105, 46_000, true),
            (110, 51_000, true),
        ] {
            p.sampled_at = at + std::time::Duration::from_secs(seconds);
            p.progress_ms = position;
            p.is_playing = playing;
            let s = clock.observe(&p).unwrap();
            assert_eq!(s.occurrence, id);
        }
        assert_eq!(clock.effective_ms, 10_000);
        clock.discontinuity();
        p.sampled_at += std::time::Duration::from_secs(5);
        p.progress_ms = 56_000;
        assert_eq!(clock.observe(&p).unwrap().effective_ms, 10_000);
        clock.reset();
        assert_ne!(clock.observe(&p).unwrap().occurrence, id);
    }
    #[test]
    fn top_by_plays_ranks_by_count_then_recency_and_skips_non_tracks() {
        let e = |uri: &str| Entry {
            uri: uri.into(),
            title: uri.into(),
            ..Default::default()
        };
        let entries = [
            e("spotify:track:b"),
            e("spotify:track:a"),
            e("spotify:episode:x"),
            e("spotify:track:a"),
            e("spotify:track:c"),
        ];
        let top = top_by_plays(&entries, 2);
        assert_eq!(
            top.iter().map(|t| t.uri.as_str()).collect::<Vec<_>>(),
            ["spotify:track:a", "spotify:track:b"]
        );
        assert!(top_by_plays(&[], 5).is_empty());
    }

    #[test]
    fn missing_cached_cover_falls_back_to_the_remote_one() {
        let mut e = Entry {
            image_url: "file:///definitely/missing/cover.jpg".into(),
            remote_image_url: "https://i.scdn.co/image/x".into(),
            ..Default::default()
        };
        assert_eq!(display_image(&e), "https://i.scdn.co/image/x");
        e.remote_image_url = "http://insecure".into();
        assert_eq!(display_image(&e), "");
    }

    fn sample(id: &str, ms: u64) -> ProgressSample {
        ProgressSample {
            occurrence: id.into(),
            effective_ms: ms,
            entry: Entry {
                uri: "spotify:track:a".into(),
                duration_ms: 100_000,
                context_uri: "spotify:playlist:p".into(),
                ..Entry::default()
            },
            dj_voice: false,
        }
    }
    #[test]
    fn t6_private_session_never_queues_history_and_does_not_backfill() {
        let mut settings = velo_domain::Settings::default();
        settings.playback.private_session = true;
        let (settings, preferences) = watch::channel(settings);
        let (commands, mut rx) = mpsc::unbounded_channel();
        let ctl = HistoryCtl {
            view: watch::channel(HistoryView::default()).0,
            ui: Bus::new(),
            context_times: watch::channel(std::collections::HashMap::new()).0,
            changes: watch::channel(0).0,
            commands,
            settings: preferences,
            clock: Mutex::new(ListeningClock::default()),
            epoch: AtomicU64::new(0),
            paging: AtomicBool::new(false),
            entries: Mutex::new(Vec::new()),
        };
        for id in ["local:1", "connect:1"] {
            ctl.progress(sample(id, 0));
            ctl.progress(sample(id, RECORD_AFTER_MS));
            ctl.progress(sample(id, 30_000));
        }
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        settings.send_modify(|s| s.playback.private_session = false);
        // Seek/resume or late source events cannot backfill private occurrences.
        ctl.progress(sample("local:1", 40_000));
        ctl.progress(sample("connect:1", 40_000));
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        ctl.progress(sample("local:2", RECORD_AFTER_MS));
        assert!(matches!(rx.try_recv(), Ok(Command::Append(e, 0)) if e.entry_id == "local:2"));
        // Activating while a track is loaded also suppresses that occurrence.
        ctl.progress(sample("local:3", 0));
        settings.send_modify(|s| s.playback.private_session = true);
        ctl.progress(sample("local:3", RECORD_AFTER_MS));
        settings.send_modify(|s| s.playback.private_session = false);
        ctl.progress(sample("local:3", 30_000));
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        ctl.progress(sample("connect:2", RECORD_AFTER_MS));
        assert!(matches!(rx.try_recv(), Ok(Command::Append(e, 0)) if e.entry_id == "connect:2"));
    }
    #[test]
    fn fake_consumed_clock_pause_buffer_seek_dedupe_repeat_clear() {
        let mut clock = ListeningClock::default();
        // No audio yet (loaded, buffering): not listened.
        assert!(clock.observe(&sample("request:1", 0)).is_none());
        // v0.9.2: the first real audio records it, once per occurrence (pause/seek/repeat events).
        let e = clock.observe(&sample("request:1", 250)).unwrap();
        assert_eq!(e.context_uri, "spotify:playlist:p");
        for _ in 0..3 {
            assert!(clock.observe(&sample("request:1", 35_000)).is_none());
        }
        // A quick skip still counts.
        assert!(clock.observe(&sample("request:2", 250)).is_some());
        clock.observe(&sample("request:3", 0));
        clock.suppress_current();
        assert!(clock.observe(&sample("request:3", 30_000)).is_none());
        assert!(clock.observe(&sample("request:4", 250)).is_some());
    }
    #[test]
    fn new_entry_on_loaded_list_is_a_front_insert_not_a_replace() {
        let (commands, _rx) = mpsc::unbounded_channel();
        let ctl = HistoryCtl {
            view: watch::channel(HistoryView::default()).0,
            ui: Bus::new(),
            context_times: watch::channel(std::collections::HashMap::new()).0,
            changes: watch::channel(0).0,
            commands,
            settings: watch::channel(velo_domain::Settings::default()).1,
            clock: Mutex::new(ListeningClock::default()),
            epoch: AtomicU64::new(0),
            paging: AtomicBool::new(false),
            entries: Mutex::new(vec![Entry::default()]),
        };
        let mut ui = ctl.ui.take().unwrap();
        ctl.publish_insert(
            Entry {
                uri: "spotify:track:n".into(),
                ..Entry::default()
            },
            51,
            true,
        );
        let event = ui.try_recv().unwrap();
        assert!(matches!(&event.op, ModelOp::Insert(0, rows) if rows.len() == 1));
        assert_eq!(event.view.count, 51);
        assert_eq!(ctl.entries.lock().unwrap().len(), 2);
        assert_eq!(ctl.entries.lock().unwrap()[0].uri, "spotify:track:n");
        // The store can evict the oldest entry at its cap: the loaded model must follow.
        ctl.publish_insert(
            Entry {
                uri: "spotify:track:newer".into(),
                ..Entry::default()
            },
            2,
            false,
        );
        assert!(matches!(ui.try_recv().unwrap().op, ModelOp::Insert(0, _)));
        assert!(matches!(ui.try_recv().unwrap().op, ModelOp::Remove(2)));
        assert_eq!(ctl.entries.lock().unwrap().len(), 2);
        ctl.load();
        assert!(
            ctl.view.borrow().loading,
            "loading is visible before worker starts"
        );
    }
    #[test]
    fn short_tracks_missing_uri_and_dj_voice() {
        let mut clock = ListeningClock::default();
        let mut s = sample("1", 0);
        s.entry.duration_ms = 20_000;
        assert!(clock.observe(&s).is_none());
        s.effective_ms = 250;
        assert!(clock.observe(&s).is_some());
        s.occurrence = "2".into();
        s.entry.uri.clear();
        assert!(clock.observe(&s).is_none());
        s = sample("3", 30_000);
        s.dj_voice = true;
        assert!(clock.observe(&s).is_none());
    }
}
