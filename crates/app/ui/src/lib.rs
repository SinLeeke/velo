//! The `Velo` QML module.
//!
//! This crate has no Rust API: its build script compiles every `.qml` file under the
//! repository's `qml/` directory (except `qml/mock/` and `qml/preview/`) into a static QML
//! module with URI `Velo`, served from `qrc:/qt/qml/Velo/`, together with `qml/fonts` and
//! `qml/icons`. The `velo` binary links it with `use velo_ui as _;`.
//!
//! A file whose source contains `pragma Singleton` is registered as a singleton (e.g. `Theme`).
//! Adding a new `.qml` file only needs a rebuild: the build script walks the tree.
