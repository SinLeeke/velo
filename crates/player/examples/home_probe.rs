//! H1: búsquedas de Inicio + lectura F4 de una playlist encontrada, sin reproducir.
//! Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::sync::Arc;
use velo_domain::{MusicProvider, TimeRange};
use velo_player::{Engine, EngineConfig, playlist};
use velo_spotify::{KeyringTokenStore, SpotifyConfig, SpotifyProvider};

#[tokio::main]
async fn main() -> Result<()> {
    let id = std::env::var("VELO_SPOTIFY_CLIENT_ID")
        .context("Define VELO_SPOTIFY_CLIENT_ID con el Client ID de Velo")?;
    let api = SpotifyProvider::new(
        SpotifyConfig {
            client_id: id,
            ..Default::default()
        },
        Arc::new(KeyringTokenStore::new()),
    );
    if !api.is_signed_in().await {
        bail!("Inicia sesión Web API en Velo antes del probe");
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut found = Vec::new();
    for term in ["Discover Weekly", "Release Radar", "Daily Mix", "daylist"] {
        match api.search(term, 0, 10).await {
            Ok(page) => {
                let mut count = 0;
                for p in page.playlists.items {
                    let name = p.name.trim().to_lowercase();
                    let matches = name == term.to_lowercase()
                        || (term == "Daily Mix"
                            && name.strip_prefix("daily mix ").is_some_and(|s| {
                                !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
                            }));
                    if (p.owner_id == "spotify" || p.owner == "Spotify")
                        && matches
                        && ids.insert(p.id.clone())
                        && names.insert(name)
                        && count < 6
                    {
                        println!("{} · {} · {}", p.name, p.owner, p.uri);
                        found.push(p.uri);
                        count += 1;
                    }
                }
                println!("{term}: {count} playlists aceptadas");
            }
            Err(_) => println!("{term}: búsqueda no disponible"),
        }
    }
    match api.top_tracks(TimeRange::ShortTerm, 0, 20).await {
        Ok(page) => println!("Top tracks short_term: {} canciones", page.items.len()),
        Err(_) => println!("Top tracks: no disponible"),
    }
    let uri = std::env::args().nth(1).or_else(|| found.first().cloned());
    let Some(uri) = uri else {
        bail!("No se encontró ninguna playlist; para probar F4 pasa spotify:playlist:<ID>");
    };
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let result = async {
        let items = playlist::playlist_items(&engine, &uri).await?;
        let tracks = playlist::tracks(&engine, &items[..items.len().min(10)]).await;
        println!(
            "F4 {uri}: {} filas; {} metadatos de las primeras 10",
            items.len(),
            tracks.len()
        );
        if tracks.is_empty() {
            bail!("Spotify no devolvió canciones para esta playlist");
        }
        Ok(())
    }
    .await;
    engine.shutdown().await;
    result
}
