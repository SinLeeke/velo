//! Live-only probe: cargo run -p velo-player --example discover_probe -- <seed URI> [more seeds].
//! Uses the saved audio login, never logs tokens or raw response bodies. No playback/audio load.
use anyhow::{Context, Result};
use velo_player::{
    Engine, EngineConfig,
    discover::{Discover, Seed},
};

#[tokio::main]
async fn main() -> Result<()> {
    let uris: Vec<String> = std::env::args().skip(1).collect();
    let uri = uris
        .first()
        .context("Pass a canonical Spotify track/artist/album/playlist URI")?;
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let seeds: Vec<Seed> = uris
        .iter()
        .map(|uri| Seed {
            uri: uri.clone(),
            title: uri.clone(),
            subtitle: String::new(),
            image_url: None,
        })
        .collect();
    let discover = Discover::new(&engine, &seeds);
    println!("Radio: {:#?}", discover.radio(uri).await);
    println!(
        "Para ti (radios de las semillas proporcionadas): {:#?}",
        discover.for_you().await
    );
    engine.shutdown().await;
    Ok(())
}
