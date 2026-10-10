//! Real-account probe of the E2 engine (the leader runs it; unit tests use no network).
//!
//! `cargo run -p velo-player --example play_probe -- [--login] [uri] [segundos=20] [--pause-seconds N] [--pcm]`
//!
//! Starts the Connect device "Velo" with the stored audio login (`--login` opens the browser
//! once and stores the refresh token in the keyring), plays `uri` (a track, or a playlist/album/
//! artist context), prints the engine state as librespot pushes it and reports CPU/RAM while
//! playing and, optionally, while paused. Without `uri` it only waits: transfer playback to
//! "Velo" from the phone to try Spotify Connect.

use std::{env, process::ExitCode, time::Duration};

use anyhow::{Context, Result, bail};
use velo_player::{Engine, EngineConfig, EngineState, LoadTarget};

#[path = "support/metrics.rs"]
mod metrics;

const USAGE: &str = "cargo run -p velo-player --example play_probe -- [--login] [spotify:track:<id>|spotify:playlist:<id>] [segundos=20] [--pause-seconds N] [--pcm]";

#[derive(Debug)]
struct Options {
    login: bool,
    uri: Option<String>,
    seconds: Duration,
    pause: Option<Duration>,
    pcm: bool,
}

fn seconds(value: &str) -> Result<Duration> {
    let value: u64 = value
        .parse()
        .context("Los segundos deben ser un entero entre 1 y 3600")?;
    if !(1..=3600).contains(&value) {
        bail!("Los segundos deben estar entre 1 y 3600");
    }
    Ok(Duration::from_secs(value))
}

fn options(args: impl IntoIterator<Item = String>) -> Result<Option<Options>> {
    let mut options = Options {
        login: false,
        uri: None,
        seconds: Duration::from_secs(20),
        pause: None,
        pcm: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--login" => options.login = true,
            "--pcm" => options.pcm = true,
            "--pause-seconds" => {
                options.pause = Some(seconds(
                    &args.next().context("Falta N en --pause-seconds N")?,
                )?);
            }
            uri if uri.starts_with("spotify:") && options.uri.is_none() => {
                options.uri = Some(uri.to_owned());
            }
            value if !value.starts_with("--") => options.seconds = seconds(value)?,
            _ => bail!("Argumento desconocido. Uso: {USAGE}"),
        }
    }
    Ok(Some(options))
}

fn target(uri: &str) -> LoadTarget {
    if uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:") {
        LoadTarget::Tracks {
            uris: vec![uri.to_owned()],
            index: 0,
            shuffle: false,
        }
    } else {
        LoadTarget::Context {
            uri: uri.to_owned(),
            index: None,
            shuffle: false,
        }
    }
}

fn describe(s: &EngineState) -> String {
    let track = s
        .track
        .as_ref()
        .map_or_else(|| "-".to_owned(), |t| format!("{} — {}", t.title, t.artist));
    format!(
        "activo={} sonando={} pos={}ms vol={} shuffle={} repeat={:?} pista={track}",
        s.active, s.playing, s.position_ms, s.volume, s.shuffle, s.repeat
    )
}

async fn watch_for(engine: &Engine, duration: Duration, pcm: &mut Option<PcmCount>) -> Result<()> {
    let mut state = engine.state();
    let mut stopped = engine.stopped();
    let mut pcm_rx = pcm.as_ref().and_then(|_| engine.take_pcm());
    let end = tokio::time::sleep(duration);
    tokio::pin!(end);
    loop {
        tokio::select! {
            _ = &mut end => return Ok(()),
            changed = state.changed() => {
                changed.context("El motor se cerró")?;
                println!("{}", describe(&state.borrow_and_update()));
            }
            _ = stopped.wait_for(|s| *s) => bail!("La sesión de Spotify Connect terminó"),
            packet = async { pcm_rx.as_mut()?.recv().await }, if pcm_rx.is_some() => {
                if let (Some(packet), Some(count)) = (packet, pcm.as_mut()) {
                    count.packets += 1;
                    count.peak = packet.samples.iter().fold(count.peak, |p, s| p.max(s.abs()));
                }
            }
        }
    }
}

#[derive(Default)]
struct PcmCount {
    packets: u64,
    peak: f64,
}

async fn run() -> Result<()> {
    let Some(options) = options(env::args().skip(1))? else {
        println!("{USAGE}");
        return Ok(());
    };
    let start = metrics::Snapshot::take().context("No se pudieron leer métricas del proceso")?;
    let config = EngineConfig::velo("Velo");
    println!(
        "Dispositivo Spotify Connect: {} ({})",
        config.name, config.device_id
    );
    let engine = if options.login {
        Engine::start_login(config).await?
    } else {
        Engine::from_keyring(config)
            .await
            .context("Sin login de audio guardado: repite con --login")?
    };
    println!("Listo. Desde el celular, «Velo» aparece en la lista de dispositivos.");
    let mut pcm = options.pcm.then(PcmCount::default);
    engine.set_pcm_enabled(options.pcm);
    if let Some(uri) = &options.uri {
        engine.load(target(uri))?;
    }
    let playing = metrics::Snapshot::take()?;
    watch_for(&engine, options.seconds, &mut pcm).await?;
    playing.report_until(&metrics::Snapshot::take()?, "Reproducción");
    engine.set_pcm_enabled(false);
    if let Some(duration) = options.pause {
        engine.pause()?;
        println!("Pausa: librespot conserva su keepalive de red; Velo no tiene sondeo propio.");
        let paused = metrics::Snapshot::take()?;
        watch_for(&engine, duration, &mut None).await?;
        paused.report_until(&metrics::Snapshot::take()?, "Pausa");
    }
    if let Some(count) = pcm {
        println!(
            "PCM en memoria: {} paquetes, pico {:.4}",
            count.packets, count.peak
        );
        if count.packets == 0 {
            bail!("No se recibieron muestras PCM");
        }
    }
    engine.shutdown().await;
    start.report_until(&metrics::Snapshot::take()?, "Proceso completo");
    Ok(())
}

#[tokio::main(worker_threads = 2)]
async fn main() -> ExitCode {
    // `RUST_LOG=librespot=debug` shows librespot's own reasons (it logs via `log`).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("play_probe: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const URI: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    #[test]
    fn arguments_are_parsed_before_any_login() {
        let defaults = options([URI.to_owned()]).unwrap().unwrap();
        assert_eq!(defaults.uri.as_deref(), Some(URI));
        assert_eq!(defaults.seconds, Duration::from_secs(20));
        assert!(!defaults.login && !defaults.pcm && defaults.pause.is_none());
        let explicit =
            options(["--login", URI, "3", "--pause-seconds", "130", "--pcm"].map(str::to_owned))
                .unwrap()
                .unwrap();
        assert!(explicit.login && explicit.pcm);
        assert_eq!(explicit.seconds, Duration::from_secs(3));
        assert_eq!(explicit.pause, Some(Duration::from_secs(130)));
        assert!(
            options(Vec::<String>::new())
                .unwrap()
                .unwrap()
                .uri
                .is_none()
        );
        assert!(options(["--help".to_owned()]).unwrap().is_none());
    }

    #[test]
    fn bad_arguments_are_rejected_and_uris_pick_a_target() {
        for args in [
            vec!["0"],
            vec!["--pause-seconds"],
            vec!["--unknown"],
            vec!["99999"],
        ] {
            assert!(options(args.into_iter().map(str::to_owned)).is_err());
        }
        assert!(matches!(target(URI), LoadTarget::Tracks { .. }));
        assert!(matches!(
            target("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
            LoadTarget::Context { .. }
        ));
    }
}
