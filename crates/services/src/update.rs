//! GitHub release updates. Call only from an async worker, never from the UI thread.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{io::AsyncWriteExt, sync::watch};

pub const API_BASE: &str = "https://api.github.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub release_url: String,
    pub setup_url: String,
    pub setup_size: u64,
    pub sums_url: Option<String>,
}
impl UpdateInfo {
    pub fn setup_name(&self) -> String {
        format!("Velo-Setup-{}.exe", self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("No se pudo conectar. Inténtalo de nuevo.")]
    Network,
    #[error("La publicación no es válida.")]
    InvalidRelease,
    #[error("La publicación no incluye el instalador de Velo.")]
    MissingSetup,
    #[error("Falta la suma SHA-256 del instalador. No se puede instalar.")]
    MissingChecksum,
    #[error("La descarga está incompleta. Inténtalo de nuevo.")]
    SizeMismatch,
    #[error("La suma SHA-256 no coincide. No se puede instalar.")]
    ChecksumMismatch,
    #[error("No se pudo guardar el instalador. Revisa el espacio disponible.")]
    Disk,
    #[error("Descarga cancelada.")]
    Cancelled,
}
type Result<T> = std::result::Result<T, UpdateError>;

fn version(value: &str) -> Option<[u64; 3]> {
    let parts: Vec<_> = value
        .strip_prefix('v')
        .unwrap_or(value)
        .split('.')
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let mut result = [0; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        result[i] = part.parse().ok()?;
    }
    Some(result)
}
fn client(current: &str, timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("Velo/{current}"))
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(20))
        .timeout(timeout)
        .build()
        .map_err(|_| UpdateError::Network)
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// `api_base` is configurable for local HTTP mock servers. Empty means GitHub.
pub async fn check(api_base: &str, current: &str) -> Result<Option<UpdateInfo>> {
    let current_version = version(current).ok_or(UpdateError::InvalidRelease)?;
    let base = if api_base.is_empty() {
        API_BASE
    } else {
        api_base
    };
    let response = client(current, Duration::from_secs(8))?
        .get(format!(
            "{}/repos/SinLeeke/velo/releases/latest",
            base.trim_end_matches('/')
        ))
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|_| UpdateError::Network)?;
    let bytes = bounded_body(response, 1024 * 1024).await?;
    let release: Release =
        serde_json::from_slice(&bytes).map_err(|_| UpdateError::InvalidRelease)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let latest = version(&release.tag_name).ok_or(UpdateError::InvalidRelease)?;
    if latest <= current_version {
        return Ok(None);
    }
    let version = release.tag_name.trim_start_matches('v').to_owned();
    let setup_name = format!("Velo-Setup-{version}.exe");
    let setup = release
        .assets
        .iter()
        .find(|a| a.name == setup_name)
        .ok_or(UpdateError::MissingSetup)?;
    if setup.size == 0 {
        return Err(UpdateError::InvalidRelease);
    }
    Ok(Some(UpdateInfo {
        version,
        release_url: release.html_url,
        setup_url: setup.browser_download_url.clone(),
        setup_size: setup.size,
        sums_url: release
            .assets
            .iter()
            .find(|a| a.name == "SHA256SUMS.txt")
            .map(|a| a.browser_download_url.clone()),
    }))
}
async fn bounded_body(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
        if bytes.len().saturating_add(chunk.len()) > max {
            return Err(UpdateError::InvalidRelease);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Downloads to a `.part`, verifies exact byte count and SHA-256, then atomically renames.
/// Progress receives (downloaded bytes, expected bytes). Dropping the sender does not cancel.
pub async fn download(
    info: &UpdateInfo,
    dir: &Path,
    mut progress_cb: impl FnMut(u64, u64),
    mut cancel: watch::Receiver<bool>,
) -> Result<PathBuf> {
    if version(&info.version).is_none() {
        return Err(UpdateError::InvalidRelease);
    }
    let name = info.setup_name();
    let part = dir.join(format!("{name}.part"));
    let target = dir.join(&name);
    let work = async {
        let sums_url = info.sums_url.as_ref().ok_or(UpdateError::MissingChecksum)?;
        let http = client(env!("CARGO_PKG_VERSION"), Duration::from_secs(15 * 60))?;
        let sums = http
            .get(sums_url)
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| UpdateError::Network)?;
        let sums = bounded_body(sums, 64 * 1024).await?;
        let sums = std::str::from_utf8(&sums).map_err(|_| UpdateError::MissingChecksum)?;
        let expected = sums
            .lines()
            .find_map(|line| {
                let (hash, filename) = line.split_once("  ")?;
                (filename == name
                    && hash.len() == 64
                    && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| hash.to_ascii_lowercase())
            })
            .ok_or(UpdateError::MissingChecksum)?;
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|_| UpdateError::Disk)?;
        let mut response = http
            .get(&info.setup_url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| UpdateError::Network)?;
        let mut file = tokio::fs::File::create(&part)
            .await
            .map_err(|_| UpdateError::Disk)?;
        let mut hash = Sha256::new();
        let mut received = 0u64;
        progress_cb(0, info.setup_size);
        while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or(UpdateError::SizeMismatch)?;
            if received > info.setup_size {
                return Err(UpdateError::SizeMismatch);
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| UpdateError::Disk)?;
            hash.update(&chunk);
            progress_cb(received, info.setup_size);
        }
        if received != info.setup_size {
            return Err(UpdateError::SizeMismatch);
        }
        if format!("{:x}", hash.finalize()) != expected {
            return Err(UpdateError::ChecksumMismatch);
        }
        file.flush().await.map_err(|_| UpdateError::Disk)?;
        file.sync_all().await.map_err(|_| UpdateError::Disk)?;
        drop(file);
        tokio::fs::rename(&part, &target)
            .await
            .map_err(|_| UpdateError::Disk)?;
        Ok(target)
    };
    let result = tokio::select! { biased; _ = cancelled(&mut cancel) => Err(UpdateError::Cancelled), result = work => result };
    if result.is_err() {
        let _ = tokio::fs::remove_file(part).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    async fn release(tag: &str, draft: bool, prerelease: bool, setup: bool) -> MockServer {
        let server = MockServer::start().await;
        let assets = if setup {
            vec![
                serde_json::json!({"name": format!("Velo-Setup-{}.exe", tag.trim_start_matches('v')), "size": 3, "browser_download_url": format!("{}/setup", server.uri())}),
            ]
        } else {
            vec![]
        };
        Mock::given(method("GET")).and(path("/repos/SinLeeke/velo/releases/latest")).and(header("User-Agent", "Velo/0.9.1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tag_name":tag,"html_url":"https://github.com/SinLeeke/velo/releases","draft":draft,"prerelease":prerelease,"assets":assets})))
            .expect(1).mount(&server).await;
        server
    }
    #[tokio::test]
    async fn newer_version() {
        let s = release("v0.10.0", false, false, true).await;
        let info = check(&s.uri(), "0.9.1").await.unwrap().unwrap();
        assert_eq!(info.version, "0.10.0");
        assert_eq!(info.setup_size, 3);
        assert!(info.sums_url.is_none());
    }
    #[tokio::test]
    async fn equal_version() {
        let s = release("v0.9.1", false, false, true).await;
        assert!(check(&s.uri(), "0.9.1").await.unwrap().is_none());
    }
    #[tokio::test]
    async fn older_version() {
        let s = release("v0.9.0", false, false, true).await;
        assert!(check(&s.uri(), "0.9.1").await.unwrap().is_none());
    }
    #[tokio::test]
    async fn prerelease_ignored() {
        let s = release("v1.0.0-beta", false, true, true).await;
        assert!(check(&s.uri(), "0.9.1").await.unwrap().is_none());
    }
    #[tokio::test]
    async fn draft_ignored() {
        let s = release("v1.0.0", true, false, true).await;
        assert!(check(&s.uri(), "0.9.1").await.unwrap().is_none());
    }
    #[tokio::test]
    async fn missing_setup() {
        let s = release("v1.0.0", false, false, false).await;
        assert_eq!(
            check(&s.uri(), "0.9.1").await.unwrap_err(),
            UpdateError::MissingSetup
        );
    }
    async fn fixture(hash: &str, size: u64) -> (MockServer, UpdateInfo) {
        let s = MockServer::start().await;
        Mock::given(path("/sums"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!("{hash}  Velo-Setup-1.0.0.exe\n")),
            )
            .mount(&s)
            .await;
        Mock::given(path("/setup"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc"))
            .mount(&s)
            .await;
        let info = UpdateInfo {
            version: "1.0.0".into(),
            release_url: String::new(),
            setup_url: format!("{}/setup", s.uri()),
            setup_size: size,
            sums_url: Some(format!("{}/sums", s.uri())),
        };
        (s, info)
    }
    #[tokio::test]
    async fn download_with_progress() {
        let (_s, info) = fixture(&format!("{:x}", Sha256::digest(b"abc")), 3).await;
        let dir = tempfile::tempdir().unwrap();
        let mut progress = Vec::new();
        let (_tx, cancel) = watch::channel(false);
        let path = download(
            &info,
            dir.path(),
            |n, total| progress.push((n, total)),
            cancel,
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"abc");
        assert_eq!(progress.first(), Some(&(0, 3)));
        assert_eq!(progress.last(), Some(&(3, 3)));
        assert!(!dir.path().join("Velo-Setup-1.0.0.exe.part").exists());
    }
    #[tokio::test]
    async fn incorrect_checksum() {
        let (_s, info) = fixture(&"0".repeat(64), 3).await;
        let dir = tempfile::tempdir().unwrap();
        let (_tx, cancel) = watch::channel(false);
        assert_eq!(
            download(&info, dir.path(), |_, _| {}, cancel)
                .await
                .unwrap_err(),
            UpdateError::ChecksumMismatch
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn missing_sums_never_installs() {
        let (_s, mut info) = fixture("", 3).await;
        info.sums_url = None;
        let dir = tempfile::tempdir().unwrap();
        let (_tx, cancel) = watch::channel(false);
        assert_eq!(
            download(&info, dir.path(), |_, _| {}, cancel)
                .await
                .unwrap_err(),
            UpdateError::MissingChecksum
        );
    }
    #[tokio::test]
    async fn wrong_size() {
        let (_s, info) = fixture(&format!("{:x}", Sha256::digest(b"abc")), 4).await;
        let dir = tempfile::tempdir().unwrap();
        let (_tx, cancel) = watch::channel(false);
        assert_eq!(
            download(&info, dir.path(), |_, _| {}, cancel)
                .await
                .unwrap_err(),
            UpdateError::SizeMismatch
        );
    }
    #[tokio::test]
    async fn cancelled_download() {
        let (_s, info) = fixture("", 3).await;
        let dir = tempfile::tempdir().unwrap();
        let (_tx, cancel) = watch::channel(true);
        assert_eq!(
            download(&info, dir.path(), |_, _| {}, cancel)
                .await
                .unwrap_err(),
            UpdateError::Cancelled
        );
    }
    #[test]
    fn strict_versions() {
        for s in ["1.2", "1.2.3.4", "1.2.3-beta", "1.+2.3", "v../bad"] {
            assert!(version(s).is_none());
        }
        assert!(version("v1.2.3") > version("0.99.99"));
    }
}
