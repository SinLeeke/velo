//! `Lyrics` singleton (docs/CONTRATO.md): `model` (roles `text`, `timeMs`), `activeIndex`,
//! `status`, `plainText`, `source`. A view of `engine::lyrics`; no timer of its own.

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
        #[qproperty(i32, active_index, READ, NOTIFY)]
        #[qproperty(QString, status, READ, NOTIFY)]
        #[qproperty(QString, plain_text, READ, NOTIFY)]
        #[qproperty(QString, source, READ, NOTIFY)]
        type Lyrics = super::LyricsRust;
    }

    impl cxx_qt::Threading for Lyrics {}
    impl cxx_qt::Initialize for Lyrics {}
}

use core::pin::Pin;

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::engine::core;
use crate::engine::lyrics::LyricsView;
use crate::engine::rows::LYRIC_ROLES;
use crate::model::{ListModel, ModelOp};

pub struct LyricsRust {
    model: *mut crate::model::ffi::QAbstractListModel,
    active_index: i32,
    status: QString,
    plain_text: QString,
    source: QString,
    list: Option<ListModel>,
}

impl Default for LyricsRust {
    fn default() -> Self {
        let v = core().lyrics.view.borrow().clone();
        Self {
            model: std::ptr::null_mut(),
            active_index: v.active_index,
            status: QString::from(v.status.as_str()),
            plain_text: QString::from(v.plain_text.as_str()),
            source: QString::from(v.source.as_str()),
            list: None,
        }
    }
}

impl cxx_qt::Initialize for ffi::Lyrics {
    fn initialize(mut self: Pin<&mut Self>) {
        // Upcast the QObject itself, not the `Pin` (the blanket `Upcast<T> for T` would
        // otherwise hand Qt the address of this stack variable as the parent).
        let parent = std::ptr::from_ref::<cxx_qt::QObject>((*self).upcast())
            .cast_mut()
            .cast();
        let list = ListModel::new(LYRIC_ROLES, parent);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.model = list.as_model();
            rust.list = Some(list);
        }

        let c = core();
        let thread = self.qt_thread();
        let mut view = c.lyrics.view.subscribe();
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

        if let Some(mut ops) = c.lyrics.ui.take() {
            let thread = self.qt_thread();
            c.env.rt.spawn(async move {
                while let Some(op) = ops.recv().await {
                    if thread.queue(move |q| q.apply_op(op)).is_err() {
                        break;
                    }
                }
            });
        }
    }
}

impl ffi::Lyrics {
    fn apply(mut self: Pin<&mut Self>, v: &LyricsView) {
        // La fuente antes que el estado: al cambiar `status` la UI ya lee la fuente nueva.
        set_ro!(
            self,
            source,
            source_changed,
            QString::from(v.source.as_str())
        );
        set_ro!(
            self,
            status,
            status_changed,
            QString::from(v.status.as_str())
        );
        set_ro!(
            self,
            plain_text,
            plain_text_changed,
            QString::from(v.plain_text.as_str())
        );
        set_ro!(self, active_index, active_index_changed, v.active_index);
    }

    fn apply_op(self: Pin<&mut Self>, op: ModelOp) {
        if let Some(list) = &self.list {
            list.apply(op);
        }
    }
}
