//! E1 diagnosis: librespot's spclient refuses audio for tokens minted with a third-party
//! Client ID (400 InvalidArgument; with the default session id, INVALID_CREDENTIALS).
//! This probe signs in with librespot's own OAuth flow (librespot-oauth, keymaster client,
//! scope `streaming`) and plays a track, to confirm that path works before E2.
//!
//! `cargo run -p velo-player --example ls_login_probe -- spotify:track:<id> [seconds]`
//! Opens the browser once; nothing is stored (the token lives only in this process).

use std::time::Duration;

use librespot_core::{Session, SessionConfig, SpotifyUri, authentication::Credentials};
use librespot_oauth::OAuthClientBuilder;
use librespot_playback::{
    audio_backend,
    config::{AudioFormat, PlayerConfig},
    mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
    player::Player,
};

const LIBRESPOT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";

#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    velo_player::install_crypto_provider();

    let mut args = std::env::args().skip(1);
    let uri = args
        .next()
        .unwrap_or_else(|| "spotify:track:4wajJ1o7jWIg62YqpkHC7S".into());
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(15);

    let oauth = OAuthClientBuilder::new(LIBRESPOT_CLIENT_ID, REDIRECT_URI, vec!["streaming"])
        .open_in_browser()
        .with_custom_message("<!doctype html><p>Velo: puedes volver a la terminal.</p>")
        .build()?;
    let token = oauth.get_access_token_async().await?;
    println!("token OK (scopes: {:?})", token.scopes);

    let session = Session::new(SessionConfig::default(), None);
    session
        .connect(Credentials::with_access_token(token.access_token), false)
        .await?;
    println!("sesión OK, país {}", session.country());

    let mixer = SoftMixer::open(MixerConfig::default())?;
    mixer.set_volume(u16::MAX / 2);
    let backend = audio_backend::find(Some("rodio".into())).expect("rodio backend");
    let player = Player::new(
        PlayerConfig::default(),
        session.clone(),
        mixer.get_soft_volume(),
        move || backend(None, AudioFormat::default()),
    );
    let mut events = player.get_player_event_channel();
    player.load(SpotifyUri::from_uri(&uri)?, true, 0);

    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            ev = events.recv() => match ev {
                Some(ev) => println!("evento: {}", format!("{ev:?}").chars().take(160).collect::<String>()),
                None => break,
            },
        }
    }
    player.stop();
    session.shutdown();
    Ok(())
}
