//! `Actions` singleton: context-menu actions shared by every list.
//! A thin view of `engine::actions`; every invokable returns at once (work runs on tokio).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, inspected_uri, READ, NOTIFY)]
        #[qproperty(i32, inspected_saved, READ, NOTIFY)]
        #[qproperty(i32, sleep_minutes, READ, NOTIFY)]
        #[qproperty(f64, sleep_ends_at_ms, READ, NOTIFY)]
        type Actions = super::ActionsRust;

        #[qinvokable]
        fn inspect_saved(self: &Actions, uri: &QString);
        #[qinvokable]
        fn set_saved(self: &Actions, uri: &QString, saved: bool);
        #[qinvokable]
        fn add_context_to_queue(self: &Actions, uri: &QString);
        #[qinvokable]
        fn open_current(self: &Actions, kind: &QString);
        #[qinvokable]
        fn set_sleep_timer(self: &Actions, minutes: i32);
        #[qinvokable]
        fn share_url(self: &Actions, uri: &QString) -> QString;
        #[qinvokable]
        fn request_credits(self: &Actions, uri: &QString);
        #[qinvokable]
        fn start_dj_on(self: &Actions, device_id: &QString);
        #[qinvokable]
        fn start_dj(self: &Actions);
        #[qinvokable]
        fn dj_next_segment(self: &Actions);
        #[qsignal]
        fn credits_ready(self: Pin<&mut Actions>, uri: QString, json: QString);

        /// A save/remove succeeded: rows showing `uri` update their heart.
        #[qsignal]
        fn saved_changed(self: Pin<&mut Actions>, uri: QString, saved: bool);
        /// "Ir al artista / álbum" of the playing song resolved: open that page.
        #[qsignal]
        fn navigate_requested(self: Pin<&mut Actions>, kind: QString, id: QString);
    }

    impl cxx_qt::Threading for Actions {}
    impl cxx_qt::Initialize for Actions {}
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::engine::actions::{ActionsUi, share_url};
use crate::engine::core;

/// `inspected_saved`: -1 unknown (asking or failed), 0 not saved, 1 saved.
pub struct ActionsRust {
    inspected_uri: QString,
    inspected_saved: i32,
    sleep_minutes: i32,
    sleep_ends_at_ms: f64,
}

impl Default for ActionsRust {
    fn default() -> Self {
        Self {
            inspected_uri: QString::default(),
            inspected_saved: -1,
            sleep_minutes: 0,
            sleep_ends_at_ms: 0.0,
        }
    }
}

impl cxx_qt::Initialize for ffi::Actions {
    fn initialize(self: Pin<&mut Self>) {
        let c = core();
        if let Some(mut events) = c.actions.ui.take() {
            let thread = self.qt_thread();
            c.env.rt.spawn(async move {
                while let Some(event) = events.recv().await {
                    if thread.queue(move |q| q.apply(event)).is_err() {
                        break;
                    }
                }
            });
        }
    }
}

impl ffi::Actions {
    fn apply(mut self: Pin<&mut Self>, event: ActionsUi) {
        match event {
            ActionsUi::Inspected {
                generation,
                uri,
                saved,
            } => {
                if !core().actions.inspection_current(generation, &uri) {
                    return;
                }
                let uri = QString::from(uri.as_str());
                let saved = saved.map_or(-1, i32::from);
                // Ambos valores se fijan antes del primer NOTIFY: los lectores síncronos ven la pareja nueva.
                let (uri_changed, saved_changed) =
                    self.as_mut().rust_mut().update_inspection(uri, saved);
                if uri_changed {
                    self.as_mut().inspected_uri_changed();
                }
                if saved_changed {
                    self.as_mut().inspected_saved_changed();
                }
            }
            ActionsUi::Saved { uri, saved } => {
                self.as_mut()
                    .saved_changed(QString::from(uri.as_str()), saved);
            }
            ActionsUi::Navigate {
                kind,
                id,
                generation,
            } => {
                if !core().player.metadata_current(generation) {
                    return;
                }
                self.as_mut()
                    .navigate_requested(QString::from(kind), QString::from(id.as_str()));
            }
            ActionsUi::Sleep {
                generation,
                minutes,
                ends_at_ms,
            } => {
                if !core().actions.sleep_current(generation) {
                    return;
                }
                set_ro!(self, sleep_minutes, sleep_minutes_changed, minutes);
                set_ro!(self, sleep_ends_at_ms, sleep_ends_at_ms_changed, ends_at_ms);
            }
            ActionsUi::Credits { uri, json } => {
                self.as_mut()
                    .credits_ready(QString::from(uri.as_str()), QString::from(json.as_str()));
            }
        }
    }

    pub fn inspect_saved(&self, uri: &QString) {
        core().actions.inspect_saved(uri.to_string());
    }

    pub fn request_credits(&self, uri: &QString) {
        core().actions.request_credits(uri.to_string());
    }

    pub fn start_dj_on(&self, device_id: &QString) {
        core().actions.start_dj_on(device_id.to_string());
    }

    pub fn start_dj(&self) {
        core().actions.start_dj();
    }

    pub fn dj_next_segment(&self) {
        core().actions.dj_next_segment();
    }

    pub fn set_saved(&self, uri: &QString, saved: bool) {
        core().actions.set_saved(uri.to_string(), saved);
    }

    pub fn add_context_to_queue(&self, uri: &QString) {
        core().actions.add_context_to_queue(uri.to_string());
    }

    pub fn open_current(&self, kind: &QString) {
        core().actions.open_current(kind.to_string());
    }

    pub fn set_sleep_timer(&self, minutes: i32) {
        core().actions.set_sleep_timer(minutes);
    }

    pub fn share_url(&self, uri: &QString) -> QString {
        QString::from(share_url(&uri.to_string()).unwrap_or_default().as_str())
    }
}

impl ActionsRust {
    fn update_inspection(&mut self, uri: QString, saved: i32) -> (bool, bool) {
        let changed = (self.inspected_uri != uri, self.inspected_saved != saved);
        self.inspected_uri = uri;
        self.inspected_saved = saved;
        changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn m2_both_fields_are_current_at_the_first_notification() {
        let mut state = ActionsRust::default();
        state.update_inspection(QString::from("a"), 1);
        let changed = state.update_inspection(QString::from("b"), -1);
        assert_eq!(changed, (true, true));
        for notify in [changed.0, changed.1] {
            if notify {
                assert_eq!(
                    (state.inspected_uri.to_string(), state.inspected_saved),
                    ("b".into(), -1)
                );
            }
        }
    }
}
