//! QML ↔ Rust bridge, registered in the QML module `Velo.Backend`.
//!
//! One file per singleton (`Player`, `Lyrics`, `Visualizer`, `Library`, `Account`, `Settings`)
//! plus `AppInfo` here. They are thin views of the Qt-free `crate::engine`; the playback sources
//! (`source*.rs`) live here too. Any `.rs` file in this directory that declares a cxx-qt bridge
//! module is compiled automatically by `build.rs`, so helper files must not mention that
//! attribute.

/// Sets a read-only (READ + NOTIFY) property from Rust: stores the value and emits the signal
/// only when it changed.
macro_rules! set_ro {
    ($this:ident, $field:ident, $notify:ident, $value:expr) => {{
        let value = $value;
        if $this.$field != value {
            $this.as_mut().rust_mut().$field = value;
            $this.as_mut().$notify();
        }
    }};
}

pub mod source;
pub mod source_local;
pub mod source_remote;
pub mod source_velo;

mod account;
mod actions;
pub mod desktop;
mod library;
mod lyrics;
mod player;
mod settings;
mod updater;
mod visualizer;

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, version, READ, CONSTANT)]
        #[qproperty(QString, name, READ, CONSTANT)]
        type AppInfo = super::AppInfoRust;
    }
}

use cxx_qt_lib::QString;

/// Backing data of the `AppInfo` QML singleton.
pub struct AppInfoRust {
    version: QString,
    name: QString,
}

impl Default for AppInfoRust {
    fn default() -> Self {
        Self {
            version: QString::from(env!("CARGO_PKG_VERSION")),
            name: QString::from(crate::APP_NAME),
        }
    }
}
