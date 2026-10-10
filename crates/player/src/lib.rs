//! Velo's own playback engine (task E2): librespot streams with the user's Premium account and
//! publishes this computer as the Spotify Connect device "Velo". Independent of Qt; start it
//! from a Tokio runtime worker, never from the UI thread.
//!
//! - [`login`]: the one-time audio login (librespot's OAuth client, scope `streaming`); only
//!   its refresh token is kept, in the OS keyring.
//! - [`Engine`]: Session + Player + Spirc. State by `watch` channel, pushed by librespot events;
//!   commands go straight to Spirc.
//! - PCM hook for the visualizer: off by default, bounded, lossy, memory only.
//!
//! Optional bounded librespot cache contains encrypted compressed audio only. Credentials
//! stay in the OS keyring; volume is never persisted by librespot.

pub mod artist;
pub mod collection;
pub mod descriptors;
pub mod discover;
pub mod dj;
mod engine;
pub mod eq;
pub mod extras;
pub mod login;
pub mod lyrics;
mod output;
mod pcm;
pub mod playlist;
pub mod queue;
pub mod session_api;
mod state;

pub use dj::{DjSpeech, DjState};
pub use engine::{Engine, EngineConfig, EngineOptions, LoadTarget, stable_device_id};
pub use eq::EqConfig;
pub use librespot_playback::player::PlayerEvent as PlaybackTransportEvent;
pub use pcm::{PcmPacket, PlaybackProgress, TrackEndTimerState, VoiceClip, VoiceProgress};
pub use state::{
    EngineState, EngineTrack, SLOW_LOAD, TrackFailure, percent_to_volume, repeat_mode,
    repeat_switches, volume_to_percent,
};

use librespot_core::SpotifyUri;

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("No audio login stored; enable playback in Velo first")]
    NotSignedIn,
    #[error("The audio login was cancelled in the browser")]
    LoginCancelled,
    #[error("The audio login was not completed in time")]
    LoginTimeout,
    #[error("The audio login could not start (port 8898 in use?)")]
    LoginPortBusy,
    #[error("The audio login failed")]
    LoginFailed,
    #[error("System credential store operation failed")]
    TokenStore,
    #[error("OAuth request failed or timed out")]
    OAuthNetwork,
    #[error("Spotify rejected the audio login")]
    OAuthRejected,
    #[error("Spotify refused the playback session (Premium required?)")]
    Session,
    #[error("Spotify playback session timed out")]
    SessionTimeout,
    #[error("Audio output unavailable")]
    AudioBackend,
    #[error("The playback engine has stopped")]
    Stopped,
    #[error("Not a playable Spotify URI")]
    InvalidUri,
    /// The song changed (or Velo stopped being the active device) while editing the queue.
    #[error("The current track changed while editing the queue")]
    QueueChanged,
    #[error("Spotify no entregó la sesión de DJ Livi; vuelve a intentarlo")]
    DjUnavailable,
    #[error("DJ Livi no está sonando en Velo")]
    DjInactive,
    #[error("No se pudo resolver el contexto completo de la cola")]
    QueueContext,
    #[error("Invalid playback engine options")]
    InvalidOptions,
    #[error("Encrypted audio cache could not be opened")]
    AudioCache,
}

/// Strictly parses `spotify:track:<22-character base62 id>` (no URLs, local files or suffixes).
pub fn parse_track_uri(uri: &str) -> Result<SpotifyUri, PlayerError> {
    let id = uri
        .strip_prefix("spotify:track:")
        .ok_or(PlayerError::InvalidUri)?;
    if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(PlayerError::InvalidUri);
    }
    SpotifyUri::from_uri(uri).map_err(|_| PlayerError::InvalidUri)
}

/// Both rustls crypto providers are compiled into the workspace (reqwest brings aws-lc-rs,
/// librespot's TLS stack brings ring), and rustls panics when it has to pick one on its own.
/// Installs aws-lc-rs as the process default; harmless if something already installed one.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn track_uri_round_trips() {
        let uri = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
        assert_eq!(parse_track_uri(uri).unwrap().to_uri().unwrap(), uri);
    }
    #[test]
    fn rejects_noncanonical_or_nontrack_uris() {
        for uri in [
            "",
            "spotify:track:short",
            "spotify:album:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:local:a:b:c:12",
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC:extra",
            "spotify:track:4uLU6hMCjMI75M1A2tKUQ!",
            " spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:track:ZZZZZZZZZZZZZZZZZZZZZZ",
        ] {
            assert!(matches!(parse_track_uri(uri), Err(PlayerError::InvalidUri)));
        }
    }
}
