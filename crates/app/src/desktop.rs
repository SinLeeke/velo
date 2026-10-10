//! Política portable de cierre y acciones de la bandeja. Ningún trabajo pesado en Qt.

#[unsafe(no_mangle)]
pub extern "C" fn velo_should_hide_on_close(
    preference: bool,
    tray_available: bool,
    quitting: bool,
) -> bool {
    preference && tray_available && !quitting
}

#[unsafe(no_mangle)]
pub extern "C" fn velo_desktop_action(action: u32) {
    let c = crate::engine::core();
    c.env.rt.spawn(async move {
        let player = &crate::engine::core().player;
        match action {
            1 => player.play_pause(),
            2 => player.next(),
            3 => player.previous(),
            6 => player.toggle_liked(),
            _ => {}
        }
    });
}

unsafe extern "C" {
    fn velo_desktop_install(engine: *mut std::ffi::c_void);
    fn velo_desktop_show();
    fn velo_window_command(cmd: u32);
    fn velo_set_maximize_button_rect(x: f64, y: f64, width: f64, height: f64);
    fn velo_desktop_update(
        tooltip: *const std::ffi::c_char,
        playing: bool,
        controllable: bool,
        liked: bool,
        like_available: bool,
    );
}

pub fn install(engine: &mut cxx::UniquePtr<cxx_qt_lib::QQmlApplicationEngine>) {
    // SAFETY: engine vive durante la llamada; la bandeja se parenta a la ventana.
    unsafe {
        velo_desktop_install(engine.as_mut_ptr().cast());
    }
}

/// Solo desde el hilo de Qt (invocable/qt_thread.queue).
pub fn show() {
    // SAFETY: el singleton se invoca con QGuiApplication vivo y en su hilo.
    unsafe {
        velo_desktop_show();
    }
}

/// Comando de ventana nativo (anima con DWM): minimizar o alternar maximizado/restaurado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowCommand {
    Minimize = 1,
    ToggleMaximize = 4,
}

/// Solo desde el hilo de Qt.
pub fn window_command(cmd: WindowCommand) {
    // SAFETY: el singleton se invoca con QGuiApplication vivo y en su hilo.
    unsafe {
        velo_window_command(cmd as u32);
    }
}

/// Scene coordinates in logical pixels; a zero-sized rectangle disables Snap hit testing.
pub fn set_maximize_button_rect(x: f64, y: f64, width: f64, height: f64) {
    // SAFETY: called by the QML singleton on Qt's thread with QGuiApplication alive.
    unsafe {
        velo_set_maximize_button_rect(x, y, width, height);
    }
}

pub fn update(tooltip: &str, playing: bool, controllable: bool, liked: bool, like_available: bool) {
    // Texto de Spotify no puede adelantar el terminador del tooltip C.
    let tooltip = tooltip.replace('\0', "");
    if let Ok(text) = std::ffi::CString::new(tooltip) {
        // SAFETY: CString vive durante la llamada síncrona desde qt_thread.queue.
        unsafe {
            velo_desktop_update(text.as_ptr(), playing, controllable, liked, like_available);
        }
    }
}

/// Portable thumbnail button policy; transport availability and saving are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbnailPolicy {
    pub playing: bool,
    pub transport: bool,
    pub liked: bool,
    pub like_available: bool,
}
pub fn thumbnail_policy(
    uri: &str,
    playing: bool,
    controllable: bool,
    liked: bool,
    saving: bool,
) -> ThumbnailPolicy {
    ThumbnailPolicy {
        playing,
        transport: !uri.is_empty() && controllable,
        liked,
        like_available: saving
            && uri.starts_with("spotify:track:")
            && velo_player::collection::savable(uri),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnail_buttons_follow_track_transport_and_saving_state() {
        for playing in [false, true] {
            for control in [false, true] {
                for saved in [false, true] {
                    let p = thumbnail_policy(
                        "spotify:track:0000000000000000000000",
                        playing,
                        control,
                        saved,
                        true,
                    );
                    assert_eq!(p.playing, playing);
                    assert_eq!(p.transport, control);
                    assert_eq!(p.liked, saved);
                    assert!(p.like_available);
                    let empty = thumbnail_policy("", playing, control, saved, true);
                    assert!(!empty.transport && !empty.like_available);
                    assert!(
                        !thumbnail_policy(
                            "spotify:episode:0000000000000000000000",
                            playing,
                            control,
                            saved,
                            true
                        )
                        .like_available
                    );
                    assert!(
                        !thumbnail_policy(
                            "spotify:track:0000000000000000000000",
                            playing,
                            control,
                            saved,
                            false
                        )
                        .like_available
                    );
                }
            }
        }
    }
    #[test]
    fn close_hides_only_when_recoverable_and_exit_always_quits() {
        assert!(velo_should_hide_on_close(true, true, false));
        assert!(!velo_should_hide_on_close(false, true, false));
        assert!(!velo_should_hide_on_close(true, false, false));
        for pref in [false, true] {
            for tray in [false, true] {
                assert!(!velo_should_hide_on_close(pref, tray, true));
            }
        }
    }
    #[test]
    fn hidden_window_stops_fft_and_position_clock_without_pausing_audio() {
        let flags = crate::engine::clock::Flags {
            playing: true,
            visible: false,
            focused: true,
            viz_enabled: true,
            local_source_available: true,
            now_playing_visible: true,
            pause_in_background: true,
            ..Default::default()
        };
        assert!(!crate::engine::clock::running(&flags));
        assert!(!crate::engine::clock::should_tick(&flags));
        assert!(flags.playing);
    }
}
