//! Builds the `Velo` QML module from the repository's `qml/` directory.
//!
//! - Every `.qml` file is found by walking `qml/` recursively, so new views or components
//!   need no change here. `qml/mock/` and `qml/preview/` are skipped (preview-only code).
//! - Files declaring `pragma Singleton` are registered as singletons (`Theme`).
//! - `qml/fonts` and `qml/icons` are added as resources next to the QML files, so
//!   `Qt.resolvedUrl("../fonts/...")` and `"../../icons/x.svg"` keep working from qrc.
//! - `.js` files and folder `qmldir` files (e.g. `theme/qmldir`) anywhere in `qml/` are added
//!   too, so `import "scallop.js"` and `import "../../theme"` resolve inside qrc as on disk.
//!
//! Everything lands under `qrc:/qt/qml/Velo/`, mirroring the `qml/` layout
//! (e.g. `qrc:/qt/qml/Velo/Main.qml`, `qrc:/qt/qml/Velo/icons/close.svg`).
//! Note: QML type names come from file names, so two `.qml` files with the same name in
//! different folders would clash in the module's qmldir.

use std::path::{Path, PathBuf};

use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlFile, QmlModule};

/// Top-level folders of `qml/` that are never compiled into the app.
const EXCLUDED_DIRS: &[&str] = &["mock", "preview"];
/// Resource folders and the extensions taken from them.
const RESOURCE_DIRS: &[(&str, &[&str])] = &[("fonts", &["ttf", "otf"]), ("icons", &["svg", "png"])];

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let qml_root = manifest
        .join("../../../qml")
        .canonicalize()
        .expect("qml/ directory next to crates/");

    // Rerun when anything under qml/ is added, removed or edited (Cargo scans directories recursively).
    println!("cargo::rerun-if-changed={}", qml_root.display());

    let mut qml_files = Vec::new();
    let mut resources = Vec::new();
    walk(&qml_root, &qml_root, &mut qml_files, &mut resources);
    qml_files.sort();
    resources.sort();

    let module = QmlModule::new("Velo").qml_files(qml_files.iter().map(|rel| {
        let source = std::fs::read_to_string(qml_root.join(rel)).unwrap_or_default();
        QmlFile::from(rel).singleton(is_singleton(&source))
    }));

    // cxx-qt-build uses each QML path, as given, as its resource alias. Relative paths from
    // the qml/ root therefore reproduce the source tree inside qrc:/qt/qml/Velo/.
    std::env::set_current_dir(&qml_root).expect("enter qml/");

    let builder = CxxQtBuilder::new_qml_module(module)
        .qt_module("Qml")
        .qt_module("Quick")
        .qrc_resources(QResources::new().resource(QResource::new().files(resources)));
    // qmlcachegen turns qsTr("…") strings into C++ literals; without /utf-8 MSVC reads them in
    // the system code page and "canción" shows up as "canciÃ³n".
    // SAFETY: only adds a compiler flag; does not change what cxx-qt-build links or generates.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.flag_if_supported("/utf-8");
        })
    };
    builder.build().export();
}

/// Collects paths relative to `root`, with forward slashes.
fn walk(root: &Path, dir: &Path, qml_files: &mut Vec<String>, resources: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .expect("walked path inside root")
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            if dir == root && EXCLUDED_DIRS.contains(&rel.as_str()) {
                continue;
            }
            walk(root, &path, qml_files, resources);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if ext == "qml" {
            qml_files.push(rel);
        } else if ext == "js" || (name == "qmldir" && dir != root) {
            resources.push(rel);
        } else if let Some((_, exts)) = RESOURCE_DIRS
            .iter()
            .find(|(folder, _)| rel.starts_with(&format!("{folder}/")))
        {
            if exts.contains(&ext.as_str()) {
                resources.push(rel);
            }
        }
    }
}

fn is_singleton(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .take_while(|line| !line.starts_with('{'))
        .any(|line| line.starts_with("pragma Singleton"))
}
