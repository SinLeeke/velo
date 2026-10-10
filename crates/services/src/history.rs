//! Account-local listening history. Disk work is serialized on blocking workers, never Qt.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub const PAGE_SIZE: usize = 50;
pub const MAX_ENTRIES: usize = 10_000;
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub entry_id: String,
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub image_url: String,
    /// Remote cover (https), kept next to the cached file URL, which can be pruned.
    #[serde(default)]
    pub remote_image_url: String,
    pub artist_id: String,
    pub album_id: String,
    pub context_uri: String,
    pub duration_ms: u64,
    pub played_at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    entries: Vec<Entry>,
}

pub struct HistoryStore {
    path: PathBuf,
    entries: Mutex<Option<Vec<Entry>>>,
    epoch: AtomicU64,
}

impl HistoryStore {
    pub fn for_account(data_dir: &Path, account: &str) -> Arc<Self> {
        // Injective encoding: no traversal, no dependency on a randomized hasher.
        let key = account
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Self::at(data_dir.join("history").join(key).join("history.json"))
    }

    pub fn at(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            entries: Mutex::new(None),
            epoch: AtomicU64::new(0),
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Sidebar recency groups by the actual playback context, never the track's album.
    pub async fn context_times(
        self: &Arc<Self>,
    ) -> io::Result<std::collections::HashMap<String, u64>> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut cached = this.entries.lock().unwrap_or_else(|e| e.into_inner());
            if cached.is_none() {
                *cached = Some(this.read()?);
            }
            let mut times = std::collections::HashMap::<String, u64>::new();
            for e in cached.as_ref().expect("loaded") {
                if !e.context_uri.is_empty() {
                    times
                        .entry(e.context_uri.clone())
                        .and_modify(|t| *t = (*t).max(e.played_at_ms))
                        .or_insert(e.played_at_ms);
                }
            }
            Ok(times)
        })
        .await
        .map_err(io::Error::other)?
    }

    fn read(&self) -> io::Result<Vec<Entry>> {
        match fs::metadata(&self.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
            Ok(m) if m.len() > MAX_BYTES => {
                return Err(io::Error::other("History exceeds size limit"));
            }
            _ => {}
        }
        let mut doc: Document = serde_json::from_slice(&fs::read(&self.path)?)?;
        if doc.version != 1 {
            return Err(io::Error::other("Unknown history version"));
        }
        let mut seen = HashSet::new();
        doc.entries.retain(|e| {
            !e.uri.is_empty() && !e.entry_id.is_empty() && seen.insert(e.entry_id.clone())
        });
        doc.entries
            .sort_by_key(|e| std::cmp::Reverse(e.played_at_ms));
        doc.entries.truncate(MAX_ENTRIES);
        Ok(doc.entries)
    }

    fn write(&self, entries: &[Entry]) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("Missing history directory"))?;
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec(&Document {
            version: 1,
            entries: entries.to_vec(),
        })?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(io::Error::other("History exceeds size limit"));
        }
        let temporary = self.path.with_extension("json.tmp");
        use std::io::Write;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Windows rename replaces a file atomically (MoveFileExW); no delete-before-rename.
        fs::rename(temporary, &self.path)
    }

    pub async fn page(self: &Arc<Self>, offset: usize) -> io::Result<(Vec<Entry>, usize)> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut cached = this.entries.lock().unwrap_or_else(|e| e.into_inner());
            if cached.is_none() {
                *cached = Some(this.read()?);
            }
            let entries = cached.as_ref().expect("loaded");
            Ok((
                entries
                    .iter()
                    .skip(offset)
                    .take(PAGE_SIZE)
                    .cloned()
                    .collect(),
                entries.len(),
            ))
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn append(self: &Arc<Self>, entry: Entry, epoch: u64) -> io::Result<bool> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut cached = this.entries.lock().unwrap_or_else(|e| e.into_inner());
            if this.epoch() != epoch || entry.uri.is_empty() || entry.entry_id.is_empty() {
                return Ok(false);
            }
            if cached.is_none() {
                *cached = Some(match this.read() {
                    Ok(entries) => entries,
                    // Corrupt, unknown version or oversize: keep the file as .bak and start
                    // empty, so listening is never lost forever to a permanent error.
                    Err(_) => {
                        fs::rename(&this.path, this.path.with_extension("json.bak"))?;
                        Vec::new()
                    }
                });
            }
            let mut next = cached.as_ref().expect("loaded").clone();
            if next.iter().any(|e| e.entry_id == entry.entry_id) {
                return Ok(false);
            }
            next.insert(0, entry);
            next.truncate(MAX_ENTRIES);
            this.write(&next)?;
            *cached = Some(next);
            Ok(true)
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Invalidates queued appends immediately, before the blocking worker is scheduled.
    pub fn invalidate(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub async fn clear(self: &Arc<Self>) -> io::Result<()> {
        self.invalidate();
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut cached = this.entries.lock().unwrap_or_else(|e| e.into_inner());
            this.write(&[])?;
            *cached = Some(Vec::new());
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(i: usize) -> Entry {
        Entry {
            entry_id: i.to_string(),
            uri: "spotify:track:a".into(),
            played_at_ms: i as u64,
            ..Entry::default()
        }
    }
    #[tokio::test]
    async fn paginates_deduplicates_and_is_account_local() {
        let dir = tempfile::tempdir().unwrap();
        let a = HistoryStore::for_account(dir.path(), "../alice");
        let b = HistoryStore::for_account(dir.path(), "bob");
        for i in 0..103 {
            assert!(a.append(entry(i), 0).await.unwrap());
        }
        assert!(!a.append(entry(10), 0).await.unwrap());
        assert_eq!(a.page(0).await.unwrap().0.len(), 50);
        assert_eq!(a.page(50).await.unwrap().0.len(), 50);
        assert_eq!(a.page(100).await.unwrap().0.len(), 3);
        assert_eq!(b.page(0).await.unwrap().1, 0);
        assert_eq!(
            HistoryStore::at(a.path.clone()).page(0).await.unwrap().1,
            103
        );
    }
    #[tokio::test]
    async fn clear_invalidates_pending_append_and_corrupt_json_can_be_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.json"));
        store.append(entry(1), 0).await.unwrap();
        let pending = store.append(entry(2), 0);
        store.clear().await.unwrap();
        assert!(!pending.await.unwrap());
        assert_eq!(store.page(0).await.unwrap().1, 0);
        fs::write(&store.path, b"{broken").unwrap();
        let corrupt = HistoryStore::at(store.path.clone());
        assert!(corrupt.page(0).await.is_err());
        corrupt.clear().await.unwrap();
        assert_eq!(corrupt.page(0).await.unwrap().1, 0);
    }
    #[tokio::test]
    async fn append_over_a_corrupt_file_keeps_a_backup_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        fs::write(&path, b"{broken").unwrap();
        let store = HistoryStore::at(path.clone());
        assert!(store.append(entry(1), 0).await.unwrap());
        assert_eq!(store.page(0).await.unwrap().1, 1);
        assert_eq!(
            fs::read(path.with_extension("json.bak")).unwrap(),
            b"{broken"
        );
        fs::write(&path, br#"{"version":9,"entries":[]}"#).unwrap();
        let store = HistoryStore::at(path.clone());
        assert!(store.append(entry(2), 0).await.unwrap());
        assert_eq!(store.page(0).await.unwrap().1, 1);
    }
    #[tokio::test]
    async fn clear_wins_against_concurrently_scheduled_old_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.json"));
        let mut writers = Vec::new();
        for i in 0..32 {
            let s = store.clone();
            writers.push(tokio::spawn(
                async move { s.append(entry(i), 0).await.unwrap() },
            ));
        }
        store.clear().await.unwrap();
        for writer in writers {
            writer.await.unwrap();
        }
        assert_eq!(store.page(0).await.unwrap().1, 0);
        assert_eq!(
            HistoryStore::at(store.path.clone())
                .page(0)
                .await
                .unwrap()
                .1,
            0
        );
    }
    #[tokio::test]
    async fn version_cap_and_duplicate_file_entries_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let entries = (0..MAX_ENTRIES + 5).map(entry).chain([entry(5)]).collect();
        fs::write(
            &path,
            serde_json::to_vec(&Document {
                version: 1,
                entries,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            HistoryStore::at(path.clone()).page(0).await.unwrap().1,
            MAX_ENTRIES
        );
        fs::write(&path, br#"{"version":2,"entries":[]}"#).unwrap();
        assert!(HistoryStore::at(path).page(0).await.is_err());
    }
}
