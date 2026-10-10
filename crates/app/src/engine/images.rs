//! Asynchronous thumbnails for list rows: download once, shrink, keep on disk (velo-services
//! cache), then hand QML a `file:///` URL. Rows are shown immediately with an empty image.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Semaphore;
use velo_domain::CoverArtService;
use velo_services::CoverArtCache;

use super::rows::file_url;

/// Limits parallel downloads/decodes so a 50-row page does not flood the network or CPU.
const PARALLEL: usize = 4;

/// Invalidates in-flight thumbnail work when a model is replaced.
#[derive(Default)]
pub struct Generation(AtomicU64);

impl Generation {
    /// Starts a new generation (the model was replaced); returns its number.
    pub fn bump(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct Thumbnailer {
    covers: Arc<CoverArtCache>,
    permits: Arc<Semaphore>,
    rt: tokio::runtime::Handle,
}

impl Thumbnailer {
    pub fn new(rt: tokio::runtime::Handle, covers: Arc<CoverArtCache>) -> Self {
        Self {
            covers,
            permits: Arc::new(Semaphore::new(PARALLEL)),
            rt,
        }
    }

    /// For each `(row, remote url)`: produce the thumbnail and call `done(row, file_url)`,
    /// unless `generation` moved on from `mine` in the meantime.
    pub fn fill<F>(
        &self,
        generation: Arc<Generation>,
        mine: u64,
        items: Vec<(usize, String)>,
        px: u32,
        done: F,
    ) where
        F: Fn(usize, String) + Send + Sync + 'static,
    {
        let done = Arc::new(done);
        for (row, url) in items {
            let covers = self.covers.clone();
            let permits = self.permits.clone();
            let generation = generation.clone();
            let done = done.clone();
            self.rt.spawn(async move {
                let Ok(_permit) = permits.acquire_owned().await else {
                    return;
                };
                if generation.current() != mine {
                    return;
                }
                match covers.thumbnail(&url, px).await {
                    Ok(path) if generation.current() == mine => done(row, file_url(&path)),
                    Ok(_) => {}
                    Err(e) => tracing::debug!(%e, "thumbnail failed"),
                }
            });
        }
    }

    /// One thumbnail without generation tracking (avatars, headers).
    pub async fn one(&self, url: &str, px: u32) -> Option<String> {
        self.covers
            .thumbnail(url, px)
            .await
            .ok()
            .map(|p| file_url(&p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_invalidates_older_work() {
        let g = Generation::default();
        let a = g.bump();
        assert_eq!(g.current(), a);
        let b = g.bump();
        assert_ne!(g.current(), a);
        assert_eq!(g.current(), b);
    }
}
