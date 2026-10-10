//! X2: probe manual; requiere red y logins guardados. No se ejecuta en tests.
use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use velo_domain::MusicProvider;
use velo_player::{Engine, EngineConfig, LoadTarget, parse_track_uri, queue};
use velo_spotify::{KeyringTokenStore, SpotifyConfig, SpotifyProvider};

const USAGE: &str = "Cierra Velo antes de ejecutar.\n\
cargo run -p velo-player --example queue_probe -- clear spotify:track:4uLU6hMCjMI75M1A2tKUQC\n\
cargo run -p velo-player --example queue_probe -- save \"X2 cola de prueba\"\n\
clear reproduce tres ocurrencias de la canción en Velo y vacía las posteriores en pausa.\n\
save crea una playlist privada desde la canción y cola del dispositivo activo (Web API).\n\
Para save: define VELO_SPOTIFY_CLIENT_ID y usa el login Web API ya guardado.";

async fn clear(engine: &Engine, uri: &str) -> Result<()> {
    let mut state = engine.state();
    engine.load(LoadTarget::Tracks {
        uris: vec![uri.into(); 3],
        index: 0,
        shuffle: false,
    })?;
    tokio::time::timeout(
        Duration::from_secs(45),
        state.wait_for(|s| s.active && s.playing && s.track.as_ref().is_some_and(|t| t.uri == uri)),
    )
    .await
    .context("La canción no empezó en 45 s")??;
    tokio::time::sleep(Duration::from_secs(3)).await;
    engine.pause()?;
    tokio::time::timeout(Duration::from_secs(5), state.wait_for(|s| !s.playing))
        .await
        .context("No se confirmó la pausa")??;
    let before = state.borrow().clone();
    engine.replace_queue(uri, &[], &[])?;
    tokio::time::timeout(
        Duration::from_secs(10),
        state.wait_for(|s| {
            s.active
                && !s.playing
                && s.position_at != before.position_at
                && s.track.as_ref().is_some_and(|t| t.uri == uri)
        }),
    )
    .await
    .context("Spirc no confirmó la recarga en pausa")??;
    let after = state.borrow().clone();
    ensure!(
        after.position_ms.abs_diff(before.position_ms) <= 500,
        "La posición cambió más de 500 ms al vaciar"
    );
    ensure!(
        engine.known_tracks() == Some(vec![uri.to_string()]),
        "Quedaron otras ocurrencias en la lista recargada"
    );
    ensure!(
        !after.shuffle && after.repeat == before.repeat,
        "No se conservaron los modos previstos"
    );
    println!(
        "Cola vaciada: misma pista, posición {} → {} ms, pausa y repetición conservadas.",
        before.position_ms, after.position_ms
    );
    engine.play()?;
    tokio::time::timeout(Duration::from_secs(5), state.wait_for(|s| s.playing)).await??;
    println!("Reanudado: escucha continuidad y ausencia de clics durante 5 s.");
    tokio::time::sleep(Duration::from_secs(5)).await;
    Ok(())
}

async fn save(name: &str) -> Result<()> {
    let client_id = std::env::var("VELO_SPOTIFY_CLIENT_ID")
        .context("Define VELO_SPOTIFY_CLIENT_ID (el mismo Client ID del login Web API)")?;
    let api = SpotifyProvider::new(
        SpotifyConfig {
            client_id,
            ..SpotifyConfig::default()
        },
        Arc::new(KeyringTokenStore::new()),
    );
    ensure!(
        api.is_signed_in().await,
        "Inicia sesión Web API antes de ejecutar este probe"
    );
    let before = api
        .playback()
        .await?
        .context("No hay reproducción activa")?;
    let current = before.track.context("No se conoce la URI actual")?.uri;
    let visible = api
        .queue()
        .await?
        .into_iter()
        .map(|t| t.uri)
        .collect::<Vec<_>>();
    let after = api
        .playback()
        .await?
        .context("Se perdió el dispositivo activo")?;
    ensure!(
        after.track.as_ref().is_some_and(|t| t.uri == current),
        "Cambió la canción durante la consulta; vuelve a ejecutar"
    );
    let saved = queue::save_as_playlist(&api, name, &current, &visible).await?;
    println!(
        "Playlist privada creada: {} ({})",
        saved.playlist.name, saved.playlist.id
    );
    saved
        .items_result
        .context("La playlist existe, pero la cola quedó incompleta")?;
    println!(
        "Guardadas {} ocurrencias, en orden y con duplicados. Comprueba la playlist en Spotify.",
        visible.len() + 1
    );
    Ok(())
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [action, uri] if action == "clear" => {
            parse_track_uri(uri).context("URI de canción inválida")?;
            let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
            let result = clear(&engine, uri).await;
            engine.shutdown().await;
            result
        }
        [action] if action == "shuffle" => {
            let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
            let result = queue::connect_snapshot::shuffle_probe(&engine).await;
            engine.shutdown().await;
            Ok(result?)
        }
        [action, name] if action == "save" => save(name).await,
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    }
}
