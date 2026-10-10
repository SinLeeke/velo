//! HTTP loopback con respuestas liberadas por el test: ninguna espera de reloj real.
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use velo_spotify::{MemoryTokenStore, SpotifyConfig, SpotifyProvider, TokenStore};

pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Value,
    reply: oneshot::Sender<(u16, Value)>,
}

impl Request {
    pub fn respond(self, status: u16, body: Value) {
        self.reply.send((status, body)).unwrap();
    }
}

pub struct Server {
    pub uri: String,
    pub requests: mpsc::UnboundedReceiver<Request>,
    task: JoinHandle<()>,
}

impl Server {
    pub async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let (tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let (end, size) = loop {
                        let mut chunk = [0; 4096];
                        let n = stream.read(&mut chunk).await.unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&chunk[..n]);
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            let header = String::from_utf8_lossy(&bytes[..end]);
                            let size = header
                                .lines()
                                .find_map(|l| {
                                    let (key, value) = l.split_once(':')?;
                                    key.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            break (end + 4, size);
                        }
                    };
                    while bytes.len() < end + size {
                        let mut chunk = [0; 4096];
                        let n = stream.read(&mut chunk).await.unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&chunk[..n]);
                    }
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let mut first = header.lines().next().unwrap().split_whitespace();
                    let method = first.next().unwrap().to_string();
                    let path = first.next().unwrap().to_string();
                    let (status, body) = if path == "/api/token" {
                        (200, json!({"access_token":"fixture", "expires_in":3600}))
                    } else {
                        let (reply, response) = oneshot::channel();
                        tx.send(Request {
                            method,
                            path,
                            body: serde_json::from_slice(&bytes[end..end + size])
                                .unwrap_or(Value::Null),
                            reply,
                        })
                        .unwrap();
                        let Ok(response) = response.await else { return };
                        response
                    };
                    let body = if status == 204 {
                        String::new()
                    } else {
                        body.to_string()
                    };
                    let response = format!(
                        "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        Self {
            uri,
            requests,
            task,
        }
    }

    pub fn provider(&self) -> Arc<SpotifyProvider> {
        let tokens = Arc::new(MemoryTokenStore::new());
        tokens.save("fixture-refresh").unwrap();
        Arc::new(SpotifyProvider::new(
            SpotifyConfig {
                client_id: "fixture".into(),
                api_base: self.uri.clone(),
                accounts_base: self.uri.clone(),
                redirect_port: 0,
            },
            tokens,
        ))
    }

    pub async fn next(&mut self) -> Request {
        self.requests.recv().await.unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Evita el avance automático de Tokio mientras el mock espera IO; solo avanza el test.
pub struct ManualTime(JoinHandle<()>);
impl ManualTime {
    pub fn new() -> Self {
        Self(tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        }))
    }
}
impl Drop for ManualTime {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
}

pub async fn settle() {
    // Drena las tareas que ya pueden avanzar; no representa una espera en milisegundos.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}
