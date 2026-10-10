//! Asynchronous disk-backed services for Velo.
mod covers;
pub mod history;
mod lyrics;
mod palette;
mod settings;
pub mod update;

pub use covers::{CoverArtCache, DEFAULT_CACHE_MAX_BYTES};
pub use lyrics::{LrcLibClient, parse_lrc};
pub use settings::TomlSettingsStore;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use velo_domain::{Result, VeloError};

fn error(e: impl std::fmt::Display) -> VeloError {
    VeloError::Other(e.to_string())
}
fn hash(key: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(key.as_bytes()))
}
fn project_dirs() -> Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("app", "Velo", "Velo")
        .ok_or_else(|| error("no user configuration directory"))
}

// Same-directory temporary files keep rename atomic, including on Windows.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or_else(|| error("path has no parent"))?;
    std::fs::create_dir_all(parent).map_err(error)?;
    let temp = parent.join(format!(
        ".velo-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(error)?;
        file.write_all(bytes).map_err(error)?;
        file.sync_all().map_err(error)?;
        drop(file);
        std::fs::rename(&temp, path).map_err(error)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
