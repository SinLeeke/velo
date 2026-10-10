//! Prueba en vivo de F4, sin reproducción ni cuerpos de respuesta en logs.
use anyhow::{Context, Result};
use velo_player::{Engine, EngineConfig, playlist};

#[tokio::main]
async fn main() -> Result<()> {
    let uri = std::env::args()
        .nth(1)
        .context("Indica spotify:playlist:<id>")?;
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let result = async {
        let items = playlist::playlist_items(&engine, &uri).await?;
        println!(
            "F4: {} canciones; {} con fecha",
            items.len(),
            items.iter().filter(|i| i.added_at.is_some()).count()
        );
        let tracks = playlist::tracks(&engine, &items[..items.len().min(10)]).await;
        for track in tracks {
            println!(
                "{} · {} · {} · {} ms",
                track.uri,
                track.title,
                track.artist_line(),
                track.duration_ms
            );
        }
        Ok(())
    }
    .await;
    engine.shutdown().await;
    result
}
