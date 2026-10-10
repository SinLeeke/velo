//! `Settings` singleton: user preferences, saved by themselves with debounce.
//! Values are read once at construction; QML writes flow to `engine::prefs` (which debounces
//! on the runtime / the settings writer thread).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_QString = cxx_qt_lib::QList<QString>;
        type QList_f64 = cxx_qt_lib::QList<f64>;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, theme)]
        #[qproperty(QString, language)]
        #[qproperty(bool, reduce_motion)]
        #[qproperty(bool, performance_mode)]
        #[qproperty(bool, pause_in_background)]
        #[qproperty(bool, performance_keep_visualizer)]
        #[qproperty(bool, cover_blur)]
        #[qproperty(bool, drift)]
        #[qproperty(bool, tint_ui)]
        #[qproperty(bool, show_lyrics)]
        #[qproperty(bool, close_to_tray)]
        #[qproperty(QString, window_controls)]
        #[qproperty(i32, cache_size_mb, READ, NOTIFY)]
        #[qproperty(i32, cover_cache_limit_mb)]
        #[qproperty(i32, audio_cache_limit_mb)]
        #[qproperty(i32, used_cache_mb, READ, NOTIFY)]
        #[qproperty(QString, player_bar_visualizer)]
        #[qproperty(QString, connect_device_name)]
        #[qproperty(i32, audio_quality)]
        #[qproperty(bool, normalize_volume)]
        #[qproperty(bool, autoplay)]
        #[qproperty(bool, gapless)]
        #[qproperty(i32, output_buffer_ms)]
        #[qproperty(bool, audio_cache_enabled)]
        #[qproperty(bool, sidebar_compact)]
        #[qproperty(bool, compact_tracks)]
        #[qproperty(f64, ui_zoom)]
        #[qproperty(QString, library_sort)]
        #[qproperty(QString, playback_status, READ, NOTIFY)]
        #[qproperty(bool, playback_restart_required, READ, NOTIFY)]
        #[qproperty(i32, used_audio_cache_mb, READ, NOTIFY)]
        #[qproperty(bool, storage_busy, READ, NOTIFY)]
        #[qproperty(bool, credentials_stored, READ, NOTIFY)]
        #[qproperty(QString, proxy_mode)]
        #[qproperty(QString, proxy_host)]
        #[qproperty(QString, proxy_user)]
        #[qproperty(i32, proxy_port)]
        #[qproperty(bool, proxy_password_stored, READ, NOTIFY)]
        #[qproperty(bool, proxy_busy, READ, NOTIFY)]
        #[qproperty(QString, proxy_error, READ, NOTIFY)]
        #[qproperty(QList_QString, supported_proxy_modes, READ, NOTIFY)]
        #[qproperty(bool, eq_enabled)]
        #[qproperty(f64, eq_preamp_db)]
        #[qproperty(QList_f64, eq_gains_db, READ, NOTIFY)]
        #[qproperty(QString, eq_preset, READ, NOTIFY)]
        #[qproperty(QList_QString, eq_presets, READ, CONSTANT)]
        type Settings = super::SettingsRust;

        #[qinvokable]
        fn clear_cache(self: Pin<&mut Settings>);

        #[qinvokable]
        fn refresh_cache_size(self: Pin<&mut Settings>);

        #[qinvokable]
        fn open_config_folder(self: &Settings);

        #[qinvokable]
        fn reconnect_playback(self: Pin<&mut Settings>);
        #[qinvokable]
        fn clear_audio_cache(self: Pin<&mut Settings>);
        #[qinvokable]
        fn clear_history(self: Pin<&mut Settings>);
        #[qinvokable]
        fn apply_proxy(self: Pin<&mut Settings>, password: QString, replace_password: bool);
        #[qinvokable]
        fn clear_proxy_password(self: Pin<&mut Settings>);
        #[qinvokable]
        fn set_eq_band(self: Pin<&mut Settings>, index: i32, db: f64);
        #[qinvokable]
        fn set_eq_preset(self: Pin<&mut Settings>, preset: QString);
        #[qinvokable]
        fn reset_eq(self: Pin<&mut Settings>);
    }

    impl cxx_qt::Threading for Settings {}
    impl cxx_qt::Initialize for Settings {}
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString};

use crate::engine::core;

pub struct SettingsRust {
    theme: QString,
    language: QString,
    reduce_motion: bool,
    performance_mode: bool,
    pause_in_background: bool,
    performance_keep_visualizer: bool,
    cover_blur: bool,
    drift: bool,
    tint_ui: bool,
    show_lyrics: bool,
    close_to_tray: bool,
    window_controls: QString,
    cache_size_mb: i32,
    cover_cache_limit_mb: i32,
    audio_cache_limit_mb: i32,
    used_cache_mb: i32,
    player_bar_visualizer: QString,
    connect_device_name: QString,
    audio_quality: i32,
    normalize_volume: bool,
    autoplay: bool,
    gapless: bool,
    output_buffer_ms: i32,
    audio_cache_enabled: bool,
    sidebar_compact: bool,
    compact_tracks: bool,
    ui_zoom: f64,
    library_sort: QString,
    playback_status: QString,
    playback_restart_required: bool,
    used_audio_cache_mb: i32,
    storage_busy: bool,
    credentials_stored: bool,
    proxy_mode: QString,
    proxy_host: QString,
    proxy_user: QString,
    proxy_port: i32,
    proxy_password_stored: bool,
    proxy_busy: bool,
    proxy_error: QString,
    supported_proxy_modes: QList<QString>,
    eq_enabled: bool,
    eq_preamp_db: f64,
    eq_gains_db: QList<f64>,
    eq_preset: QString,
    eq_presets: QList<QString>,
    refreshing: bool,
}

impl Default for SettingsRust {
    fn default() -> Self {
        let prefs = &core().env.prefs;
        let s = prefs.settings();
        Self {
            theme: QString::from(s.appearance.theme.as_str()),
            language: QString::from(s.appearance.language.as_str()),
            reduce_motion: s.appearance.reduce_motion,
            performance_mode: s.appearance.performance_mode,
            pause_in_background: s.appearance.pause_in_background,
            performance_keep_visualizer: s.appearance.performance_keep_visualizer,
            cover_blur: s.background.cover_blur,
            drift: s.background.drift,
            tint_ui: s.background.tint_ui,
            show_lyrics: s.playback.show_lyrics,
            close_to_tray: prefs.extras().close_to_tray,
            window_controls: QString::from(prefs.extras().window_controls.as_str()),
            cache_size_mb: 0,
            cover_cache_limit_mb: s.cache.cover_limit_mb as i32,
            audio_cache_limit_mb: s.cache.audio_limit_mb as i32,
            used_cache_mb: 0,
            player_bar_visualizer: QString::from(s.visualizer.player_bar_mode.as_str()),
            connect_device_name: QString::from(s.playback.device_name.as_str()),
            audio_quality: s.playback.quality as i32,
            normalize_volume: s.playback.normalize_volume,
            autoplay: s.playback.autoplay,
            gapless: s.playback.gapless,
            output_buffer_ms: s.playback.output_buffer_ms as i32,
            audio_cache_enabled: s.cache.audio_enabled,
            sidebar_compact: s.appearance.sidebar_compact,
            compact_tracks: s.appearance.compact_tracks,
            ui_zoom: s.appearance.ui_zoom,
            library_sort: QString::from(s.library.sort.as_str()),
            playback_status: QString::from("off"),
            playback_restart_required: false,
            used_audio_cache_mb: 0,
            storage_busy: false,
            credentials_stored: false,
            proxy_mode: QString::from(s.proxy.mode.as_str()),
            proxy_host: QString::from(s.proxy.host.as_str()),
            proxy_user: QString::from(s.proxy.user.as_str()),
            proxy_port: s.proxy.port as i32,
            proxy_password_stored: false,
            proxy_busy: false,
            proxy_error: QString::default(),
            supported_proxy_modes: proxy_modes(),
            eq_enabled: s.equalizer.enabled,
            eq_preamp_db: f64::from(s.equalizer.preamp_db),
            eq_gains_db: s.equalizer.gains_db.into_iter().map(f64::from).collect(),
            eq_preset: QString::from(s.equalizer.preset.as_str()),
            eq_presets: velo_domain::EQ_PRESETS
                .iter()
                .map(|p| QString::from(p.name))
                .collect(),
            refreshing: false,
        }
    }
}

/// Bytes → whole MB, rounded up so a non-empty cache never reads 0.
pub fn bytes_to_mb(bytes: u64) -> i32 {
    i32::try_from(bytes.div_ceil(1024 * 1024)).unwrap_or(i32::MAX)
}

/// Pushes every QML-visible value to the preferences (they ignore unchanged values).
fn persist(q: &ffi::Settings) {
    if q.refreshing {
        return;
    }
    let prefs = &core().env.prefs;
    prefs.update_settings(|s| {
        s.appearance.theme = q.theme().to_string();
        s.appearance.language = q.language().to_string();
        s.appearance.reduce_motion = *q.reduce_motion();
        s.appearance.performance_mode = *q.performance_mode();
        s.appearance.pause_in_background = *q.pause_in_background();
        s.appearance.performance_keep_visualizer = *q.performance_keep_visualizer();
        s.background.cover_blur = *q.cover_blur();
        s.background.drift = *q.drift();
        s.background.tint_ui = *q.tint_ui();
        s.playback.show_lyrics = *q.show_lyrics();
        s.cache.cover_limit_mb = (*q.cover_cache_limit_mb()).clamp(1, 500) as u32;
        s.cache.audio_limit_mb =
            velo_domain::audio_cache_limit((*q.audio_cache_limit_mb()).max(0) as u32);
        s.visualizer.player_bar_mode = q.player_bar_visualizer().to_string();
        s.playback.device_name = q.connect_device_name().to_string();
        s.playback.quality = (*q.audio_quality()).max(0) as u32;
        s.playback.normalize_volume = *q.normalize_volume();
        s.playback.autoplay = *q.autoplay();
        s.playback.gapless = *q.gapless();
        s.playback.output_buffer_ms = (*q.output_buffer_ms()).max(0) as u32;
        s.cache.audio_enabled = *q.audio_cache_enabled();
        s.appearance.sidebar_compact = *q.sidebar_compact();
        s.appearance.compact_tracks = *q.compact_tracks();
        s.appearance.ui_zoom = *q.ui_zoom();
        s.library.sort = q.library_sort().to_string();
        s.equalizer.enabled = *q.eq_enabled();
        let preamp = *q.eq_preamp_db() as f32;
        if preamp != s.equalizer.preamp_db {
            s.equalizer.set_preamp(preamp);
        }
    });
    let controls = q.window_controls().to_string();
    prefs.update_extras(|e| {
        e.window_controls = controls;
        e.close_to_tray = *q.close_to_tray();
    });
}

fn proxy_modes() -> QList<QString> {
    let modes = if core().env.audio.settings_supported() {
        velo_services::TomlSettingsStore::supported_proxy_modes()
    } else {
        &["off"]
    };
    modes.iter().map(|mode| QString::from(*mode)).collect()
}
fn persist_and_sync(mut q: Pin<&mut ffi::Settings>) {
    if q.refreshing {
        return;
    }
    persist(&q);
    q.as_mut().sync_preferences(&core().env.prefs.settings());
    core()
        .env
        .audio
        .settings_runtime
        .saved(&core().env.prefs.settings());
}

impl cxx_qt::Initialize for ffi::Settings {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut()
            .on_player_bar_visualizer_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_connect_device_name_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_audio_quality_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_normalize_volume_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_autoplay_changed(persist_and_sync)
            .release();
        self.as_mut().on_gapless_changed(persist_and_sync).release();
        self.as_mut()
            .on_output_buffer_ms_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_audio_cache_enabled_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_sidebar_compact_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_compact_tracks_changed(persist_and_sync)
            .release();
        self.as_mut().on_ui_zoom_changed(persist_and_sync).release();
        self.as_mut()
            .on_library_sort_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_eq_enabled_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_eq_preamp_db_changed(persist_and_sync)
            .release();
        self.as_mut()
            .on_close_to_tray_changed(|q| persist(&q))
            .release();
        self.as_mut().on_theme_changed(|q| persist(&q)).release();
        self.as_mut().on_language_changed(|q| persist(&q)).release();
        self.as_mut()
            .on_reduce_motion_changed(|q| persist(&q))
            .release();
        self.as_mut()
            .on_performance_mode_changed(|q| persist(&q))
            .release();
        self.as_mut()
            .on_pause_in_background_changed(|q| persist(&q))
            .release();
        self.as_mut()
            .on_performance_keep_visualizer_changed(|q| persist(&q))
            .release();
        self.as_mut()
            .on_cover_blur_changed(|q| persist(&q))
            .release();
        self.as_mut().on_drift_changed(|q| persist(&q)).release();
        self.as_mut().on_tint_ui_changed(|q| persist(&q)).release();
        self.as_mut()
            .on_show_lyrics_changed(|q| persist(&q))
            .release();
        self.as_mut()
            .on_window_controls_changed(|q| persist(&q))
            .release();
        self.as_mut()
            .on_cover_cache_limit_mb_changed(|mut q| {
                let value = (*q.cover_cache_limit_mb()).clamp(1, 500);
                q.as_mut().set_cover_cache_limit_mb(value);
                persist(&q);
            })
            .release();
        self.as_mut()
            .on_audio_cache_limit_mb_changed(|mut q| {
                if q.refreshing { return; }
                let value = velo_domain::audio_cache_limit((*q.audio_cache_limit_mb()).max(0) as u32) as i32;
                q.as_mut().set_audio_cache_limit_mb(value);
                persist_and_sync(q);
            })
            .release();
        let env = core().env.clone();
        let thread = self.qt_thread();
        let mut preferences = env.prefs.watch_settings();
        let mut runtime = env.audio.settings_runtime.view.subscribe();
        let mut audio = env.audio.view.subscribe();
        env.rt.clone().spawn(async move {
            loop {
                let settings = preferences.borrow_and_update().clone();
                let state = runtime.borrow_and_update().clone();
                let playback = *audio.borrow_and_update();
                let _ = thread.queue(move |mut q| {
                    if core().env.prefs.settings() == settings {
                        q.as_mut().sync_preferences(&settings);
                    }
                    q.as_mut().sync_runtime(state, playback);
                });
                tokio::select! {
                    result = preferences.changed() => if result.is_err() { break; },
                    result = runtime.changed() => if result.is_err() { break; },
                    result = audio.changed() => {
                        if result.is_err() { break; }
                        env.audio.settings_runtime.refresh_credentials().await;
                    },
                }
            }
        });
        self.refresh_cache_size();
    }
}

impl ffi::Settings {
    fn sync_preferences(mut self: Pin<&mut Self>, s: &velo_domain::Settings) {
        self.as_mut().rust_mut().refreshing = true;
        self.as_mut().set_cover_blur(s.background.cover_blur);
        self.as_mut().set_drift(s.background.drift);
        self.as_mut().set_tint_ui(s.background.tint_ui);
        self.as_mut()
            .set_player_bar_visualizer(QString::from(s.visualizer.player_bar_mode.as_str()));
        self.as_mut()
            .set_connect_device_name(QString::from(s.playback.device_name.as_str()));
        self.as_mut().set_audio_quality(s.playback.quality as i32);
        self.as_mut()
            .set_normalize_volume(s.playback.normalize_volume);
        self.as_mut().set_autoplay(s.playback.autoplay);
        self.as_mut().set_gapless(s.playback.gapless);
        self.as_mut()
            .set_output_buffer_ms(s.playback.output_buffer_ms as i32);
        self.as_mut().set_audio_cache_enabled(s.cache.audio_enabled);
        self.as_mut()
            .set_audio_cache_limit_mb(s.cache.audio_limit_mb as i32);
        self.as_mut()
            .set_sidebar_compact(s.appearance.sidebar_compact);
        self.as_mut()
            .set_compact_tracks(s.appearance.compact_tracks);
        self.as_mut().set_ui_zoom(s.appearance.ui_zoom);
        self.as_mut()
            .set_library_sort(QString::from(s.library.sort.as_str()));
        self.as_mut().set_eq_enabled(s.equalizer.enabled);
        self.as_mut()
            .set_eq_preamp_db(f64::from(s.equalizer.preamp_db));
        let gains: QList<f64> = s.equalizer.gains_db.into_iter().map(f64::from).collect();
        if self.eq_gains_db != gains {
            self.as_mut().rust_mut().eq_gains_db = gains;
            self.as_mut().eq_gains_db_changed();
        }
        let preset = QString::from(s.equalizer.preset.as_str());
        if self.eq_preset != preset {
            self.as_mut().rust_mut().eq_preset = preset;
            self.as_mut().eq_preset_changed();
        }
        self.as_mut().rust_mut().refreshing = false;
    }

    fn sync_runtime(
        mut self: Pin<&mut Self>,
        state: crate::engine::audio::settings_runtime::RuntimeView,
        playback: crate::engine::audio::AudioView,
    ) {
        macro_rules! update {
            ($field:ident, $notify:ident, $value:expr) => {
                if self.$field != $value {
                    self.as_mut().rust_mut().$field = $value;
                    self.as_mut().$notify();
                }
            };
        }
        update!(
            playback_restart_required,
            playback_restart_required_changed,
            state.restart_required
        );
        update!(storage_busy, storage_busy_changed, state.storage_busy);
        update!(
            credentials_stored,
            credentials_stored_changed,
            state.credentials_stored
        );
        update!(
            proxy_password_stored,
            proxy_password_stored_changed,
            state.proxy_password_stored
        );
        update!(proxy_busy, proxy_busy_changed, state.proxy_busy);
        let error = QString::from(state.proxy_error.as_str());
        update!(proxy_error, proxy_error_changed, error);
        let modes = proxy_modes();
        update!(supported_proxy_modes, supported_proxy_modes_changed, modes);
        let status = if playback.ready {
            "ready"
        } else if playback.busy {
            if state.reconnecting {
                "reconnecting"
            } else {
                "connecting"
            }
        } else if state.playback_error {
            "error"
        } else {
            "off"
        };
        let status = QString::from(status);
        update!(playback_status, playback_status_changed, status);
    }

    pub fn set_eq_band(mut self: Pin<&mut Self>, index: i32, db: f64) {
        if !(0..10).contains(&index) || !db.is_finite() {
            return;
        }
        core().env.prefs.update_settings(|s| {
            s.equalizer
                .set_band(index as usize, db.clamp(-12., 12.) as f32);
        });
        self.as_mut().sync_preferences(&core().env.prefs.settings());
    }
    pub fn set_eq_preset(mut self: Pin<&mut Self>, preset: QString) {
        core().env.prefs.update_settings(|s| {
            s.equalizer.set_preset(&preset.to_string());
        });
        self.as_mut().sync_preferences(&core().env.prefs.settings());
    }
    pub fn reset_eq(mut self: Pin<&mut Self>) {
        core().env.prefs.update_settings(|s| s.equalizer.reset());
        self.as_mut().sync_preferences(&core().env.prefs.settings());
    }
    pub fn reconnect_playback(self: Pin<&mut Self>) {
        let env = core().env.clone();
        env.rt.clone().spawn(async move {
            let _ = env.audio.reconnect_playback(false).await;
        });
    }
    pub fn clear_audio_cache(self: Pin<&mut Self>) {
        let env = core().env.clone();
        let thread = self.qt_thread();
        env.rt.clone().spawn(async move {
            let _ = env.audio.settings_runtime.clear_audio(&env).await;
            let _ = thread.queue(|q| q.refresh_cache_size());
        });
    }
    pub fn clear_history(self: Pin<&mut Self>) {
        let env = core().env.clone();
        env.rt.clone().spawn(async move {
            if let Err(e) = env.audio.settings_runtime.clear_history().await {
                env.report(&e);
            }
        });
    }
    pub fn apply_proxy(self: Pin<&mut Self>, password: QString, replace_password: bool) {
        let draft = velo_domain::ProxySettings {
            mode: self.proxy_mode().to_string(),
            host: self.proxy_host().to_string(),
            port: (*self.proxy_port()).max(0) as u32,
            user: self.proxy_user().to_string(),
        };
        let env = core().env.clone();
        let password = password.to_string();
        env.rt.clone().spawn(async move {
            if let Err(e) = env
                .audio
                .settings_runtime
                .apply_proxy(&env, draft, password, replace_password)
                .await
            {
                env.audio
                    .settings_runtime
                    .view
                    .send_modify(|v| v.proxy_error = e.to_string());
            }
        });
    }
    pub fn clear_proxy_password(self: Pin<&mut Self>) {
        let env = core().env.clone();
        env.rt.clone().spawn(async move {
            if let Err(e) = env.audio.settings_runtime.clear_proxy_password().await {
                env.audio
                    .settings_runtime
                    .view
                    .send_modify(|v| v.proxy_error = e.to_string());
            }
        });
    }
    fn set_cache_size_value(mut self: Pin<&mut Self>, value: i32) {
        if self.used_cache_mb != value {
            self.as_mut().rust_mut().used_cache_mb = value;
            self.as_mut().used_cache_mb_changed();
        }
        if self.cache_size_mb != value {
            self.as_mut().rust_mut().cache_size_mb = value;
            self.as_mut().cache_size_mb_changed();
        }
    }

    /// Measures the cache on the runtime and publishes the result.
    pub fn refresh_cache_size(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        let env = core().env.clone();
        env.rt.spawn({
            let env = env.clone();
            async move {
                if let Ok(bytes) = crate::engine::audio::settings_runtime::audio_size_bytes().await
                {
                    let mb = bytes_to_mb(bytes);
                    let _ = thread.queue(move |mut q| {
                        if q.used_audio_cache_mb != mb {
                            q.as_mut().rust_mut().used_audio_cache_mb = mb;
                            q.as_mut().used_audio_cache_mb_changed();
                        }
                    });
                }
                match env.covers.cache_size_bytes().await {
                    Ok(bytes) => {
                        let mb = bytes_to_mb(bytes);
                        let _ = thread.queue(move |q| q.set_cache_size_value(mb));
                    }
                    Err(e) => env.report(&e),
                }
            }
        });
    }

    pub fn clear_cache(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        let env = core().env.clone();
        env.rt.spawn({
            let env = env.clone();
            async move {
                if let Err(e) = env.covers.prune(0).await {
                    env.report(&e);
                }
                if let Ok(bytes) = env.covers.cache_size_bytes().await {
                    let mb = bytes_to_mb(bytes);
                    let _ = thread.queue(move |q| q.set_cache_size_value(mb));
                }
            }
        });
    }

    pub fn open_config_folder(&self) {
        let dir = core().env.prefs.config_dir().to_path_buf();
        core().env.rt.spawn_blocking(move || {
            #[cfg(windows)]
            let opener = "explorer";
            #[cfg(not(windows))]
            let opener = "xdg-open";
            if let Err(e) = std::process::Command::new(opener).arg(&dir).spawn() {
                tracing::warn!(%e, "could not open the config folder");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::bytes_to_mb;

    #[test]
    fn cache_size_rounds_up_to_whole_megabytes() {
        assert_eq!(bytes_to_mb(0), 0);
        assert_eq!(bytes_to_mb(1), 1);
        assert_eq!(bytes_to_mb(1024 * 1024), 1);
        assert_eq!(bytes_to_mb(84 * 1024 * 1024 + 1), 85);
    }
}
