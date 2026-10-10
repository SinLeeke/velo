use crate::{atomic_write, error, hash, palette, project_dirs};
use async_trait::async_trait;
use image::{DynamicImage, ImageFormat, RgbImage, imageops::FilterType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Mutex;
use velo_domain::{CoverArtService, CoverAssets, Palette, Result};

const MAX_DOWNLOAD_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const FAILURE_TTL: Duration = Duration::from_secs(5 * 60);

pub struct CoverArtCache {
    root: PathBuf,
    client: crate::settings::proxy::HttpClient,
    locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    failures: Mutex<HashMap<String, (Instant, String)>>,
    disk: Mutex<()>,
    max_bytes: AtomicU64,
}
impl CoverArtCache {
    pub fn new() -> Result<Self> {
        Self::with_cache_dir(project_dirs()?.cache_dir())
    }
    pub fn with_cache_dir(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            root: root.into(),
            client: crate::settings::proxy::HttpClient::new(Duration::from_secs(10))?,
            locks: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
            disk: Mutex::new(()),
            max_bytes: AtomicU64::new(DEFAULT_CACHE_MAX_BYTES),
        })
    }
    /// Solo actualiza la política; la poda se solicita desde una tarea Tokio.
    pub fn set_limit_bytes(&self, max_bytes: u64) {
        self.max_bytes.store(max_bytes, Ordering::Relaxed);
    }

    pub fn limit_bytes(&self) -> u64 {
        self.max_bytes.load(Ordering::Relaxed)
    }

    pub async fn prune_to_limit(&self) -> Result<()> {
        self.prune(self.limit_bytes()).await
    }
    async fn lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
            return lock;
        }
        locks.retain(|_, v| v.strong_count() > 0);
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key.into(), Arc::downgrade(&lock));
        lock
    }
    async fn cached(&self, key: &str, blur: bool) -> Result<Option<CoverAssets>> {
        let dir = self.root.join("covers");
        let cover = dir.join(format!("{key}.jpg"));
        let blurred = dir.join(format!("{key}.png"));
        if !tokio::fs::try_exists(&cover).await.map_err(error)?
            || (blur && !tokio::fs::try_exists(&blurred).await.map_err(error)?)
        {
            return Ok(None);
        }
        let data = match tokio::fs::read(dir.join(format!("{key}.json"))).await {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(error(e)),
        };
        let palette = tokio::task::spawn_blocking(move || serde_json::from_slice::<Palette>(&data))
            .await
            .map_err(error)?;
        Ok(palette.ok().map(|palette| CoverAssets {
            cover,
            blurred: if blur { blurred } else { PathBuf::new() },
            palette,
        }))
    }
    async fn build(&self, key: String, bytes: Vec<u8>, blur: bool) -> Result<CoverAssets> {
        let _guard = self.disk.lock().await;
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || build_assets(&root, &key, &bytes, blur))
            .await
            .map_err(error)?
    }
    /// Flat background: reuse cover/palette without decoding or computing a blur.
    pub async fn process_with_blur(&self, url: &str, blur: bool) -> Result<CoverAssets> {
        let key = hash(url);
        let lock = self.lock(&key).await;
        let _guard = lock.lock().await;
        if let Some(cached) = self.cached(&key, blur).await? {
            return Ok(cached);
        }
        // Re-enable blur or upgrade an old palette using the cached cover, offline.
        let bytes = match tokio::fs::read(self.root.join("covers").join(format!("{key}.jpg"))).await
        {
            Ok(bytes) => bytes,
            Err(_) => self.download(url).await?,
        };
        let assets = self.build(key, bytes, blur).await?;
        self.prune_to_limit().await?;
        Ok(assets)
    }
    pub async fn process_bytes_with_blur(
        &self,
        key: &str,
        bytes: Vec<u8>,
        blur: bool,
    ) -> Result<CoverAssets> {
        let key = hash(key);
        let lock = self.lock(&key).await;
        let _guard = lock.lock().await;
        if let Some(cached) = self.cached(&key, blur).await? {
            return Ok(cached);
        }
        let bytes = if bytes.is_empty() {
            tokio::fs::read(self.root.join("covers").join(format!("{key}.jpg")))
                .await
                .map_err(error)?
        } else {
            bytes
        };
        let assets = self.build(key, bytes, blur).await?;
        self.prune_to_limit().await?;
        Ok(assets)
    }
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        {
            let mut failures = self.failures.lock().await;
            failures.retain(|_, (at, _)| at.elapsed() < FAILURE_TTL);
            if let Some((_, message)) = failures.get(url) {
                return Err(error(message));
            }
        }
        let result = self.download_bounded(url).await;
        if let Err(e) = &result {
            self.failures
                .lock()
                .await
                .insert(url.into(), (Instant::now(), e.to_string()));
        }
        result
    }
    async fn download_bounded(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(error)?
            .error_for_status()
            .map_err(error)?;
        if response
            .content_length()
            .is_some_and(|n| n > MAX_DOWNLOAD_BYTES as u64)
        {
            return Err(error("cover exceeds 8 MB"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(error)? {
            if chunk.len() > MAX_DOWNLOAD_BYTES - bytes.len() {
                return Err(error("cover exceeds 8 MB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    /// Size of covers and thumbnails, for Settings. Runs filesystem work off-thread.
    pub async fn cache_size_bytes(&self) -> Result<u64> {
        let _guard = self.disk.lock().await;
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || Ok(cache_files(&root)?.iter().map(|f| f.2).sum()))
            .await
            .map_err(error)?
    }
    /// Elimina los archivos más antiguos por acceso/modificación; límite por defecto: 50 MiB.
    pub async fn prune(&self, max_bytes: u64) -> Result<()> {
        let _guard = self.disk.lock().await;
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let mut files = cache_files(&root)?;
            files.sort_by_key(|f| f.1);
            let mut size: u64 = files.iter().map(|f| f.2).sum();
            for (path, _, bytes) in files {
                if size <= max_bytes {
                    break;
                }
                match std::fs::remove_file(path) {
                    Ok(()) => size = size.saturating_sub(bytes),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        size = size.saturating_sub(bytes)
                    }
                    Err(e) => return Err(error(e)),
                }
            }
            Ok(())
        })
        .await
        .map_err(error)?
    }
}
#[async_trait]
impl CoverArtService for CoverArtCache {
    async fn process(&self, url: &str) -> Result<CoverAssets> {
        self.process_with_blur(url, true).await
    }
    async fn process_bytes(&self, key: &str, bytes: Vec<u8>) -> Result<CoverAssets> {
        self.process_bytes_with_blur(key, bytes, true).await
    }
    async fn thumbnail(&self, url: &str, max_px: u32) -> Result<PathBuf> {
        if max_px == 0 {
            return Err(error("thumbnail size must be positive"));
        }
        let max_px = max_px.min(1024);
        let key = hash(url);
        let path = self
            .root
            .join("thumbs")
            .join(max_px.to_string())
            .join(format!("{key}.jpg"));
        let lock = self.lock(&format!("thumb/{max_px}/{key}")).await;
        let _guard = lock.lock().await;
        if tokio::fs::try_exists(&path).await.map_err(error)? {
            return Ok(path);
        }
        let cover = self.process_with_blur(url, false).await?.cover;
        let result = path.clone();
        let bytes = tokio::fs::read(cover).await.map_err(error)?;
        let guard = self.disk.lock().await;
        tokio::task::spawn_blocking(move || {
            let image = decode(&bytes)?;
            let thumbnail = fit(&image, max_px);
            atomic_write(&result, &jpeg(&thumbnail)?)
        })
        .await
        .map_err(error)??;
        drop(guard);
        self.prune_to_limit().await?;
        Ok(path)
    }
}
fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(error)
}
fn cache_files(root: &Path) -> Result<Vec<(PathBuf, SystemTime, u64)>> {
    fn visit(dir: &Path, files: &mut Vec<(PathBuf, SystemTime, u64)>) -> Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(error(e)),
        };
        for entry in entries {
            let entry = entry.map_err(error)?;
            let metadata = entry.path().symlink_metadata().map_err(error)?;
            if metadata.is_dir() {
                visit(&entry.path(), files)?;
            } else if metadata.is_file() {
                let modified = metadata.modified().map_err(error)?;
                let accessed = metadata.accessed().unwrap_or(modified);
                files.push((entry.path(), accessed.max(modified), metadata.len()));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(&root.join("covers"), &mut files)?;
    visit(&root.join("thumbs"), &mut files)?;
    Ok(files)
}
fn fit(image: &DynamicImage, max: u32) -> DynamicImage {
    if image.width() <= max && image.height() <= max {
        image.clone()
    } else {
        image.resize(max, max, FilterType::CatmullRom)
    }
}
fn jpeg(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 88)
        .encode_image(&image.to_rgb8())
        .map_err(error)?;
    Ok(bytes)
}
// Sliding windows give O(pixels) cost independent of radius, on the 96px image only.
fn box_pass(input: &RgbImage, horizontal: bool) -> RgbImage {
    let (width, height) = input.dimensions();
    let mut output = RgbImage::new(width, height);
    let (lines, length) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    let radius = 6i64;
    for line in 0..lines {
        let pixel = |position: i64| {
            let position = position.clamp(0, i64::from(length) - 1) as u32;
            if horizontal {
                input.get_pixel(position, line).0
            } else {
                input.get_pixel(line, position).0
            }
        };
        let mut sum = [0u32; 3];
        for position in -radius..=radius {
            for (s, v) in sum.iter_mut().zip(pixel(position)) {
                *s += u32::from(v);
            }
        }
        for position in 0..length {
            let value = image::Rgb(sum.map(|s| (s / 13) as u8));
            if horizontal {
                output.put_pixel(position, line, value);
            } else {
                output.put_pixel(line, position, value);
            }
            for (s, v) in sum.iter_mut().zip(pixel(i64::from(position) - radius)) {
                *s -= u32::from(v);
            }
            for (s, v) in sum.iter_mut().zip(pixel(i64::from(position) + radius + 1)) {
                *s += u32::from(v);
            }
        }
    }
    output
}
fn build_assets(root: &Path, key: &str, bytes: &[u8], blur: bool) -> Result<CoverAssets> {
    let image = decode(bytes)?;
    let cover = fit(&image, 600);
    let small = fit(&image, 96);
    // Reuse the unblurred reduction for the 48x48 color sample.
    let palette = palette::extract(&small);
    let dir = root.join("covers");
    let assets = CoverAssets {
        cover: dir.join(format!("{key}.jpg")),
        blurred: if blur {
            dir.join(format!("{key}.png"))
        } else {
            PathBuf::new()
        },
        palette,
    };
    atomic_write(&assets.cover, &jpeg(&cover)?)?;
    if blur {
        let mut blurred = small.to_rgb8();
        for _ in 0..3 {
            blurred = box_pass(&box_pass(&blurred, true), false);
        }
        let mut png = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(blurred)
            .write_to(&mut png, ImageFormat::Png)
            .map_err(error)?;
        atomic_write(&assets.blurred, &png.into_inner())?;
    }
    // Palette is the completion marker; readers never reuse a partially produced entry.
    atomic_write(
        &dir.join(format!("{key}.json")),
        &serde_json::to_vec(&assets.palette).map_err(error)?,
    )?;
    Ok(assets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    #[tokio::test]
    async fn flat_background_skips_blur_and_enabling_it_reuses_the_cached_cover() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        let flat = cache
            .process_bytes_with_blur("flat", synthetic(640), false)
            .await
            .unwrap();
        assert!(flat.blurred.as_os_str().is_empty());
        let blur_path = dir
            .path()
            .join("covers")
            .join(format!("{}.png", hash("flat")));
        assert!(!blur_path.exists());
        assert_eq!(
            flat,
            cache
                .process_bytes_with_blur("flat", vec![], false)
                .await
                .unwrap()
        );
        let blurred = cache
            .process_bytes_with_blur("flat", vec![], true)
            .await
            .unwrap();
        assert_eq!(flat.cover, blurred.cover);
        assert!(blurred.blurred.exists());
        let flat_again = cache
            .process_bytes_with_blur("flat", vec![], false)
            .await
            .unwrap();
        assert!(flat_again.blurred.as_os_str().is_empty());
        assert_eq!(flat_again.palette, blurred.palette);
    }
    #[tokio::test]
    async fn configured_limit_applies_to_writes_and_clear_preserves_other_data() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        assert_eq!(DEFAULT_CACHE_MAX_BYTES, 50 * 1024 * 1024);
        cache.set_limit_bytes(10_000);
        for key in 0..10 {
            cache
                .process_bytes(&key.to_string(), synthetic(64))
                .await
                .unwrap();
            assert!(cache.cache_size_bytes().await.unwrap() <= 10_000);
        }
        let unrelated = dir.path().join("settings.toml");
        atomic_write(&unrelated, b"keep").unwrap();
        cache.prune(0).await.unwrap();
        assert_eq!(cache.cache_size_bytes().await.unwrap(), 0);
        assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
        // Borrar no desactiva el límite ni impide volver a generar carátulas.
        cache.process_bytes("new", synthetic(64)).await.unwrap();
        assert!(cache.cache_size_bytes().await.unwrap() > 0);
        cache.set_limit_bytes(1);
        cache.prune_to_limit().await.unwrap();
        assert!(cache.cache_size_bytes().await.unwrap() <= 1);
    }
    fn synthetic(size: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(size, size, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 80])
        }));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }
    #[tokio::test]
    async fn download_size_header_and_failure_expiration() {
        let server = MockServer::start().await;
        Mock::given(path("/large"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![0; MAX_DOWNLOAD_BYTES + 1]),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/fail"))
            .respond_with(ResponseTemplate::new(503))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(path("/boundary"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; MAX_DOWNLOAD_BYTES]))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        let large = format!("{}/large", server.uri());
        assert_eq!(
            cache
                .download(&format!("{}/boundary", server.uri()))
                .await
                .unwrap()
                .len(),
            MAX_DOWNLOAD_BYTES
        );
        let failure = cache.process(&large).await.unwrap_err().to_string();
        assert!(failure.contains("8 MB"), "{failure}");
        assert!(cache.thumbnail(&large, 64).await.is_err());
        let fail = format!("{}/fail", server.uri());
        assert!(cache.process(&fail).await.is_err());
        assert!(cache.process(&fail).await.is_err());
        cache.failures.lock().await.get_mut(&fail).unwrap().0 = Instant::now() - FAILURE_TTL;
        assert!(cache.process(&fail).await.is_err());
        assert_eq!(cache.cache_size_bytes().await.unwrap(), 0);
        server.verify().await;
    }
    #[tokio::test]
    async fn chunked_download_enforces_cap_without_content_length() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let chunk = vec![0; 64 * 1024];
            for _ in 0..=MAX_DOWNLOAD_BYTES / chunk.len() {
                if socket
                    .write_all(b"10000\r\n")
                    .and_then(|_| socket.write_all(&chunk))
                    .and_then(|_| socket.write_all(b"\r\n"))
                    .is_err()
                {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n");
        });
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        let failure = cache.process(&url).await.unwrap_err().to_string();
        assert!(failure.contains("8 MB"), "{failure}");
        tokio::task::spawn_blocking(move || worker.join().unwrap())
            .await
            .unwrap();
    }
    #[test]
    fn decode_rejects_dimensions_and_allocation_before_pixels() {
        // Valid PNG headers with oversized dimensions/allocation; retain the small payload.
        fn header(width: u32, height: u32, depth: u8, color: u8) -> Vec<u8> {
            let mut bytes = synthetic(1);
            bytes[16..20].copy_from_slice(&width.to_be_bytes());
            bytes[20..24].copy_from_slice(&height.to_be_bytes());
            bytes[24] = depth;
            bytes[25] = color;
            let mut crc = u32::MAX;
            for byte in &bytes[12..29] {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
                }
            }
            bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
            bytes
        }
        for bytes in [
            header(4097, 1, 8, 2),
            header(1, 4097, 8, 2),
            header(4096, 4096, 16, 6),
        ] {
            assert!(decode(&bytes).unwrap_err().to_string().contains("limit"));
        }
        assert!(decode(&synthetic(16)).is_ok());
    }
    #[tokio::test]
    async fn prune_orders_both_directories_and_reports_size() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        assert_eq!(cache.cache_size_bytes().await.unwrap(), 0);
        cache.prune(DEFAULT_CACHE_MAX_BYTES).await.unwrap();
        let old = dir.path().join("covers/old.jpg");
        let new = dir.path().join("thumbs/64/new.jpg");
        let time = SystemTime::now();
        for (path, age) in [(&old, 120), (&new, 60)] {
            atomic_write(path, &[0; 20]).unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new()
                        .set_accessed(time - Duration::from_secs(age))
                        .set_modified(time - Duration::from_secs(age)),
                )
                .unwrap();
        }
        atomic_write(&dir.path().join("lyrics/keep.json"), &[0; 100]).unwrap();
        assert_eq!(cache.cache_size_bytes().await.unwrap(), 40);
        cache.prune(20).await.unwrap();
        assert!(!old.exists());
        assert!(new.exists());
        assert_eq!(cache.cache_size_bytes().await.unwrap(), 20);
        cache.prune(0).await.unwrap();
        assert_eq!(cache.cache_size_bytes().await.unwrap(), 0);
        assert!(dir.path().join("lyrics/keep.json").exists());
    }
    #[tokio::test]
    async fn thumbnail_caps_size_and_rejects_zero() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        assert!(cache.thumbnail("unused", 0).await.is_err());
        cache.process_bytes("unused", synthetic(64)).await.unwrap();
        let path = cache.thumbnail("unused", u32::MAX).await.unwrap();
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "1024");
        assert_eq!(path, cache.thumbnail("unused", 1024).await.unwrap());
        assert!(image::open(path).unwrap().width() <= 1024);
    }
    #[tokio::test]
    async fn new_assets_automatically_prune_to_default_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        let old = dir.path().join("thumbs/64/old.jpg");
        atomic_write(&old, &[]).unwrap();
        let file = std::fs::File::options().write(true).open(&old).unwrap();
        file.set_len(DEFAULT_CACHE_MAX_BYTES + 1).unwrap();
        let time = SystemTime::now() - Duration::from_secs(120);
        file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(time)
                .set_modified(time),
        )
        .unwrap();
        drop(file);
        let assets = cache.process_bytes("new", synthetic(16)).await.unwrap();
        assert!(!old.exists());
        assert!(assets.cover.exists());
        assert!(assets.blurred.exists());
        assert!(cache.cache_size_bytes().await.unwrap() <= DEFAULT_CACHE_MAX_BYTES);
    }
    #[tokio::test]
    async fn cache_is_disk_only_and_thumbnails_are_reused() {
        let server = MockServer::start().await;
        Mock::given(path("/cover"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(synthetic(640)))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        let url = format!("{}/cover", server.uri());
        let first = cache.process(&url).await.unwrap();
        let modified = tokio::fs::metadata(&first.cover)
            .await
            .unwrap()
            .modified()
            .unwrap();
        // A fresh instance proves the cache is on disk, not just in memory.
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        assert_eq!(first, cache.process(&url).await.unwrap());
        assert_eq!(
            modified,
            tokio::fs::metadata(&first.cover)
                .await
                .unwrap()
                .modified()
                .unwrap()
        );
        let thumb = cache.thumbnail(&url, 64).await.unwrap();
        assert_eq!(thumb, cache.thumbnail(&url, 64).await.unwrap());
        let bytes = tokio::fs::read(thumb).await.unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 64);
        assert_eq!(image::open(first.cover).unwrap().width(), 600);
        assert_eq!(image::open(first.blurred).unwrap().width(), 96);
        server.verify().await;
    }
    #[tokio::test]
    async fn simultaneous_process_calls_download_once() {
        let server = MockServer::start().await;
        Mock::given(path("/cover"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(synthetic(64))
                    .set_delay(Duration::from_millis(50)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        let url = format!("{}/cover", server.uri());
        let (a, b) = tokio::join!(cache.process(&url), cache.process(&url));
        assert_eq!(a.unwrap(), b.unwrap());
        server.verify().await;
    }
    #[tokio::test]
    async fn process_bytes_reuses_key_and_recovers_incomplete_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverArtCache::with_cache_dir(dir.path()).unwrap();
        let a = cache.process_bytes("smtc", synthetic(64)).await.unwrap();
        assert_eq!(a, cache.process_bytes("smtc", vec![]).await.unwrap());
        tokio::fs::remove_file(&a.blurred).await.unwrap();
        assert_eq!(a, cache.process_bytes("smtc", synthetic(64)).await.unwrap());
    }
    #[test]
    #[ignore = "release benchmark: cargo test -p velo-services --release benchmark_640 -- --ignored --nocapture"]
    fn benchmark_640() {
        let bytes = synthetic(640);
        let dir = tempfile::tempdir().unwrap();
        let start = std::time::Instant::now();
        build_assets(dir.path(), "benchmark", &bytes, true).unwrap();
        println!(
            "640x640 decode + resize + blur + palette + atomic encodes: {:.2} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}
