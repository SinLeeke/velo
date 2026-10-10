//! Prueba en vivo de letras Spotify; no imprime versos, tokens ni respuestas brutas.
use anyhow::{Context, Result};
use velo_player::{Engine, EngineConfig, lyrics};

#[tokio::main]
async fn main() -> Result<()> {
    let uris: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        !uris.is_empty() && uris.len() <= 10,
        "Indica de una a diez URIs spotify:track:<id>"
    );
    for uri in &uris {
        velo_player::parse_track_uri(uri).context("URI de canción inválida")?;
    }
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let mut found = 0;
    for uri in uris {
        match lyrics::lyrics(&engine, &uri).await {
            Ok(Some(lyrics)) => {
                found += 1;
                println!(
                    "{uri}: fuente=spotify; sincronizadas={}; líneas={}; texto={}; primer_ms={:?}; último_ms={:?}",
                    !lyrics.synced.is_empty(),
                    lyrics.synced.len(),
                    lyrics.plain.is_some(),
                    lyrics.synced.first().map(|line| line.time_ms),
                    lyrics.synced.last().map(|line| line.time_ms)
                );
            }
            Ok(None) => println!("{uri}: Spotify sin letras; la app recurrirá a LRCLIB"),
            Err(error) => println!("{uri}: {error}; la app recurrirá a LRCLIB"),
        }
    }
    engine.shutdown().await;
    anyhow::ensure!(
        found > 0,
        "Ninguna canción entregó letras de Spotify; prueba otra y comprueba el respaldo LRCLIB en Velo"
    );
    Ok(())
}
