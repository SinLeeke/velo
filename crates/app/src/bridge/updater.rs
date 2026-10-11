//! U1: thin Qt view; HTTP, files and process creation stay on Tokio workers.
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
        #[qproperty(QString, state, READ, NOTIFY)]
        #[qproperty(QString, latest_version, READ, NOTIFY)]
        #[qproperty(QString, current_version, READ, CONSTANT)]
        #[qproperty(f64, progress, READ, NOTIFY)]
        #[qproperty(QString, error, READ, NOTIFY)]
        #[qproperty(bool, portable, READ, NOTIFY)]
        type Updater = super::UpdaterRust;
        #[qinvokable]
        fn check_now(self: Pin<&mut Updater>);
        #[qinvokable]
        fn install(self: Pin<&mut Updater>);
        #[qinvokable]
        fn dismiss(self: Pin<&mut Updater>);
        #[qinvokable]
        fn open_release_page(self: &Updater);
        /// A manual check completed; false also accompanies an error (see `error`).
        #[qsignal]
        fn check_finished(self: Pin<&mut Updater>, newer: bool);
    }
    impl cxx_qt::Threading for Updater {}
    impl cxx_qt::Initialize for Updater {}
}
use crate::engine::core;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use std::{pin::Pin, time::Duration};
use velo_services::update::{self, UpdateInfo};

pub struct UpdaterRust {
    state: QString,
    latest_version: QString,
    current_version: QString,
    progress: f64,
    error: QString,
    portable: bool,
    detected: bool,
    checking: bool,
    manual: bool,
    dismissed: bool,
    info: Option<UpdateInfo>,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
}
impl Default for UpdaterRust {
    fn default() -> Self {
        Self {
            state: QString::from("idle"),
            latest_version: QString::default(),
            current_version: QString::from(env!("CARGO_PKG_VERSION")),
            progress: 0.,
            error: QString::default(),
            portable: true,
            detected: false,
            checking: false,
            manual: false,
            dismissed: false,
            info: None,
            cancel: None,
        }
    }
}
impl Drop for UpdaterRust {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            let _ = cancel.send(true);
        }
    }
}
fn is_portable(exe: &std::path::Path) -> bool {
    !exe.parent()
        .is_some_and(|dir| dir.join("unins000.exe").is_file())
}
impl cxx_qt::Initialize for ffi::Updater {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        core().env.rt.spawn(async move {
            let portable = tokio::task::spawn_blocking(|| {
                std::env::current_exe().map_or(true, |exe| is_portable(&exe))
            })
            .await
            .unwrap_or(true);
            let _ = thread.queue(move |mut q| {
                set_ro!(q, portable, portable_changed, portable);
                q.as_mut().rust_mut().detected = true;
            });
            tokio::time::sleep(Duration::from_secs(3)).await;
            if core().env.prefs.settings().updates.check_updates {
                let _ = thread.queue(|q| q.check(false));
            }
        });
    }
}
impl ffi::Updater {
    fn set_state(mut self: Pin<&mut Self>, state: &str) {
        set_ro!(self, state, state_changed, QString::from(state));
    }
    fn fail(mut self: Pin<&mut Self>, message: &str) {
        set_ro!(self, error, error_changed, QString::from(message));
        self.set_state("error");
    }
    pub fn check_now(self: Pin<&mut Self>) {
        self.check(true);
    }
    fn check(mut self: Pin<&mut Self>, manual: bool) {
        if self.state.to_string() == "downloading" || self.state.to_string() == "ready" {
            return;
        }
        if self.checking {
            self.as_mut().rust_mut().manual |= manual;
            return;
        }
        if !manual && self.dismissed {
            return;
        }
        self.as_mut().rust_mut().checking = true;
        self.as_mut().rust_mut().manual = manual;
        self.as_mut().rust_mut().info = None;
        set_ro!(self, error, error_changed, QString::default());
        set_ro!(
            self,
            latest_version,
            latest_version_changed,
            QString::default()
        );
        self.as_mut().set_state("idle");
        let thread = self.qt_thread();
        core().env.rt.spawn(async move {
            let result = update::check(update::API_BASE, env!("CARGO_PKG_VERSION")).await;
            let _ = thread.queue(move |mut q| {
                q.as_mut().rust_mut().checking = false;
                let manual = q.manual;
                let newer = matches!(&result, Ok(Some(_)));
                match result {
                    Ok(Some(info)) => {
                        set_ro!(
                            q,
                            latest_version,
                            latest_version_changed,
                            QString::from(info.version.as_str())
                        );
                        q.as_mut().rust_mut().info = Some(info);
                        if !q.dismissed {
                            q.as_mut().set_state("available");
                        }
                    }
                    Ok(None) => q.as_mut().set_state("idle"),
                    Err(e) if manual => q.as_mut().fail(&e.to_string()),
                    Err(_) => q.as_mut().set_state("idle"),
                }
                if manual {
                    q.as_mut().check_finished(newer);
                }
            });
        });
    }
    pub fn dismiss(mut self: Pin<&mut Self>) {
        if self.state.to_string() == "downloading" || self.state.to_string() == "ready" {
            return;
        }
        self.as_mut().rust_mut().dismissed = true;
        self.set_state("idle");
    }
    pub fn open_release_page(&self) {
        let Some(info) = &self.info else {
            return;
        };
        let url = info.release_url.clone();
        // This is metadata from GitHub, but never pass a local path or another protocol to the shell.
        if !url.starts_with("https://github.com/SinLeeke/velo/releases/") {
            return;
        }
        core().env.rt.spawn_blocking(move || {
            #[cfg(windows)]
            let opener = "explorer";
            #[cfg(not(windows))]
            let opener = "xdg-open";
            let _ = std::process::Command::new(opener).arg(url).spawn();
        });
    }
    pub fn install(mut self: Pin<&mut Self>) {
        if !self.detected
            || self.checking
            || matches!(self.state.to_string().as_str(), "downloading" | "ready")
        {
            return;
        }
        let Some(info) = self.info.clone() else {
            self.check_now();
            return;
        };
        if self.portable {
            self.open_release_page();
            return;
        }
        set_ro!(self, progress, progress_changed, 0.);
        set_ro!(self, error, error_changed, QString::default());
        self.as_mut().set_state("downloading");
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        self.as_mut().rust_mut().cancel = Some(cancel);
        let thread = self.qt_thread();
        core().env.rt.spawn(async move {
            let dir = std::env::temp_dir().join("Velo-update");
            let mut last_percent = None;
            let result = update::download(
                &info,
                &dir,
                |bytes, total| {
                    let percent = bytes.saturating_mul(100) / total.max(1);
                    if last_percent == Some(percent) {
                        return;
                    }
                    last_percent = Some(percent);
                    let progress = (bytes as f64 / total.max(1) as f64).clamp(0., 1.);
                    let _ = thread.queue(move |mut q| {
                        set_ro!(q, progress, progress_changed, progress);
                    });
                },
                receiver,
            )
            .await;
            match result {
                Ok(path) => {
                    let _ = thread.queue(|q| q.set_state("ready"));
                    let launched = tokio::task::spawn_blocking(move || {
                        std::process::Command::new(path)
                            .args([
                                // No installer window at all; it reopens Velo when done.
                                "/VERYSILENT",
                                "/SUPPRESSMSGBOXES",
                                "/NORESTART",
                                "/CLOSEAPPLICATIONS",
                            ])
                            .spawn()
                            .is_ok()
                    })
                    .await
                    .unwrap_or(false);
                    if launched {
                        // Let the card read «Instalando…» before Velo closes for the installer.
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    }
                    let _ = thread.queue(move |mut q| {
                        q.as_mut().rust_mut().cancel = None;
                        if launched {
                            crate::desktop::quit();
                        } else {
                            q.fail("No se pudo abrir el instalador. Inténtalo de nuevo.");
                        }
                    });
                }
                Err(e) => {
                    let _ = thread.queue(move |mut q| {
                        q.as_mut().rust_mut().cancel = None;
                        q.fail(&e.to_string());
                    });
                }
            }
        });
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn portable_requires_adjacent_uninstaller() {
        let dir = std::env::temp_dir().join(format!("velo-u1-detect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("velo.exe");
        assert!(super::is_portable(&exe));
        std::fs::write(dir.join("unins000.exe"), []).unwrap();
        assert!(!super::is_portable(&exe));
        std::fs::remove_file(dir.join("unins000.exe")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
