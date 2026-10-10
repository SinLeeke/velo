//! Builds the `Velo.Backend` QML module (Rust QObjects from `src/bridge/`) and the native
//! window chrome (`cpp/window_chrome.cpp`).
//!
//! The `Velo` QML module itself (every file under `qml/`) is built by the `velo-ui` crate
//! (`crates/app/ui`), because cxx-qt-build supports one QML module per crate.
//!
//! Every `.rs` file directly inside `src/bridge/` that contains a `#[cxx_qt::bridge]` is picked
//! up automatically. cxx-qt requires all bridges of a QML module to live in one directory.

use std::path::Path;

use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // S9: activar al integrar C3: bridge/history.rs is picked up automatically once present.
    let bridges = bridge_files(Path::new("src/bridge"));
    println!("cargo::rerun-if-changed=src/bridge");

    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("Velo.Backend"))
        .files(&bridges)
        // A distinct QRC name avoids colliding with velo-ui's generated resources_0.qrc.
        .qrc("velo_backend_assets.qrc")
        .qt_module("Gui")
        .qt_module("Qml")
        .qt_module("Quick")
        .cpp_file("cpp/window_chrome.cpp")
        .cpp_file("cpp/desktop.cpp")
        // Generic list model used by every list the bridge exposes (header is run through moc).
        .cpp_file("cpp/list_model.h")
        .cpp_file("cpp/list_model.cpp");
    builder.build();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo::rustc-link-lib=dwmapi");
        println!("cargo::rustc-link-lib=user32");
        println!("cargo::rustc-link-lib=shell32");
        println!("cargo::rustc-link-lib=gdi32");
        println!("cargo::rustc-link-lib=ole32");
        println!("cargo::rustc-link-lib=uuid");
        embed_exe_icon();
    }
}

/// Compiles `cpp/app_icon.rc` with the Windows SDK `rc.exe` and links it into the binary so the
/// executable carries the Velo icon. Missing SDK tooling only warns (the build still works).
fn embed_exe_icon() {
    println!("cargo::rerun-if-changed=cpp/app_icon.rc");
    println!("cargo::rerun-if-changed=assets/velo.ico");
    let Some(rc) = find_rc() else {
        println!("cargo::warning=rc.exe not found: the executable will have no embedded icon");
        return;
    };
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let res = out.join("app_icon.res");
    let ok = std::process::Command::new(&rc)
        .arg("/nologo")
        .arg(format!("/fo{}", res.display()))
        .arg("cpp/app_icon.rc")
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        println!("cargo::rustc-link-arg-bins={}", res.display());
    } else {
        println!("cargo::warning=rc.exe failed: the executable will have no embedded icon");
    }
}

fn find_rc() -> Option<std::path::PathBuf> {
    let roots = [
        std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into()),
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into()),
    ];
    let mut best: Option<std::path::PathBuf> = None;
    for root in roots {
        let bin = Path::new(&root).join("Windows Kits").join("10").join("bin");
        let Ok(entries) = std::fs::read_dir(&bin) else {
            continue;
        };
        for entry in entries.flatten() {
            let candidate = entry.path().join("x64").join("rc.exe");
            if candidate.is_file() && best.as_ref().is_none_or(|b| candidate > *b) {
                best = Some(candidate);
            }
        }
        let flat = bin.join("x64").join("rc.exe");
        if best.is_none() && flat.is_file() {
            best = Some(flat);
        }
    }
    best
}

fn bridge_files(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .expect("src/bridge exists")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| {
            std::fs::read_to_string(path).is_ok_and(|source| source.contains("cxx_qt::bridge"))
        })
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();
    files.sort();
    files
}
