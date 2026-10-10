//! Official Spotify Web API remote control. This crate never plays audio.
mod models;
pub mod oauth;
mod tokens;

pub use tokens::{KeyringTokenStore, MemoryTokenStore, TokenStore};

use async_trait::async_trait;
use reqwest::{Client, Method, Response, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};
use velo_domain::*;

/// Connection pools follow applyProxy; credentials are never part of this route.
pub struct ProxyHttpClient {
    cached: std::sync::Mutex<(u64, Client)>,
}
impl Default for ProxyHttpClient {
    fn default() -> Self {
        Self::new()
    }
}
impl ProxyHttpClient {
    pub fn new() -> Self {
        let route = proxy_route().borrow().clone();
        Self {
            cached: std::sync::Mutex::new((route.generation, Self::build(&route))),
        }
    }
    fn build(route: &ProxyRoute) -> Client {
        let mut builder = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(url) = &route.url {
            // Only trusted, credential-free local CONNECT endpoints are published.
            builder = builder.proxy(reqwest::Proxy::all(url).expect("validated proxy route"));
        }
        builder.build().expect("HTTP client configuration")
    }
    fn client(&self) -> Client {
        let route = proxy_route().borrow().clone();
        let mut cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        if cached.0 != route.generation {
            *cached = (route.generation, Self::build(&route));
        }
        cached.1.clone()
    }
    pub fn post(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.client().post(url)
    }
    pub fn get(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.client().get(url)
    }
    pub fn request(&self, method: Method, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.client().request(method, url)
    }
}

#[derive(Clone, Debug)]
pub struct SpotifyConfig {
    pub client_id: String,
    pub api_base: String,
    pub accounts_base: String,
    /// Loopback OAuth callback port; 0 asks the OS to choose a free port.
    pub redirect_port: u16,
}

impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            client_id: oauth::DEFAULT_WEB_CLIENT_ID.into(),
            api_base: "https://api.spotify.com/v1".into(),
            accounts_base: "https://accounts.spotify.com".into(),
            redirect_port: oauth::WEB_REDIRECT_PORT,
        }
    }
}

#[derive(Clone)]
struct AccessToken {
    value: String,
    expires_at: Instant,
}

#[derive(Default)]
struct AuthState {
    access: Option<AccessToken>,
    // Prevent a login already in progress from undoing a later sign-out.
    epoch: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

/// L1: where the Web API access token comes from when it is not Velo's own PKCE login, e.g.
/// the login5 token of the librespot session. Tokens stay in memory; never log them.
#[async_trait]
pub trait TokenSource: Send + Sync {
    /// A valid access token and how long it stays valid. `rejected` is the token the API just
    /// answered 401 to, so the source can replace it.
    async fn token(&self, rejected: Option<&str>) -> Result<(String, Duration)>;
}

pub struct SpotifyProvider {
    config: SpotifyConfig,
    client: ProxyHttpClient,
    token_store: Arc<dyn TokenStore>,
    /// External tokens (L1); `None`: PKCE + refresh token in `token_store`.
    token_source: Option<Arc<dyn TokenSource>>,
    auth: Mutex<AuthState>,
    login: Mutex<()>,
    /// After a long 429, requests fail locally until this instant instead of hitting Spotify.
    rate_until: std::sync::Mutex<Option<Instant>>,
    pub app_unavailable: tokio::sync::broadcast::Sender<&'static str>,
}

/// Identidad y cola de una única respuesta: /me/player puede avanzar antes que este endpoint.
#[derive(Debug)]
pub struct QueueSnapshot {
    pub current: Option<String>,
    pub tracks: Vec<Track>,
}

impl SpotifyProvider {
    /// Publica una autorización ya verificada. El candidato usa MemoryTokenStore;
    /// el token sólo llega al keyring después de comprobar /me y la cuenta esperada.
    pub async fn adopt_session_from(&self, source: &Self) -> Result<()> {
        let auth = source.auth.lock().await;
        let refresh = source
            .store_load()
            .await?
            .ok_or(VeloError::NotAuthenticated)?;
        self.store_save(refresh).await?;
        self.auth.lock().await.access = auth.access.clone();
        Ok(())
    }

    /// Invalida peticiones anteriores sin borrar el grant que acaba de reemplazarlas.
    pub async fn retire(&self) {
        let mut auth = self.auth.lock().await;
        auth.epoch = auth.epoch.wrapping_add(1);
        auth.access = None;
    }

    pub async fn queue_snapshot(&self) -> Result<QueueSnapshot> {
        let response = self.get("/me/player/queue", &[]).await?;
        if !response["queue"].is_array() {
            return Err(VeloError::Other("Spotify returned an invalid queue".into()));
        }
        Ok(QueueSnapshot {
            current: response["currently_playing"]["uri"]
                .as_str()
                .map(str::to_owned),
            tracks: models::list(&response["queue"], models::track),
        })
    }

    pub fn new(config: SpotifyConfig, token_store: Arc<dyn TokenStore>) -> Self {
        Self {
            config,
            client: ProxyHttpClient::new(),
            token_store,
            token_source: None,
            auth: Mutex::new(AuthState::default()),
            login: Mutex::new(()),
            rate_until: std::sync::Mutex::new(None),
            app_unavailable: tokio::sync::broadcast::channel(4).0,
        }
    }

    /// A provider whose access tokens come from `source` (no browser login, nothing stored).
    pub fn with_token_source(config: SpotifyConfig, source: Arc<dyn TokenSource>) -> Self {
        Self {
            token_source: Some(source),
            ..Self::new(config, Arc::new(MemoryTokenStore::new()))
        }
    }

    /// Remaining time of the local 429 pause, if any.
    fn rate_limited(&self) -> Option<Duration> {
        let mut until = self.rate_until.lock().unwrap_or_else(|e| e.into_inner());
        let left = until.map(|u| u.saturating_duration_since(Instant::now()));
        match left {
            Some(left) if !left.is_zero() => Some(left),
            _ => {
                *until = None;
                None
            }
        }
    }

    fn pause_requests(&self, delay: Duration) {
        let until = Instant::now() + delay.min(Duration::from_secs(3600));
        let mut slot = self.rate_until.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none_or(|current| current < until) {
            *slot = Some(until);
        }
    }

    fn report_quota(&self) {
        // An external source is not Velo's app: its quota is not something the user can fix.
        if self.token_source.is_none() {
            let _ = self.app_unavailable.send("quota");
        }
    }

    async fn store_load(&self) -> Result<Option<String>> {
        let store = self.token_store.clone();
        tokio::task::spawn_blocking(move || store.load())
            .await
            .map_err(|_| VeloError::Other("Token store worker failed".into()))
    }

    async fn store_save(&self, token: String) -> Result<()> {
        let store = self.token_store.clone();
        tokio::task::spawn_blocking(move || store.save(&token))
            .await
            .map_err(|_| {
                VeloError::Other(
                    "No se pudo guardar la sesión en el Administrador de credenciales".into(),
                )
            })?
            .map_err(|_| {
                VeloError::Other(
                    "No se pudo guardar la sesión en el Administrador de credenciales".into(),
                )
            })
    }

    async fn store_clear(&self) -> Result<()> {
        let store = self.token_store.clone();
        tokio::task::spawn_blocking(move || store.clear())
            .await
            .map_err(|_| {
                VeloError::Other(
                    "No se pudo borrar la sesión del Administrador de credenciales".into(),
                )
            })?
            .map_err(|_| {
                VeloError::Other(
                    "No se pudo borrar la sesión del Administrador de credenciales".into(),
                )
            })
    }

    async fn token_request(&self, fields: &[(&str, &str)]) -> Result<TokenResponse> {
        // reqwest's form feature is not enabled in the workspace. Encode with
        // the already available url crate instead of adding a dependency.
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields.iter().copied())
            .finish();
        let url = format!(
            "{}/api/token",
            self.config.accounts_base.trim_end_matches('/')
        );
        let mut rate_retried = false;
        loop {
            let response = self
                .client
                .post(&url)
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(body.clone())
                .send()
                .await
                .map_err(network_error)?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                let delay = retry_after(&response);
                if !rate_retried && delay <= Duration::from_secs(3) {
                    rate_retried = true;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                self.report_quota();
                return Err(VeloError::RateLimited(delay));
            }
            if !response.status().is_success() {
                let status = response.status();
                let body: Value = response.json().await.map_err(network_error)?;
                return Err(if body["error"] == "invalid_client" {
                    let _ = self.app_unavailable.send("invalid_client");
                    VeloError::Other("invalid_client".into())
                } else if body["error"] == "invalid_grant" {
                    VeloError::NotAuthenticated
                } else {
                    VeloError::Other(format!(
                        "Spotify rechazó la solicitud OAuth (HTTP {})",
                        status.as_u16()
                    ))
                });
            }
            let token: TokenResponse = response.json().await.map_err(network_error)?;
            if token.access_token.is_empty() || token.expires_in == 0 {
                return Err(VeloError::Other("Invalid Spotify token response".into()));
            }
            return Ok(token);
        }
    }

    async fn install_token(&self, auth: &mut AuthState, token: TokenResponse) -> Result<String> {
        let expires_at = Instant::now()
            .checked_add(Duration::from_secs(token.expires_in))
            .ok_or_else(|| VeloError::Other("Invalid token expiration".into()))?;
        if let Some(refresh) = token.refresh_token.filter(|s| !s.is_empty()) {
            self.store_save(refresh).await?;
        }
        let value = token.access_token;
        auth.access = Some(AccessToken {
            value: value.clone(),
            expires_at,
        });
        Ok(value)
    }

    async fn access_token(&self, rejected: Option<&str>) -> Result<String> {
        // Hold one async lock across refresh and persistence. Concurrent 401s
        // reuse the replacement instead of rotating the refresh token twice.
        let mut auth = self.auth.lock().await;
        if let Some(access) = &auth.access {
            let fresh = access.expires_at.saturating_duration_since(Instant::now())
                > Duration::from_secs(60);
            if fresh && rejected != Some(access.value.as_str()) {
                return Ok(access.value.clone());
            }
        } else if rejected.is_some() && self.token_source.is_none() {
            return Err(VeloError::NotAuthenticated);
        }
        auth.access = None;
        if let Some(source) = &self.token_source {
            let (value, valid_for) = source.token(rejected).await?;
            if value.is_empty() || rejected == Some(value.as_str()) {
                return Err(VeloError::NotAuthenticated);
            }
            auth.access = Some(AccessToken {
                value: value.clone(),
                expires_at: Instant::now() + valid_for.min(Duration::from_secs(86_400)),
            });
            return Ok(value);
        }
        let refresh = self
            .store_load()
            .await?
            .filter(|s| !s.is_empty())
            .ok_or(VeloError::NotAuthenticated)?;
        let response = self
            .token_request(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
                ("client_id", &self.config.client_id),
            ])
            .await;
        match response {
            Ok(token) => self.install_token(&mut auth, token).await,
            Err(VeloError::NotAuthenticated) => {
                self.store_clear().await?;
                Err(VeloError::NotAuthenticated)
            }
            Err(error) => Err(error),
        }
    }

    async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
        epoch: u64,
    ) -> Result<()> {
        let mut auth = self.auth.lock().await;
        if auth.epoch != epoch {
            return Err(VeloError::NotAuthenticated);
        }
        let token = self
            .token_request(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", &self.config.client_id),
                ("code_verifier", verifier),
            ])
            .await?;
        // Require the new account's refresh token before replacing the session.
        // Saving overwrites atomically; cancellation or failure never clears it.
        if token.refresh_token.as_ref().is_none_or(|s| s.is_empty()) {
            return Err(VeloError::Other(
                "Spotify no devolvió un token para guardar la sesión".into(),
            ));
        }
        self.install_token(&mut auth, token).await?;
        Ok(())
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Response> {
        if let Some(left) = self.rate_limited() {
            return Err(VeloError::RateLimited(left));
        }
        let mut token = self.access_token(None).await?;
        let mut auth_retried = false;
        let mut rate_retried = false;
        let mut url = url::Url::parse(&format!(
            "{}{}",
            self.config.api_base.trim_end_matches('/'),
            path
        ))
        .map_err(|_| VeloError::Other("Invalid API base URL".into()))?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
        }
        loop {
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .bearer_auth(&token);
            if let Some(body) = &body {
                request = request.json(body);
            } else if method != Method::GET {
                // Spotify answers 411 Length Required to bodiless PUT/POST (play, pause,
                // next…) unless the request says explicitly that it has no body.
                request = request.header(reqwest::header::CONTENT_LENGTH, 0);
            }
            let response = request.send().await.map_err(network_error)?;
            match response.status() {
                StatusCode::UNAUTHORIZED => {
                    if auth_retried {
                        let mut auth = self.auth.lock().await;
                        if auth.access.as_ref().is_some_and(|a| a.value == token) {
                            auth.access = None;
                        }
                        return Err(VeloError::NotAuthenticated);
                    }
                    auth_retried = true;
                    token = self.access_token(Some(&token)).await?;
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    let delay = retry_after(&response);
                    if rate_retried || delay > Duration::from_secs(3) {
                        self.pause_requests(delay);
                        self.report_quota();
                        return Err(VeloError::RateLimited(delay));
                    }
                    rate_retried = true;
                    tokio::time::sleep(delay).await;
                }
                status if status.is_success() => return Ok(response),
                _ => return Err(api_error(response).await),
            }
        }
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        self.request(Method::GET, path, query, None)
            .await?
            .json()
            .await
            .map_err(network_error)
    }

    async fn command(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<()> {
        self.request(method, path, query, body).await?;
        Ok(())
    }

    async fn paged<T>(
        &self,
        path: &str,
        offset: u32,
        limit: u32,
        map: impl Fn(&Value) -> Option<T>,
    ) -> Result<Page<T>> {
        Ok(models::page(
            &self.get(path, &pagination(offset, limit)).await?,
            map,
        ))
    }

    async fn top<T>(
        &self,
        kind: &str,
        range: TimeRange,
        offset: u32,
        limit: u32,
        map: impl Fn(&Value) -> Option<T>,
    ) -> Result<Page<T>> {
        let mut query = pagination(offset, limit);
        query.push((
            "time_range",
            match range {
                TimeRange::ShortTerm => "short_term",
                TimeRange::MediumTerm => "medium_term",
                TimeRange::LongTerm => "long_term",
            }
            .into(),
        ));
        Ok(models::page(
            &self.get(&format!("/me/top/{kind}"), &query).await?,
            map,
        ))
    }
}

fn pagination(offset: u32, limit: u32) -> Vec<(&'static str, String)> {
    vec![
        ("offset", offset.to_string()),
        ("limit", limit.clamp(1, 50).to_string()),
    ]
}

/// U4: Spotify user ids are not base62 like the catalogue ids: they may contain `.`, `_` or
/// `-` (old accounts). Anything else is rejected rather than escaped into the path.
fn user_path(id: &str, suffix: &str) -> Result<String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        return Err(VeloError::Other("Invalid Spotify user id".into()));
    }
    Ok(format!("/users/{id}{suffix}"))
}

fn id_path(collection: &str, id: &str, suffix: &str) -> Result<String> {
    // IDs, unlike URIs, are opaque alphanumeric path segments. Reject path or
    // query injection rather than allowing an id to change the API operation.
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err(VeloError::Other("Invalid Spotify id".into()));
    }
    Ok(format!("/{collection}/{id}{suffix}"))
}

fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(|| VeloError::Other("Invalid Spotify response".into()))
}

fn network_error(error: reqwest::Error) -> VeloError {
    // Do not expose URLs, OAuth response bodies or credentials via errors/logs.
    if error.is_decode() {
        VeloError::Other("Invalid Spotify JSON response".into())
    } else if error.is_timeout() {
        VeloError::Network("Spotify request timed out".into())
    } else {
        VeloError::Network("Spotify request failed".into())
    }
}

fn io_error(_: std::io::Error) -> VeloError {
    VeloError::Network("OAuth loopback connection failed".into())
}

fn retry_after(response: &Response) -> Duration {
    // Missing/malformed headers must not trigger an immediate retry storm.
    Duration::from_secs(
        response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(5),
    )
}

/// Largest page `GET /artists/{id}/albums` accepts.
const ARTIST_ALBUMS_MAX: u32 = 10;

async fn api_error(response: Response) -> VeloError {
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    let reason = body["error"]["reason"].as_str().or(body["reason"].as_str());
    match status {
        StatusCode::UNAUTHORIZED => VeloError::NotAuthenticated,
        StatusCode::FORBIDDEN if reason == Some("PREMIUM_REQUIRED") => VeloError::PremiumRequired,
        StatusCode::FORBIDDEN => {
            VeloError::Forbidden("Spotify denied access to this resource".into())
        }
        StatusCode::NOT_FOUND if reason == Some("NO_ACTIVE_DEVICE") => VeloError::NoActiveDevice,
        StatusCode::NOT_FOUND => VeloError::NotFound,
        _ => match body["error"]["message"].as_str().filter(|m| !m.is_empty()) {
            Some(message) => VeloError::Other(format!(
                "Spotify returned HTTP {}: {message}",
                status.as_u16()
            )),
            None => VeloError::Other(format!("Spotify returned HTTP {}", status.as_u16())),
        },
    }
}

#[async_trait]
impl MusicProvider for SpotifyProvider {
    fn name(&self) -> &'static str {
        "Spotify"
    }

    async fn is_signed_in(&self) -> bool {
        let auth = self.auth.lock().await;
        self.token_source.is_some()
            || auth.access.is_some()
            || self
                .store_load()
                .await
                .ok()
                .flatten()
                .is_some_and(|s| !s.is_empty())
    }

    async fn sign_in(&self) -> Result<UserProfile> {
        let _login = self.login.lock().await;
        if self.token_source.is_some() {
            return Err(VeloError::Other(
                "Spotify sign-in belongs to the session token source".into(),
            ));
        }
        if self.config.client_id.trim().is_empty() {
            return Err(VeloError::Other("Spotify client id is required".into()));
        }
        tokio::time::timeout(Duration::from_secs(300), async {
            let epoch = self.auth.lock().await.epoch;
            let pending = oauth::PendingLogin::bind(
                &self.config.accounts_base,
                &self.config.client_id,
                self.config.redirect_port,
            )
            .await?;
            let url = pending.authorization_url.to_string();
            tokio::task::spawn_blocking(move || webbrowser::open(&url))
                .await
                .map_err(|_| VeloError::Other("Browser worker failed".into()))?
                .map_err(|_| VeloError::Other("Could not open the system browser".into()))?;
            let code = pending.receive_code().await?;
            self.exchange_code(&code, &pending.redirect_uri, &pending.verifier, epoch)
                .await?;
            self.me().await
        })
        .await
        .map_err(|_| VeloError::Other("Spotify sign-in timed out".into()))?
    }

    async fn sign_out(&self) -> Result<()> {
        let mut auth = self.auth.lock().await;
        auth.epoch = auth.epoch.wrapping_add(1);
        auth.access = None;
        self.store_clear().await
    }

    async fn me(&self) -> Result<UserProfile> {
        required(models::user(&self.get("/me", &[]).await?))
    }

    async fn playback(&self) -> Result<Option<PlaybackState>> {
        let response = self.request(Method::GET, "/me/player", &[], None).await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let value = response.json().await.map_err(network_error)?;
        Ok(Some(models::playback(&value)))
    }

    async fn devices(&self) -> Result<Vec<Device>> {
        Ok(models::list(
            &self.get("/me/player/devices", &[]).await?["devices"],
            models::device,
        ))
    }
    async fn transfer(&self, device_id: &str, play: bool) -> Result<()> {
        self.command(
            Method::PUT,
            "/me/player",
            &[],
            Some(json!({"device_ids":[device_id],"play":play})),
        )
        .await
    }
    async fn play(&self) -> Result<()> {
        self.command(Method::PUT, "/me/player/play", &[], None)
            .await
    }
    async fn pause(&self) -> Result<()> {
        self.command(Method::PUT, "/me/player/pause", &[], None)
            .await
    }
    async fn next(&self) -> Result<()> {
        self.command(Method::POST, "/me/player/next", &[], None)
            .await
    }
    async fn previous(&self) -> Result<()> {
        self.command(Method::POST, "/me/player/previous", &[], None)
            .await
    }
    async fn seek(&self, position_ms: u64) -> Result<()> {
        self.command(
            Method::PUT,
            "/me/player/seek",
            &[("position_ms", position_ms.to_string())],
            None,
        )
        .await
    }
    async fn set_volume(&self, percent: u8) -> Result<()> {
        self.command(
            Method::PUT,
            "/me/player/volume",
            &[("volume_percent", percent.min(100).to_string())],
            None,
        )
        .await
    }
    async fn set_shuffle(&self, on: bool) -> Result<()> {
        self.command(
            Method::PUT,
            "/me/player/shuffle",
            &[("state", on.to_string())],
            None,
        )
        .await
    }
    async fn set_repeat(&self, mode: RepeatMode) -> Result<()> {
        let state = match mode {
            RepeatMode::Off => "off",
            RepeatMode::Context => "context",
            RepeatMode::Track => "track",
        };
        self.command(
            Method::PUT,
            "/me/player/repeat",
            &[("state", state.into())],
            None,
        )
        .await
    }

    async fn play_context(&self, context_uri: &str, offset: Option<u32>) -> Result<()> {
        let mut body = json!({"context_uri":context_uri});
        if let Some(position) = offset {
            body["offset"] = json!({"position":position});
        }
        self.command(Method::PUT, "/me/player/play", &[], Some(body))
            .await
    }

    async fn play_uris(&self, uris: &[String], offset: Option<u32>) -> Result<()> {
        let mut body = json!({"uris":uris});
        if let Some(position) = offset {
            body["offset"] = json!({"position":position});
        }
        self.command(Method::PUT, "/me/player/play", &[], Some(body))
            .await
    }

    async fn queue(&self) -> Result<Vec<Track>> {
        Ok(self.queue_snapshot().await?.tracks)
    }
    async fn add_to_queue(&self, uri: &str) -> Result<()> {
        self.command(
            Method::POST,
            "/me/player/queue",
            &[("uri", uri.into())],
            None,
        )
        .await
    }

    async fn my_playlists(&self, offset: u32, limit: u32) -> Result<Page<Playlist>> {
        self.paged("/me/playlists", offset, limit, models::playlist)
            .await
    }
    async fn playlist(&self, id: &str) -> Result<Playlist> {
        required(models::playlist(
            &self.get(&id_path("playlists", id, "")?, &[]).await?,
        ))
    }
    async fn playlist_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>> {
        self.paged(&id_path("playlists", id, "/items")?, offset, limit, |v| {
            models::dated_track(v, "item")
        })
        .await
    }
    async fn saved_tracks(&self, offset: u32, limit: u32) -> Result<Page<Track>> {
        self.paged("/me/tracks", offset, limit, |v| {
            models::dated_track(v, "track")
        })
        .await
    }
    async fn saved_albums(&self, offset: u32, limit: u32) -> Result<Page<Album>> {
        self.paged("/me/albums", offset, limit, |v| models::album(&v["album"]))
            .await
    }
    async fn followed_artists(&self, limit: u32) -> Result<Vec<Artist>> {
        let mut artists = Vec::new();
        let mut after: Option<String> = None;
        while artists.len() < limit as usize {
            let mut query = vec![
                ("type", "artist".into()),
                (
                    "limit",
                    (limit as usize - artists.len()).min(50).to_string(),
                ),
            ];
            if let Some(cursor) = &after {
                query.push(("after", cursor.clone()));
            }
            let response = self.get("/me/following", &query).await?;
            let page = &response["artists"];
            artists.extend(models::list(&page["items"], models::artist));
            let cursor = page["cursors"]["after"].as_str().map(str::to_owned);
            // Follow cursors, never the remote `next` URL: all requests keep
            // the configured API origin and its authenticated client.
            if page["next"].as_str().is_none() || cursor.is_none() || cursor == after {
                break;
            }
            after = cursor;
        }
        artists.truncate(limit as usize);
        Ok(artists)
    }

    async fn is_saved(&self, uri: &str) -> Result<bool> {
        required(
            self.get("/me/library/contains", &[("uris", uri.into())])
                .await?[0]
                .as_bool(),
        )
    }

    async fn set_saved(&self, uri: &str, saved: bool) -> Result<()> {
        self.command(
            if saved { Method::PUT } else { Method::DELETE },
            "/me/library",
            &[("uris", uri.into())],
            None,
        )
        .await
    }

    async fn recently_played(&self, limit: u32) -> Result<Vec<Track>> {
        Ok(models::list(
            &self
                .get(
                    "/me/player/recently-played",
                    &[("limit", limit.clamp(1, 50).to_string())],
                )
                .await?["items"],
            |v| models::track(&v["track"]),
        ))
    }

    async fn top_tracks(&self, range: TimeRange, offset: u32, limit: u32) -> Result<Page<Track>> {
        self.top("tracks", range, offset, limit, models::track)
            .await
    }
    async fn top_artists(&self, range: TimeRange, offset: u32, limit: u32) -> Result<Page<Artist>> {
        self.top("artists", range, offset, limit, models::artist)
            .await
    }
    async fn album(&self, id: &str) -> Result<Album> {
        required(models::album(
            &self.get(&id_path("albums", id, "")?, &[]).await?,
        ))
    }
    async fn album_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>> {
        let album = self.album(id).await?;
        let mut page = self
            .paged(
                &id_path("albums", id, "/tracks")?,
                offset,
                limit,
                models::track,
            )
            .await?;
        for track in &mut page.items {
            track.album.clone_from(&album.name);
            track.album_id = Some(album.id.clone());
            track.images.clone_from(&album.images);
        }
        Ok(page)
    }
    async fn artist(&self, id: &str) -> Result<Artist> {
        required(models::artist(
            &self.get(&id_path("artists", id, "")?, &[]).await?,
        ))
    }
    async fn artist_albums(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Album>> {
        // Since 2026 this endpoint answers 400 "Invalid limit" above 10 (checked live).
        let mut query = pagination(offset, limit.min(ARTIST_ALBUMS_MAX));
        query.push(("include_groups", "album,single".into()));
        Ok(models::page(
            &self
                .get(&id_path("artists", id, "/albums")?, &query)
                .await?,
            models::album,
        ))
    }

    async fn create_playlist(
        &self,
        name: &str,
        description: &str,
        public: bool,
    ) -> Result<Playlist> {
        let value = self
            .request(
                Method::POST,
                "/me/playlists",
                &[],
                Some(json!({"name":name,"description":description,"public":public})),
            )
            .await?
            .json()
            .await
            .map_err(network_error)?;
        required(models::playlist(&value))
    }
    async fn update_playlist_details(
        &self,
        playlist_id: &str,
        name: &str,
        description: &str,
    ) -> Result<()> {
        // https://developer.spotify.com/documentation/web-api/reference/change-playlist-details
        let path = id_path("playlists", playlist_id, "")?;
        self.command(
            Method::PUT,
            &path,
            &[],
            Some(json!({"name":name,"description":description})),
        )
        .await
    }
    async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        let path = id_path("playlists", playlist_id, "/items")?;
        for batch in uris.chunks(100) {
            self.command(Method::POST, &path, &[], Some(json!({"uris":batch})))
                .await?;
        }
        Ok(())
    }
    async fn remove_from_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        let path = id_path("playlists", playlist_id, "/items")?;
        for batch in uris.chunks(100) {
            // The 2026 /items endpoint accepts `items: [{uri}]`, replacing the
            // deprecated /tracks body's `tracks` key. URI-only removal removes
            // every occurrence; the domain contract has no position/snapshot.
            // https://developer.spotify.com/documentation/web-api/reference/remove-items-playlist
            let items: Vec<_> = batch.iter().map(|uri| json!({"uri":uri})).collect();
            self.command(Method::DELETE, &path, &[], Some(json!({"items":items})))
                .await?;
        }
        Ok(())
    }
    async fn user_profile(&self, user_id: &str) -> Result<PublicProfile> {
        let path = if user_id == "me" {
            "/me".to_string()
        } else {
            user_path(user_id, "")?
        };
        required(models::public_profile(&self.get(&path, &[]).await?))
    }

    async fn user_public_playlists(
        &self,
        user_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<Playlist>> {
        let path = user_path(user_id, "/playlists")?;
        self.paged(&path, offset, limit, |v| {
            models::profile_playlist(v, user_id)
        })
        .await
    }

    async fn search(&self, query: &str, offset: u32, limit: u32) -> Result<SearchResults> {
        let limit = limit.clamp(1, 10);
        if offset.saturating_add(limit) > 1000 {
            fn empty<T>(offset: u32, limit: u32) -> Page<T> {
                Page {
                    items: Vec::new(),
                    offset,
                    limit,
                    total: 0,
                }
            }
            return Ok(SearchResults {
                tracks: empty(offset, limit),
                playlists: empty(offset, limit),
                albums: empty(offset, limit),
                artists: empty(offset, limit),
            });
        }
        let mut params = pagination(offset, limit);
        params.extend([
            ("q", query.into()),
            ("type", "track,album,artist,playlist".into()),
        ]);
        let value = self.get("/search", &params).await?;
        Ok(SearchResults {
            tracks: models::page(&value["tracks"], models::track),
            playlists: models::page(&value["playlists"], models::playlist),
            albums: models::page(&value["albums"], models::album),
            artists: models::page(&value["artists"], models::artist),
        })
    }
}

#[cfg(test)]
mod tests;
