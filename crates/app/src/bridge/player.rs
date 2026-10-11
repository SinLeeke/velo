//! `Player` singleton (docs/CONTRATO.md). A thin view of `engine::player`: state arrives through
//! watch channels (no polling), actions return immediately and run on the tokio runtime.
//! `positionMs` is pushed by the engine clock: at most 4 Hz, only while playing and visible.

#[path = "history.rs"]
mod history;

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_QString = cxx_qt_lib::QList<QString>;
        type QList_f64 = cxx_qt_lib::QList<f64>;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
        include!("QtCore/QAbstractListModel");
        type QAbstractListModel = crate::model::ffi::QAbstractListModel;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, title, READ, NOTIFY)]
        #[qproperty(QString, artist, READ, NOTIFY)]
        #[qproperty(QString, album, READ, NOTIFY)]
        #[qproperty(QString, artist_id, READ, NOTIFY)]
        #[qproperty(QString, album_id, READ, NOTIFY)]
        #[qproperty(QString, queue_status, READ, NOTIFY)]
        #[qproperty(QString, queue_error, READ, NOTIFY)]
        #[qproperty(QString, track_uri, READ, NOTIFY)]
        #[qproperty(QUrl, cover_url, READ, NOTIFY)]
        #[qproperty(QUrl, art_url, READ, NOTIFY)]
        #[qproperty(bool, dj_available, READ, NOTIFY)]
        #[qproperty(bool, dj_active, READ, NOTIFY)]
        #[qproperty(bool, dj_speaking, READ, NOTIFY)]
        #[qproperty(QString, dj_line, READ, NOTIFY)]
        #[qproperty(bool, mixing, READ, NOTIFY)]
        #[qproperty(bool, mix_available, READ, NOTIFY)]
        #[qproperty(QString, context_uri, READ, NOTIFY)]
        #[qproperty(QString, context_title, READ, NOTIFY)]
        #[qproperty(QUrl, blur_url, READ, NOTIFY)]
        #[qproperty(i32, duration_ms, READ, NOTIFY)]
        #[qproperty(i32, position_ms, READ, NOTIFY)]
        #[qproperty(bool, is_playing, READ, NOTIFY)]
        #[qproperty(bool, shuffle, READ, NOTIFY)]
        #[qproperty(i32, repeat_mode, READ, NOTIFY)]
        #[qproperty(i32, volume, READ, NOTIFY)]
        #[qproperty(i32, last_audible_volume, READ, NOTIFY)]
        #[qproperty(bool, liked, READ, NOTIFY)]
        #[qproperty(bool, can_control, READ, NOTIFY)]
        #[qproperty(bool, queue_editable, READ, NOTIFY)]
        #[qproperty(bool, queue_clearable, READ, NOTIFY)]
        #[qproperty(QString, device_name, READ, NOTIFY)]
        #[qproperty(bool, remote, READ, NOTIFY)]
        #[qproperty(QString, device_icon, READ, NOTIFY)]
        #[qproperty(bool, has_session, READ, NOTIFY)]
        #[qproperty(QString, palette_base, READ, NOTIFY)]
        #[qproperty(QString, palette_glow, READ, NOTIFY)]
        #[qproperty(QString, palette_veil_dark, READ, NOTIFY)]
        #[qproperty(QString, palette_veil_light, READ, NOTIFY)]
        #[qproperty(QString, palette_ink_dark, READ, NOTIFY)]
        #[qproperty(QString, palette_ink_light, READ, NOTIFY)]
        #[qproperty(QList_QString, palette_colors, READ, NOTIFY)]
        #[qproperty(QList_f64, palette_weights, READ, NOTIFY)]
        #[qproperty(f64, palette_veil_dark_opacity, READ, NOTIFY)]
        #[qproperty(f64, palette_veil_light_opacity, READ, NOTIFY)]
        #[qproperty(f64, palette_veil_dark_moving_opacity, READ, NOTIFY)]
        #[qproperty(f64, palette_veil_light_moving_opacity, READ, NOTIFY)]
        #[qproperty(f64, palette_blob_dark_moving_opacity, READ, NOTIFY)]
        #[qproperty(f64, palette_blob_light_moving_opacity, READ, NOTIFY)]
        #[qproperty(f64, palette_scrim_dark_opacity, READ, NOTIFY)]
        #[qproperty(f64, palette_scrim_light_opacity, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, queue, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, devices, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, outputs, READ, CONSTANT)]
        type Player = super::PlayerRust;

        #[qsignal]
        fn palette_ready(self: Pin<&mut Player>);

        #[qsignal]
        fn error(self: Pin<&mut Player>, message: &QString);

        #[qsignal]
        fn devices_refreshed(self: Pin<&mut Player>, error_code: &QString);

        #[qsignal]
        fn queue_saved(self: Pin<&mut Player>, playlist_id: &QString, name: &QString);

        #[qinvokable]
        fn play_pause(self: &Player);
        #[qinvokable]
        fn next(self: &Player);
        #[qinvokable]
        fn previous(self: &Player);
        #[qinvokable]
        fn seek(self: &Player, ms: i32);
        #[qinvokable]
        fn set_volume(self: Pin<&mut Player>, v: i32);
        #[qinvokable]
        fn toggle_shuffle(self: &Player);
        #[qinvokable]
        fn cycle_repeat(self: &Player);
        #[qinvokable]
        fn toggle_liked(self: &Player);
        #[qinvokable]
        fn refresh_queue(self: &Player);
        #[qinvokable]
        fn clear_queue(self: &Player);
        #[qinvokable]
        fn save_queue_as_playlist(self: &Player, name: &QString);
        #[qinvokable]
        fn remove_from_queue(self: &Player, index: i32);
        #[qinvokable]
        fn move_in_queue(self: &Player, from: i32, to: i32);
        #[qinvokable]
        fn skip_to_queue(self: &Player, index: i32);
        #[qinvokable]
        fn inspect_queue_track(self: &Player, index: i32);
        #[qinvokable]
        fn set_queue_liked(self: &Player, index: i32, saved: bool);
        #[qinvokable]
        fn refresh_devices(self: &Player);
        #[qinvokable]
        fn refresh_outputs(self: &Player);
        #[qinvokable]
        fn set_output(self: &Player, output_id: &QString);
        #[qinvokable]
        fn transfer_to(self: &Player, device_id: &QString);
        #[qinvokable]
        fn start_radio(self: &Player, seed_uri: &QString);
    }

    impl cxx_qt::Threading for Player {}
    impl cxx_qt::Initialize for Player {}
}

use core::pin::Pin;

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString, QUrl};
use velo_domain::Palette;

use crate::engine::core;
use crate::engine::player::QUEUE_ROLES;
use crate::engine::player::{DEVICES_ROLES, PlayerUi, PlayerView};
use crate::model::ListModel;

pub struct PlayerRust {
    title: QString,
    artist: QString,
    album: QString,
    artist_id: QString,
    album_id: QString,
    queue_status: QString,
    queue_error: QString,
    track_uri: QString,
    cover_url: QUrl,
    art_url: QUrl,
    dj_available: bool,
    dj_active: bool,
    dj_speaking: bool,
    dj_line: QString,
    mixing: bool,
    mix_available: bool,
    context_uri: QString,
    context_title: QString,
    blur_url: QUrl,
    duration_ms: i32,
    position_ms: i32,
    is_playing: bool,
    shuffle: bool,
    repeat_mode: i32,
    volume: i32,
    last_audible_volume: i32,
    liked: bool,
    can_control: bool,
    queue_editable: bool,
    queue_clearable: bool,
    device_name: QString,
    remote: bool,
    device_icon: QString,
    has_session: bool,
    palette_base: QString,
    palette_glow: QString,
    palette_veil_dark: QString,
    palette_veil_light: QString,
    palette_ink_dark: QString,
    palette_ink_light: QString,
    palette_colors: QList<QString>,
    palette_weights: QList<f64>,
    palette_veil_dark_opacity: f64,
    palette_veil_light_opacity: f64,
    palette_veil_dark_moving_opacity: f64,
    palette_veil_light_moving_opacity: f64,
    palette_blob_dark_moving_opacity: f64,
    palette_blob_light_moving_opacity: f64,
    palette_scrim_dark_opacity: f64,
    palette_scrim_light_opacity: f64,
    queue: *mut crate::model::ffi::QAbstractListModel,
    devices: *mut crate::model::ffi::QAbstractListModel,
    outputs: *mut crate::model::ffi::QAbstractListModel,
    queue_model: Option<ListModel>,
    devices_model: Option<ListModel>,
    outputs_model: Option<ListModel>,
    applied_palette: Option<Palette>,
}

impl Default for PlayerRust {
    fn default() -> Self {
        let c = core();
        let v = c.player.view.borrow().clone();
        // Palette defaults match qml/theme/Theme.qml until the first track arrives.
        Self {
            title: QString::from(v.title.as_str()),
            artist: QString::from(v.artist.as_str()),
            album: QString::from(v.album.as_str()),
            artist_id: QString::from(v.artist_id.as_str()),
            album_id: QString::from(v.album_id.as_str()),
            queue_status: QString::from(v.queue_status.as_str()),
            queue_error: QString::from(v.queue_error.as_str()),
            track_uri: QString::from(v.uri.as_str()),
            cover_url: QUrl::from(v.cover_url.as_str()),
            art_url: QUrl::from(v.cover_url.as_str()),
            dj_available: v.dj_available,
            dj_active: v.dj_active,
            dj_speaking: v.dj_speaking,
            dj_line: QString::from(v.dj_line.as_str()),
            mixing: v.mixing,
            mix_available: v.mix_available,
            context_uri: QString::from(v.context_uri.as_str()),
            context_title: QString::from(v.context_title.as_str()),
            blur_url: QUrl::from(v.blur_url.as_str()),
            duration_ms: clamp_ms(v.duration_ms),
            position_ms: clamp_ms(*c.env.position.borrow()),
            is_playing: v.is_playing,
            shuffle: v.shuffle,
            repeat_mode: v.repeat,
            volume: v.volume,
            last_audible_volume: v.volume_memory.audible(),
            liked: v.liked,
            can_control: v.can_control,
            queue_editable: v.queue_editable,
            queue_clearable: v.queue_clearable,
            device_name: QString::from(v.device_name.as_str()),
            remote: v.remote,
            device_icon: QString::from(v.device_icon.as_str()),
            has_session: v.has_session,
            palette_base: QString::from("#2E6F86"),
            palette_glow: QString::from("#7CC9E6"),
            palette_veil_dark: QString::from("#14323D"),
            palette_veil_light: QString::from("#D7E8EE"),
            palette_ink_dark: QString::from("#F4FAFC"),
            palette_ink_light: QString::from("#0D2129"),
            palette_colors: QList::default(),
            palette_weights: QList::default(),
            palette_veil_dark_opacity: 1.0,
            palette_veil_light_opacity: 1.0,
            palette_veil_dark_moving_opacity: 1.0,
            palette_veil_light_moving_opacity: 1.0,
            palette_blob_dark_moving_opacity: 1.0,
            palette_blob_light_moving_opacity: 1.0,
            palette_scrim_dark_opacity: 0.0,
            palette_scrim_light_opacity: 0.0,
            queue: std::ptr::null_mut(),
            devices: std::ptr::null_mut(),
            outputs: std::ptr::null_mut(),
            queue_model: None,
            devices_model: None,
            outputs_model: None,
            applied_palette: None,
        }
    }
}

/// Milliseconds as the QML `int` (saturating).
pub fn clamp_ms(ms: u64) -> i32 {
    i32::try_from(ms).unwrap_or(i32::MAX)
}

impl cxx_qt::Initialize for ffi::Player {
    fn initialize(mut self: Pin<&mut Self>) {
        // Upcast the QObject itself, not the `Pin` (the blanket `Upcast<T> for T` would
        // otherwise hand Qt the address of this stack variable as the parent).
        let parent = std::ptr::from_ref::<cxx_qt::QObject>((*self).upcast())
            .cast_mut()
            .cast();
        let queue = ListModel::new(QUEUE_ROLES, parent);
        let devices = ListModel::new(DEVICES_ROLES, parent);
        let outputs = ListModel::new(crate::engine::audio_output::OUTPUT_ROLES, parent);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.queue = queue.as_model();
            rust.devices = devices.as_model();
            rust.queue_model = Some(queue);
            rust.devices_model = Some(devices);
            rust.outputs = outputs.as_model();
            rust.outputs_model = Some(outputs);
        }

        let c = core();

        // System endpoints arrive by COM notifications, never on a timer.
        let thread = self.qt_thread();
        let mut outputs = c.outputs.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let rows = crate::engine::audio_output::rows(&outputs.borrow_and_update());
                if thread
                    .queue(move |q| {
                        if let Some(model) = &q.outputs_model {
                            model.apply(crate::model::ModelOp::Replace(rows));
                        }
                    })
                    .is_err()
                {
                    break;
                }
                if outputs.changed().await.is_err() {
                    break;
                }
            }
        });

        // State.
        let thread = self.qt_thread();
        let mut view = c.player.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let v = view.borrow_and_update().clone();
                if thread.queue(move |q| q.apply(&v)).is_err() {
                    break;
                }
                if view.changed().await.is_err() {
                    break;
                }
            }
        });

        // Position: already throttled by the engine clock.
        let thread = self.qt_thread();
        let mut position = c.env.position.subscribe();
        c.env.rt.spawn(async move {
            loop {
                position.borrow_and_update();
                let latest = c.env.position.clone();
                if thread
                    .queue(move |q| q.apply_position(clamp_ms(*latest.borrow())))
                    .is_err()
                {
                    break;
                }
                if position.changed().await.is_err() {
                    break;
                }
            }
        });

        // Queue and devices rows.
        if let Some(mut ops) = c.player.ui.take() {
            let thread = self.qt_thread();
            c.env.rt.spawn(async move {
                while let Some(op) = ops.recv().await {
                    if thread.queue(move |q| q.apply_ui(op)).is_err() {
                        break;
                    }
                }
            });
        }

        // User-facing errors.
        let thread = self.qt_thread();
        let mut errors = c.env.errors.subscribe();
        c.env.rt.spawn(async move {
            loop {
                match errors.recv().await {
                    Ok(message) => {
                        let message = QString::from(message.as_str());
                        if thread.queue(move |q| q.error(&message)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

impl ffi::Player {
    pub fn start_radio(&self, seed_uri: &QString) {
        crate::engine::discover::start_radio(&core().env, seed_uri.to_string());
    }

    fn apply(mut self: Pin<&mut Self>, _v: &PlayerView) {
        let latest = core().player.view.borrow().clone();
        let v = &latest;
        let cover = QUrl::from(v.cover_url.as_str());
        let blur = QUrl::from(v.blur_url.as_str());
        let cover_changed = self.cover_url != cover;
        let blur_changed = self.blur_url != blur;
        {
            let mut rust = self.as_mut().rust_mut();
            rust.cover_url = cover.clone();
            rust.art_url = cover;
            rust.blur_url = blur;
        }
        // Source-change handlers must see the new palette and both images together.
        if let Some(p) = &v.palette {
            if self.applied_palette.as_ref() != Some(p) {
                self.as_mut().apply_palette(p);
            }
        }
        if cover_changed {
            self.as_mut().cover_url_changed();
            self.as_mut().art_url_changed();
        }
        if blur_changed {
            self.as_mut().blur_url_changed();
        }
        set_ro!(
            self,
            artist_id,
            artist_id_changed,
            QString::from(v.artist_id.as_str())
        );
        set_ro!(
            self,
            album_id,
            album_id_changed,
            QString::from(v.album_id.as_str())
        );
        set_ro!(
            self,
            queue_status,
            queue_status_changed,
            QString::from(v.queue_status.as_str())
        );
        set_ro!(
            self,
            queue_error,
            queue_error_changed,
            QString::from(v.queue_error.as_str())
        );
        set_ro!(self, dj_available, dj_available_changed, v.dj_available);
        set_ro!(self, dj_active, dj_active_changed, v.dj_active);
        set_ro!(self, dj_speaking, dj_speaking_changed, v.dj_speaking);
        set_ro!(self, mixing, mixing_changed, v.mixing);
        set_ro!(self, mix_available, mix_available_changed, v.mix_available);
        set_ro!(
            self,
            context_uri,
            context_uri_changed,
            QString::from(v.context_uri.as_str())
        );
        set_ro!(
            self,
            context_title,
            context_title_changed,
            QString::from(v.context_title.as_str())
        );
        set_ro!(
            self,
            dj_line,
            dj_line_changed,
            QString::from(v.dj_line.as_str())
        );
        set_ro!(self, title, title_changed, QString::from(v.title.as_str()));
        set_ro!(
            self,
            track_uri,
            track_uri_changed,
            QString::from(v.uri.as_str())
        );
        set_ro!(
            self,
            artist,
            artist_changed,
            QString::from(v.artist.as_str())
        );
        set_ro!(self, album, album_changed, QString::from(v.album.as_str()));
        set_ro!(
            self,
            duration_ms,
            duration_ms_changed,
            clamp_ms(v.duration_ms)
        );
        set_ro!(self, is_playing, is_playing_changed, v.is_playing);
        set_ro!(self, shuffle, shuffle_changed, v.shuffle);
        set_ro!(self, repeat_mode, repeat_mode_changed, v.repeat);
        set_ro!(self, volume, volume_changed, v.volume);
        // Includes slider/shortcut changes and authoritative local or Connect events.
        set_ro!(
            self,
            last_audible_volume,
            last_audible_volume_changed,
            v.volume_memory.audible()
        );
        set_ro!(self, liked, liked_changed, v.liked);
        set_ro!(self, can_control, can_control_changed, v.can_control);
        set_ro!(
            self,
            queue_editable,
            queue_editable_changed,
            v.queue_editable
        );
        set_ro!(
            self,
            queue_clearable,
            queue_clearable_changed,
            v.queue_clearable
        );
        set_ro!(
            self,
            device_name,
            device_name_changed,
            QString::from(v.device_name.as_str())
        );
        set_ro!(self, remote, remote_changed, v.remote);
        set_ro!(
            self,
            device_icon,
            device_icon_changed,
            QString::from(v.device_icon.as_str())
        );
        set_ro!(self, has_session, has_session_changed, v.has_session);
    }

    /// All palette properties change together, then `paletteReady()` fires.
    fn apply_palette(mut self: Pin<&mut Self>, p: &Palette) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.palette_base = QString::from(p.base.as_str());
            rust.palette_glow = QString::from(p.glow.as_str());
            rust.palette_veil_dark = QString::from(p.veil_dark.as_str());
            rust.palette_veil_light = QString::from(p.veil_light.as_str());
            rust.palette_ink_dark = QString::from(p.ink_dark.as_str());
            rust.palette_ink_light = QString::from(p.ink_light.as_str());
            rust.palette_colors = p
                .colors
                .iter()
                .map(|(color, _)| QString::from(color.as_str()))
                .collect();
            rust.palette_weights = p
                .colors
                .iter()
                .map(|(_, weight)| f64::from(*weight))
                .collect();
            rust.palette_veil_dark_opacity = f64::from(p.veil_dark_alpha) / 255.0;
            rust.palette_veil_light_opacity = f64::from(p.veil_light_alpha) / 255.0;
            rust.palette_veil_dark_moving_opacity = f64::from(p.veil_dark_moving_alpha) / 255.0;
            rust.palette_veil_light_moving_opacity = f64::from(p.veil_light_moving_alpha) / 255.0;
            rust.palette_blob_dark_moving_opacity =
                f64::from(p.blob_dark_moving_alpha.unwrap_or(255)) / 255.0;
            rust.palette_blob_light_moving_opacity =
                f64::from(p.blob_light_moving_alpha.unwrap_or(255)) / 255.0;
            rust.palette_scrim_dark_opacity = f64::from(p.scrim_dark_alpha) / 255.0;
            rust.palette_scrim_light_opacity = f64::from(p.scrim_light_alpha) / 255.0;
            rust.applied_palette = Some(p.clone());
        }
        self.as_mut().palette_base_changed();
        self.as_mut().palette_glow_changed();
        self.as_mut().palette_veil_dark_changed();
        self.as_mut().palette_veil_light_changed();
        self.as_mut().palette_ink_dark_changed();
        self.as_mut().palette_ink_light_changed();
        self.as_mut().palette_colors_changed();
        self.as_mut().palette_weights_changed();
        self.as_mut().palette_veil_dark_opacity_changed();
        self.as_mut().palette_veil_light_opacity_changed();
        self.as_mut().palette_veil_dark_moving_opacity_changed();
        self.as_mut().palette_veil_light_moving_opacity_changed();
        self.as_mut().palette_blob_dark_moving_opacity_changed();
        self.as_mut().palette_blob_light_moving_opacity_changed();
        self.as_mut().palette_scrim_dark_opacity_changed();
        self.as_mut().palette_scrim_light_opacity_changed();
        self.as_mut().palette_ready();
    }

    fn apply_position(mut self: Pin<&mut Self>, ms: i32) {
        // The engine gates the ticker. Events (seek, pause, new anchors) also reach Qt.
        set_ro!(self, position_ms, position_ms_changed, ms);
    }

    fn apply_ui(self: Pin<&mut Self>, op: PlayerUi) {
        match op {
            PlayerUi::RefreshPlaylists => core().library.refresh_playlists(),
            PlayerUi::QueueSaved { playlist_id, name } => {
                self.queue_saved(
                    &QString::from(playlist_id.as_str()),
                    &QString::from(name.as_str()),
                );
            }
            PlayerUi::Queue(op) => {
                if let Some(m) = &self.queue_model {
                    m.apply(op);
                }
            }
            PlayerUi::DevicesFinished { rows, error } => {
                if let (Some(m), Some(op)) = (&self.devices_model, rows) {
                    m.apply(op);
                }
                self.devices_refreshed(&QString::from(error));
            }
        }
    }

    pub fn play_pause(&self) {
        core().player.play_pause();
    }

    pub fn next(&self) {
        core().player.next();
    }

    pub fn previous(&self) {
        core().player.previous();
    }

    pub fn seek(&self, ms: i32) {
        core().player.seek(i64::from(ms));
    }

    pub fn set_volume(mut self: Pin<&mut Self>, v: i32) {
        core().player.set_volume(v);
        let view = core().player.view.borrow().clone();
        set_ro!(self, volume, volume_changed, view.volume);
        set_ro!(
            self,
            last_audible_volume,
            last_audible_volume_changed,
            view.volume_memory.audible()
        );
    }

    pub fn toggle_shuffle(&self) {
        core().player.toggle_shuffle();
    }

    pub fn cycle_repeat(&self) {
        core().player.cycle_repeat();
    }

    pub fn toggle_liked(&self) {
        core().player.toggle_liked();
    }

    pub fn refresh_queue(&self) {
        core().player.refresh_queue();
    }
    pub fn clear_queue(&self) {
        core().player.clear_queue();
    }
    pub fn save_queue_as_playlist(&self, name: &QString) {
        core().player.save_queue_as_playlist(name.to_string());
    }
    pub fn remove_from_queue(&self, index: i32) {
        core().player.remove_from_queue(index);
    }
    pub fn skip_to_queue(&self, index: i32) {
        core().player.skip_to_queue(index);
    }
    pub fn move_in_queue(&self, from: i32, to: i32) {
        core().player.move_in_queue(from, to);
    }
    pub fn inspect_queue_track(&self, index: i32) {
        core().player.inspect_queue_track(index);
    }
    pub fn set_queue_liked(&self, index: i32, saved: bool) {
        core().player.set_queue_liked(index, saved);
    }

    pub fn refresh_devices(&self) {
        core().player.refresh_devices();
    }

    pub fn refresh_outputs(&self) {
        core().outputs.refresh();
    }

    pub fn set_output(&self, output_id: &QString) {
        core().outputs.set(output_id.to_string());
    }

    pub fn transfer_to(&self, device_id: &QString) {
        core().player.transfer_to(device_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_ms;

    #[test]
    fn milliseconds_saturate_at_the_qml_int() {
        assert_eq!(clamp_ms(261_000), 261_000);
        assert_eq!(clamp_ms(u64::MAX), i32::MAX);
    }
}
