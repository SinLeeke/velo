use std::path::PathBuf;
use std::sync::{Mutex, RwLock, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use velo_domain::{Result, Settings, SettingsStore};

use crate::{atomic_write, error, project_dirs};

#[path = "proxy.rs"]
pub mod proxy;

enum Message {
    Save(Box<Settings>),
    Flush,
}

/// Construct asynchronously; `load` only reads the in-memory snapshot.
/// Drop performs a blocking flush and joins the writer. Drop it outside an async
/// thread (for example with `spawn_blocking`) to preserve pending saves safely.
pub struct TomlSettingsStore {
    path: PathBuf,
    snapshot: RwLock<Settings>,
    sender: mpsc::Sender<Message>,
    worker: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl TomlSettingsStore {
    pub async fn new() -> Result<Self> {
        Self::with_path(project_dirs()?.config_dir().join("settings.toml")).await
    }

    /// `path` is the full settings file path.
    pub async fn with_path(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let snapshot = match tokio::fs::read(&path).await {
            Ok(bytes) => match tokio::task::spawn_blocking(move || {
                String::from_utf8(bytes)
                    .map_err(error)
                    .and_then(|text| toml::from_str::<Settings>(&text).map_err(error))
            })
            .await
            .map_err(error)?
            {
                Ok(mut settings) => {
                    settings.normalize();
                    settings
                }
                Err(_) => {
                    if let Err(e) = tokio::fs::rename(&path, path.with_extension("toml.bak")).await
                    {
                        tracing::warn!(%e, "could not back up corrupt settings");
                    }
                    Settings::default()
                }
            },
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%e, "could not read settings");
                }
                Settings::default()
            }
        };
        let mut snapshot = snapshot;
        // Presence, including an explicitly empty list, makes the new key authoritative.
        // This prevents old pins from being resurrected after the user unpins everything.
        let has_pins = tokio::fs::read_to_string(&path)
            .await
            .ok()
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
            .is_some_and(|s| {
                s.get("library")
                    .and_then(|s| s.get("pinned_uris"))
                    .is_some()
            });
        if !has_pins
            && let Some(parent) = path.parent()
            && let Ok(text) = tokio::fs::read_to_string(parent.join("ui.toml")).await
        {
            migrate_pins(&mut snapshot, &text);
        }
        let (sender, receiver) = mpsc::channel();
        let output = path.clone();
        #[cfg(test)]
        let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        #[cfg(test)]
        let worker_writes = writes.clone();
        let worker = std::thread::Builder::new()
            .name("velo-settings".into())
            .spawn(move || {
                let write = |settings: Box<Settings>| {
                    let result = toml::to_string_pretty(&settings)
                        .map_err(error)
                        .and_then(|text| atomic_write(&output, text.as_bytes()));
                    if let Err(e) = result {
                        tracing::warn!(%e, "could not save settings");
                    }
                    #[cfg(test)]
                    worker_writes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                };
                while let Ok(Message::Save(mut pending)) = receiver.recv() {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    loop {
                        if Instant::now() >= deadline {
                            write(pending);
                            break;
                        }
                        let wait = Duration::from_millis(500)
                            .min(deadline.saturating_duration_since(Instant::now()));
                        match receiver.recv_timeout(wait) {
                            Ok(Message::Save(next)) => pending = next,
                            Ok(Message::Flush) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                                write(pending);
                                return;
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                write(pending);
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(error)?;
        let this = Self {
            path,
            snapshot: RwLock::new(snapshot),
            sender,
            worker: Mutex::new(Some(worker)),
            #[cfg(test)]
            writes,
        };
        // Persist migration before returning, outside the runtime worker. A delayed
        // initial save could overwrite a second reader's explicit edits to this path.
        let initial = this.load();
        let initial_path = this.path.clone();
        let result = tokio::task::spawn_blocking(move || {
            let text = toml::to_string_pretty(&initial).map_err(error)?;
            if std::fs::read_to_string(&initial_path).ok().as_deref() != Some(text.as_str()) {
                atomic_write(&initial_path, text.as_bytes())?;
            }
            Ok::<_, velo_domain::VeloError>(())
        })
        .await
        .map_err(error)?;
        if let Err(e) = result {
            tracing::warn!(%e, "could not persist settings migration");
        }
        Ok(this)
    }

    /// Public boundary until C4 registers `pub mod proxy` in services/lib.rs.
    pub async fn prepare_proxy(
        settings: &velo_domain::ProxySettings,
        password: Option<String>,
    ) -> Result<proxy::ProxyAdapter> {
        proxy::ProxyAdapter::start(settings, password).await
    }

    pub fn supported_proxy_modes() -> &'static [&'static str] {
        proxy::SUPPORTED_MODES
    }
}

fn migrate_pins(settings: &mut Settings, ui: &str) {
    let Ok(ui) = toml::from_str::<toml::Value>(ui) else {
        return;
    };
    if let Some(pins) = ui.get("pinned_playlists").and_then(toml::Value::as_array) {
        settings
            .library
            .pinned_uris
            .extend(pins.iter().filter_map(toml::Value::as_str).map(|id| {
                if id.starts_with("spotify:") {
                    id.to_owned()
                } else {
                    format!("spotify:playlist:{id}")
                }
            }));
    }
    settings.normalize();
}
impl SettingsStore for TomlSettingsStore {
    fn load(&self) -> Settings {
        self.snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn save(&self, settings: &Settings) {
        let mut snapshot = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
        let mut settings = settings.clone();
        settings.normalize();
        *snapshot = settings.clone();
        // Keep snapshot and queue ordering identical even for concurrent callers.
        if let Err(e) = self.sender.send(Message::Save(Box::new(settings))) {
            tracing::warn!(%e, "settings writer stopped");
        }
    }
    fn path(&self) -> PathBuf {
        self.path.clone()
    }
}
impl Drop for TomlSettingsStore {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Flush);
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn s9_old_toml_migrates_audio_and_pins_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        tokio::fs::write(
            &path,
            "[cache]\naudio_limit_mb = 200\n[appearance]\ntheme = 'dark'\n",
        )
        .await
        .unwrap();
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        tokio::fs::write(
            dir.path().join("ui.toml"),
            format!("pinned_playlists = ['{id}', 'spotify:playlist:{id}', 'bad']\n"),
        )
        .await
        .unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        let s = store.load();
        assert_eq!(s.appearance.theme, "dark");
        assert_eq!(s.cache.audio_limit_mb, 1024);
        assert_eq!(
            s.library.pinned_uris,
            vec![format!("spotify:playlist:{id}")]
        );
        let mut s = s;
        s.library.pinned_uris.clear();
        store.save(&s);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert!(store.load().library.pinned_uris.is_empty());
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn s9_schema_round_trips_every_new_key_and_never_passwords() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        let mut s = store.load();
        s.visualizer.player_bar_mode = "bars".into();
        s.background.cover_blur = false;
        s.background.drift = false;
        s.background.tint_ui = false;
        s.playback.device_name = "Office".into();
        s.playback.quality = 96;
        s.playback.normalize_volume = false;
        s.playback.autoplay = false;
        s.playback.gapless = false;
        s.playback.output_buffer_ms = 50;
        s.cache.audio_enabled = false;
        s.cache.audio_limit_mb = 4096;
        s.appearance.sidebar_compact = true;
        s.appearance.compact_tracks = true;
        s.appearance.ui_zoom = 0.75;
        s.library.sort = "creator".into();
        s.library.pinned_uris = vec!["spotify:album:4uLU6hMCjMI75M1A2tKUQC".into()];
        s.proxy = velo_domain::ProxySettings {
            mode: "socks5".into(),
            host: "localhost".into(),
            port: 1080,
            user: "fixture-user".into(),
        };
        s.equalizer.enabled = true;
        s.equalizer.set_preset("Rock");
        store.save(&s);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!text.contains("password"));
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load(), s);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn s9_invalid_values_preserve_unrelated_old_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        tokio::fs::write(&path,"[appearance]\ntheme = 'dark'\nui_zoom = nan\n[cache]\naudio_limit_mb = -1\n[playback]\nquality = -1\noutput_buffer_ms = 255\n[equalizer]\ngains_db = [nan, 30.0, -30.0]\npreamp_db = inf\npreset = 'Flat'\n").await.unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        let s = store.load();
        assert_eq!(s.appearance.theme, "dark");
        assert_eq!(s.appearance.ui_zoom, 1.0);
        assert_eq!(s.cache.audio_limit_mb, 512);
        assert_eq!(s.playback.quality, 320);
        assert_eq!(s.playback.output_buffer_ms, 100);
        assert_eq!(
            s.equalizer.gains_db,
            [0., 12., -12., 0., 0., 0., 0., 0., 0., 0.]
        );
        assert_eq!(s.equalizer.preamp_db, 0.);
        assert_eq!(s.equalizer.preset, "Custom");
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn cache_limits_migrate_normalize_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        tokio::fs::write(&path, "[appearance]\ntheme = 'dark'\n")
            .await
            .unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load().cache, velo_domain::CacheSettings::default());
        let mut settings = store.load();
        settings.cache.cover_limit_mb = 10;
        settings.cache.audio_limit_mb = 512;
        store.save(&settings);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load(), settings);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            "[cache]\ncover_limit_mb = 0\naudio_limit_mb = 4294967295\n",
        )
        .await
        .unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load().cache.cover_limit_mb, 1);
        assert_eq!(store.load().cache.audio_limit_mb, 4096);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn continuous_changes_write_within_max_wait_and_flush_latest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        let mut settings = Settings::default();
        for width in 1100..1112 {
            settings.window.width = width;
            store.save(&settings);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        // All gaps were below the debounce, so this write must be the max-wait.
        assert_eq!(store.writes.load(std::sync::atomic::Ordering::SeqCst), 1);
        let persisted: Settings =
            toml::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert!((1100..1112).contains(&persisted.window.width));
        let count = store.writes.clone();
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        let persisted: Settings =
            toml::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(persisted, settings);
    }
    #[tokio::test]
    async fn debounce_and_drop_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load(), Settings::default());
        let mut settings = Settings::default();
        for theme in ["light", "dark", "system"] {
            settings.appearance.theme = theme.into();
            store.save(&settings);
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(store.writes.load(std::sync::atomic::Ordering::SeqCst), 1);
        settings.appearance.language = "es".into();
        store.save(&settings);
        let count = store.writes.clone();
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load(), settings);
    }
    #[tokio::test]
    async fn performance_options_default_off_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        // Un settings.toml anterior a S1 no tiene las claves: quedan apagadas.
        let old = "[appearance]\ntheme = 'dark'\nreduce_motion = true\n";
        tokio::fs::write(&path, old).await.unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        let loaded = store.load().appearance;
        assert_eq!(loaded.theme, "dark");
        assert!(loaded.reduce_motion);
        assert!(!loaded.performance_mode);
        assert!(!loaded.pause_in_background);
        assert!(!loaded.performance_keep_visualizer);
        let mut settings = store.load();
        settings.appearance.performance_mode = true;
        settings.appearance.pause_in_background = true;
        settings.appearance.performance_keep_visualizer = true;
        store.save(&settings);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(text.contains("performance_mode = true"));
        assert!(text.contains("pause_in_background = true"));
        assert!(text.contains("performance_keep_visualizer = true"));
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load(), settings);
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn corrupt_file_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        tokio::fs::write(&path, "[broken").await.unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load(), Settings::default());
        assert_eq!(
            tokio::fs::read_to_string(path.with_extension("toml.bak"))
                .await
                .unwrap(),
            "[broken"
        );
        tokio::fs::write(&path, [0xff, 0xfe]).await.unwrap();
        let store = TomlSettingsStore::with_path(&path).await.unwrap();
        assert_eq!(store.load(), Settings::default());
        assert_eq!(
            tokio::fs::read(path.with_extension("toml.bak"))
                .await
                .unwrap(),
            [0xff, 0xfe]
        );
    }
}
