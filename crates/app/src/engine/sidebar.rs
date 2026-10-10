//! Unified library. Filtering, sorting, metadata and image work run on Tokio, never Qt.
use super::{
    Env,
    rows::{THUMB_PX, file_url, image_of},
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use velo_domain::{CoverArtService, MusicProvider, Playlist, Track};
use velo_player::session_api::LibraryEntry;

/// Rows whose cover is loaded without being asked for (bounded; the rest wait for a request).
const PREFETCH_ROWS: usize = 300;
pub const ROLES: &str =
    "kind,objectId,uri,name,creator,imageUrl,pinned,owned,addedAtMs,lastPlayedAtMs";

/// C3 integration point. Read by contextUri, never infer context from the track's album.
pub trait HistoryReader: Send + Sync {
    fn recent(&self, limit: usize) -> Vec<Track>;
    fn last_played_contexts(&self) -> HashMap<String, i64>;
    /// Most played tracks of the local history (G3: Inicio when Spotify gives no top list).
    fn top(&self, _limit: usize) -> Vec<Track> {
        Vec::new()
    }
}
pub struct EmptyHistory;
impl HistoryReader for EmptyHistory {
    fn recent(&self, _: usize) -> Vec<Track> {
        Vec::new()
    }
    fn last_played_contexts(&self) -> HashMap<String, i64> {
        HashMap::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SidebarView {
    pub rows: Vec<Value>,
    pub filter: String,
    pub query: String,
    pub status: String,
    pub error: String,
    pub shows_status: String,
}
impl Default for SidebarView {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            filter: "all".into(),
            query: String::new(),
            status: "none".into(),
            error: String::new(),
            shows_status: "none".into(),
        }
    }
}
#[derive(Default)]
struct Inner {
    generation: u64,
    rows: Vec<Value>,
    images: HashMap<String, Option<String>>,
    pins: Vec<String>,
    last_played: HashMap<String, i64>,
    requested: HashSet<String>,
    cached_images: HashMap<String, String>,
    task: Option<tokio::task::AbortHandle>,
}
pub struct SidebarCtl {
    env: Arc<Env>,
    inner: Mutex<Inner>,
    permits: Arc<tokio::sync::Semaphore>,
    weak: std::sync::Weak<Self>,
    pub view: watch::Sender<SidebarView>,
}

/// Unknown dates sort after known dates; pin grouping is independent of the chosen order.
pub fn select(rows: &[Value], filter: &str, query: &str, sort: &str) -> Vec<Value> {
    let query = query.trim().to_lowercase();
    let mut selected: Vec<_> = rows
        .iter()
        .filter(|r| {
            (filter == "all" || r["kind"] == filter)
                && (query.is_empty()
                    || ["name", "creator"].iter().any(|k| {
                        r[*k]
                            .as_str()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&query)
                    }))
        })
        .cloned()
        .collect();
    let text = |r: &Value, key: &str| r[key].as_str().unwrap_or_default().to_lowercase();
    selected.sort_by(|a, b| {
        b["pinned"]
            .as_bool()
            .cmp(&a["pinned"].as_bool())
            .then_with(|| match sort {
                "recent_added" => b["addedAtMs"].as_i64().cmp(&a["addedAtMs"].as_i64()),
                "creator" => text(a, "creator").cmp(&text(b, "creator")),
                "alphabetical" => std::cmp::Ordering::Equal,
                _ => b["lastPlayedAtMs"]
                    .as_i64()
                    .cmp(&a["lastPlayedAtMs"].as_i64()),
            })
            .then_with(|| text(a, "name").cmp(&text(b, "name")))
            .then_with(|| a["uri"].as_str().cmp(&b["uri"].as_str()))
    });
    selected
}

fn row(e: &LibraryEntry, owned: bool) -> Value {
    json!({"kind":e.kind,"objectId":e.object_id,"uri":e.uri,"name":e.name,"creator":e.creator,
        "imageUrl":"","pinned":false,"owned":owned,"addedAtMs":e.added_at.unwrap_or(-1),"lastPlayedAtMs":-1})
}

impl SidebarCtl {
    pub fn new(env: &Arc<Env>) -> Arc<Self> {
        let pins = env
            .prefs
            .extras()
            .pinned_playlists
            .iter()
            .map(|id| format!("spotify:playlist:{id}"))
            .collect();
        let this = Arc::new_cyclic(|weak| Self {
            env: env.clone(),
            weak: weak.clone(),
            inner: Mutex::new(Inner {
                pins,
                ..Default::default()
            }),
            permits: Arc::new(tokio::sync::Semaphore::new(4)),
            view: watch::channel(SidebarView::default()).0,
        });
        let weak = Arc::downgrade(&this);
        let mut settings = env.prefs.watch_settings();
        env.rt.spawn(async move {
            loop {
                settings.borrow_and_update();
                let Some(this) = weak.upgrade() else { break };
                {
                    let mut inner = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                    this.publish(&mut inner);
                }
                drop(this);
                if settings.changed().await.is_err() {
                    break;
                }
            }
        });
        this
    }
    fn publish(&self, inner: &mut Inner) {
        // C1 owns library.sort/pinned_uris. Schema-neutral reading keeps this worktree buildable.
        let settings = serde_json::to_value(self.env.prefs.settings()).unwrap_or_default();
        if let Some(pins) = settings["library"]["pinned_uris"].as_array() {
            inner.pins = pins
                .iter()
                .filter_map(|p| p.as_str().map(str::to_owned))
                .collect();
        }
        for row in &mut inner.rows {
            let uri = row["uri"].as_str().unwrap_or_default().to_owned();
            row["pinned"] = json!(inner.pins.contains(&uri));
            row["lastPlayedAtMs"] = json!(inner.last_played.get(&uri).copied().unwrap_or(-1));
        }
        self.view.send_if_modified(|v| {
            let rows = select(
                &inner.rows,
                &v.filter,
                &v.query,
                settings["library"]["sort"]
                    .as_str()
                    .unwrap_or("recent_played"),
            );
            if v.rows == rows {
                false
            } else {
                v.rows = rows;
                true
            }
        });
        // Nothing in QML asks for covers, so rows get theirs here, in view order.
        if let Some(this) = self.weak.upgrade() {
            let uris: Vec<String> = self
                .view
                .borrow()
                .rows
                .iter()
                .filter(|r| r["imageUrl"] == "")
                .filter_map(|r| r["uri"].as_str())
                .filter(|u| !inner.requested.contains(*u))
                .map(str::to_owned)
                .take(PREFETCH_ROWS)
                .collect();
            for uri in uris {
                this.request_in(inner, uri);
            }
        }
    }
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        let generation = inner.generation + 1;
        *inner = Inner {
            generation,
            ..Default::default()
        };
        self.view.send_replace(SidebarView::default());
    }
    pub fn set_playlists(self: &Arc<Self>, playlists: &[Playlist], user: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.rows.retain(|r| r["kind"] != "playlist");
        let mut seen = HashSet::new();
        for p in playlists {
            if !seen.insert(p.uri.clone()) {
                continue;
            }
            let e = LibraryEntry {
                kind: "playlist".into(),
                object_id: p.id.clone(),
                uri: p.uri.clone(),
                name: p.name.clone(),
                creator: p.owner.clone(),
                images: p.images.clone(),
                added_at: None,
            };
            let mut r = row(&e, !user.is_empty() && p.owner_id == user);
            if let Some(url) = inner.cached_images.get(&e.uri) {
                r["imageUrl"] = json!(url);
            }
            inner.rows.push(r);
            inner.images.insert(p.uri.clone(), image_of(&p.images));
        }
        self.publish(&mut inner);
    }
    pub fn pinned_playlists(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pins
            .iter()
            .filter_map(|u| u.strip_prefix("spotify:playlist:").map(str::to_owned))
            .collect()
    }
    pub fn history_changed(&self, contexts: HashMap<String, i64>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.last_played = contexts;
        self.publish(&mut inner);
    }
    pub fn filter(&self, filter: String) {
        if !matches!(
            filter.as_str(),
            "all" | "playlist" | "album" | "artist" | "show"
        ) {
            return;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.view.send_modify(|v| v.filter = filter);
        self.publish(&mut inner);
    }
    pub fn search(&self, query: String) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.view.send_modify(|v| v.query = query);
        self.publish(&mut inner);
    }
    pub fn toggle_pin(&self, uri: String) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.rows.iter().any(|r| r["uri"] == uri) {
            return;
        }
        if let Some(i) = inner.pins.iter().position(|p| p == &uri) {
            inner.pins.remove(i);
        } else {
            inner.pins.push(uri);
        }
        let pins = inner.pins.clone();
        self.env.prefs.update_settings(|s| {
            if let Ok(mut value) = serde_json::to_value(&*s) {
                if value["library"].is_object() {
                    value["library"]["pinned_uris"] = json!(pins);
                    if let Ok(next) = serde_json::from_value(value) {
                        *s = next;
                    }
                }
            }
        });
        self.publish(&mut inner);
    }
    pub fn refresh(self: &Arc<Self>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        inner.generation += 1;
        inner.requested.clear();
        let ticket = inner.generation;
        self.view.send_modify(|v| {
            v.status = "loading".into();
            v.error.clear();
            v.shows_status = "loading".into();
        });
        let this = self.clone();
        inner.task = Some(
            self.env
                .rt
                .spawn(async move {
                    let collection = if this.env.account.session_mode() {
                        velo_player::session_api::SessionApi::new(this.env.audio.session_source())
                            .sidebar_collection()
                            .await
                    } else {
                        fallback_collection(this.env.account.api()).await
                    };
                    let shows = if let Some(engine) = this
                        .env
                        .audio
                        .engine()
                        .filter(|_| this.env.audio.is_ready())
                    {
                        velo_player::session_api::shows::saved(engine.session()).await
                    } else {
                        Err(velo_domain::VeloError::NotAuthenticated)
                    };
                    let mut inner = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                    if inner.generation != ticket {
                        return;
                    }
                    inner.task = None;
                    let failed = collection.is_err();
                    let shows_failed = shows.is_err();
                    let shows_empty = shows.as_ref().is_ok_and(Vec::is_empty);
                    inner.rows.retain(|r| r["kind"] == "playlist");
                    for e in collection
                        .unwrap_or_default()
                        .into_iter()
                        .chain(shows.unwrap_or_default())
                    {
                        if inner.rows.iter().any(|r| r["uri"] == e.uri) {
                            continue;
                        }
                        inner.images.insert(e.uri.clone(), image_of(&e.images));
                        let mut r = row(&e, false);
                        if let Some(url) = inner.cached_images.get(&e.uri) {
                            r["imageUrl"] = json!(url);
                        }
                        inner.rows.push(r);
                    }
                    this.view.send_modify(|v| {
                        v.status = if failed && inner.rows.is_empty() {
                            "unavailable"
                        } else if failed || shows_failed {
                            "partial"
                        } else {
                            "ready"
                        }
                        .into();
                        v.shows_status = if shows_failed {
                            "unavailable"
                        } else if shows_empty {
                            "none"
                        } else {
                            "ready"
                        }
                        .into();
                    });
                    this.publish(&mut inner);
                })
                .abort_handle(),
        );
    }
    /// Requested by a visible delegate or opening a playlist, never for all mosaics at startup.
    pub fn request_image(self: &Arc<Self>, uri: String) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.request_in(&mut inner, uri);
    }
    fn request_in(self: &Arc<Self>, inner: &mut Inner, uri: String) {
        if !inner.rows.iter().any(|r| r["uri"] == uri) || !inner.requested.insert(uri.clone()) {
            return;
        }
        let image = inner.images.get(&uri).cloned().flatten();
        let ticket = inner.generation;
        let this = self.clone();
        self.env.rt.spawn(async move {
            let Ok(_permit) = this.permits.clone().acquire_owned().await else {
                return;
            };
            if this
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .generation
                != ticket
            {
                return;
            }
            let path = if let Some(url) = image {
                this.env.covers.thumbnail(&url, THUMB_PX).await.ok()
            } else {
                this.mosaic(&uri).await
            };
            let mut inner = this.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.generation != ticket {
                return;
            }
            if let Some(path) = path {
                inner.cached_images.insert(uri.clone(), file_url(&path));
                if let Some(row) = inner.rows.iter_mut().find(|r| r["uri"] == uri) {
                    row["imageUrl"] = json!(file_url(&path));
                }
                this.publish(&mut inner);
            }
        });
    }
    async fn mosaic(&self, uri: &str) -> Option<std::path::PathBuf> {
        let id = uri.strip_prefix("spotify:playlist:")?;
        if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        // Cached composition is read before fetching playlist tracks on later sessions.
        let cache = directories::ProjectDirs::from("app", "Velo", "Velo")?
            .cache_dir()
            .join("sidebar-mosaics");
        let path = cache.join(format!("{id}.png"));
        if tokio::fs::metadata(&path).await.is_ok() {
            return Some(path);
        }
        let api = self.env.account.api()?;
        let tracks = api.playlist_tracks(id, 0, 8).await.ok()?;
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        for url in tracks
            .items
            .iter()
            .filter_map(|t| image_of(&t.images))
            .filter(|u| seen.insert(u.clone()))
            .take(4)
        {
            if let Ok(path) = self.env.covers.thumbnail(&url, THUMB_PX).await {
                paths.push(path);
            }
        }
        if paths.is_empty() {
            return None;
        }
        let path_copy = path.clone();
        tokio::task::spawn_blocking(move || {
            let bytes = compose(&paths)?;
            std::fs::create_dir_all(&cache).ok()?;
            // At most 256 demanded mosaics; prune oldest on demand, never periodically.
            let mut entries: Vec<_> = std::fs::read_dir(&cache)
                .ok()?
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
                .map(|e| (e.metadata().and_then(|m| m.modified()).ok(), e.path()))
                .collect();
            entries.sort_by_key(|e| e.0);
            let excess = entries.len().saturating_sub(255);
            for (_, file) in entries.into_iter().take(excess) {
                let _ = std::fs::remove_file(file);
            }
            let temp = path_copy.with_extension("tmp");
            std::fs::write(&temp, bytes).ok()?;
            std::fs::rename(&temp, &path_copy).ok()?;
            Some(path_copy)
        })
        .await
        .ok()?
    }
}

fn compose(paths: &[std::path::PathBuf]) -> Option<Vec<u8>> {
    let mut canvas = image::RgbaImage::new(THUMB_PX, THUMB_PX);
    let side = if paths.len() == 1 {
        THUMB_PX
    } else {
        THUMB_PX / 2
    };
    for n in 0..if paths.len() == 1 { 1 } else { 4 } {
        let source = image::open(&paths[n % paths.len()])
            .ok()?
            .resize_to_fill(side, side, image::imageops::FilterType::Triangle)
            .to_rgba8();
        image::imageops::replace(
            &mut canvas,
            &source,
            i64::from((n as u32 % 2) * side),
            i64::from((n as u32 / 2) * side),
        );
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

async fn fallback_collection(
    api: Option<Arc<dyn MusicProvider>>,
) -> velo_domain::Result<Vec<LibraryEntry>> {
    let api = api.ok_or(velo_domain::VeloError::NotAuthenticated)?;
    let mut rows = Vec::new();
    let mut offset = 0;
    while offset < 10_000 {
        let page = api.saved_albums(offset, 50).await?;
        let more = page.has_more();
        offset += page.limit.max(1);
        rows.extend(page.items.into_iter().map(|a| {
            LibraryEntry {
                kind: "album".into(),
                object_id: a.id,
                uri: a.uri,
                name: a.name,
                creator: a
                    .artists
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                images: a.images,
                added_at: None,
            }
        }));
        if !more {
            break;
        }
    }
    rows.extend(
        api.followed_artists(10_000)
            .await?
            .into_iter()
            .map(|a| LibraryEntry {
                kind: "artist".into(),
                object_id: a.id,
                uri: a.uri,
                name: a.name,
                creator: String::new(),
                images: a.images,
                added_at: None,
            }),
    );
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_queries_pin_groups_and_unknown_dates() {
        let rows = vec![
            json!({"uri":"z","name":"Zebra","creator":"Maker","kind":"album","pinned":false,"addedAtMs":-1,"lastPlayedAtMs":-1}),
            json!({"uri":"a","name":"Alpha","creator":"Other","kind":"artist","pinned":false,"addedAtMs":42,"lastPlayedAtMs":10}),
            json!({"uri":"p","name":"Pinned","creator":"Maker","kind":"show","pinned":true,"addedAtMs":-1,"lastPlayedAtMs":-1}),
        ];
        for sort in ["recent_added", "recent_played", "alphabetical", "creator"] {
            assert_eq!(select(&rows, "all", "", sort)[0]["uri"], "p");
        }
        assert_eq!(select(&rows, "all", "", "recent_added")[2]["uri"], "z");
        assert_eq!(select(&rows, "album", " MAKER ", "creator").len(), 1);
        assert_eq!(select(&rows, "artist", "alpha", "alphabetical").len(), 1);
        assert!(select(&rows, "playlist", "", "recent_played").is_empty());
        assert_eq!(select(&rows, "show", "", "recent_played").len(), 1);
    }
    #[test]
    fn mosaics_are_local_and_use_at_most_four_tiles() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..4)
            .map(|n| {
                let path = dir.path().join(format!("{n}.png"));
                image::RgbaImage::from_pixel(2, 2, image::Rgba([n, 0, 0, 255]))
                    .save(&path)
                    .unwrap();
                path
            })
            .collect();
        let bytes = compose(&paths).unwrap();
        let out = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (128, 128));
        assert_eq!(out.get_pixel(100, 100)[0], 3);
    }
}
