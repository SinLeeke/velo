//! Eventos del escritorio; Qt recibe solo trabajo encolado, sin polling.
#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Desktop = super::DesktopRust;
        #[qinvokable]
        fn show_window(self: &Desktop);
        /// Minimizar con la animación nativa de Windows (WM_SYSCOMMAND/SC_MINIMIZE).
        #[qinvokable]
        fn minimize_window(self: &Desktop);
        /// Maximizar o restaurar con la animación nativa (SC_MAXIMIZE/SC_RESTORE).
        #[qinvokable]
        fn toggle_maximize_window(self: &Desktop);
        /// Scene coordinates; call with zeros when Windows controls are hidden.
        #[qinvokable]
        fn set_maximize_button_rect(self: &Desktop, x: f64, y: f64, width: f64, height: f64);
        #[qsignal]
        fn show_requested(self: Pin<&mut Desktop>);
    }
    impl cxx_qt::Threading for Desktop {}
    impl cxx_qt::Initialize for Desktop {}
}

use crate::engine::core;
use cxx_qt::Threading;

#[derive(Default)]
pub struct DesktopRust;

pub fn launches() -> &'static crate::engine::Bus<Vec<String>> {
    static BUS: std::sync::OnceLock<crate::engine::Bus<Vec<String>>> = std::sync::OnceLock::new();
    BUS.get_or_init(crate::engine::Bus::new)
}

impl cxx_qt::Initialize for ffi::Desktop {
    fn initialize(self: std::pin::Pin<&mut Self>) {
        let thread = self.qt_thread();
        let mut player = core().player.view.subscribe();
        let mut account = core().env.account.view.subscribe();
        core().env.rt.spawn(async move {
            let mut previous = None;
            loop {
                let v = player.borrow_and_update().clone();
                account.borrow_and_update();
                let tooltip = if v.title.is_empty() {
                    crate::APP_NAME.into()
                } else {
                    format!("{} — {} · {}", v.title, v.artist, crate::APP_NAME)
                };
                let state = (
                    tooltip,
                    crate::desktop::thumbnail_policy(
                        &v.uri,
                        v.is_playing,
                        v.can_control,
                        v.liked,
                        core().env.account.api().is_some(),
                    ),
                );
                if previous.as_ref() != Some(&state) {
                    let value = state.clone();
                    if thread
                        .queue(move |_| {
                            crate::desktop::update(
                                &value.0,
                                value.1.playing,
                                value.1.transport,
                                value.1.liked,
                                value.1.like_available,
                            )
                        })
                        .is_err()
                    {
                        break;
                    }
                    previous = Some(state);
                }
                tokio::select! {
                    result = player.changed() => if result.is_err() { break },
                    result = account.changed() => if result.is_err() { break },
                }
            }
        });
        if let Some(mut events) = launches().take() {
            let thread = self.qt_thread();
            core().env.rt.spawn(async move {
                while let Some(args) = events.recv().await {
                    crate::engine::links::launch(args);
                    if thread.queue(|q| q.show_requested()).is_err() {
                        break;
                    }
                }
            });
        }
    }
}

impl ffi::Desktop {
    pub fn set_maximize_button_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        crate::desktop::set_maximize_button_rect(x, y, width, height);
    }
    pub fn show_window(&self) {
        crate::desktop::show();
    }

    pub fn minimize_window(&self) {
        crate::desktop::window_command(crate::desktop::WindowCommand::Minimize);
    }

    pub fn toggle_maximize_window(&self) {
        crate::desktop::window_command(crate::desktop::WindowCommand::ToggleMaximize);
    }
}
