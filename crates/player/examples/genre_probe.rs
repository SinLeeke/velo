//! F5: solo consulta metadata/descriptores con el login de audio guardado.
use anyhow::{Result, bail};
use librespot_metadata::{Metadata, Track};
use std::time::Duration;
use velo_player::{Engine, EngineConfig, descriptors::track_descriptors};

#[tokio::main]
async fn main() -> Result<()> {
    let uris: Vec<_> = std::env::args().skip(1).collect();
    if uris.is_empty() || uris.len() > 10 {
        bail!("Uso: genre_probe <1 a 10 URI de canciones>");
    }
    for uri in &uris {
        velo_player::parse_track_uri(uri)?;
    }
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let mut successes = 0;
    for uri in uris {
        let id = velo_player::parse_track_uri(&uri)?;
        match tokio::time::timeout(Duration::from_secs(15), Track::get(engine.session(), &id)).await
        {
            Ok(Ok(track)) => println!(
                "{uri}: {} tags sin taxonomía; no se interpretan como géneros/ánimos",
                track.tags.len()
            ),
            _ => println!("{uri}: metadata no disponible o timeout"),
        }
        match track_descriptors(engine.session(), &uri).await {
            Ok(descriptors) => {
                successes += 1;
                println!("{uri}: {descriptors:?}");
            }
            Err(error) => println!("{uri}: {error}"),
        }
    }
    engine.shutdown().await;
    if successes == 0 {
        bail!(
            "Ninguna canción devolvió géneros/ánimos decodificables. No habilitar chips; ver docs/F5.md."
        );
    }
    println!(
        "{successes} canciones con descriptores; verificar cobertura antes de integrar filtros."
    );
    Ok(())
}
