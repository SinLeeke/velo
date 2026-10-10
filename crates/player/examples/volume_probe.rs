//! R3: prueba auditiva de volumen, arrastre a 16 ms y cambio de volumen en pausa.
//! El usuario la ejecuta con red y login de audio guardado; no exporta ni guarda PCM.
//! `cargo run -p velo-player --example volume_probe -- spotify:track:<id>`

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use velo_player::{Engine, EngineConfig, LoadTarget, parse_track_uri};

const USAGE: &str = "cargo run -p velo-player --example volume_probe -- spotify:track:<id>";

async fn wait_volume(engine: &Engine, volume: u8) -> Result<()> {
    let mut state = engine.state();
    let sent = Instant::now();
    engine.set_volume(volume)?;
    tokio::time::timeout(
        Duration::from_secs(5),
        state.wait_for(|s| s.volume == volume),
    )
    .await
    .context("Spotify tardó más de 5 s en confirmar el volumen")?
    .context("Se cerró el motor")?;
    println!(
        "Volumen {volume}%: evento confirmado en {} ms (no mide latencia del altavoz).",
        sent.elapsed().as_millis()
    );
    Ok(())
}

async fn exercise(engine: &Engine, uri: String) -> Result<()> {
    let mut state = engine.state();
    engine.load(LoadTarget::Tracks {
        uris: vec![uri.clone()],
        index: 0,
        shuffle: false,
    })?;
    tokio::time::timeout(
        Duration::from_secs(45),
        state.wait_for(|s| s.active && s.playing && s.track.as_ref().is_some_and(|t| t.uri == uri)),
    )
    .await
    .context("La pista no empezó en 45 s")?
    .context("Se cerró el motor")?;
    println!("Escucha si hay clics al silenciar/restaurar a 20% y al pasar entre 20% y 40%.");
    for volume in [20, 0, 20, 40, 20] {
        wait_volume(engine, volume).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    println!("Arrastre 0 → 40 → 0, un comando cada 16 ms; escucha saltos o retrasos.");
    for volume in (0..=40).chain((0..40).rev()) {
        engine.set_volume(volume)?;
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
    wait_volume(engine, 0).await?;
    engine.pause()?;
    tokio::time::timeout(Duration::from_secs(5), state.wait_for(|s| !s.playing))
        .await
        .context("No se confirmó la pausa")?
        .context("Se cerró el motor")?;
    println!("En pausa: cambia a 20%. Al reanudar debe sonar a ese nivel desde el inicio.");
    wait_volume(engine, 20).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    engine.play()?;
    tokio::time::sleep(Duration::from_secs(4)).await;
    println!(
        "R3b: fade de pausa 120 ms / entrada 150 ms. Alternar rápido debe invertir sin clics."
    );
    for volume in [20, 80] {
        wait_volume(engine, volume).await?;
        for hold in [40, 70, 200, 40, 300] {
            engine.pause()?;
            tokio::time::timeout(Duration::from_secs(5), state.wait_for(|s| !s.playing)).await??;
            tokio::time::sleep(Duration::from_millis(hold)).await;
            engine.play()?;
            tokio::time::timeout(Duration::from_secs(5), state.wait_for(|s| s.playing)).await??;
            tokio::time::sleep(Duration::from_millis(hold)).await;
        }
        println!(
            "Volumen {volume}%: seek atrás y adelante; debe haber solo un micro-fade, sin audio viejo."
        );
        let position = velo_player::queue::position_now(&state.borrow(), Instant::now());
        engine.seek(position.saturating_sub(3000))?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        engine.seek(position)?;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    println!(
        "Temporizador al final: seek a los últimos 3 s; debe acabar el PCM de esta canción y cerrar la salida."
    );
    let mut ending = engine.sleep_at_track_end(&uri)?;
    let duration = state.borrow().track.as_ref().map_or(0, |t| t.duration_ms);
    engine.seek(duration.saturating_sub(3000))?;
    tokio::time::timeout(
        Duration::from_secs(15),
        ending.wait_for(|s| *s != velo_player::TrackEndTimerState::Armed),
    )
    .await??;
    if *ending.borrow() != velo_player::TrackEndTimerState::Finished {
        bail!("Se canceló el temporizador antes del fin del PCM");
    }
    if state.borrow().audio_errors > 0 {
        bail!("Hubo un error de salida de audio durante la prueba");
    }
    Ok(())
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        println!("{USAGE}\nCierra Velo antes de ejecutarlo. Usa el login de audio ya guardado.");
        return Ok(());
    }
    let [uri] = args.as_slice() else {
        bail!("Uso: {USAGE}")
    };
    parse_track_uri(uri).context("Se necesita una URI de canción válida")?;
    let mut config = EngineConfig::velo("Velo");
    config.initial_volume = 20;
    let engine = Engine::from_keyring(config)
        .await
        .context("No se pudo abrir el motor con el login de audio guardado")?;
    let result = exercise(&engine, uri.clone()).await;
    engine.shutdown().await;
    result
}
