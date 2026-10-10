//! Velo: a light Qt Quick front end for the user's Spotify account.
//!
//! Environment:
//! - `VELO_LOG`: tracing filter (e.g. `debug`, `velo=trace`). Default `info`.
//! - `VELO_MICA=1`: Windows 11 Mica backdrop behind transparent parts of the window.
//! - `VELO_SCREENSHOT=<path.png>`: save a grab of the window shortly after start (verification).

// Without a console window in release builds; debug builds keep it for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod desktop;
mod engine;
mod model;
mod single_instance;
mod window;

// Links the static `Velo` QML module (qml/, fonts, icons) built by crates/app/ui.
use velo_ui as _;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use tracing_subscriber::EnvFilter;

/// Product name, kept in one place (working name, the user may rename it).
pub const APP_NAME: &str = "Velo";
const MAIN_QML: &str = "qrc:/qt/qml/Velo/Main.qml";

/// Explicit metadata probes for the integrator; no window, OAuth, audio export or token output.
fn run_s9_probe() -> bool {
    let args: Vec<_> = std::env::args().collect();
    let Some(mode) = args
        .get(1)
        .filter(|a| matches!(a.as_str(), "--probe-shows" | "--probe-home"))
    else {
        return false;
    };
    let c = engine::core();
    c.env.rt.block_on(async {
        let Some(session) = c.env.audio.session_source().session().await else {
            eprintln!("S9 probe: stored session unavailable");
            return;
        };
        if mode == "--probe-shows" {
            let Some(set) = args.get(2) else {
                eprintln!("usage: --probe-shows <candidate-set>");
                return;
            };
            match velo_player::session_api::shows::probe(&session, set).await {
                Ok(rows) => println!(
                    "S9 SHOW metadata: {} podcasts; {} with added_at",
                    rows.len(),
                    rows.iter().filter(|r| r.added_at.is_some()).count()
                ),
                Err(_) => eprintln!("S9 showsStatus=unavailable; candidate/format not validated"),
            }
        } else {
            match velo_player::session_api::home::shortcuts(&session).await {
                Ok(rows) => println!(
                    "S9 HomeShorts: {} entries; kinds={:?}",
                    rows.len(),
                    rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>()
                ),
                Err(_) => eprintln!("S9 HomeShorts unavailable"),
            }
        }
    });
    engine::shutdown();
    true
}

fn main() {
    init_logging();
    if run_s9_probe() {
        return;
    }
    let args = single_instance::launch_args();
    // Antes de Qt, audio o keyring: la segunda ejecución solo entrega argumentos y sale.
    let (instance, launches) = match single_instance::acquire(args.clone()) {
        Ok(single_instance::Outcome::First(guard, receiver)) => (Some(guard), Some(receiver)),
        Ok(single_instance::Outcome::Forwarded) => return,
        Ok(single_instance::Outcome::Busy) => {
            tracing::error!("Velo ya está abierto pero no respondió");
            std::process::exit(1);
        }
        Err(e) => {
            // Sin directorio o sin loopback: Velo abre igual, solo sin instancia única.
            tracing::warn!(%e, "instancia única no disponible");
            (None, None)
        }
    };
    // Initialize disk-backed services and SMTC before creating the Qt UI, on a worker.
    std::thread::spawn(engine::core)
        .join()
        .expect("engine initialization");
    if !args.is_empty() {
        bridge::desktop::launches().send(args);
    }
    if let Some(mut launches) = launches {
        engine::core().env.rt.spawn(async move {
            while let Some(args) = launches.recv().await {
                bridge::desktop::launches().send(args);
            }
        });
    }
    let mica = std::env::var("VELO_MICA").is_ok_and(|v| v == "1");
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        mica,
        "starting {APP_NAME}"
    );

    let mut app = QGuiApplication::new();
    if let Some(mut app) = app.as_mut() {
        app.as_mut().set_application_name(&QString::from(APP_NAME));
        app.as_mut().set_organization_name(&QString::from(APP_NAME));
        app.set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
    }
    window::prepare(mica);

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(MAIN_QML));
    }
    if !window::install(&mut engine, mica) {
        tracing::error!("{MAIN_QML} did not create a window; see the QML errors above");
        std::process::exit(1);
    }
    desktop::install(&mut engine);

    let code = app.as_mut().map_or(1, |app| app.exec());
    // Pending preferences (settings saves are debounced) are written before the process ends.
    engine::shutdown();
    tracing::info!(code, "exit");
    // The engine (and its windows) must go before the application object.
    drop(engine);
    drop(app);
    drop(instance);
    std::process::exit(code);
}

fn init_logging() {
    let filter = EnvFilter::try_from_env("VELO_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
