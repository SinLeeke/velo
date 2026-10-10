//! Credential-free local CONNECT facade for librespot's AP, dealer and HTTP client.
//! Authentication lives only in memory and the caller's keyring, never in URLs/errors.
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use velo_domain::{ProxySettings, Result, VeloError};

// System is intentionally absent: Windows static proxy + PAC are not implemented.
pub const SUPPORTED_MODES: &[&str] = &["off", "http", "socks5"];

struct Secret(String);
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}
struct Upstream {
    settings: ProxySettings,
    password: Secret,
}

pub struct ProxyAdapter {
    url: Option<String>,
    task: Option<JoinHandle<()>>,
}
impl fmt::Debug for ProxyAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyAdapter")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}
impl ProxyAdapter {
    pub async fn start(settings: &ProxySettings, password: Option<String>) -> Result<Self> {
        settings.validate().map_err(failure)?;
        if !SUPPORTED_MODES.contains(&settings.mode.as_str()) {
            return Err(failure("Modo de proxy no disponible"));
        }
        if settings.mode == "off" {
            return Ok(Self {
                url: None,
                task: None,
            });
        }
        if settings.user.len() > 255 || password.as_ref().is_some_and(|p| p.len() > 255) {
            return Err(failure("Credenciales de proxy demasiado largas"));
        }
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| failure("No se pudo abrir el adaptador de proxy"))?;
        let port = listener
            .local_addr()
            .map_err(|_| failure("No se pudo abrir el adaptador de proxy"))?
            .port();
        let upstream = Arc::new(Upstream {
            settings: settings.clone(),
            password: Secret(password.unwrap_or_default()),
        });
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        if connections.len() >= 64 { continue; }
                        let upstream = upstream.clone();
                        connections.spawn(async move { let _ = serve(stream, &upstream).await; });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
            // Dropping JoinSet cancels every tunnel. No timers while idle.
        });
        Ok(Self {
            url: Some(format!("http://127.0.0.1:{port}")),
            task: Some(task),
        })
    }
    pub fn url(&self) -> Option<String> {
        self.url.clone()
    }
}
impl Drop for ProxyAdapter {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
fn failure(message: &'static str) -> VeloError {
    VeloError::Other(message.into())
}
fn io_failure() -> std::io::Error {
    std::io::Error::other("Proxy handshake failed")
}

async fn header(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(1024);
    loop {
        if bytes.len() >= 16 * 1024 {
            return Err(io_failure());
        }
        bytes.push(stream.read_u8().await?);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
}
fn target(authority: &str) -> std::io::Result<(String, u16)> {
    let (host, port) = authority.rsplit_once(':').ok_or_else(io_failure)?;
    let host = host.trim_matches(['[', ']']);
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(io_failure)?;
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '?' | '#' | '\\'))
    {
        return Err(io_failure());
    }
    Ok((host.into(), port))
}
async fn serve(mut downstream: TcpStream, upstream: &Upstream) -> std::io::Result<()> {
    let setup = tokio::time::timeout(Duration::from_secs(15), async {
        let bytes = header(&mut downstream).await?;
        let text = std::str::from_utf8(&bytes).map_err(|_| io_failure())?;
        let first = text.lines().next().ok_or_else(io_failure)?;
        let parts: Vec<_> = first.split_whitespace().collect();
        if parts.len() != 3 || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
            return Err(io_failure());
        }
        let connect = parts[0] == "CONNECT";
        let (host, port, origin) = if connect {
            let (host, port) = target(parts[1])?;
            (host, port, String::new())
        } else {
            let url = url::Url::parse(parts[1]).map_err(|_| io_failure())?;
            if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
                return Err(io_failure());
            }
            let host = url.host_str().ok_or_else(io_failure)?.to_owned();
            let mut origin = url.path().to_owned();
            if let Some(q) = url.query() {
                origin.push('?');
                origin.push_str(q);
            }
            (
                host,
                url.port_or_known_default().ok_or_else(io_failure)?,
                origin,
            )
        };
        let mut remote = TcpStream::connect((
            upstream.settings.host.trim_matches(['[', ']']),
            upstream.settings.port as u16,
        ))
        .await?;
        let authority = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        if upstream.settings.mode == "socks5" {
            socks(&mut remote, upstream, &host, port).await?;
        } else if connect {
            let auth = authorization(upstream);
            remote
                .write_all(
                    format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{auth}\r\n")
                        .as_bytes(),
                )
                .await?;
            let response = header(&mut remote).await?;
            let status = std::str::from_utf8(&response)
                .map_err(|_| io_failure())?
                .lines()
                .next()
                .and_then(|s| s.split_whitespace().nth(1));
            if status != Some("200") {
                return Err(io_failure());
            }
        }
        if connect {
            downstream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await?;
        } else {
            let destination = if upstream.settings.mode == "socks5" {
                origin.as_str()
            } else {
                parts[1]
            };
            let mut request = format!("{} {destination} {}\r\n", parts[0], parts[2]);
            for line in text.split("\r\n").skip(1).filter(|l| !l.is_empty()) {
                let name = line.split_once(':').ok_or_else(io_failure)?.0;
                if !name.eq_ignore_ascii_case("proxy-authorization")
                    && !name.eq_ignore_ascii_case("proxy-connection")
                {
                    request.push_str(line);
                    request.push_str("\r\n");
                }
            }
            if upstream.settings.mode == "http" {
                request.push_str(&authorization(upstream));
            }
            request.push_str("\r\n");
            remote.write_all(request.as_bytes()).await?;
        }
        Ok::<_, std::io::Error>(remote)
    })
    .await;
    let mut remote = match setup {
        Ok(Ok(remote)) => remote,
        _ => {
            let _ = downstream
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return Err(io_failure());
        }
    };
    tokio::io::copy_bidirectional(&mut downstream, &mut remote).await?;
    Ok(())
}
fn authorization(upstream: &Upstream) -> String {
    if upstream.settings.user.is_empty() {
        return String::new();
    }
    let data = format!("{}:{}", upstream.settings.user, upstream.password.0);
    format!("Proxy-Authorization: Basic {}\r\n", base64(data.as_bytes()))
}
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        out.push(DIGITS[(a >> 2) as usize] as char);
        out.push(DIGITS[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            DIGITS[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            DIGITS[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
async fn socks(
    remote: &mut TcpStream,
    upstream: &Upstream,
    host: &str,
    port: u16,
) -> std::io::Result<()> {
    let authenticated = !upstream.settings.user.is_empty();
    remote
        .write_all(if authenticated {
            &[5, 1, 2]
        } else {
            &[5, 1, 0]
        })
        .await?;
    let mut response = [0; 2];
    remote.read_exact(&mut response).await?;
    if response != [5, if authenticated { 2 } else { 0 }] {
        return Err(io_failure());
    }
    if authenticated {
        let user = upstream.settings.user.as_bytes();
        let password = upstream.password.0.as_bytes();
        let mut request = vec![1, user.len() as u8];
        request.extend_from_slice(user);
        request.push(password.len() as u8);
        request.extend_from_slice(password);
        remote.write_all(&request).await?;
        remote.read_exact(&mut response).await?;
        if response != [1, 0] {
            return Err(io_failure());
        }
    }
    // Domain name addressing delegates DNS to the proxy.
    if host.len() > 255 {
        return Err(io_failure());
    }
    let mut request = vec![5, 1, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Ok(std::net::IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            request.extend_from_slice(&[3, host.len() as u8]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    remote.write_all(&request).await?;
    let mut prefix = [0; 4];
    remote.read_exact(&mut prefix).await?;
    if prefix[0..3] != [5, 0, 0] {
        return Err(io_failure());
    }
    let length = match prefix[3] {
        1 => 4,
        4 => 16,
        3 => remote.read_u8().await? as usize,
        _ => return Err(io_failure()),
    };
    let mut rest = vec![0; length + 2];
    remote.read_exact(&mut rest).await?;
    Ok(())
}

/// All affected reqwest clients share route changes and disable implicit env proxies.
pub(crate) fn client(timeout: Duration) -> Result<reqwest::Client> {
    let route = velo_domain::proxy_route().borrow().clone();
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none());
    if let Some(url) = route.url {
        builder = builder.proxy(reqwest::Proxy::all(url).map_err(|_| failure("Proxy inválido"))?);
    }
    builder
        .build()
        .map_err(|_| failure("No se pudo configurar la conexión"))
}

pub(crate) struct HttpClient {
    timeout: Duration,
    cached: std::sync::Mutex<(u64, reqwest::Client)>,
}
impl HttpClient {
    pub fn new(timeout: Duration) -> Result<Self> {
        Ok(Self {
            timeout,
            cached: std::sync::Mutex::new((
                velo_domain::proxy_route().borrow().generation,
                client(timeout)?,
            )),
        })
    }
    pub fn get(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        let generation = velo_domain::proxy_route().borrow().generation;
        let mut cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        if cached.0 != generation {
            // The published route was already validated/built by applyProxy.
            if let Ok(next) = client(self.timeout) {
                *cached = (generation, next);
            }
        }
        cached.1.get(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_redacted_and_modes_are_explicit() {
        assert_eq!(
            format!("{:?}", Secret("fixture-secret".into())),
            "[redacted]"
        );
        assert!(!SUPPORTED_MODES.contains(&"system"));
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
    }
    #[tokio::test]
    async fn http_authentication_is_only_an_upstream_header() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let settings = ProxySettings {
            mode: "http".into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port() as u32,
            user: "fixture-user".into(),
        };
        let adapter = ProxyAdapter::start(&settings, Some("fixture-secret".into()))
            .await
            .unwrap();
        let url = adapter.url().unwrap();
        assert!(!url.contains("fixture"));
        assert!(!format!("{adapter:?}").contains("fixture"));
        let upstream = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let text = String::from_utf8(header(&mut stream).await.unwrap()).unwrap();
            assert!(text.starts_with("CONNECT example.test:443"));
            assert!(text.contains(&format!(
                "Proxy-Authorization: Basic {}",
                base64(b"fixture-user:fixture-secret")
            )));
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        });
        let url = url::Url::parse(&url).unwrap();
        let mut local = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
            .await
            .unwrap();
        local
            .write_all(b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n")
            .await
            .unwrap();
        assert!(
            header(&mut local)
                .await
                .unwrap()
                .starts_with(b"HTTP/1.1 200")
        );
        local.write_all(b"ping").await.unwrap();
        let mut echo = [0; 4];
        local.read_exact(&mut echo).await.unwrap();
        assert_eq!(&echo, b"ping");
        upstream.await.unwrap();
    }
    #[tokio::test]
    async fn socks5_authentication_and_remote_dns_cover_connect_tunnels() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let settings = ProxySettings {
            mode: "socks5".into(),
            host: "127.0.0.1".into(),
            port: u32::from(listener.local_addr().unwrap().port()),
            user: "u".into(),
        };
        let adapter = ProxyAdapter::start(&settings, Some("p".into()))
            .await
            .unwrap();
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut hello = [0; 3];
            socket.read_exact(&mut hello).await.unwrap();
            assert_eq!(hello, [5, 1, 2]);
            socket.write_all(&[5, 2]).await.unwrap();
            let mut auth = [0; 5];
            socket.read_exact(&mut auth).await.unwrap();
            assert_eq!(auth, [1, 1, b'u', 1, b'p']);
            socket.write_all(&[1, 0]).await.unwrap();
            let mut prefix = [0; 5];
            socket.read_exact(&mut prefix).await.unwrap();
            assert_eq!(prefix, [5, 1, 0, 3, 12]);
            let mut target = [0; 14];
            socket.read_exact(&mut target).await.unwrap();
            assert_eq!(&target[..12], b"example.test");
            assert_eq!(&target[12..], &443u16.to_be_bytes());
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 1])
                .await
                .unwrap();
            let mut echo = [0; 4];
            socket.read_exact(&mut echo).await.unwrap();
            socket.write_all(&echo).await.unwrap();
        });
        let url = url::Url::parse(&adapter.url().unwrap()).unwrap();
        let mut local = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
            .await
            .unwrap();
        local
            .write_all(b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n")
            .await
            .unwrap();
        assert!(
            header(&mut local)
                .await
                .unwrap()
                .starts_with(b"HTTP/1.1 200")
        );
        local.write_all(b"ping").await.unwrap();
        let mut echo = [0; 4];
        local.read_exact(&mut echo).await.unwrap();
        assert_eq!(&echo, b"ping");
        upstream.await.unwrap();
    }
    #[tokio::test]
    async fn plain_http_requests_use_the_same_adapter_without_secret_urls() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let settings = ProxySettings {
            mode: "http".into(),
            host: "127.0.0.1".into(),
            port: u32::from(listener.local_addr().unwrap().port()),
            user: "u".into(),
        };
        let adapter = ProxyAdapter::start(&settings, Some("p".into()))
            .await
            .unwrap();
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(header(&mut socket).await.unwrap()).unwrap();
            assert!(request.starts_with("GET http://example.test/image HTTP/1.1"));
            assert!(request.contains("Proxy-Authorization: Basic dTpw"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(adapter.url().unwrap()).unwrap())
            .build()
            .unwrap();
        assert_eq!(
            client
                .get("http://example.test/image")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        upstream.await.unwrap();
    }
}
