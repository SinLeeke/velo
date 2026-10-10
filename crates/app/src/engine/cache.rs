//! Poda al arrancar y al cambiar el límite; espera eventos sin timer periódico.
use std::sync::Arc;

use super::Env;

pub fn start(env: &Arc<Env>) {
    super::audio::settings_runtime::start(env);
    let mut settings = env.prefs.watch_settings();
    let env = env.clone();
    env.rt.clone().spawn(async move {
        let mut previous = None;
        loop {
            let limit = settings.borrow_and_update().cache.cover_limit_mb;
            if previous != Some(limit) {
                previous = Some(limit);
                env.covers.set_limit_bytes(u64::from(limit) * 1024 * 1024);
                if let Err(e) = env.covers.prune_to_limit().await {
                    env.report(&e);
                }
            }
            if settings.changed().await.is_err() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn startup_prunes_existing_files_and_settings_changes_prune_again() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (dir, env) = super::super::test_env(&rt);
        let covers = dir.path().join("covers/covers");
        std::fs::create_dir_all(&covers).unwrap();
        let path = covers.join("old.jpg");
        std::fs::write(&path, vec![0; 2 * 1024 * 1024]).unwrap();
        env.prefs.update_settings(|s| s.cache.cover_limit_mb = 1);
        start(&env);
        rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while path.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            env.prefs.update_settings(|s| s.cache.cover_limit_mb = 2);
            tokio::time::timeout(Duration::from_secs(5), async {
                while env.covers.limit_bytes() != 2 * 1024 * 1024 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            std::fs::write(&path, vec![0; 2 * 1024 * 1024]).unwrap();
            env.prefs.update_settings(|s| s.cache.cover_limit_mb = 1);
            tokio::time::timeout(Duration::from_secs(5), async {
                while path.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        });
    }
}
