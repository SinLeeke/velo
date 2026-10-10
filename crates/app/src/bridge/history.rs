//! Local history singleton; ordered model/view updates reach Qt together, without polling.
#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("QtCore/QAbstractListModel");
        type QAbstractListModel = crate::model::ffi::QAbstractListModel;
    }
    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(*mut QAbstractListModel, model, READ, CONSTANT)]
        #[qproperty(bool, loading, READ, NOTIFY)]
        #[qproperty(bool, has_more, READ, NOTIFY)]
        #[qproperty(QString, error, READ, NOTIFY)]
        #[qproperty(i32, count, READ, NOTIFY)]
        type History = super::HistoryRust;
        #[qinvokable]
        fn load(self: &History);
        #[qinvokable]
        fn load_more(self: &History);
        #[qinvokable]
        fn play_entry(self: &History, entry_id: &QString);
        #[qinvokable]
        fn clear(self: &History);
        #[qsignal]
        fn changed(self: Pin<&mut History>);
        #[qsignal]
        fn cleared(self: Pin<&mut History>);
    }
    impl cxx_qt::Threading for History {}
    impl cxx_qt::Initialize for History {}
}
use crate::{
    engine::{
        core,
        player::history::{HistoryUi, ROLES},
    },
    model::ListModel,
};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading, casting::Upcast};
use cxx_qt_lib::QString;

pub struct HistoryRust {
    model: *mut crate::model::ffi::QAbstractListModel,
    loading: bool,
    has_more: bool,
    error: QString,
    count: i32,
    list: Option<ListModel>,
}
impl Default for HistoryRust {
    fn default() -> Self {
        Self {
            model: std::ptr::null_mut(),
            loading: false,
            has_more: false,
            error: QString::default(),
            count: 0,
            list: None,
        }
    }
}
impl cxx_qt::Initialize for ffi::History {
    fn initialize(mut self: Pin<&mut Self>) {
        let parent = std::ptr::from_ref::<cxx_qt::QObject>((*self).upcast())
            .cast_mut()
            .cast();
        let list = ListModel::new(ROLES, parent);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.model = list.as_model();
            rust.list = Some(list);
        }
        let c = core();
        if let Some(mut events) = c.player.history.ui.take() {
            let thread = self.qt_thread();
            c.env.rt.spawn(async move {
                while let Some(event) = events.recv().await {
                    if thread.queue(move |q| q.apply(event)).is_err() {
                        break;
                    }
                }
            });
        }
        // Loading is event driven; model/count notifications travel with the model operation.
        let mut view = c.player.history.view.subscribe();
        let thread = self.qt_thread();
        c.env.rt.spawn(async move {
            loop {
                view.borrow_and_update();
                if thread
                    .queue(move |mut q| {
                        let loading = core().player.history.view.borrow().loading;
                        set_ro!(q, loading, loading_changed, loading);
                    })
                    .is_err()
                {
                    break;
                }
                if view.changed().await.is_err() {
                    break;
                }
            }
        });
        c.player.history.load();
    }
}
impl ffi::History {
    fn apply(mut self: Pin<&mut Self>, event: HistoryUi) {
        if let Some(list) = &self.list {
            list.apply(event.op);
        }
        set_ro!(self, loading, loading_changed, event.view.loading);
        set_ro!(self, has_more, has_more_changed, event.view.has_more);
        set_ro!(
            self,
            error,
            error_changed,
            QString::from(event.view.error.as_str())
        );
        set_ro!(self, count, count_changed, event.view.count);
        self.as_mut().changed();
        if event.cleared {
            self.cleared();
        }
    }
    pub fn load(&self) {
        core().player.history.load();
    }
    pub fn load_more(&self) {
        core().player.history.load_more();
    }
    pub fn play_entry(&self, entry_id: &QString) {
        core().player.play_history_entry(&entry_id.to_string());
    }
    pub fn clear(&self) {
        core().player.history.clear();
    }
}
