//! Manual check against the real Spotify account.
//!
//! `cargo run -p velo-spotify --example login`
//! The Client ID comes from `VELO_SPOTIFY_CLIENT_ID`, or from `velo.local.env` at the repo root
//! (untracked; never commit it). Opens the browser the first time; later runs reuse the refresh
//! token in the OS keyring. Pass `--sign-out` to forget the stored token.

use std::sync::Arc;

use velo_domain::MusicProvider;
use velo_spotify::{KeyringTokenStore, SpotifyConfig, SpotifyProvider};

const LOCAL_ENV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../velo.local.env");

fn client_id() -> String {
    if let Ok(id) = std::env::var("VELO_SPOTIFY_CLIENT_ID") {
        return id;
    }
    std::fs::read_to_string(LOCAL_ENV)
        .ok()
        .and_then(|text| {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .find(|(key, _)| key.trim() == "VELO_SPOTIFY_CLIENT_ID")
                .map(|(_, value)| value.trim().to_string())
        })
        .expect(
            "set VELO_SPOTIFY_CLIENT_ID or create velo.local.env with VELO_SPOTIFY_CLIENT_ID=<id>",
        )
}

#[tokio::main]
async fn main() {
    let client_id = client_id();
    let config = SpotifyConfig {
        client_id,
        ..SpotifyConfig::default()
    };
    let spotify = SpotifyProvider::new(config, Arc::new(KeyringTokenStore::new()));

    if std::env::args().any(|a| a == "--sign-out") {
        spotify.sign_out().await.expect("sign out");
        println!("signed out");
        return;
    }

    if !spotify.is_signed_in().await {
        println!("opening the browser to sign in…");
        spotify.sign_in().await.expect("sign in");
    }

    let me = spotify.me().await.expect("me");
    println!(
        "user: {} ({}), product: {}",
        me.display_name, me.id, me.product
    );

    match spotify.playback().await {
        Ok(Some(p)) => println!(
            "playback: {} — {} | playing={} pos={}ms device={:?}",
            p.track.as_ref().map(|t| t.title.as_str()).unwrap_or("-"),
            p.track
                .as_ref()
                .map(|t| t.artist_line())
                .unwrap_or_default(),
            p.is_playing,
            p.position_now(),
            p.device.as_ref().map(|d| d.name.as_str())
        ),
        Ok(None) => println!("playback: nothing active"),
        Err(e) => println!("playback error: {e}"),
    }

    match spotify.devices().await {
        Ok(d) => println!(
            "devices: {:?}",
            d.iter()
                .map(|d| (&d.name, &d.kind, &d.id, d.is_active))
                .collect::<Vec<_>>()
        ),
        Err(e) => println!("devices error: {e}"),
    }

    match spotify.my_playlists(0, 5).await {
        Ok(p) => {
            println!("playlists ({} total):", p.total);
            for pl in &p.items {
                let listable = pl.items_listable_by(&me.id);
                println!(
                    "  - {} [{} tracks, listable={listable}]",
                    pl.name, pl.track_count
                );
            }
        }
        Err(e) => println!("playlists error: {e}"),
    }

    match spotify.saved_tracks(0, 3).await {
        Ok(p) => println!(
            "liked songs: {} total, first: {:?}",
            p.total,
            p.items.first().map(|t| &t.title)
        ),
        Err(e) => println!("liked error: {e}"),
    }

    match spotify.search("radiohead", 0, 10).await {
        Ok(r) => println!(
            "search: {} tracks, first: {:?}",
            r.tracks.items.len(),
            r.tracks.items.first().map(|t| &t.title)
        ),
        Err(e) => println!("search error: {e}"),
    }
}
