//! Authorization Code + PKCE helpers. No browser is needed to test this module.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{Rng, distr::Alphanumeric};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;
use velo_domain::{Result, VeloError};

// Adaptado de Spotifast (reproductor de Spotify de Carmine Paolino, MIT, © 2026):
// app pública usada también por spotify-player/ncspot. Spotify puede limitar su cuota
// compartida o retirarla; Velo ofrece una app personal como respaldo.
pub const DEFAULT_WEB_CLIENT_ID: &str = "d420a117a32841c2b3474932e49fb54b";
pub const WEB_REDIRECT_PORT: u16 = 8989;
pub const REDIRECT_PATH: &str = "/login";
pub const WEB_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";

pub fn valid_client_id(id: &str) -> bool {
    let id = id.trim();
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub const SCOPES: &str = "streaming user-read-playback-state user-modify-playback-state user-read-currently-playing user-read-private playlist-read-private playlist-read-collaborative playlist-modify-private playlist-modify-public user-library-read user-library-modify user-read-recently-played user-top-read user-follow-read";

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_string() -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

/// Parse only the callback request line; reject duplicate parameters and state
/// mismatches before accepting either an authorization code or a denial.
pub fn parse_callback(request: &str, expected_state: &str) -> Result<String> {
    let invalid = || VeloError::Other("Invalid OAuth callback".into());
    let mut parts = request
        .lines()
        .next()
        .ok_or_else(invalid)?
        .split_whitespace();
    if parts.next() != Some("GET") {
        return Err(invalid());
    }
    let target = parts.next().ok_or_else(invalid)?;
    if !target.starts_with("/login?")
        || !matches!(parts.next(), Some("HTTP/1.1" | "HTTP/1.0"))
        || parts.next().is_some()
    {
        return Err(invalid());
    }
    let url = Url::parse(&format!("http://127.0.0.1{target}")).map_err(|_| invalid())?;
    if url.path() != REDIRECT_PATH || url.fragment().is_some() {
        return Err(invalid());
    }
    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "state" => &mut state,
            "code" => &mut code,
            "error" => &mut error,
            _ => continue,
        };
        if slot.replace(value.into_owned()).is_some() {
            return Err(invalid());
        }
    }
    if expected_state.is_empty() || state.as_deref() != Some(expected_state) {
        return Err(invalid());
    }
    if error.is_some() {
        return Err(VeloError::NotAuthenticated);
    }
    code.filter(|value| !value.is_empty()).ok_or_else(invalid)
}

pub struct PendingLogin {
    listener: TcpListener,
    pub redirect_uri: String,
    pub verifier: String,
    state: String,
    pub authorization_url: Url,
}

impl PendingLogin {
    pub async fn bind(accounts_base: &str, client_id: &str, redirect_port: u16) -> Result<Self> {
        Self::bind_with_scopes(accounts_base, client_id, redirect_port, SCOPES).await
    }

    pub async fn bind_with_scopes(
        accounts_base: &str,
        client_id: &str,
        redirect_port: u16,
        scopes: &str,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", redirect_port))
            .await
            .map_err(|error| {
                if redirect_port != 0 && error.kind() == std::io::ErrorKind::AddrInUse {
                    VeloError::Other(format!(
                        "El puerto {redirect_port} está en uso. Cierra la otra app que esté iniciando sesión con Spotify y reintenta."
                    ))
                } else {
                    super::io_error(error)
                }
            })?;
        let port = listener.local_addr().map_err(super::io_error)?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}{REDIRECT_PATH}");
        let verifier = random_string();
        let state = random_string();
        let mut authorization_url = Url::parse(&format!(
            "{}/authorize",
            accounts_base.trim_end_matches('/')
        ))
        .map_err(|_| VeloError::Other("Invalid accounts base URL".into()))?;
        authorization_url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", &redirect_uri),
            ("code_challenge_method", "S256"),
            ("code_challenge", &pkce_challenge(&verifier)),
            ("state", &state),
            ("scope", scopes),
        ]);
        Ok(Self {
            listener,
            redirect_uri,
            verifier,
            state,
            authorization_url,
        })
    }

    pub async fn receive_code(&self) -> Result<String> {
        loop {
            let (mut socket, _) = self.listener.accept().await.map_err(super::io_error)?;
            // Bound each connection so a partial request cannot monopolize the
            // five-minute login window. Never echo codes, state or query text.
            let request = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let mut bytes = Vec::new();
                let mut chunk = [0; 1024];
                loop {
                    let n = socket.read(&mut chunk).await?;
                    if n == 0 || bytes.len() + n > 16 * 1024 {
                        return Ok::<_, std::io::Error>(None);
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                        return Ok(String::from_utf8(bytes).ok());
                    }
                }
            })
            .await;
            let Ok(Ok(Some(request))) = request else {
                continue;
            };
            let result = parse_callback(&request, &self.state);
            let accepted = result.is_ok() || matches!(result, Err(VeloError::NotAuthenticated));
            let status = if accepted {
                "200 OK"
            } else {
                "400 Bad Request"
            };
            let body = if accepted {
                "<!doctype html><meta charset=\"utf-8\"><title>Velo</title><p>Velo: ya puedes cerrar esta pestaña / You can close this tab</p>"
            } else {
                "<!doctype html><title>Velo</title><p>Invalid callback</p>"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                socket.write_all(response.as_bytes()).await?;
                socket.shutdown().await
            })
            .await;
            if accepted || request.starts_with("GET /login?") {
                return result;
            }
        }
    }
}

/// Token exchange for the audio login; deliberately has no Debug implementation.
#[derive(serde::Deserialize)]
pub struct AudioGrant {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
}
pub async fn audio_token(fields: &[(&str, &str)]) -> Result<AudioGrant> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let response = crate::ProxyHttpClient::new()
        .post("https://accounts.spotify.com/api/token")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|_| VeloError::Network("No se pudo conectar con Spotify".into()))?;
    if !response.status().is_success() {
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(if body["error"] == "invalid_grant" {
            VeloError::NotAuthenticated
        } else if status.is_server_error() {
            VeloError::Network("Spotify no respondió".into())
        } else {
            VeloError::Other("Spotify rechazó la autorización".into())
        });
    }
    response
        .json()
        .await
        .map_err(|_| VeloError::Other("Respuesta de autorización inválida".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_include_streaming_once() {
        assert_eq!(
            SCOPES
                .split_whitespace()
                .filter(|s| *s == "streaming")
                .count(),
            1
        );
    }

    #[test]
    fn pkce_matches_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn callback_decodes_code_and_rejects_forgery() {
        assert_eq!(
            parse_callback(
                "GET /login?code=a%2Bb&state=expected HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
                "expected"
            )
            .unwrap(),
            "a+b"
        );
        for request in [
            "GET /login?code=a&state=wrong HTTP/1.1",
            "GET /login?code=a&state=expected&state=expected HTTP/1.1",
            "GET /login?code=a&code=b&state=expected HTTP/1.1",
            "POST /login?code=a&state=expected HTTP/1.1",
            "GET /other?code=a&state=expected HTTP/1.1",
            "GET /login?code=&state=expected HTTP/1.1",
        ] {
            assert!(parse_callback(request, "expected").is_err());
        }
        assert!(matches!(
            parse_callback(
                "GET /login?error=access_denied&state=expected HTTP/1.1",
                "expected"
            ),
            Err(VeloError::NotAuthenticated)
        ));
    }

    #[tokio::test]
    async fn loopback_uses_dynamic_ip_redirect_and_serves_html_without_browser() {
        let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let login = PendingLogin::bind("https://accounts.spotify.com", "client", port)
            .await
            .unwrap();
        assert_eq!(login.verifier.len(), 64);
        assert!(login.verifier.bytes().all(|c| c.is_ascii_alphanumeric()));
        let redirect = Url::parse(&login.redirect_uri).unwrap();
        assert_eq!(redirect.host_str(), Some("127.0.0.1"));
        assert_eq!(redirect.as_str(), format!("http://127.0.0.1:{port}/login"));
        let query: std::collections::HashMap<_, _> =
            login.authorization_url.query_pairs().into_owned().collect();
        assert_eq!(query["code_challenge"], pkce_challenge(&login.verifier));
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["scope"], SCOPES);
        let callback = format!("{}?code=example&state={}", login.redirect_uri, login.state);
        let (code, response) = tokio::join!(login.receive_code(), reqwest::get(callback));
        assert_eq!(code.unwrap(), "example");
        assert!(
            response
                .unwrap()
                .text()
                .await
                .unwrap()
                .contains("You can close this tab")
        );
    }

    #[tokio::test]
    async fn zero_redirect_port_uses_dynamic_port() {
        let login = PendingLogin::bind("https://accounts.spotify.com", "client", 0)
            .await
            .unwrap();
        let redirect = Url::parse(&login.redirect_uri).unwrap();
        assert_eq!(redirect.host_str(), Some("127.0.0.1"));
        assert!(redirect.port().unwrap() > 0);
    }
}
