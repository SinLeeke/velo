//! Inicio adaptado a los datos que Velo ya carga, sin polling ni trabajo en Qt.
//! Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;
use velo_domain::{Artist, MusicProvider, Page, Playlist, Result, TimeRange, Track};
use velo_player::LoadTarget;

use super::{
    Env,
    images::Generation,
    rows::{THUMB_PX, file_url, image_of},
};

pub const HOME_ROLES: [&str; 4] = [
    "playlistId,name,description,imageUrl,owner",
    "kind,objectId,name,imageUrl,uri",
    super::rows::TRACK_ROLES,
    super::rows::TRACK_ROLES,
];
// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
pub const DISCOVER_TERMS: [&str; 4] = ["Discover Weekly", "Release Radar", "Daily Mix", "daylist"];

#[derive(Clone, Debug, PartialEq)]
pub struct HomeView {
    pub status: String,
    pub models: [Vec<Value>; 4],
}
impl Default for HomeView {
    fn default() -> Self {
        Self {
            status: "none".into(),
            models: Default::default(),
        }
    }
}

#[derive(Default)]
struct Inner {
    generation: u64,
    task: Option<tokio::task::AbortHandle>,
    made: Vec<Playlist>,
    playlists: Vec<Playlist>,
    recent: Vec<Track>,
    top: Vec<Track>,
    recommended: Vec<Track>,
    user: String,
    spotify_shortcuts: bool,
}

pub struct HomeCtl {
    env: Arc<Env>,
    player: Arc<super::player::PlayerCtl>,
    inner: Mutex<Inner>,
    images: [Arc<Generation>; 4],
    pub view: watch::Sender<HomeView>,
    #[cfg(test)]
    play_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
pub fn is_made_for_you(name: &str, term: &str) -> bool {
    let name = name.trim().to_lowercase();
    let term = term.to_lowercase();
    name == term
        || (term == "daily mix"
            && name
                .strip_prefix("daily mix ")
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())))
}

// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
fn matching_playlists(term: &str, playlists: Vec<Playlist>) -> Vec<Playlist> {
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    playlists
        .into_iter()
        .filter(|p| {
            !p.id.is_empty()
                && (p.owner_id == "spotify" || p.owner == "Spotify")
                && is_made_for_you(&p.name, term)
                && ids.insert(p.id.clone())
                && names.insert(p.name.trim().to_lowercase())
        })
        .take(6)
        .collect()
}

/// Una búsqueda por término; errores parciales conservan las playlists válidas.
// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
async fn made_for_you(api: &dyn MusicProvider) -> (Vec<Playlist>, bool) {
    let mut playlists = Vec::new();
    let mut failed = false;
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    // Secuencial para no multiplicar ráfagas sobre la cuota Web API.
    for term in DISCOVER_TERMS {
        match tokio::time::timeout(Duration::from_secs(15), api.search(term, 0, 10)).await {
            Ok(Ok(results)) => {
                for p in matching_playlists(term, results.playlists.items) {
                    if ids.insert(p.id.clone()) && names.insert(p.name.trim().to_lowercase()) {
                        playlists.push(p);
                    }
                }
            }
            _ => failed = true,
        }
    }
    order_numbered(&mut playlists);
    (playlists, failed)
}

/// «Daily Mix 4, 2, 3…» came in search order: numbered playlists sharing a name (Daily Mix N)
/// are put in number order among the slots they already take; everything else stays put.
fn order_numbered(playlists: &mut [Playlist]) {
    fn split(name: &str) -> Option<(String, u32)> {
        let name = name.trim();
        let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let prefix = name[..name.len() - digits].trim().to_lowercase();
        let number = name[name.len() - digits..].parse().ok()?;
        (!prefix.is_empty()).then_some((prefix, number))
    }
    let keys: Vec<Option<(String, u32)>> = playlists.iter().map(|p| split(&p.name)).collect();
    let mut prefixes: Vec<&String> = keys.iter().flatten().map(|(p, _)| p).collect();
    prefixes.dedup();
    for prefix in prefixes.into_iter().collect::<HashSet<_>>() {
        let slots: Vec<usize> = (0..playlists.len())
            .filter(|&i| keys[i].as_ref().is_some_and(|(p, _)| p == prefix))
            .collect();
        if slots.len() < 2 {
            continue;
        }
        let mut items: Vec<(u32, Playlist)> = slots
            .iter()
            .map(|&i| {
                (
                    keys[i].as_ref().map_or(0, |(_, n)| *n),
                    playlists[i].clone(),
                )
            })
            .collect();
        items.sort_by_key(|(n, _)| *n);
        for (&slot, (_, item)) in slots.iter().zip(items) {
            playlists[slot] = item;
        }
    }
}

/// Las tres lecturas ya existentes en loadHome, con top tracks de corto plazo (20).
// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
pub async fn home_reads(
    api: &dyn MusicProvider,
) -> (
    Result<Vec<Track>>,
    Result<Page<Artist>>,
    Result<Page<Track>>,
) {
    tokio::join!(
        api.recently_played(50),
        api.top_artists(TimeRange::MediumTerm, 0, 20),
        api.top_tracks(TimeRange::ShortTerm, 0, 20)
    )
}

/// Me gusta + primeras playlists; si faltan tiles, álbumes recientes sin pedir más datos.
// Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
fn quick_access(
    user: &str,
    playlists: &[Playlist],
    recent: &[Track],
) -> (Vec<Value>, Vec<Option<String>>) {
    if user.is_empty() {
        return Default::default();
    }
    let mut rows = vec![json!({"kind":"liked","objectId":"","name":"","imageUrl":"",
        "uri":format!("spotify:user:{user}:collection")})];
    let mut images = vec![None];
    let mut seen = HashSet::new();
    for p in playlists {
        if rows.len() == 8 {
            break;
        }
        if p.id.is_empty() || !seen.insert(p.uri.clone()) {
            continue;
        }
        rows.push(
            json!({"kind":"playlist","objectId":p.id,"name":p.name,"imageUrl":"","uri":p.uri}),
        );
        images.push(image_of(&p.images));
    }
    for t in recent {
        if rows.len() == 8 {
            break;
        }
        let Some(id) = t.album_id.as_ref().filter(|id| !id.is_empty()) else {
            continue;
        };
        let uri = format!("spotify:album:{id}");
        if !seen.insert(uri.clone()) {
            continue;
        }
        rows.push(json!({"kind":"album","objectId":id,"name":t.album,"imageUrl":"","uri":uri}));
        images.push(image_of(&t.images));
    }
    (rows, images)
}

impl HomeCtl {
    pub fn new(env: &Arc<Env>, player: &Arc<super::player::PlayerCtl>) -> Arc<Self> {
        Arc::new(Self {
            env: env.clone(),
            player: player.clone(),
            inner: Mutex::new(Inner::default()),
            images: std::array::from_fn(|_| Arc::new(Generation::default())),
            view: watch::channel(HomeView::default()).0,
            #[cfg(test)]
            play_task: Mutex::new(None),
        })
    }
    /// Cancela la búsqueda al navegar; conserva datos completos para abrir una playlist.
    pub fn leave(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.generation += 1;
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        self.view.send_modify(|v| {
            if v.status == "loading" {
                v.status = if inner.made.is_empty() {
                    "none"
                } else {
                    "ready"
                }
                .into();
            }
        });
    }
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        let generation = inner.generation + 1;
        *inner = Inner {
            generation,
            ..Inner::default()
        };
        for g in &self.images {
            g.bump();
        }
        self.view.send_replace(HomeView::default());
    }
    pub fn playlist(&self, id: &str) -> Option<Playlist> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .made
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }
    pub fn begin(self: &Arc<Self>, api: Arc<dyn MusicProvider>) -> u64 {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        inner.generation += 1;
        let ticket = inner.generation;
        self.view.send_modify(|v| v.status = "loading".into());
        let this = self.clone();
        let task = self.env.rt.spawn(async move {
            let (made, failed) = made_for_you(&*api).await;
            let mut inner = this.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.generation != ticket {
                return;
            }
            inner.made = made;
            let rows = inner
                .made
                .iter()
                .map(|p| {
                    json!({"playlistId":p.id,"name":p.name,
                "description":p.description,"imageUrl":"","owner":p.owner})
                })
                .collect();
            let images = inner.made.iter().map(|p| image_of(&p.images)).collect();
            this.view.send_modify(|v| {
                v.status = if !inner.made.is_empty() {
                    "ready"
                } else if failed {
                    "unavailable"
                } else {
                    "none"
                }
                .into()
            });
            this.replace_model(0, rows, images);
        });
        inner.task = Some(task.abort_handle());
        ticket
    }
    /// Llamar desde Tokio; comparte las playlists de biblioteca, incluidos los pins.
    pub fn set_playlists(self: &Arc<Self>, user: String, playlists: Vec<Playlist>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.user = user;
        inner.playlists = playlists;
        self.publish_quick(&inner);
    }
    pub fn load_spotify_shortcuts(self: &Arc<Self>) {
        let ticket = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation;
        let this = self.clone();
        self.env.rt.spawn(async move {
            let source = this.env.audio.session_source();
            let Some(session) = source.session().await else { return };
            let Ok(shortcuts) = velo_player::session_api::home::shortcuts(&session).await else { return };
            let mut inner = this.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.generation != ticket { return }
            inner.spotify_shortcuts = true;
            let rows = shortcuts.iter().map(|s| json!({"kind":s.kind,"objectId":s.object_id,"uri":s.uri,"name":s.name,"imageUrl":""})).collect();
            let images = shortcuts.iter().map(|s| image_of(&s.images)).collect();
            this.replace_model(1, rows, images);
        });
    }
    pub fn set_recent(self: &Arc<Self>, ticket: u64, recent: Vec<Track>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != ticket {
            return;
        }
        inner.recent = recent;
        self.publish_quick(&inner);
    }
    fn publish_quick(self: &Arc<Self>, inner: &Inner) {
        if inner.spotify_shortcuts {
            return;
        }
        let (rows, images) = quick_access(&inner.user, &inner.playlists, &inner.recent);
        self.replace_model(1, rows, images);
    }
    pub fn set_top(
        self: &Arc<Self>,
        ticket: u64,
        tracks: Vec<Track>,
        rows: Vec<Value>,
        images: Vec<Option<String>>,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != ticket {
            return;
        }
        inner.top = tracks;
        self.replace_model(2, rows, images);
    }
    /// G3: "Recomendado para ti" (radios de canciones semilla por la sesión).
    pub fn set_recommended(
        self: &Arc<Self>,
        ticket: u64,
        tracks: Vec<Track>,
        rows: Vec<Value>,
        images: Vec<Option<String>>,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != ticket {
            return;
        }
        inner.recommended = tracks;
        self.replace_model(3, rows, images);
    }
    /// U4: el perfil propio muestra estas mismas canciones (`topTracks`).
    pub fn has_top(&self) -> bool {
        !self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .top
            .is_empty()
    }
    /// U4: si se abrió el perfil antes de que Inicio recibiera sus canciones (la navegación
    /// invalida el ticket de Inicio), el perfil las pide y las deja aquí, si siguen faltando.
    pub fn adopt_top(
        self: &Arc<Self>,
        tracks: Vec<Track>,
        rows: Vec<Value>,
        images: Vec<Option<String>>,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.top.is_empty() || tracks.is_empty() {
            return;
        }
        inner.top = tracks;
        self.replace_model(2, rows, images);
    }
    /// Solo publica file URLs. Serializa miniaturas con clear/reemplazo para evitar filas viejas.
    fn replace_model(
        self: &Arc<Self>,
        model: usize,
        rows: Vec<Value>,
        images: Vec<Option<String>>,
    ) {
        let generation = self.images[model].bump();
        self.view.send_modify(|v| v.models[model] = rows);
        let permits = Arc::new(tokio::sync::Semaphore::new(4));
        for (row, url) in images.into_iter().enumerate() {
            let Some(url) = url else { continue };
            let this = self.clone();
            let permits = permits.clone();
            self.env.rt.spawn(async move {
                use velo_domain::CoverArtService;
                let Ok(_permit) = permits.acquire_owned().await else {
                    return;
                };
                if this.images[model].current() != generation {
                    return;
                }
                let Ok(path) = this.env.covers.thumbnail(&url, THUMB_PX).await else {
                    return;
                };
                let _inner = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                if this.images[model].current() != generation {
                    return;
                }
                this.view.send_modify(|v| {
                    if let Some(r) = v.models[model].get_mut(row) {
                        r["imageUrl"] = json!(file_url(&path));
                    }
                });
            });
        }
    }
    // Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
    pub fn play_top(self: &Arc<Self>, index: i32) {
        let uris = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            top_uris(&inner.top, index)
        };
        self.play_uris(uris);
    }
    /// G3: reproduce la fila y las siguientes de "Recomendado para ti".
    pub fn play_recommended(self: &Arc<Self>, index: i32) {
        let uris = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            top_uris(&inner.recommended, index)
        };
        self.play_uris(uris);
    }
    fn play_uris(self: &Arc<Self>, uris: Option<Vec<String>>) {
        let Some(uris) = uris else { return };
        let generation = self.player.begin_playback();
        let this = self.clone();
        let _task = self.env.rt.spawn(async move {
            if !this.player.playback_current(generation) {
                return;
            }
            let route = match this.player.prepare_playback().await {
                Ok(Some(route)) if this.player.playback_current(generation) => route,
                Ok(_) => return,
                Err(e) => return this.env.report(&e),
            };
            if route == crate::bridge::source::PlayRoute::Velo {
                if let Some(engine) = this.env.audio.engine() {
                    if let Err(e) = engine.load(LoadTarget::Tracks {
                        uris,
                        index: 0,
                        shuffle: false,
                    }) {
                        let _ = this.env.errors.send(super::audio::audio_message(&e));
                    }
                }
            } else if let Some(api) = this.env.account.api() {
                let result: Result<()> = async {
                    api.set_shuffle(false).await?;
                    if !this.player.playback_current(generation) {
                        return Ok(());
                    }
                    api.play_uris(&uris, None).await
                }
                .await;
                match result {
                    Ok(()) => this.player.resync(),
                    Err(e) => this.env.report(&e),
                }
            }
        });
        #[cfg(test)]
        {
            *self.play_task.lock().unwrap() = Some(_task);
        }
    }
}
fn top_uris(tracks: &[Track], index: i32) -> Option<Vec<String>> {
    let index = usize::try_from(index).ok()?;
    (index < tracks.len()).then(|| tracks[index..].iter().map(|t| t.uri.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::super::r4b_test_support::{ManualTime, Request, Server, runtime};
    use super::*;
    use velo_domain::ArtistRef;
    use velo_spotify::{MemoryTokenStore, SpotifyConfig, SpotifyProvider, TokenStore};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{path, query_param},
    };

    async fn playback_request(
        task: &mut tokio::task::JoinHandle<()>,
        server: &mut Server,
    ) -> Request {
        tokio::select! {
            request = server.next() => request,
            result = task => {
                result.unwrap();
                panic!("La reproducción terminó sin emitir la petición esperada");
            }
        }
    }

    async fn bounded_playback_test(test: impl std::future::Future<Output = ()>) {
        // ManualTime mantiene el reloj pausado incluso mientras el mock espera IO:
        // un timeout en ese mismo reloj tampoco vencería si falta una petición.
        // Este guardián solo limita un fallo; las respuestas gobiernan el orden del test.
        let (finished, completion) = std::sync::mpsc::channel::<()>();
        let mut watchdog =
            tokio::task::spawn_blocking(move || completion.recv_timeout(Duration::from_secs(3)));
        tokio::select! {
            () = test => drop(finished),
            result = &mut watchdog => panic!("El test de reproducción excedió 3 s: {result:?}"),
        }
        let _ = watchdog.await.unwrap();
    }

    async fn finish_with_mock(ctl: &HomeCtl, server: &mut Server) -> Vec<String> {
        let mut task = ctl.play_task.lock().unwrap().take().unwrap();
        let mut commands = Vec::new();
        loop {
            tokio::select! {
                result = &mut task => { result.unwrap(); return commands; }
                request = server.next() => {
                    commands.push(request.path.clone());
                    if request.path == "/me/player/devices" {
                        request.respond(200, json!({"devices":[{"id":"phone","name":"Teléfono","type":"Smartphone","is_active":true}]}));
                    } else {
                        request.respond(204, Value::Null);
                    }
                }
            }
        }
    }

    #[test]
    fn r4b_top_play_captures_before_first_poll_and_respects_later_pause_and_shuffle_wait() {
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        rt.block_on(bounded_playback_test(async {
            let _time = ManualTime::new();
            let mut server = Server::new().await;
            env.account.set_test_provider(server.provider());
            let player = super::super::player::PlayerCtl::for_test(&env);
            let ctl = HomeCtl::new(&env, &player);
            ctl.inner.lock().unwrap().top = vec![track("original", None), track("cola", None)];
            // Ninguna tarea ha tenido su primer poll: pausa es una intención posterior real.
            ctl.play_top(0);
            player.view.send_modify(|v| {
                v.can_control = true;
                v.is_playing = true;
            });
            player.play_pause();
            // Solo la pausa debe invalidar el play; otro begin_playback ocultaría un fallo.
            assert!(finish_with_mock(&ctl, &mut server).await.is_empty());
            assert!(!player.view.borrow().is_playing);
            assert!(server.requests.try_recv().is_err());

            ctl.play_top(0);
            let mut task = ctl.play_task.lock().unwrap().take().unwrap();
            ctl.inner.lock().unwrap().top = vec![track("recarga", None)];
            let devices = playback_request(&mut task, &mut server).await;
            assert_eq!(devices.path, "/me/player/devices");
            devices.respond(200, json!({"devices":[{"id":"phone","name":"Teléfono","type":"Smartphone","is_active":true}]}));
            let shuffle = playback_request(&mut task, &mut server).await;
            assert_eq!(shuffle.path, "/me/player/shuffle?state=false");
            assert!(!task.is_finished());
            shuffle.respond(204, Value::Null);
            let play = playback_request(&mut task, &mut server).await;
            assert_eq!(play.path, "/me/player/play");
            assert_eq!(play.body["uris"], json!(["spotify:track:original", "spotify:track:cola"]));
            play.respond(204, Value::Null);
            task.await.unwrap();

            ctl.play_top(0);
            let mut task = ctl.play_task.lock().unwrap().take().unwrap();
            let devices = playback_request(&mut task, &mut server).await;
            assert_eq!(devices.path, "/me/player/devices");
            devices.respond(200, json!({"devices":[{"id":"phone","name":"Teléfono","type":"Smartphone","is_active":true}]}));
            let shuffle = playback_request(&mut task, &mut server).await;
            assert_eq!(shuffle.path, "/me/player/shuffle?state=false");
            // Otra acción durante la espera del PUT shuffle cancela el comando play siguiente.
            let later = player.begin_playback();
            tokio::time::advance(Duration::from_secs(2)).await;
            assert!(!task.is_finished());
            shuffle.respond(204, Value::Null);
            task.await.unwrap();
            assert!(player.playback_current(later));
            assert!(server.requests.try_recv().is_err());
        }));
    }

    fn playlist(id: &str, name: &str) -> Playlist {
        Playlist {
            id: id.into(),
            uri: format!("spotify:playlist:{id}"),
            name: name.into(),
            description: String::new(),
            owner: "Spotify".into(),
            owner_id: "spotify".into(),
            collaborative: false,
            images: vec![],
            track_count: 3,
        }
    }
    fn track(id: &str, album: Option<&str>) -> Track {
        Track {
            id: Some(id.into()),
            uri: format!("spotify:track:{id}"),
            title: id.into(),
            artists: vec![ArtistRef {
                id: Some("artist".into()),
                name: "Artista".into(),
            }],
            album: album.unwrap_or_default().into(),
            album_id: album.map(str::to_string),
            duration_ms: 1000,
            explicit: false,
            images: vec![],
            added_at: None,
        }
    }
    async fn api(server: &MockServer) -> Arc<dyn MusicProvider> {
        let tokens = Arc::new(MemoryTokenStore::new());
        tokens.save("fixture-refresh").unwrap();
        Mock::given(path("/api/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token":"fixture-access","expires_in":3600})),
            )
            .expect(1)
            .mount(server)
            .await;
        Arc::new(SpotifyProvider::new(
            SpotifyConfig {
                client_id: "fixture".into(),
                api_base: server.uri(),
                accounts_base: server.uri(),
                redirect_port: 0,
            },
            tokens,
        ))
    }
    async fn search(
        server: &MockServer,
        term: &str,
        rows: Vec<Playlist>,
        status: u16,
        delay: Duration,
    ) {
        let items: Vec<_> = rows.into_iter().map(|p|json!({"id":p.id,"uri":p.uri,"name":p.name,
            "description":p.description,"owner":{"id":p.owner_id,"display_name":p.owner},"images":[]})).collect();
        Mock::given(path("/search"))
            .and(query_param("q", term))
            .and(query_param("limit", "10"))
            .and(query_param("offset", "0"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(
                        json!({"playlists":{"items":items,"total":10,"offset":0,"limit":10}}),
                    )
                    .set_delay(delay),
            )
            .expect(1)
            .mount(server)
            .await;
    }
    async fn idle(ctl: &HomeCtl) {
        let mut v = ctl.view.subscribe();
        tokio::time::timeout(Duration::from_secs(4), async {
            while v.borrow_and_update().status == "loading" {
                v.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn names_owners_deduplication_and_per_term_limit() {
        for (name, term, expected) in [
            (" Discover Weekly ", "Discover Weekly", true),
            ("release radar", "Release Radar", true),
            ("daylist", "daylist", true),
            ("Daily Mix 3", "Daily Mix", true),
            ("Daily Mix", "Daily Mix", true),
            ("Daily Mix Radio", "Daily Mix", false),
            ("Daily Mix ٣", "Daily Mix", false),
            ("This Is Discover Weekly", "Discover Weekly", false),
            ("Discover Weekly Mix", "Discover Weekly", false),
            ("Release Radar Radio", "Release Radar", false),
        ] {
            assert_eq!(is_made_for_you(name, term), expected);
        }
        let mut impostor = playlist("fake", "Daily Mix 1");
        impostor.owner = "Persona".into();
        impostor.owner_id = "persona".into();
        let mut lists = vec![
            impostor,
            playlist("one", "Daily Mix 1"),
            playlist("dup", "daily mix 1"),
        ];
        lists.extend((2..=9).map(|i| playlist(&format!("p{i}"), &format!("Daily Mix {i}"))));
        let found = matching_playlists("Daily Mix", lists);
        assert_eq!(found.len(), 6);
        assert_eq!(found[0].id, "one");
        assert_eq!(found[5].name, "Daily Mix 6");
    }

    #[test]
    fn quick_access_liked_playlists_then_unique_recent_albums_no_extra_network() {
        let playlists = vec![
            playlist("p1", "Uno"),
            playlist("p1", "Uno"),
            playlist("p2", "Dos"),
        ];
        let recent = vec![
            track("t1", Some("a1")),
            track("t2", Some("a1")),
            track("t3", None),
            track("t4", Some("a2")),
        ];
        let (rows, images) = quick_access("juan", &playlists, &recent);
        assert_eq!(rows.len(), 5);
        assert_eq!(images.len(), 5);
        assert_eq!(rows[0]["kind"], "liked");
        assert_eq!(rows[0]["uri"], "spotify:user:juan:collection");
        assert_eq!(rows[1]["objectId"], "p1");
        assert_eq!(rows[3]["objectId"], "a1");
        for row in &rows {
            for role in HOME_ROLES[1].split(',') {
                assert!(row.get(role).is_some());
            }
        }
        let many: Vec<_> = (0..20).map(|i| playlist(&i.to_string(), "P")).collect();
        assert_eq!(quick_access("juan", &many, &recent).0.len(), 8);
        assert!(quick_access("", &many, &recent).0.is_empty());
        assert_eq!(
            top_uris(&recent, 1).unwrap(),
            ["spotify:track:t2", "spotify:track:t3", "spotify:track:t4"]
        );
        assert!(top_uris(&recent, -1).is_none());
        assert!(top_uris(&recent, 4).is_none());
    }

    #[test]
    fn daily_mixes_follow_their_number_and_others_stay_put() {
        let named = |n: &str| playlist(n, n);
        let mut list: Vec<Playlist> = [
            "Release Radar",
            "Daily Mix 4",
            "Daily Mix 2",
            "Daily Mix 3",
            "Daily Mix 6",
            "Daily Mix 5",
            "daylist",
        ]
        .into_iter()
        .map(named)
        .collect();
        order_numbered(&mut list);
        let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Release Radar",
                "Daily Mix 2",
                "Daily Mix 3",
                "Daily Mix 4",
                "Daily Mix 5",
                "Daily Mix 6",
                "daylist"
            ]
        );
    }
    #[test]
    fn made_for_you_partial_failure_ready_and_cached_header_for_f4() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, env) = super::super::test_env(&rt);
            let server = MockServer::start().await;
            let api = api(&server).await;
            for term in DISCOVER_TERMS {
                search(
                    &server,
                    term,
                    vec![playlist(term, term)],
                    if term == "Release Radar" { 403 } else { 200 },
                    Duration::ZERO,
                )
                .await;
            }
            let ctl = HomeCtl::new(&env, &super::super::player::PlayerCtl::start(&env));
            ctl.begin(api);
            assert_eq!(ctl.view.borrow().status, "loading");
            idle(&ctl).await;
            assert_eq!(ctl.view.borrow().status, "ready");
            assert_eq!(ctl.view.borrow().models[0].len(), 3);
            let header = ctl.playlist("Discover Weekly").unwrap();
            assert!(!header.items_listable_by("juan"));
            for row in &ctl.view.borrow().models[0] {
                assert_eq!(row["imageUrl"], "");
                for role in HOME_ROLES[0].split(',') {
                    assert!(row.get(role).is_some());
                }
            }
            ctl.leave();
            assert!(ctl.playlist("Discover Weekly").is_some());
            ctl.clear();
            assert!(ctl.playlist("Discover Weekly").is_none());
            assert!(ctl.view.borrow().models.iter().all(Vec::is_empty));
        });
    }

    #[test]
    fn empty_success_none_failure_unavailable() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, env) = super::super::test_env(&rt);
            let ctl = HomeCtl::new(&env, &super::super::player::PlayerCtl::start(&env));
            for (status, expected) in [(200, "none"), (403, "unavailable")] {
                let server = MockServer::start().await;
                let api = api(&server).await;
                for term in DISCOVER_TERMS {
                    search(&server, term, vec![], status, Duration::ZERO).await;
                }
                ctl.begin(api);
                idle(&ctl).await;
                assert_eq!(ctl.view.borrow().status, expected);
            }
        });
    }

    #[test]
    fn clear_and_leave_discard_inflight_response_and_old_core_rows() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, env) = super::super::test_env(&rt);
            let server = MockServer::start().await;
            let api = api(&server).await;
            search(
                &server,
                DISCOVER_TERMS[0],
                vec![playlist("old", "Discover Weekly")],
                200,
                Duration::from_millis(200),
            )
            .await;
            let ctl = HomeCtl::new(&env, &super::super::player::PlayerCtl::start(&env));
            let ticket = ctl.begin(api);
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|r| r.url.path() == "/search")
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            ctl.leave();
            assert_eq!(ctl.view.borrow().status, "none");
            ctl.clear();
            ctl.set_recent(ticket, vec![track("old", Some("old"))]);
            ctl.set_top(
                ticket,
                vec![track("old", None)],
                vec![json!({"uri":"old"})],
                vec![None],
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert_eq!(ctl.view.borrow().status, "none");
            assert!(ctl.view.borrow().models.iter().all(Vec::is_empty));
            assert!(ctl.playlist("old").is_none());
        });
    }

    #[tokio::test]
    async fn existing_reads_are_once_with_short_term_top_tracks() {
        let server = MockServer::start().await;
        let api = api(&server).await;
        for (endpoint, limit, range) in [
            ("/me/player/recently-played", "50", None),
            ("/me/top/artists", "20", Some("medium_term")),
            ("/me/top/tracks", "20", Some("short_term")),
        ] {
            let mut mock = Mock::given(path(endpoint)).and(query_param("limit", limit));
            if let Some(range) = range {
                mock = mock.and(query_param("time_range", range));
            }
            mock.respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"items":[],"limit":20,"offset":0,"total":0})),
            )
            .expect(1)
            .mount(&server)
            .await;
        }
        let (recent, artists, tracks) = home_reads(&*api).await;
        assert!(recent.unwrap().is_empty());
        assert!(artists.unwrap().items.is_empty());
        assert!(tracks.unwrap().items.is_empty());
    }
}
