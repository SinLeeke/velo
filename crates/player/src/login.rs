//! The Spotify login. Spotify only streams audio to sessions authorized by librespot's own OAuth
//! client (E1: Velo's Client ID authenticates but spclient refuses the audio), so the engine
//! signs in once with that client, scope `streaming`, and keeps ONLY its refresh token in the OS
//! keyring under its own entry. L1: by default this is Velo's only login; the library is read
//! through the same session (`session_api`, login5 tokens), so the scope stays `streaming`
//! (login5 tokens do not depend on it). Only a personal Spotify app keeps a second, Web API
//! login with its own entry. Access tokens live in memory; nothing here logs or prints a token.

use std::{sync::Arc, time::Duration};

#[cfg(test)]
use librespot_oauth::OAuthError;

use velo_spotify::{KeyringTokenStore, TokenStore};

use crate::PlayerError;

/// librespot's public OAuth client (the one Spotify streams audio to).
pub const LIBRESPOT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
/// Registered for that client; librespot-oauth listens here for the browser redirect.
pub const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
#[cfg(test)]
const REDIRECT_ADDR: &str = "127.0.0.1:8898";
pub const SCOPE: &str = "streaming";
/// Keyring entry (service `velo`) of the audio refresh token.
pub const KEYRING_USER: &str = "spotify-librespot-refresh-token";

/// The user has this long to finish the browser login.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(20);

/// The keyring entry of the audio login.
pub fn keyring_store() -> Arc<dyn TokenStore> {
    Arc::new(KeyringTokenStore::with_user(KEYRING_USER))
}

/// Whether an audio login is stored (no network).
pub async fn has_stored_login(store: Arc<dyn TokenStore>) -> bool {
    load(store).await.ok().flatten().is_some()
}

/// Forgets the audio login.
pub async fn forget_login(store: Arc<dyn TokenStore>) -> Result<(), PlayerError> {
    tokio::task::spawn_blocking(move || store.clear())
        .await
        .map_err(|_| PlayerError::TokenStore)?
        .map_err(|_| PlayerError::TokenStore)
}

async fn load(store: Arc<dyn TokenStore>) -> Result<Option<String>, PlayerError> {
    tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|_| PlayerError::TokenStore)
        .map(|token| token.filter(|t| !t.is_empty()))
}

async fn save(store: Arc<dyn TokenStore>, refresh: String) -> Result<(), PlayerError> {
    tokio::task::spawn_blocking(move || store.save(&refresh))
        .await
        .map_err(|_| PlayerError::TokenStore)?
        .map_err(|_| PlayerError::TokenStore)
}

/// The refresh token to store after a token response, if it changed.
pub(crate) fn rotated_refresh(old: Option<&str>, returned: &str) -> Option<String> {
    (!returned.is_empty() && old != Some(returned)).then(|| returned.to_string())
}

/// What a failed OAuth call means for the stored login. Only the error kind is kept: the
/// upstream text is never logged or shown.
#[cfg(test)]
pub(crate) fn classify(error: &OAuthError) -> PlayerError {
    match error {
        OAuthError::AuthCodeListenerBind { .. } => PlayerError::LoginPortBusy,
        // The browser came back without a code: the user denied access or closed the flow.
        OAuthError::AuthCodeNotFound { .. } | OAuthError::AuthCodeBadUri { .. } => {
            PlayerError::LoginCancelled
        }
        OAuthError::ExchangeCode { e } if e.contains("invalid_grant") => PlayerError::NotSignedIn,
        OAuthError::ExchangeCode { e } if e.contains("Server returned error") => {
            PlayerError::OAuthRejected
        }
        OAuthError::ExchangeCode { .. } => PlayerError::OAuthNetwork,
        _ => PlayerError::LoginFailed,
    }
}

/// Opens the system browser on Spotify's consent page (librespot's client, scope `streaming`),
/// waits for the redirect, stores the refresh token and returns an access token.
///
/// librespot-oauth accepts the redirect with a blocking `TcpListener`, so the flow runs on its
/// own thread (never on a runtime worker, never on the UI thread). After [`LOGIN_TIMEOUT`] the
/// listener is unblocked with a local request so the port is released.
pub async fn login_in_browser(store: Arc<dyn TokenStore>) -> Result<String, PlayerError> {
    login_with_route(store).await
}

/// Uses Velo's configurable HTTP transport. Callback is a direct loopback listener.
async fn login_with_route(store: Arc<dyn TokenStore>) -> Result<String, PlayerError> {
    crate::install_crypto_provider();
    let pending = velo_spotify::oauth::PendingLogin::bind_with_scopes(
        "https://accounts.spotify.com",
        LIBRESPOT_CLIENT_ID,
        8898,
        SCOPE,
    )
    .await
    .map_err(|_| PlayerError::LoginPortBusy)?;
    let url = pending.authorization_url.to_string();
    tokio::task::spawn_blocking(move || {
        #[cfg(windows)]
        let result = std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn();
        #[cfg(not(windows))]
        let result = std::process::Command::new("xdg-open").arg(&url).spawn();
        result.map(|_| ()).map_err(|_| PlayerError::LoginFailed)
    })
    .await
    .map_err(|_| PlayerError::LoginFailed)??;
    let code = tokio::time::timeout(LOGIN_TIMEOUT, pending.receive_code())
        .await
        .map_err(|_| PlayerError::LoginTimeout)?
        .map_err(|_| PlayerError::LoginCancelled)?;
    let token = velo_spotify::oauth::audio_token(&[
        ("grant_type", "authorization_code"),
        ("client_id", LIBRESPOT_CLIENT_ID),
        ("redirect_uri", &pending.redirect_uri),
        ("code", &code),
        ("code_verifier", &pending.verifier),
    ])
    .await
    .map_err(|e| audio_oauth_error(&e))?;
    if token.access_token.is_empty() || token.refresh_token.is_empty() {
        return Err(PlayerError::OAuthRejected);
    }
    save(store, token.refresh_token).await?;
    Ok(token.access_token)
}

/// A fresh access token from the stored refresh token (no browser). A revoked grant clears the
/// entry and reports [`PlayerError::NotSignedIn`]; transient errors keep it.
pub async fn access_token_from_store(store: Arc<dyn TokenStore>) -> Result<String, PlayerError> {
    crate::install_crypto_provider();
    let refresh = load(store.clone()).await?.ok_or(PlayerError::NotSignedIn)?;
    let token = match tokio::time::timeout(
        REFRESH_TIMEOUT,
        velo_spotify::oauth::audio_token(&[
            ("grant_type", "refresh_token"),
            ("client_id", LIBRESPOT_CLIENT_ID),
            ("refresh_token", &refresh),
        ]),
    )
    .await
    {
        Err(_) => return Err(PlayerError::OAuthNetwork),
        Ok(Err(e)) => {
            let error = audio_oauth_error(&e);
            if matches!(error, PlayerError::NotSignedIn) {
                forget_login(store).await?;
            }
            return Err(error);
        }
        Ok(Ok(token)) => token,
    };
    if token.access_token.is_empty() {
        return Err(PlayerError::OAuthRejected);
    }
    if let Some(rotated) = rotated_refresh(Some(&refresh), &token.refresh_token) {
        save(store, rotated).await?;
    }
    Ok(token.access_token)
}

fn audio_oauth_error(error: &velo_domain::VeloError) -> PlayerError {
    match error {
        velo_domain::VeloError::NotAuthenticated => PlayerError::NotSignedIn,
        velo_domain::VeloError::Network(_) => PlayerError::OAuthNetwork,
        _ => PlayerError::OAuthRejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velo_spotify::MemoryTokenStore;

    #[test]
    fn only_a_new_nonempty_refresh_token_is_stored() {
        assert_eq!(rotated_refresh(None, "r1").as_deref(), Some("r1"));
        assert_eq!(rotated_refresh(Some("r1"), "r2").as_deref(), Some("r2"));
        assert_eq!(rotated_refresh(Some("r1"), "r1"), None);
        assert_eq!(rotated_refresh(Some("r1"), ""), None);
        assert_eq!(rotated_refresh(None, ""), None);
    }

    #[test]
    fn oauth_errors_map_to_actionable_kinds_without_upstream_text() {
        let exchange = |e: &str| OAuthError::ExchangeCode { e: e.into() };
        assert!(matches!(
            classify(&exchange(
                "Server returned error response: invalid_grant: Refresh token revoked"
            )),
            PlayerError::NotSignedIn
        ));
        assert!(matches!(
            classify(&exchange("Server returned error response: invalid_client")),
            PlayerError::OAuthRejected
        ));
        assert!(matches!(
            classify(&exchange("Request failed: dns error")),
            PlayerError::OAuthNetwork
        ));
        assert!(matches!(
            classify(&OAuthError::AuthCodeNotFound {
                uri: "http://localhost/login?error=access_denied".into()
            }),
            PlayerError::LoginCancelled
        ));
        assert!(matches!(
            classify(&OAuthError::AuthCodeListenerBind {
                addr: REDIRECT_ADDR.parse().unwrap(),
                e: std::io::Error::other("in use"),
            }),
            PlayerError::LoginPortBusy
        ));
        let secret = classify(&exchange("Server returned error response: fixture-secret"));
        assert!(!format!("{secret:?} {secret}").contains("fixture-secret"));
    }

    #[test]
    fn the_audio_login_has_its_own_keyring_entry_and_client() {
        assert_ne!(KEYRING_USER, "spotify-refresh-token");
        assert_eq!(SCOPE, "streaming");
        assert!(REDIRECT_URI.starts_with(&format!("http://{REDIRECT_ADDR}/")));
    }

    #[tokio::test]
    async fn stored_login_is_detected_and_forgotten_without_network() {
        let store = Arc::new(MemoryTokenStore::new());
        assert!(!has_stored_login(store.clone()).await);
        assert!(matches!(
            access_token_from_store(store.clone()).await,
            Err(PlayerError::NotSignedIn)
        ));
        store.save("").unwrap();
        assert!(!has_stored_login(store.clone()).await);
        store.save("fixture-refresh").unwrap();
        assert!(has_stored_login(store.clone()).await);
        forget_login(store.clone()).await.unwrap();
        assert!(store.load().is_none());
    }
}
