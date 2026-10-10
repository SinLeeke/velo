//! Event-driven saved/applied settings, transport and storage operations. No polling.
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use velo_domain::{EngineOptions, EqConfig, ProxySettings, Settings};
use velo_player::{Engine, EngineConfig, PlayerError};
use velo_services::TomlSettingsStore;
use velo_spotify::{KeyringTokenStore, TokenStore};

use super::super::Env;

#[derive(Clone, Copy)]
pub struct EngineHooks {
    pub configure: fn(&mut EngineConfig, &EngineOptions),
    pub equalizer: fn(&Engine, EqConfig),
    pub output_buffer: fn(&Engine, u32) -> Result<(), PlayerError>,
}

impl EngineHooks {
    /// The real `velo-player` engine: name, hot EQ and output buffer.
    pub fn player() -> Self {
        Self {
            configure: |config, options| config.name = options.device_name.clone(),
            equalizer: |engine, eq| engine.set_eq(eq),
            output_buffer: |engine, ms| engine.set_output_buffer_ms(ms),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RestartConfig {
    device_name: String,
    quality: u32,
    normalize: bool,
    autoplay: bool,
    gapless: bool,
    cache_enabled: bool,
    cache_mb: u32,
    proxy: ProxySettings,
}
impl From<&Settings> for RestartConfig {
    fn from(s: &Settings) -> Self {
        Self {
            device_name: s.playback.device_name.clone(),
            quality: s.playback.quality,
            normalize: s.playback.normalize_volume,
            autoplay: s.playback.autoplay,
            gapless: s.playback.gapless,
            cache_enabled: s.cache.audio_enabled,
            cache_mb: s.cache.audio_limit_mb,
            proxy: s.proxy.clone(),
        }
    }
}
#[derive(Default)]
struct Generations {
    saved: Option<RestartConfig>,
    applied: Option<RestartConfig>,
    generation: u64,
}
impl Generations {
    fn saved(&mut self, s: &Settings) {
        let next = RestartConfig::from(s);
        if self.saved.as_ref() != Some(&next) {
            self.generation = self.generation.wrapping_add(1);
            self.saved = Some(next);
        }
    }
    fn applied(&mut self, s: &Settings) {
        self.applied = Some(RestartConfig::from(s));
    }
    fn pending(&self) -> bool {
        self.saved != self.applied
    }
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuntimeView {
    pub restart_required: bool,
    pub storage_busy: bool,
    pub credentials_stored: bool,
    pub proxy_password_stored: bool,
    pub proxy_busy: bool,
    pub proxy_error: String,
    pub playback_error: bool,
    pub reconnecting: bool,
}
pub type ClearHistory = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = velo_domain::Result<()>> + Send>>
        + Send
        + Sync,
>;
pub struct SettingsRuntime {
    generations: Mutex<Generations>,
    pub view: watch::Sender<RuntimeView>,
    proxy: tokio::sync::Mutex<Option<Box<dyn Send>>>,
    active_proxy: Mutex<Option<ProxySettings>>,
    storage: tokio::sync::Mutex<()>,
    history: Mutex<Option<ClearHistory>>,
    proxy_store: Arc<dyn TokenStore>,
    audio_store: Arc<dyn TokenStore>,
}

impl SettingsRuntime {
    pub fn new() -> Self {
        if cfg!(test) {
            return Self::with_stores(
                Arc::new(velo_spotify::MemoryTokenStore::new()),
                Arc::new(velo_spotify::MemoryTokenStore::new()),
            );
        }
        Self::with_stores(velo_player::login::keyring_store(), proxy_store())
    }
    pub fn with_stores(audio_store: Arc<dyn TokenStore>, proxy_store: Arc<dyn TokenStore>) -> Self {
        Self {
            generations: Mutex::new(Generations::default()),
            view: watch::channel(RuntimeView::default()).0,
            proxy: tokio::sync::Mutex::new(None),
            active_proxy: Mutex::new(None),
            storage: tokio::sync::Mutex::new(()),
            history: Mutex::new(None),
            proxy_store,
            audio_store,
        }
    }
    pub fn saved(&self, s: &Settings) {
        let mut g = self.generations.lock().unwrap_or_else(|e| e.into_inner());
        g.saved(s);
        self.view.send_modify(|v| v.restart_required = g.pending());
    }
    pub fn applied(&self, s: &Settings) {
        let mut g = self.generations.lock().unwrap_or_else(|e| e.into_inner());
        g.applied(s);
        self.view.send_modify(|v| v.restart_required = g.pending());
    }
    pub fn set_history_clearer(&self, clear: ClearHistory) {
        *self.history.lock().unwrap_or_else(|e| e.into_inner()) = Some(clear);
    }
    pub async fn clear_history(&self) -> velo_domain::Result<()> {
        let callback = self
            .history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(callback) = callback {
            callback().await
        } else {
            Err(velo_domain::VeloError::Other(
                "No se pudo borrar el historial".into(),
            ))
        }
    }
    pub async fn refresh_credentials(&self) {
        let audio = velo_player::login::has_stored_login(self.audio_store.clone()).await;
        let store = self.proxy_store.clone();
        let proxy = tokio::task::spawn_blocking(move || store.load().is_some())
            .await
            .unwrap_or(false);
        self.view.send_modify(|v| {
            v.credentials_stored = audio;
            v.proxy_password_stored = proxy;
        });
    }
    pub async fn clear_proxy_password(&self) -> velo_domain::Result<()> {
        let store = self.proxy_store.clone();
        tokio::task::spawn_blocking(move || store.clear())
            .await
            .map_err(|_| proxy_error())?
            .map_err(|_| proxy_error())?;
        self.view.send_modify(|v| v.proxy_password_stored = false);
        Ok(())
    }
    pub async fn restore_proxy(
        &self,
        settings: &Settings,
        supported: bool,
    ) -> velo_domain::Result<()> {
        let mut adapter = self.proxy.lock().await;
        if self
            .active_proxy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            == Some(&settings.proxy)
        {
            return Ok(());
        }
        if settings.proxy.mode != "off" && !supported {
            return Err(velo_domain::VeloError::Other(
                "No se pudo aplicar el proxy".into(),
            ));
        }
        let store = self.proxy_store.clone();
        let password = tokio::task::spawn_blocking(move || store.load())
            .await
            .map_err(|_| proxy_error())?;
        let next = TomlSettingsStore::prepare_proxy(&settings.proxy, password).await?;
        velo_domain::proxy_route().send_modify(|route| {
            route.generation = route.generation.wrapping_add(1);
            route.url = next.url();
        });
        *adapter = Some(Box::new(next));
        *self.active_proxy.lock().unwrap_or_else(|e| e.into_inner()) = Some(settings.proxy.clone());
        Ok(())
    }
    pub async fn apply_proxy(
        &self,
        env: &Arc<Env>,
        draft: ProxySettings,
        password: String,
        replace: bool,
    ) -> velo_domain::Result<()> {
        let mut adapter = self.proxy.lock().await;
        draft
            .validate()
            .map_err(|e| velo_domain::VeloError::Other(e.into()))?;
        if draft.mode != "off" && !env.audio.settings_supported() {
            return Err(velo_domain::VeloError::Other(
                "No se pudo aplicar el proxy".into(),
            ));
        }
        // The saved password only belongs to the saved host/port/user: never send it elsewhere.
        if !replace && !may_reuse_saved_password(&env.prefs.settings().proxy, &draft) {
            return Err(velo_domain::VeloError::Other(
                "Introduce la contraseña del proxy para el nuevo servidor o usuario".into(),
            ));
        }
        self.view.send_modify(|v| {
            v.proxy_busy = true;
            v.proxy_error.clear();
        });
        let result = async {
            let secret = if replace {
                (!password.is_empty()).then_some(password)
            } else {
                let store = self.proxy_store.clone();
                tokio::task::spawn_blocking(move || store.load())
                    .await
                    .map_err(|_| proxy_error())?
            };
            let next = TomlSettingsStore::prepare_proxy(&draft, secret.clone()).await?;
            if replace {
                let store = self.proxy_store.clone();
                tokio::task::spawn_blocking(move || match secret {
                    Some(secret) => store.save(&secret),
                    None => store.clear(),
                })
                .await
                .map_err(|_| proxy_error())?
                .map_err(|_| proxy_error())?;
            }
            velo_domain::proxy_route().send_modify(|route| {
                route.generation = route.generation.wrapping_add(1);
                route.url = next.url();
            });
            *adapter = Some(Box::new(next));
            *self.active_proxy.lock().unwrap_or_else(|e| e.into_inner()) = Some(draft.clone());
            env.prefs.update_settings(|s| s.proxy = draft);
            self.saved(&env.prefs.settings());
            // Reconstruct the session: no transport may retain a direct connection pool.
            drop(adapter); // launch restores the route under this same mutex.
            if env.audio.is_ready() || env.audio.view.borrow().busy {
                env.audio.reconnect_playback(false).await.map_err(|_| {
                    velo_domain::VeloError::Other(
                        "No se pudo reconectar el motor con el proxy".into(),
                    )
                })?;
            }
            Ok(())
        }
        .await;
        self.refresh_credentials().await;
        self.view.send_modify(|v| {
            v.proxy_busy = false;
            v.proxy_error = result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default();
        });
        result
    }
    pub async fn clear_audio(&self, env: &Arc<Env>) -> Result<(), PlayerError> {
        let _storage = self.storage.lock().await;
        self.view.send_modify(|v| v.storage_busy = true);
        let result = env.audio.reconnect_playback(true).await;
        self.view.send_modify(|v| v.storage_busy = false);
        result
    }
}
impl Default for SettingsRuntime {
    fn default() -> Self {
        Self::new()
    }
}
/// Reusing the keyring password is only valid for the same upstream endpoint and user
/// (or when the draft has no user, so no password is sent at all).
fn may_reuse_saved_password(saved: &ProxySettings, draft: &ProxySettings) -> bool {
    draft.user.is_empty()
        || (saved.host == draft.host && saved.port == draft.port && saved.user == draft.user)
}
fn proxy_store() -> Arc<dyn TokenStore> {
    Arc::new(KeyringTokenStore::with_user("spotify-proxy-password"))
}
fn proxy_error() -> velo_domain::VeloError {
    velo_domain::VeloError::Other(
        "No se pudo actualizar la contraseña del proxy en el Administrador de credenciales".into(),
    )
}

pub fn audio_directory() -> std::path::PathBuf {
    directories::ProjectDirs::from("app", "Velo", "Velo")
        .map(|d| d.cache_dir().join("audio"))
        .unwrap_or_else(|| std::env::temp_dir().join("velo").join("audio"))
}
/// Cache directory of one device under one Spotify account (injective hex of the user id, so
/// two accounts on the same PC and Windows user never share encrypted audio files).
pub fn audio_cache_dir(device_id: &str, owner: Option<&str>) -> std::path::PathBuf {
    let root = audio_directory();
    match owner.filter(|o| !o.is_empty()) {
        Some(owner) => {
            let key: String = owner.bytes().take(64).map(|b| format!("{b:02x}")).collect();
            root.join(key).join(device_id)
        }
        None => root.join(device_id),
    }
}
fn audio_owner_file() -> std::path::PathBuf {
    audio_directory().join("owner")
}
/// Spotify user whose audio cache is active (written once the profile is known).
pub async fn read_audio_owner() -> Option<String> {
    if cfg!(test) {
        return None;
    }
    tokio::fs::read_to_string(audio_owner_file())
        .await
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
/// Records the owner; returns the previous one when it differs (an account switch).
pub async fn write_audio_owner(owner: &str) -> Option<String> {
    if cfg!(test) || owner.is_empty() {
        return None;
    }
    let previous = read_audio_owner().await;
    if previous.as_deref() == Some(owner) {
        return None;
    }
    let file = audio_owner_file();
    if let Some(parent) = file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(file, owner).await;
    previous
}
pub async fn audio_size_bytes() -> std::io::Result<u64> {
    tokio::task::spawn_blocking(|| {
        let root = audio_directory();
        let mut pending = vec![root];
        let mut bytes = 0u64;
        while let Some(dir) = pending.pop() {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            for entry in entries {
                let entry = entry?;
                let meta = entry.path().symlink_metadata()?;
                if meta.is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    pending.push(entry.path());
                } else {
                    bytes = bytes.saturating_add(meta.len());
                }
            }
        }
        Ok(bytes)
    })
    .await
    .map_err(std::io::Error::other)?
}
pub async fn clear_audio_directory() -> std::io::Result<()> {
    // Fixed application-owned path; closed handles first, never credentials/volume paths.
    match tokio::fs::remove_dir_all(audio_directory()).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
pub fn start(env: &Arc<Env>) {
    env.audio.bind_settings(env.prefs.clone());
    let mut settings = env.prefs.watch_settings();
    if std::env::var("VELO_SETTINGS_PROBE").as_deref() == Ok("1") {
        let env = env.clone();
        env.rt.clone().spawn(async move {
            probe_live(env).await;
        });
    }
    let env = env.clone();
    env.rt.clone().spawn(async move {
        let runtime = env.audio.settings_runtime.clone();
        runtime.refresh_credentials().await;
        loop {
            let s = settings.borrow_and_update().clone();
            runtime.saved(&s);
            env.audio.publish_hot_settings(&s);
            if settings.changed().await.is_err() {
                break;
            }
        }
    });
}

/// Opt-in live probe for the integrator, never run by tests. Emits no tokens, URLs,
/// usernames or payloads. A ready engine proves AP connection; the session's proxy
/// config is the shared source used by AP, HTTP and dealer in librespot 0.8.
async fn probe_live(env: Arc<Env>) {
    let mut view = env.audio.view.subscribe();
    if tokio::time::timeout(
        std::time::Duration::from_secs(40),
        view.wait_for(|v| v.ready),
    )
    .await
    .is_err()
    {
        tracing::info!(probe = "settings", ready = false, "S9 settings probe");
        return;
    }
    let Some(engine) = env.audio.engine() else {
        return;
    };
    let configured = engine
        .session()
        .config()
        .proxy
        .as_ref()
        .map(ToString::to_string);
    let expected = velo_domain::proxy_route().borrow().url.clone();
    let settings = env.prefs.settings();
    let web = velo_spotify::ProxyHttpClient::new()
        .get("https://accounts.spotify.com/.well-known/openid-configuration")
        .send()
        .await;
    tracing::info!(
        probe = "settings",
        ready = true,
        hooks = env.audio.settings_supported(),
        session_route_matches = configured == expected,
        http = web.is_ok(),
        quality = settings.playback.quality,
        encrypted_cache = settings.cache.audio_enabled,
        buffer_ms = settings.playback.output_buffer_ms,
        restart_required = env.audio.settings_runtime.view.borrow().restart_required,
        "S9 settings probe"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn audio_cache_is_scoped_by_account_and_device() {
        let a = audio_cache_dir("dev", Some("alice"));
        let b = audio_cache_dir("dev", Some("bob"));
        assert_ne!(a, b);
        assert!(a.starts_with(audio_directory()) && a.ends_with("dev"));
        assert_eq!(audio_cache_dir("dev", None), audio_directory().join("dev"));
        assert_eq!(
            audio_cache_dir("dev", Some("")),
            audio_cache_dir("dev", None)
        );
    }
    #[test]
    fn saved_proxy_password_is_not_reused_for_another_host_or_user() {
        let saved = ProxySettings {
            mode: "http".into(),
            host: "proxy.test".into(),
            port: 8080,
            user: "ana".into(),
        };
        assert!(may_reuse_saved_password(&saved, &saved));
        let mut other = saved.clone();
        other.host = "evil.test".into();
        assert!(!may_reuse_saved_password(&saved, &other));
        other = saved.clone();
        other.user = "bob".into();
        assert!(!may_reuse_saved_password(&saved, &other));
        other = saved.clone();
        other.port = 1;
        assert!(!may_reuse_saved_password(&saved, &other));
        other.user.clear();
        assert!(may_reuse_saved_password(&saved, &other));
    }
    use super::*;
    #[tokio::test]
    async fn proxy_password_clear_preserves_login_and_unsupported_modes_leave_active_settings() {
        let login = Arc::new(velo_spotify::MemoryTokenStore::new());
        login.save("fixture-login").unwrap();
        let proxy = Arc::new(velo_spotify::MemoryTokenStore::new());
        proxy.save("fixture-secret").unwrap();
        let runtime = SettingsRuntime::with_stores(login.clone(), proxy.clone());
        runtime.refresh_credentials().await;
        assert!(runtime.view.borrow().credentials_stored);
        assert!(runtime.view.borrow().proxy_password_stored);
        runtime.clear_proxy_password().await.unwrap();
        assert!(!runtime.view.borrow().proxy_password_stored);
        assert_eq!(login.load().as_deref(), Some("fixture-login"));
        assert!(proxy.load().is_none());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_dir, env) = super::super::super::test_env(&rt);
        let before = env.prefs.settings();
        let draft = ProxySettings {
            mode: "system".into(),
            ..Default::default()
        };
        assert!(
            runtime
                .apply_proxy(&env, draft, "fixture-secret".into(), true)
                .await
                .is_err()
        );
        assert_eq!(env.prefs.settings(), before);
        assert!(proxy.load().is_none());
        rt.shutdown_background();
    }
    #[test]
    fn late_reconnect_keeps_newer_settings_pending_and_hot_changes_do_not_restart() {
        let mut g = Generations::default();
        let mut settings = Settings::default();
        g.saved(&settings);
        g.applied(&settings);
        settings.appearance.ui_zoom = 1.2;
        settings.equalizer.enabled = true;
        settings.playback.output_buffer_ms = 50;
        g.saved(&settings);
        assert!(!g.pending());
        settings.playback.quality = 96;
        g.saved(&settings);
        let in_flight = settings.clone();
        settings.playback.quality = 160;
        g.saved(&settings);
        g.applied(&in_flight);
        assert!(g.pending());
        g.applied(&settings);
        assert!(!g.pending());
    }
    #[tokio::test]
    async fn history_clear_is_a_service_callback_not_a_second_history_store() {
        let runtime = SettingsRuntime::new();
        let cleared = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = cleared.clone();
        runtime.set_history_clearer(Arc::new(move || {
            let counter = counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        }));
        runtime.clear_history().await.unwrap();
        assert_eq!(cleared.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
