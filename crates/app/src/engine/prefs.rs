//! User preferences: the domain `Settings` (TOML store from velo-services) plus a few UI
//! extras the domain struct has no field for (window controls style, pinned playlists), kept
//! in `ui.toml` next to `settings.toml`. Writes are debounced inside the tokio runtime.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use velo_domain::{Settings, SettingsStore};
use velo_services::TomlSettingsStore;

const EXTRAS_DEBOUNCE: Duration = Duration::from_millis(500);
const ENV_CLIENT_ID: &str = "VELO_SPOTIFY_CLIENT_ID";
// Identificador público de PKCE: entra por el entorno del compilador, nunca por un archivo
// versionado. option_env! hace que Cargo recomponga el binario si cambia esta variable.
const BUNDLED_CLIENT_ID: Option<&str> = option_env!("VELO_SPOTIFY_CLIENT_ID");
const LOCAL_ENV_FILE: &str = "velo.local.env";

/// A trailing debounce has no deadline while idle. No periodic timer is needed.
#[derive(Default)]
struct Debounce {
    deadline: Option<Instant>,
}

impl Debounce {
    fn changed(&mut self, now: Instant) {
        self.deadline = Some(now + EXTRAS_DEBOUNCE);
    }

    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.deadline.map(|at| at.saturating_duration_since(now))
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|at| now >= at) {
            self.deadline = None;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extras {
    /// Cerrar oculta la ventana si hay bandeja disponible; Salir siempre termina.
    pub close_to_tray: bool,
    /// "mac" | "windows"
    pub window_controls: String,
    #[serde(skip_serializing)] // Read migration input; Settings.library is the sole writer.
    pub pinned_playlists: Vec<String>,
}

impl Default for Extras {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            window_controls: "mac".into(),
            pinned_playlists: Vec::new(),
        }
    }
}

pub struct Prefs {
    store: Mutex<Option<TomlSettingsStore>>,
    settings: watch::Sender<Settings>,
    extras: watch::Sender<Extras>,
    extras_path: PathBuf,
    extras_writer: Mutex<Option<tokio::task::AbortHandle>>,
    extras_write: Mutex<()>,
    updates: Mutex<()>,
    config_dir: PathBuf,
    rt: tokio::runtime::Handle,
}

impl Prefs {
    #[cfg(test)]
    pub(crate) fn for_test(rt: tokio::runtime::Handle, config_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(None),
            settings: watch::channel(Settings::default()).0,
            extras: watch::channel(Extras::default()).0,
            extras_path: config_dir.join("ui.toml"),
            extras_writer: Mutex::new(None),
            extras_write: Mutex::new(()),
            updates: Mutex::new(()),
            config_dir,
            rt,
        })
    }

    /// Reads both files once at startup (a few hundred bytes; needed before the first frame so
    /// the theme does not flash). Everything after this is asynchronous.
    pub fn load(rt: &tokio::runtime::Runtime) -> Arc<Self> {
        let store = match rt.block_on(TomlSettingsStore::new()) {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!(%e, "no user config directory; settings go to the temp folder");
                rt.block_on(TomlSettingsStore::with_path(
                    std::env::temp_dir().join("velo").join("settings.toml"),
                ))
                .expect("settings store")
            }
        };
        let path = store.path();
        let config_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let extras_path = config_dir.join("ui.toml");
        let extras = std::fs::read_to_string(&extras_path)
            .ok()
            .and_then(|text| toml::from_str::<Extras>(&text).ok())
            .unwrap_or_default();
        let this = Arc::new(Self {
            settings: watch::channel(store.load()).0,
            store: Mutex::new(Some(store)),
            extras: watch::channel(extras).0,
            extras_path,
            extras_writer: Mutex::new(None),
            extras_write: Mutex::new(()),
            updates: Mutex::new(()),
            config_dir,
            rt: rt.handle().clone(),
        });
        this.start_extras_writer();
        this
    }

    pub fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }

    pub fn watch_settings(&self) -> watch::Receiver<Settings> {
        self.settings.subscribe()
    }

    pub fn extras(&self) -> Extras {
        let mut extras = self.extras.borrow().clone();
        extras.pinned_playlists = self
            .settings()
            .library
            .pinned_uris
            .iter()
            .filter_map(|uri| uri.strip_prefix("spotify:playlist:").map(str::to_owned))
            .collect();
        extras
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Applies `f`; when something changed, publishes it and queues a debounced save (the store
    /// debounces on its own writer thread).
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        let _update = self.updates.lock().unwrap_or_else(|e| e.into_inner());
        let current = self.settings();
        let mut next = current.clone();
        f(&mut next);
        next.normalize();
        if next == current {
            return;
        }
        if let Some(store) = self
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            store.save(&next);
        }
        self.settings.send_replace(next);
    }

    pub fn update_extras(self: &Arc<Self>, f: impl FnOnce(&mut Extras)) {
        let _update = self.updates.lock().unwrap_or_else(|e| e.into_inner());
        let current = self.extras();
        let mut next = current.clone();
        f(&mut next);
        if next == current {
            return;
        }
        if next.pinned_playlists != current.pinned_playlists {
            let mut settings = self.settings();
            settings
                .library
                .pinned_uris
                .retain(|uri| !uri.starts_with("spotify:playlist:"));
            settings.library.pinned_uris.extend(
                next.pinned_playlists
                    .iter()
                    .map(|id| format!("spotify:playlist:{id}")),
            );
            settings.normalize();
            if let Some(store) = self
                .store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                store.save(&settings);
            }
            self.settings.send_replace(settings);
        }
        self.extras.send_replace(next);
    }

    fn start_extras_writer(self: &Arc<Self>) {
        let this = self.clone();
        let mut changes = self.extras.subscribe();
        let task = self.rt.spawn(async move {
            let mut debounce = Debounce::default();
            loop {
                let remaining = debounce.remaining(Instant::now());
                tokio::select! {
                    changed = changes.changed() => {
                        if changed.is_err() { break; }
                        changes.borrow_and_update();
                        debounce.changed(Instant::now());
                    }
                    () = async {
                        match remaining {
                            Some(delay) => tokio::time::sleep(delay).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if debounce.take_due(Instant::now()) {
                            let writer = this.clone();
                            let _ = tokio::task::spawn_blocking(move || writer.write_extras()).await;
                        }
                    }
                }
            }
        });
        *self.extras_writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.abort_handle());
    }

    fn write_extras(&self) {
        let _write = self.extras_write.lock().unwrap_or_else(|e| e.into_inner());
        let extras = self.extras();
        match toml::to_string_pretty(&extras) {
            Ok(text) => {
                if let Err(e) = write_atomic(&self.extras_path, text.as_bytes()) {
                    tracing::warn!(%e, "could not save ui.toml");
                }
            }
            Err(e) => tracing::warn!(%e, "could not encode ui.toml"),
        }
    }

    /// Writes everything pending. Call once when the event loop ends, from the main thread.
    pub fn shutdown(&self) {
        if let Some(writer) = self
            .extras_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            writer.abort();
        }
        self.write_extras();
        // Dropping the store flushes its debounced writer.
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(store);
    }

    /// Client ID propio (ajustes/entorno/archivo local/compilación); `None`: cuenta de Spotify
    /// por la sesión de librespot (L1).
    pub fn client_id(&self) -> Option<String> {
        let from_settings = self.settings().account.client_id;
        let from_file = std::fs::read_to_string(LOCAL_ENV_FILE).ok();
        resolve_client_id(
            from_settings.as_deref(),
            std::env::var(ENV_CLIENT_ID).ok().as_deref(),
            from_file.as_deref(),
            BUNDLED_CLIENT_ID,
        )
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Client ID de una app personal de Spotify. Prioridad: selección guardada > overrides de
/// desarrollo > incluido al compilar. Los vacíos cuentan como ausentes. `None` (L1): sin app
/// personal, la cuenta entra con la sesión de librespot (un solo inicio de sesión). La app
/// pública compartida de antes (cuota agotada) cuenta como ausente.
pub fn resolve_client_id(
    setting: Option<&str>,
    env: Option<&str>,
    env_file: Option<&str>,
    bundled: Option<&str>,
) -> Option<String> {
    let clean = |s: &str| {
        let s = s.trim().trim_matches('"').trim_matches('\'').trim();
        (!s.is_empty() && s != velo_spotify::oauth::DEFAULT_WEB_CLIENT_ID).then(|| s.to_string())
    };
    setting
        .and_then(clean)
        .or_else(|| env.and_then(clean))
        .or_else(|| {
            env_file
                .and_then(|text| env_file_value(text, ENV_CLIENT_ID))
                .and_then(|s| clean(&s))
        })
        .or_else(|| bundled.and_then(clean))
}

/// `KEY=value` lines (comments with `#`), like the login example reads them.
pub fn env_file_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s9_legacy_pin_api_updates_the_single_uri_authority() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = Prefs::for_test(tokio::runtime::Handle::current(), dir.path().to_path_buf());
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        prefs.update_settings(|s| s.library.pinned_uris.push(format!("spotify:album:{id}")));
        prefs.update_extras(|e| e.pinned_playlists.push(id.into()));
        assert_eq!(
            prefs.settings().library.pinned_uris,
            vec![
                format!("spotify:album:{id}"),
                format!("spotify:playlist:{id}")
            ]
        );
        assert_eq!(prefs.extras().pinned_playlists, vec![id.to_string()]);
        prefs.update_extras(|e| e.pinned_playlists.clear());
        assert_eq!(
            prefs.settings().library.pinned_uris,
            vec![format!("spotify:album:{id}")]
        );
    }

    #[test]
    fn debounce_restarts_and_has_no_idle_deadline() {
        let t = Instant::now();
        let mut d = Debounce::default();
        assert_eq!(d.remaining(t), None);
        d.changed(t);
        d.changed(t + Duration::from_millis(400));
        assert!(!d.take_due(t + EXTRAS_DEBOUNCE));
        assert_eq!(
            d.remaining(t + EXTRAS_DEBOUNCE),
            Some(Duration::from_millis(400))
        );
        assert!(d.take_due(t + Duration::from_millis(900)));
        assert_eq!(d.remaining(t + Duration::from_secs(60)), None);
        assert!(!d.take_due(t + Duration::from_secs(60)));
    }

    #[test]
    fn client_id_priority_and_blank_handling() {
        let file = "# comment\nOTHER=1\nVELO_SPOTIFY_CLIENT_ID = abc123 \n";
        assert_eq!(
            resolve_client_id(Some("saved"), Some("env"), Some(file), Some("bundled")).as_deref(),
            Some("saved")
        );
        assert_eq!(
            resolve_client_id(Some("  "), Some("env"), Some(file), Some("bundled")).as_deref(),
            Some("env")
        );
        assert_eq!(
            resolve_client_id(None, None, Some(file), Some("bundled")).as_deref(),
            Some("abc123")
        );
        assert_eq!(
            resolve_client_id(None, Some(""), None, None).as_deref(),
            None
        );
        assert_eq!(
            resolve_client_id(None, None, Some("VELO_SPOTIFY_CLIENT_ID=\n"), None).as_deref(),
            None
        );
    }

    #[test]
    fn the_old_shared_app_counts_as_no_personal_app() {
        let shared = velo_spotify::oauth::DEFAULT_WEB_CLIENT_ID;
        assert_eq!(resolve_client_id(Some(shared), None, None, None), None);
        let file = "VELO_SPOTIFY_CLIENT_ID=own";
        assert_eq!(
            resolve_client_id(Some(shared), None, Some(file), None).as_deref(),
            Some("own")
        );
    }

    #[test]
    fn env_file_ignores_comments_and_other_keys() {
        let text = "#VELO_SPOTIFY_CLIENT_ID=nope\nX=y\nVELO_SPOTIFY_CLIENT_ID=\"quoted\"\n";
        assert_eq!(
            resolve_client_id(None, None, Some(text), None).as_deref(),
            Some("quoted")
        );
        assert_eq!(env_file_value(text, "X").as_deref(), Some("y"));
    }

    #[test]
    fn client_id_bundled_fallback_and_absence() {
        assert_eq!(
            resolve_client_id(None, None, None, Some(" bundled ")).as_deref(),
            Some("bundled")
        );
        assert_eq!(
            resolve_client_id(Some("own"), None, None, Some("bundled")).as_deref(),
            Some("own")
        );
        assert_eq!(
            resolve_client_id(
                Some(" "),
                Some("''"),
                Some("VELO_SPOTIFY_CLIENT_ID=\"\""),
                Some("bundled")
            )
            .as_deref(),
            Some("bundled")
        );
        assert_eq!(resolve_client_id(None, None, None, None).as_deref(), None);
        assert_eq!(
            resolve_client_id(None, None, None, Some(" \"\" ")).as_deref(),
            None
        );
    }

    #[test]
    fn extras_roundtrip_and_defaults() {
        let e = Extras {
            close_to_tray: false,
            window_controls: "windows".into(),
            pinned_playlists: vec!["p1".into()],
        };
        let text = toml::to_string_pretty(&e).unwrap();
        let parsed = toml::from_str::<Extras>(&text).unwrap();
        assert!(!parsed.close_to_tray);
        assert_eq!(parsed.window_controls, e.window_controls);
        assert!(parsed.pinned_playlists.is_empty());
        assert!(!text.contains("pinned_playlists"));
        assert_eq!(toml::from_str::<Extras>("").unwrap(), Extras::default());
        assert_eq!(Extras::default().window_controls, "mac");
        assert!(Extras::default().close_to_tray);
        // Un ui.toml anterior recibe el valor nuevo sin perder preferencias.
        let old: Extras =
            toml::from_str("window_controls = 'windows'\npinned_playlists = ['p1']").unwrap();
        assert!(old.close_to_tray);
        assert_eq!(old.pinned_playlists, vec!["p1"]);
    }

    #[test]
    fn atomic_write_replaces_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("ui.toml");
        write_atomic(&path, b"a=1").unwrap();
        write_atomic(&path, b"a=2").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a=2");
        assert!(!path.with_extension("toml.tmp").exists());
    }
}
