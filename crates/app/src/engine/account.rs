//! Spotify account: provider lifecycle, sign in/out, profile. Tokens live in the OS keyring
//! (velo-spotify's `KeyringTokenStore`); nothing here writes or logs them.
//!
//! L1: without a personal Spotify app (the default) the account signs in ONCE, with
//! librespot's login (the audio engine), and the library comes from that session
//! (`velo_player::session_api::SessionApi`). A personal app (Client ID in Ajustes or
//! `velo.local.env`) keeps Velo's own PKCE login for the Web API, as before.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::{broadcast, watch};
use velo_domain::{MusicProvider, UserProfile, VeloError, pick_image};
use velo_player::{login, session_api::SessionApi};
use velo_services::CoverArtCache;
use velo_spotify::oauth::valid_client_id;
use velo_spotify::{
    KeyringTokenStore, MemoryTokenStore, SpotifyConfig, SpotifyProvider, TokenStore,
};

use super::audio::AudioCtl;
use super::prefs::Prefs;
use super::rows::file_url;
use super::user_message;
use velo_domain::CoverArtService;

/// Each new login/logout invalidates older profile reads and browser waits.
struct AuthWork {
    generation: watch::Sender<u64>,
    login: bool,
}

impl Default for AuthWork {
    fn default() -> Self {
        Self {
            generation: watch::channel(0).0,
            login: false,
        }
    }
}

impl AuthWork {
    fn invalidate(&mut self) {
        self.generation.send_modify(|v| *v = v.wrapping_add(1));
        self.login = false;
    }

    fn begin_login(&mut self) -> Option<(u64, watch::Receiver<u64>)> {
        if self.login {
            return None;
        }
        self.invalidate();
        self.login = true;
        Some((*self.generation.borrow(), self.generation.subscribe()))
    }

    fn current(&self, generation: u64) -> bool {
        *self.generation.borrow() == generation
    }
}

async fn until_cancelled<T>(
    mut generation: watch::Receiver<u64>,
    expected: u64,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        () = async {
            while *generation.borrow_and_update() == expected {
                if generation.changed().await.is_err() { break; }
            }
        } => None,
        result = work => Some(result),
    }
}

fn profile_delay(attempt: usize, error: &VeloError) -> Duration {
    const SECONDS: [u64; 5] = [2, 5, 15, 30, 60];
    let delay = Duration::from_secs(SECONDS[attempt.min(SECONDS.len() - 1)]);
    match error {
        VeloError::RateLimited(retry_after) => delay.max(*retry_after),
        _ => delay,
    }
}

/// Un 403 de /me durante el login es acceso a la cuenta, no a una canción/playlist.
fn sign_in_message(error: &VeloError) -> String {
    match error {
        VeloError::Other(message) if message == "invalid_client" => "Spotify desactivó o rechazó esta app. Puedes usar tu propia app de Spotify desde Ajustes > Cuenta.".into(),
        VeloError::Other(message) if message == "Invalid OAuth callback" => "La respuesta de Spotify tiene un estado OAuth inválido. Cierra esa pestaña y vuelve a iniciar sesión.".into(),
        VeloError::RateLimited(_) => "Spotify está limitando las peticiones de esta app. Espera un minuto y vuelve a intentarlo.".into(),
        VeloError::Forbidden(_) => "Spotify no autorizó esta cuenta. En modo desarrollo, el dueño de la app debe agregar tu nombre y el correo de tu cuenta de Spotify en Spotify for Developers > tu app > User Management (Users and Access). Si ya estás agregado, pídele que revise los permisos de la app e inténtalo otra vez.".into(),
        VeloError::NotAuthenticated => "No se completó la autorización. Vuelve a iniciar sesión y acepta el acceso en Spotify.".into(),
        VeloError::Other(message) if message == "Spotify sign-in timed out" => "Se agotó el tiempo de espera. Inicia sesión otra vez y termina la autorización en el navegador.".into(),
        VeloError::Other(message) if message == "Could not open the system browser" || message == "Browser worker failed" => "No se pudo abrir el navegador. Comprueba que tienes un navegador predeterminado y vuelve a intentarlo.".into(),
        _ => user_message(error),
    }
}

async fn profile_with_retry<F, Fut, S, Sleep>(
    mut read: F,
    mut unavailable: impl FnMut(&VeloError),
    mut sleep: S,
) -> velo_domain::Result<UserProfile>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = velo_domain::Result<UserProfile>>,
    S: FnMut(Duration) -> Sleep,
    Sleep: std::future::Future<Output = ()>,
{
    let mut attempt: usize = 0;
    loop {
        match read().await {
            Ok(profile) => return Ok(profile),
            Err(e @ (VeloError::NotAuthenticated | VeloError::Forbidden(_))) => return Err(e),
            Err(e) => {
                unavailable(&e);
                sleep(profile_delay(attempt, &e)).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountView {
    pub signed_in: bool,
    pub display_name: String,
    /// `file:///` URL of the cached avatar, empty if none.
    pub avatar_url: String,
    pub is_premium: bool,
    pub client_id: String,
    pub busy: bool,
    pub user_id: String,
    pub app_warning: String,
}

pub struct Account {
    session_queue: watch::Sender<Option<velo_player::queue::connect_snapshot::Snapshot>>,
    queue_revision: std::sync::atomic::AtomicU64,
    rt: tokio::runtime::Handle,
    prefs: Arc<Prefs>,
    covers: Arc<CoverArtCache>,
    audio: Arc<AudioCtl>,
    /// Personal app (PKCE); `None` in the default mode.
    provider: RwLock<Option<Arc<SpotifyProvider>>>,
    /// L1 default mode: the data provider on the audio session.
    session_api: RwLock<Option<Arc<SessionApi>>>,
    pub view: watch::Sender<AccountView>,
    /// Reason text for `Account.signInFailed`.
    pub failures: broadcast::Sender<String>,
    work: Mutex<AuthWork>,
    // Serialize lifecycle changes, never the browser wait.
    operation: tokio::sync::Mutex<()>,
}

impl Account {
    pub fn new(
        rt: tokio::runtime::Handle,
        prefs: Arc<Prefs>,
        covers: Arc<CoverArtCache>,
        audio: Arc<AudioCtl>,
    ) -> Arc<Self> {
        let client_id = prefs.client_id().unwrap_or_default();
        let this = Arc::new(Self {
            session_queue: watch::channel(None).0,
            queue_revision: std::sync::atomic::AtomicU64::new(0),
            rt,
            prefs,
            covers,
            audio,
            provider: RwLock::new(None),
            session_api: RwLock::new(None),
            view: watch::channel(AccountView {
                client_id,
                ..AccountView::default()
            })
            .0,
            failures: broadcast::channel(4).0,
            work: Mutex::new(AuthWork::default()),
            operation: tokio::sync::Mutex::new(()),
        });
        this.observe_queue();
        this
    }

    pub fn queue_revision(&self) -> u64 {
        self.queue_revision
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn note_queue_change(&self) {
        self.queue_revision
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn session_queue_snapshot(&self) -> Option<velo_player::queue::connect_snapshot::Snapshot> {
        self.session_queue.borrow().clone()
    }
    pub fn queue_changes(
        &self,
    ) -> watch::Receiver<Option<velo_player::queue::connect_snapshot::Snapshot>> {
        self.session_queue.subscribe()
    }
    fn observe_queue(self: &Arc<Self>) {
        let this = self.clone();
        let mut audio = this.audio.view.subscribe();
        self.rt.spawn(async move {
            loop {
                let Some(engine) = this.audio.engine() else {
                    this.session_queue.send_replace(None);
                    if audio.changed().await.is_err() { break; }
                    continue;
                };
                let observing = velo_player::queue::connect_snapshot::observe(engine.session(), &this.session_queue);
                tokio::pin!(observing);
                loop {
                    tokio::select! {
                        _ = &mut observing => { let _ = audio.changed().await; break; }
                        changed = audio.changed() => {
                            if changed.is_err() { return; }
                            if !this.audio.engine().is_some_and(|e| Arc::ptr_eq(&engine, &e)) { break; }
                        }
                    }
                }
                this.session_queue.send_replace(None);
            }
        });
    }

    /// The provider for Web API calls; `None` while signed out.
    pub fn api(&self) -> Option<Arc<dyn MusicProvider>> {
        if !self.view.borrow().signed_in {
            return None;
        }
        if let Some(api) = self.current_session_api() {
            return Some(api as Arc<dyn MusicProvider>);
        }
        self.provider
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .map(|p| p as Arc<dyn MusicProvider>)
    }

    pub fn snapshot(&self) -> AccountView {
        self.view.borrow().clone()
    }

    /// D3: metadatos de la misma respuesta de cola, sin cambiar MusicProvider ni QML.
    pub(super) async fn queue_snapshot(
        &self,
        own_engine: bool,
        shuffle: bool,
    ) -> velo_domain::Result<(u64, velo_spotify::QueueSnapshot)> {
        if own_engine {
            if let Some(engine) = self.audio.engine() {
                let (_, ticket) = self.api_ticket().ok_or(VeloError::NotAuthenticated)?;
                let snapshot = self
                    .session_queue
                    .borrow()
                    .clone()
                    .ok_or(VeloError::NotFound)?;
                let current = self
                    .audio
                    .state
                    .borrow()
                    .track
                    .as_ref()
                    .map(|t| t.uri.clone());
                if snapshot.shuffle != shuffle || current.as_deref() != Some(&snapshot.current) {
                    return Err(VeloError::NotFound);
                }
                let tracks =
                    velo_player::queue::connect_snapshot::describe(engine.session(), &snapshot)
                        .await;
                if !self
                    .session_queue
                    .borrow()
                    .as_ref()
                    .is_some_and(|latest| latest.same_queue(&snapshot))
                {
                    return Err(VeloError::NotFound);
                }
                return Ok((
                    ticket,
                    velo_spotify::QueueSnapshot {
                        current: Some(snapshot.current),
                        tracks,
                    },
                ));
            }
        }
        if own_engine {
            return Err(VeloError::NotFound);
        }
        let (provider, ticket) = {
            let work = self.work.lock().unwrap_or_else(|e| e.into_inner());
            if !self.view.borrow().signed_in {
                return Err(VeloError::NotAuthenticated);
            }
            let provider = self
                .current_session_api()
                .map(|api| api.web().clone())
                .or_else(|| self.current_provider())
                .ok_or(VeloError::NotAuthenticated)?;
            (provider, *work.generation.borrow())
        };
        Ok((ticket, provider.queue_snapshot().await?))
    }

    /// Captura la cuenta al invocar; logout/login invalidan también las escrituras pendientes.
    pub(crate) fn api_ticket(&self) -> Option<(Arc<dyn MusicProvider>, u64)> {
        let work = self.work.lock().unwrap_or_else(|e| e.into_inner());
        self.api().map(|api| (api, *work.generation.borrow()))
    }

    /// La comprobación y la publicación comparten la exclusión con invalidate_work.
    pub(crate) fn commit_if_current(&self, ticket: u64, commit: impl FnOnce()) -> bool {
        let work = self.work.lock().unwrap_or_else(|e| e.into_inner());
        if !work.current(ticket) || !self.view.borrow().signed_in {
            return false;
        }
        commit();
        true
    }

    #[cfg(test)]
    pub(super) fn set_test_provider(&self, provider: Arc<SpotifyProvider>) {
        self.invalidate_work();
        *self.provider.write().unwrap() = Some(provider);
        self.view.send_modify(|v| {
            v.signed_in = true;
            v.is_premium = true;
        });
    }

    #[cfg(test)]
    pub(super) fn set_test_session_provider(&self, provider: Arc<SpotifyProvider>) {
        self.set_test_provider(provider.clone());
        *self.session_api.write().unwrap() = Some(Arc::new(SessionApi::with_web(
            self.audio.session_source(),
            provider,
        )));
    }

    fn make_provider(self: &Arc<Self>, client_id: &str) -> Arc<SpotifyProvider> {
        let provider = Arc::new(SpotifyProvider::new(
            SpotifyConfig {
                client_id: client_id.to_string(),
                ..SpotifyConfig::default()
            },
            Arc::new(KeyringTokenStore::new()),
        ));
        *self.provider.write().unwrap_or_else(|e| e.into_inner()) = Some(provider.clone());
        self.observe_provider(&provider, client_id);
        provider
    }

    fn observe_provider(self: &Arc<Self>, provider: &Arc<SpotifyProvider>, id: &str) {
        let mut errors = provider.app_unavailable.subscribe();
        let this = Arc::downgrade(self);
        let provider = Arc::downgrade(provider);
        let _ = id;
        self.rt.spawn(async move {
            while let Ok(reason) = errors.recv().await {
                let (Some(this), Some(provider)) = (this.upgrade(), provider.upgrade()) else {
                    break;
                };
                if this
                    .current_provider()
                    .is_some_and(|p| Arc::ptr_eq(&p, &provider))
                {
                    this.view.send_modify(|v| {
                        v.app_warning = sign_in_message(&if reason == "quota" {
                            VeloError::RateLimited(Duration::ZERO)
                        } else {
                            VeloError::Other("invalid_client".into())
                        })
                    });
                }
            }
        });
    }

    fn current_session_api(&self) -> Option<Arc<SessionApi>> {
        self.session_api
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether the library comes from the librespot session (default mode).
    pub fn session_mode(&self) -> bool {
        self.current_session_api().is_some()
    }

    /// The session data provider, created once per sign-in.
    pub(crate) fn install_session_api(&self) -> Arc<SessionApi> {
        let mut slot = self.session_api.write().unwrap_or_else(|e| e.into_inner());
        if let Some(api) = slot.as_ref() {
            return api.clone();
        }
        let api = Arc::new(SessionApi::new(self.audio.session_source()));
        *slot = Some(api.clone());
        api
    }

    fn current_provider(&self) -> Option<Arc<SpotifyProvider>> {
        self.provider
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Enters by itself when the keyring already holds a session.
    pub fn start(self: &Arc<Self>) {
        let (generation, cancel) = {
            let work = self.work.lock().unwrap_or_else(|e| e.into_inner());
            if work.login {
                return;
            }
            (*work.generation.borrow(), work.generation.subscribe())
        };
        let this = self.clone();
        self.rt.spawn(async move {
            until_cancelled(cancel, generation, this.restore_profile(generation)).await;
        });
    }

    async fn restore_profile(self: &Arc<Self>, generation: u64) {
        let provider = {
            let _operation = self.operation.lock().await;
            if !self
                .work
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .current(generation)
            {
                return;
            }
            let provider: Arc<dyn MusicProvider> = match self.prefs.client_id() {
                Some(id) => {
                    let provider = self
                        .current_provider()
                        .unwrap_or_else(|| self.make_provider(&id));
                    if !provider.is_signed_in().await {
                        return;
                    }
                    provider
                }
                None => {
                    // One login: the stored librespot login is the account.
                    if !login::has_stored_login(login::keyring_store()).await {
                        return;
                    }
                    // Signed in right away; the profile arrives once the session connects.
                    self.view.send_modify(|v| v.signed_in = true);
                    self.install_session_api()
                }
            };
            provider
        };
        let result = profile_with_retry(
            || provider.me(),
            |e| {
                let work = self.work.lock().unwrap_or_else(|e| e.into_inner());
                if work.current(generation) {
                    tracing::warn!(error = %e, "profile unavailable at startup");
                    self.view.send_if_modified(|v| {
                        let changed = !v.signed_in;
                        v.signed_in = true;
                        changed
                    });
                }
            },
            tokio::time::sleep,
        )
        .await;
        let _operation = self.operation.lock().await;
        let work = self.work.lock().unwrap_or_else(|e| e.into_inner());
        if !work.current(generation) {
            return;
        }
        match result {
            Ok(profile) => self.apply_profile(profile),
            Err(e) => {
                if matches!(e, VeloError::Forbidden(_)) {
                    let _ = self.failures.send(sign_in_message(&e));
                }
                self.view.send_modify(|v| {
                    v.signed_in = false;
                    v.user_id.clear();
                    v.is_premium = false;
                });
            }
        }
    }

    /// `Account.signIn()`: the personal app when one is configured, otherwise the single
    /// Spotify login (librespot, audio + library).
    pub fn sign_in(self: &Arc<Self>) {
        match self.prefs.client_id() {
            Some(id) => self.authorize_client(id),
            None => self.sign_in_session(),
        }
    }

    /// `Account.useSpotifyAccount()`: leaves a personal app chosen in Ajustes and signs in with
    /// the Spotify account (one login). `velo.local.env` still wins on a developer's PC.
    pub fn use_spotify_account(self: &Arc<Self>) {
        let had_personal = self.prefs.settings().account.client_id.is_some();
        if had_personal {
            self.prefs.update_settings(|s| s.account.client_id = None);
        }
        let effective = self.prefs.client_id();
        self.view
            .send_modify(|v| v.client_id = effective.clone().unwrap_or_default());
        if effective.is_some() {
            if !self.snapshot().signed_in {
                self.sign_in();
            }
            return;
        }
        if self.current_provider().is_some() {
            // Tokens of the personal app are useless now; drop them, then the single login.
            self.invalidate_work();
            let this = self.clone();
            self.rt.spawn(async move {
                let operation = this.operation.lock().await;
                this.sign_out_now().await;
                drop(operation);
                this.sign_in_session();
            });
        } else if !self.snapshot().signed_in {
            self.sign_in_session();
        }
    }

    /// The single login: librespot's browser login starts the audio engine; its session is the
    /// account. Cancelling abandons the browser login.
    fn sign_in_session(self: &Arc<Self>) {
        let (generation, cancel) = {
            let mut work = self.work.lock().unwrap_or_else(|e| e.into_inner());
            let Some(login) = work.begin_login() else {
                return;
            };
            self.view.send_modify(|v| v.busy = true);
            login
        };
        let this = self.clone();
        self.rt.spawn(async move {
            let mut failures = this.audio.failures.subscribe();
            let ready = until_cancelled(cancel, generation, this.audio.ensure_ready()).await;
            let operation = this.operation.lock().await;
            {
                let mut work = this.work.lock().unwrap_or_else(|e| e.into_inner());
                if !work.current(generation) {
                    return;
                }
                work.login = false;
            }
            match ready {
                Some(true) => {
                    this.install_session_api();
                    this.view.send_modify(|v| {
                        v.signed_in = true;
                        v.app_warning.clear();
                    });
                }
                Some(false) => {
                    // The engine already toasted its reason; the login screen needs it too.
                    let reason = failures.try_recv().unwrap_or_else(|_| {
                        "No se pudo iniciar sesión en Spotify. Vuelve a intentarlo.".into()
                    });
                    let _ = this.failures.send(reason);
                }
                None => {}
            }
            this.view.send_modify(|v| v.busy = false);
            let recover = this.needs_profile();
            drop(operation);
            if recover {
                this.start();
            }
        });
    }

    pub fn authorize_client(self: &Arc<Self>, id: String) {
        let id = id.trim().to_string();
        if !valid_client_id(&id) {
            let _ = self.failures.send(
                "El Client ID debe tener 32 caracteres hexadecimales. No pegues el Client Secret."
                    .into(),
            );
            return;
        }
        let expected = self.snapshot().user_id;
        if self.snapshot().signed_in && expected.is_empty() {
            let _ = self.failures.send(
                "Espera a que se verifique tu cuenta antes de cambiar de app de Spotify.".into(),
            );
            return;
        }
        let (generation, cancel) = {
            let mut work = self.work.lock().unwrap_or_else(|e| e.into_inner());
            let Some(login) = work.begin_login() else {
                return;
            };
            self.view.send_modify(|v| v.busy = true);
            login
        };
        let this = self.clone();
        self.rt.spawn(async move {
            let result = until_cancelled(cancel, generation, async {
                let provider = {
                    let _operation = this.operation.lock().await;
                    if !this
                        .work
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .current(generation)
                    {
                        return Err(VeloError::NotAuthenticated);
                    }
                    Arc::new(SpotifyProvider::new(
                        SpotifyConfig {
                            client_id: id.clone(),
                            ..SpotifyConfig::default()
                        },
                        Arc::new(MemoryTokenStore::new()),
                    ))
                };
                // Dropping this future closes the OAuth listener and releases provider locks.
                let profile = provider.sign_in().await?;
                verify_account(&expected, &profile)?;
                Ok((profile, provider))
            })
            .await;
            let _operation = this.operation.lock().await;
            if !this
                .work
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .current(generation)
            {
                return;
            }
            // Fase de publicación: Cancelar sólo aborta el navegador, no una escritura
            // ya aceptada. Logout/cambio posterior se serializa con operation.
            this.work.lock().unwrap_or_else(|e| e.into_inner()).login = false;
            match result {
                Some(Ok((profile, candidate))) => {
                    let replacement = Arc::new(SpotifyProvider::new(
                        SpotifyConfig {
                            client_id: id.clone(),
                            ..SpotifyConfig::default()
                        },
                        Arc::new(KeyringTokenStore::new()),
                    ));
                    if let Some(old) = this.current_provider() {
                        old.retire().await;
                    }
                    match replacement.adopt_session_from(&candidate).await {
                        Ok(()) => {
                            if !this
                                .work
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .current(generation)
                            {
                                let _ = replacement.sign_out().await;
                                return;
                            }
                            *this.session_api.write().unwrap_or_else(|e| e.into_inner()) = None;
                            this.prefs
                                .update_settings(|s| s.account.client_id = Some(id.clone()));
                            *this.provider.write().unwrap_or_else(|e| e.into_inner()) =
                                Some(replacement.clone());
                            this.observe_provider(&replacement, &id);
                            this.view.send_modify(|v| {
                                v.client_id = id;
                                v.app_warning.clear();
                            });
                            this.apply_profile(profile);
                        }
                        Err(e) => {
                            let _ = this.failures.send(sign_in_message(&e));
                        }
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "sign in failed");
                    if matches!(e, VeloError::RateLimited(_))
                        || matches!(&e, VeloError::Other(m) if m == "invalid_client")
                    {
                        this.view
                            .send_modify(|v| v.app_warning = sign_in_message(&e));
                    }
                    let _ = this.failures.send(sign_in_message(&e));
                }
                None => {}
            }
            this.view.send_modify(|v| v.busy = false);
            let recover = this.needs_profile();
            drop(_operation);
            if recover {
                this.start();
            }
        });
    }

    pub fn cancel_sign_in(self: &Arc<Self>) {
        let mut work = self.work.lock().unwrap_or_else(|e| e.into_inner());
        if work.login {
            work.invalidate();
            self.audio.cancel_login();
            self.view.send_modify(|v| v.busy = false);
            let _ = self.failures.send(
                "Cancelaste el inicio de sesión. Puedes volver a intentarlo cuando quieras.".into(),
            );
            let recover = self.needs_profile();
            drop(work);
            if recover {
                self.start();
            }
        }
    }

    fn needs_profile(&self) -> bool {
        let view = self.view.borrow();
        view.signed_in && view.user_id.is_empty()
    }

    fn invalidate_work(&self) {
        self.work
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invalidate();
        self.view.send_modify(|v| v.busy = false);
    }

    pub fn sign_out(self: &Arc<Self>) {
        self.invalidate_work();
        let this = self.clone();
        self.rt.spawn(async move {
            let _operation = this.operation.lock().await;
            this.sign_out_now().await;
        });
    }

    async fn sign_out_now(&self) {
        if let Some(provider) = self.current_provider() {
            if let Err(e) = provider.sign_out().await {
                tracing::warn!(error = %e, "sign out");
            }
        } else {
            let _ = tokio::task::spawn_blocking(|| KeyringTokenStore::new().clear()).await;
        }
        *self.provider.write().unwrap_or_else(|e| e.into_inner()) = None;
        let session_api = self
            .session_api
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(api) = session_api {
            let _ = api.sign_out().await;
        }
        self.view.send_modify(|v| {
            *v = AccountView {
                client_id: v.client_id.clone(),
                ..AccountView::default()
            }
        });
    }

    pub fn set_client_id(self: &Arc<Self>, id: String) {
        if !id.trim().is_empty() && !valid_client_id(&id) {
            let _ = self
                .failures
                .send("El Client ID debe tener 32 caracteres hexadecimales.".into());
            return;
        }
        if id.trim() == self.view.borrow().client_id {
            return;
        }
        self.invalidate_work();
        let this = self.clone();
        self.rt.spawn(async move {
            let _operation = this.operation.lock().await;
            let id = id.trim().to_string();
            if id == this.view.borrow().client_id {
                return;
            }
            // Tokens belong to the old application; they cannot be refreshed with another id.
            this.sign_out_now().await;
            this.prefs.update_settings(|s| {
                s.account.client_id = (!id.is_empty()).then(|| id.clone());
            });
            // Volver al ID de entorno/archivo local/compilación al vaciar el campo.
            let effective = this.prefs.client_id().unwrap_or_default();
            let single_login = effective.is_empty();
            if !single_login {
                this.make_provider(&effective);
            } else {
                *this.provider.write().unwrap_or_else(|e| e.into_inner()) = None;
            }
            this.view.send_modify(|v| v.client_id = effective);
            drop(_operation);
            if single_login {
                // The stored librespot login (if any) is the account again.
                this.start();
            }
        });
    }

    fn apply_profile(self: &Arc<Self>, profile: UserProfile) {
        let avatar = pick_image(&profile.images, 96).map(|i| i.url.clone());
        self.view.send_modify(|v| {
            v.signed_in = true;
            v.display_name = profile.display_name.clone();
            v.is_premium = profile.product == "premium";
            v.user_id = profile.id.clone();
            v.avatar_url.clear();
        });
        {
            let audio = self.audio.clone();
            let id = profile.id.clone();
            self.rt
                .spawn(async move { audio.note_account_owner(&id).await });
        }
        if let Some(url) = avatar {
            let this = self.clone();
            self.rt.spawn(async move {
                if let Ok(path) = this.covers.thumbnail(&url, 96).await {
                    this.view.send_modify(|v| {
                        if v.signed_in && v.user_id == profile.id {
                            v.avatar_url = file_url(&path);
                        }
                    });
                }
            });
        }
    }
}

fn verify_account(expected: &str, profile: &UserProfile) -> velo_domain::Result<()> {
    if !expected.is_empty() && expected != profile.id {
        return Err(VeloError::Other("Autorizaste otra cuenta de Spotify. Cambia a tu cuenta actual en el navegador y vuelve a autorizar; tu sesión sigue intacta.".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn single_login_mode_serves_the_session_provider_and_sign_out_drops_it() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        let account = env.account.clone();
        assert!(!account.session_mode() && account.api().is_none());
        let api = account.install_session_api();
        // Idempotent: one provider per sign-in.
        assert!(Arc::ptr_eq(&api, &account.install_session_api()));
        account.view.send_modify(|v| v.signed_in = true);
        assert!(account.session_mode());
        assert!(account.api().is_some());
        // The session's 429s are not the user's problem: no toast in this mode.
        let mut errors = env.errors.subscribe();
        env.report(&VeloError::RateLimited(Duration::from_secs(30)));
        assert!(errors.try_recv().is_err());
        env.report(&VeloError::NotFound);
        assert!(errors.try_recv().is_ok());
        *account.session_api.write().unwrap() = None;
        account.view.send_modify(|v| v.signed_in = false);
        assert!(!account.session_mode() && account.api().is_none());
    }

    #[test]
    fn rate_limit_message_no_longer_blames_a_shared_quota() {
        let m = sign_in_message(&VeloError::RateLimited(Duration::from_secs(5)));
        assert!(!m.contains("cuota") && !m.contains("compartida"), "{m}");
    }

    #[test]
    fn personal_authorization_requires_the_current_account() {
        assert!(verify_account("", &profile()).is_ok());
        assert!(verify_account("juan", &profile()).is_ok());
        assert!(verify_account("otra", &profile()).is_err());
    }

    fn profile() -> UserProfile {
        UserProfile {
            id: "juan".into(),
            display_name: "Juan".into(),
            product: "premium".into(),
            images: vec![],
        }
    }

    #[tokio::test]
    async fn profile_recovers_offline_and_stops_retrying_after_success() {
        let mut responses: VecDeque<_> = (0..7)
            .map(|_| Err(VeloError::Network("offline".into())))
            .collect();
        responses.push_back(Ok(profile()));
        let mut delays = Vec::new();
        let mut failures = 0;
        let found = profile_with_retry(
            || std::future::ready(responses.pop_front().expect("no read after success")),
            |_| failures += 1,
            |delay| {
                delays.push(delay.as_secs());
                std::future::ready(())
            },
        )
        .await
        .unwrap();
        assert_eq!(found.id, "juan");
        assert_eq!(failures, 7);
        assert_eq!(delays, [2, 5, 15, 30, 60, 60, 60]);
        assert!(responses.is_empty());
    }

    #[tokio::test]
    async fn expired_session_has_no_profile_retry_timer() {
        let mut sleeps = 0;
        let result = profile_with_retry(
            || std::future::ready(Err(VeloError::NotAuthenticated)),
            |_| panic!("authentication failure must stop"),
            |_| {
                sleeps += 1;
                std::future::ready(())
            },
        )
        .await;
        assert!(matches!(result, Err(VeloError::NotAuthenticated)));
        assert_eq!(sleeps, 0);
    }

    #[test]
    fn cancellation_aborts_browser_wait_and_allows_immediate_retry() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, env) = super::super::test_env(&rt);
        let account = env.account.clone();
        let (old, cancel) = account.work.lock().unwrap().begin_login().unwrap();
        account.view.send_modify(|v| v.busy = true);
        let browser = rt.spawn(until_cancelled(
            cancel,
            old,
            std::future::pending::<UserProfile>(),
        ));
        rt.block_on(tokio::task::yield_now());
        assert!(account.operation.try_lock().is_ok());
        account.cancel_sign_in();
        assert!(!account.snapshot().busy);
        let (new, _) = account.work.lock().unwrap().begin_login().unwrap();
        assert_ne!(old, new);
        assert!(account.work.lock().unwrap().begin_login().is_none());
        assert!(rt.block_on(browser).unwrap().is_none());
        assert!(account.work.lock().unwrap().current(new));
    }

    #[tokio::test]
    async fn cancellation_before_first_poll_discards_even_a_ready_result() {
        let mut work = AuthWork::default();
        let (generation, cancel) = work.begin_login().unwrap();
        work.invalidate();
        assert!(
            until_cancelled(cancel, generation, std::future::ready(profile()))
                .await
                .is_none()
        );
    }

    #[test]
    fn login_forbidden_explains_owner_allowlist_without_echoing_response() {
        let message = sign_in_message(&VeloError::Forbidden("private response".into()));
        assert!(message.contains("dueño"));
        assert!(message.contains("User Management"));
        assert!(!message.contains("private response"));
    }

    #[tokio::test]
    async fn forbidden_profile_never_enters_library_or_retries() {
        let result = profile_with_retry(
            || std::future::ready(Err(VeloError::Forbidden("denied".into()))),
            |_| panic!("cuenta no autorizada no es un fallo transitorio"),
            |_| async { panic!("sin timer de reintento para 403") },
        )
        .await;
        assert!(matches!(result, Err(VeloError::Forbidden(_))));
    }

    #[test]
    fn profile_rate_limit_can_exceed_the_normal_retry_cap() {
        assert_eq!(
            profile_delay(0, &VeloError::RateLimited(Duration::from_secs(120))),
            Duration::from_secs(120)
        );
    }
}
