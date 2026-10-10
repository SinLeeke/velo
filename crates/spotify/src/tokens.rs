//! Refresh-token storage. Access tokens never enter these stores.
use std::sync::Mutex;

/// Synchronous persistence boundary; the provider calls it on blocking workers.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, refresh_token: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

#[derive(Default)]
pub struct MemoryTokenStore(Mutex<Option<String>>);

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn save(&self, refresh_token: &str) -> Result<(), String> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(refresh_token.into());
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

pub struct KeyringTokenStore {
    user: &'static str,
}

impl Default for KeyringTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringTokenStore {
    /// The Web API session (Velo's own Client ID).
    pub fn new() -> Self {
        Self::with_user("spotify-refresh-token")
    }

    /// Another refresh token under the same `velo` service (the audio login uses a different
    /// OAuth client and must never overwrite the Web API session).
    pub fn with_user(user: &'static str) -> Self {
        Self { user }
    }

    fn entry(&self) -> keyring::Result<keyring::Entry> {
        // keyring 4's default `v1` feature wraps keyring-core. Entry::new first
        // runs its LazyLock initializer: windows_native_keyring_store::Store::new()
        // followed by keyring_core::set_default_store(store). No mock fallback.
        keyring::Entry::new("velo", self.user)
    }
}

impl TokenStore for KeyringTokenStore {
    fn load(&self) -> Option<String> {
        loaded_token(self.entry().and_then(|entry| entry.get_password()))
    }

    fn save(&self, refresh_token: &str) -> Result<(), String> {
        self.entry()
            .and_then(|entry| entry.set_password(refresh_token))
            .map_err(|_| "No se pudo guardar la sesión en el Administrador de credenciales".into())
    }

    fn clear(&self) -> Result<(), String> {
        match self.entry().and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("No se pudo borrar la sesión del Administrador de credenciales".into()),
        }
    }
}

fn loaded_token(result: keyring::Result<String>) -> Option<String> {
    match result {
        Ok(token) => Some(token),
        Err(keyring::Error::NoEntry) => None,
        Err(_) => {
            // Never log credential-store details, which may contain secrets.
            tracing::warn!("No se pudo leer la sesión del Administrador de credenciales");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tracing::{
        Event, Metadata, Subscriber,
        span::{Attributes, Id, Record},
    };

    struct WarningCounter(Arc<AtomicUsize>);

    impl Subscriber for WarningCounter {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            if *event.metadata().level() == tracing::Level::WARN {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    #[test]
    fn missing_credentials_are_quiet_but_real_load_errors_warn() {
        let warnings = Arc::new(AtomicUsize::new(0));
        tracing::subscriber::with_default(WarningCounter(warnings.clone()), || {
            assert_eq!(
                loaded_token(Ok("refresh".into())).as_deref(),
                Some("refresh")
            );
            assert!(loaded_token(Err(keyring::Error::NoEntry)).is_none());
            assert_eq!(warnings.load(Ordering::SeqCst), 0);
            assert!(loaded_token(Err(keyring::Error::NoDefaultStore)).is_none());
            assert_eq!(warnings.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn memory_store_save_and_clear_return_success() {
        let store = MemoryTokenStore::new();
        assert!(store.load().is_none());
        store.save("first").unwrap();
        store.save("replacement").unwrap();
        assert_eq!(store.load().as_deref(), Some("replacement"));
        store.clear().unwrap();
        store.clear().unwrap();
        assert!(store.load().is_none());
    }
}
