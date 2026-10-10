//! F1/F2: búsqueda con tareas cancelables y publicación por generación. Sin Timer de Qt.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{sync::watch, task::AbortHandle};
use velo_domain::{CoverArtService, MusicProvider, SearchResults, VeloError};
use velo_player::LoadTarget;

use super::{
    Env,
    player::PlayerCtl,
    rows::{file_url, image_of},
};
use crate::{
    bridge::source::{PlayRoute, activate_device, retry_device_action},
    model::ModelOp,
};

pub const SEARCH_ROLES: &str = "kind,objectId,uri,title,subtitle,imageUrl,artistId,albumId,album,year,durationMs,following,followingKnown,followingBusy";
pub const MODELS: usize = 7;
pub const PAGE: u32 = 10;
pub const DEBOUNCE: Duration = Duration::from_millis(250);
const SUGGESTIONS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct SearchView {
    pub generation: u64,
    pub query: String,
    pub suggestion_query: String,
    pub kind: String,
    pub loading: bool,
    pub suggesting: bool,
    pub has_more: bool,
    pub error: String,
    pub suggestion_error: String,
    /// Sugerencias, resultados filtrados, canciones, artistas, playlists, álbumes, mejor.
    pub models: [Vec<Value>; MODELS],
    pub followed_artist_id: String,
    pub artist_followed: bool,
    pub artist_followed_known: bool,
    pub artist_follow_busy: bool,
}
impl Default for SearchView {
    fn default() -> Self {
        Self {
            generation: 0,
            query: String::new(),
            suggestion_query: String::new(),
            kind: "all".into(),
            loading: false,
            suggesting: false,
            has_more: false,
            error: String::new(),
            suggestion_error: String::new(),
            models: Default::default(),
            followed_artist_id: String::new(),
            artist_followed: false,
            artist_followed_known: false,
            artist_follow_busy: false,
        }
    }
}

#[derive(Clone)]
struct Hit {
    row: Value,
    image: Option<String>,
}

fn hit(
    kind: &str,
    id: &str,
    uri: &str,
    title: &str,
    subtitle: String,
    image: Option<String>,
) -> Hit {
    Hit {
        row: json!({"kind":kind,"objectId":id,"uri":uri,"title":title,"subtitle":subtitle,
        "imageUrl":"","artistId":"","albumId":"","album":"","year":0,"durationMs":0,
        "following":false,"followingKnown":false,"followingBusy":false}),
        image,
    }
}

/// Orden intercalado por rango de Spotify, conservando el orden dentro de cada tipo.
fn hits(results: &SearchResults) -> Vec<Hit> {
    let mut groups: [Vec<Hit>; 4] = Default::default();
    groups[0] = results
        .tracks
        .items
        .iter()
        .map(|t| {
            let mut h = hit(
                "track",
                t.id.as_deref().unwrap_or_default(),
                &t.uri,
                &t.title,
                t.artist_line(),
                image_of(&t.images),
            );
            h.row["artistId"] = json!(
                t.artists
                    .first()
                    .and_then(|a| a.id.as_deref())
                    .unwrap_or_default()
            );
            h.row["albumId"] = json!(t.album_id.as_deref().unwrap_or_default());
            h.row["album"] = json!(t.album);
            h.row["durationMs"] = json!(t.duration_ms);
            h
        })
        .collect();
    groups[1] = results
        .artists
        .items
        .iter()
        .map(|a| {
            let mut h = hit(
                "artist",
                &a.id,
                &a.uri,
                &a.name,
                String::new(),
                image_of(&a.images),
            );
            h.row["artistId"] = json!(a.id);
            h
        })
        .collect();
    groups[2] = results
        .playlists
        .items
        .iter()
        .map(|p| {
            hit(
                "playlist",
                &p.id,
                &p.uri,
                &p.name,
                p.owner.clone(),
                image_of(&p.images),
            )
        })
        .collect();
    groups[3] = results
        .albums
        .items
        .iter()
        .map(|a| {
            let mut h = hit(
                "album",
                &a.id,
                &a.uri,
                &a.name,
                a.artists
                    .iter()
                    .map(|x| x.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                image_of(&a.images),
            );
            h.row["albumId"] = json!(a.id);
            h.row["year"] = json!(super::rows::release_year(&a.release_date));
            h
        })
        .collect();
    let mut out = Vec::new();
    for i in 0..groups.iter().map(Vec::len).max().unwrap_or(0) {
        for group in &groups {
            if let Some(h) = group.get(i) {
                out.push(h.clone());
            }
        }
    }
    out
}

/// Web API no ofrece «mejor resultado». Coincidencia exacta > prefijo > resto;
/// empate: artista, canción, álbum, playlist; luego el orden del servidor.
pub fn best_index(query: &str, rows: &[Value]) -> Option<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    rows.iter()
        .enumerate()
        .min_by_key(|(_, r)| {
            let title = r["title"].as_str().unwrap_or_default().to_lowercase();
            let score = if title == q {
                0
            } else if title.starts_with(&q) {
                1
            } else {
                2
            };
            let priority = match r["kind"].as_str() {
                Some("artist") => 0,
                Some("track") => 1,
                Some("album") => 2,
                _ => 3,
            };
            (score, priority)
        })
        .map(|(i, _)| i)
}

pub fn valid_type(kind: &str) -> bool {
    matches!(kind, "all" | "track" | "artist" | "playlist" | "album")
}
fn type_model(kind: &str) -> Option<usize> {
    match kind {
        "track" => Some(2),
        "artist" => Some(3),
        "playlist" => Some(4),
        "album" => Some(5),
        _ => None,
    }
}

pub fn row_changes(rows: &[Value], old: &[Value]) -> Vec<ModelOp> {
    row_changes_for(
        rows,
        old,
        &[
            "kind",
            "objectId",
            "uri",
            "title",
            "subtitle",
            "artistId",
            "albumId",
            "album",
            "year",
            "durationMs",
        ],
        &["imageUrl", "following", "followingKnown", "followingBusy"],
    )
}

/// Diferencias según los roles del modelo que recibe la publicación.
pub(super) fn row_changes_for(
    rows: &[Value],
    old: &[Value],
    fixed: &[&str],
    fields: &[&'static str],
) -> Vec<ModelOp> {
    if rows.len() < old.len()
        || rows
            .iter()
            .zip(old)
            .any(|(a, b)| fixed.iter().any(|key| a[*key] != b[*key]))
    {
        return vec![ModelOp::Replace(rows.to_vec())];
    }
    let mut changes = Vec::new();
    for (row, (a, b)) in rows.iter().zip(old).enumerate() {
        for &field in fields {
            if a[field] != b[field] {
                changes.push(ModelOp::SetField {
                    row,
                    field,
                    value: a[field].clone(),
                });
            }
        }
    }
    if rows.len() > old.len() {
        changes.push(ModelOp::Append(rows[old.len()..].to_vec()));
    }
    changes
}

#[derive(Default)]
struct Slot {
    generation: u64,
    task: Option<AbortHandle>,
}
impl Slot {
    fn cancel(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.generation
    }
}
#[derive(Default)]
struct Follow {
    version: u64,
    known: Option<bool>,
    busy: bool,
}
#[derive(Default)]
struct Inner {
    suggest: Slot,
    full: Slot,
    results: SearchResults,
    next_offset: u32,
    more: [bool; 4], // track, artist, playlist, album
    follows: HashMap<String, Follow>,
    account_generation: u64,
}

pub struct SearchCtl {
    env: Arc<Env>,
    player: Arc<PlayerCtl>,
    /// Mutex serializa cambio de generación y publicación; nunca se mantiene durante await.
    inner: Mutex<Inner>,
    pub view: watch::Sender<SearchView>,
    image_permits: Arc<tokio::sync::Semaphore>,
    #[cfg(test)]
    test_api: Mutex<Option<Arc<dyn MusicProvider>>>,
}

impl SearchCtl {
    pub fn new(env: &Arc<Env>, player: &Arc<PlayerCtl>) -> Arc<Self> {
        Arc::new(Self {
            env: env.clone(),
            player: player.clone(),
            inner: Mutex::new(Inner::default()),
            view: watch::channel(SearchView::default()).0,
            image_permits: Arc::new(tokio::sync::Semaphore::new(4)),
            #[cfg(test)]
            test_api: Mutex::new(None),
        })
    }
    fn api(&self) -> Option<Arc<dyn MusicProvider>> {
        #[cfg(test)]
        if let Some(api) = self
            .test_api
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Some(api);
        }
        self.env.account.api()
    }
    pub fn clear(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.suggest.cancel();
        state.full.cancel();
        state.results = SearchResults::default();
        state.follows.clear();
        state.account_generation = state.account_generation.wrapping_add(1);
        self.view.send_replace(SearchView::default());
    }
    pub fn is_current(&self, view: &SearchView) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .full
            .generation
            == view.generation
    }
    /// Navegar cancela búsquedas; conserva escrituras de seguimiento aún en vuelo.
    pub fn leave(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.suggest.cancel();
        state.full.cancel();
        state.results = SearchResults::default();
        state.more = [false; 4];
        state.next_offset = 0;
        self.view.send_replace(SearchView::default());
    }
    pub fn cancel_suggestions(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.suggest.cancel();
        self.view.send_modify(|v| {
            v.models[0].clear();
            v.suggesting = false;
            v.suggestion_query.clear();
            v.suggestion_error.clear();
        });
    }
    pub fn suggest(self: &Arc<Self>, query: String) {
        let query = query.trim().to_string();
        if query.is_empty() {
            self.cancel_suggestions();
            return;
        }
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ticket = state.suggest.cancel();
        // U4: las filas anteriores siguen visibles hasta que llegan las de la consulta nueva
        // (como Spotify): vaciarlas en cada tecla cerraba y reabría el desplegable.
        self.view.send_modify(|v| {
            v.suggestion_query = query.clone();
            v.suggesting = true;
            v.suggestion_error.clear();
        });
        let this = self.clone();
        let task = self.env.rt.spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            let result = match this.api() {
                Some(api) => api.search(&query, 0, PAGE).await,
                None => Err(VeloError::NotAuthenticated),
            };
            let mut items = result.as_ref().map(hits).unwrap_or_default();
            items.truncate(SUGGESTIONS);
            {
                let state = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                if state.suggest.generation != ticket {
                    return;
                }
                this.view.send_modify(|v| {
                    v.suggesting = false;
                    v.models[0] = items.iter().map(|h| h.row.clone()).collect();
                    v.suggestion_error = result
                        .as_ref()
                        .err()
                        .map(super::user_message)
                        .unwrap_or_default();
                    update_follow_rows(v, &state.follows);
                });
            }
            this.fill_images(ticket, true, items);
        });
        state.suggest.task = Some(task.abort_handle());
    }
    pub fn submit(self: &Arc<Self>, query: String) {
        self.cancel_suggestions();
        let query = query.trim().to_string();
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ticket = state.full.cancel();
        state.results = SearchResults::default();
        state.next_offset = 0;
        state.more = [false; 4];
        self.view.send_modify(|v| {
            v.generation = ticket;
            v.query = query.clone();
            v.kind = "all".into();
            v.loading = !query.is_empty();
            v.error.clear();
            v.has_more = false;
            for rows in &mut v.models[1..] {
                rows.clear();
            }
        });
        if !query.is_empty() {
            self.launch_full(&mut state, ticket, query, 0);
        }
    }
    fn launch_full(self: &Arc<Self>, state: &mut Inner, ticket: u64, query: String, offset: u32) {
        let this = self.clone();
        let task = self.env.rt.spawn(async move {
            let result = match this.api() {
                Some(api) => api.search(&query, offset, PAGE).await,
                None => Err(VeloError::NotAuthenticated),
            };
            let items = {
                let mut state = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                if state.full.generation != ticket {
                    return;
                }
                match result {
                    Ok(page) => {
                        state.more = [
                            page.tracks.has_more(),
                            page.artists.has_more(),
                            page.playlists.has_more(),
                            page.albums.has_more(),
                        ];
                        state.next_offset = offset + PAGE;
                        state.results.tracks.items.extend(page.tracks.items);
                        state.results.artists.items.extend(page.artists.items);
                        state.results.playlists.items.extend(page.playlists.items);
                        state.results.albums.items.extend(page.albums.items);
                        let mut items = hits(&state.results);
                        this.view.send_modify(|v| {
                            // Mantener miniaturas ya obtenidas al paginar; solo descargar las nuevas.
                            let images: HashMap<_, _> = v.models[2..6]
                                .iter()
                                .flatten()
                                .filter_map(|r| {
                                    let uri = r["uri"].as_str()?;
                                    let url = r["imageUrl"].as_str()?;
                                    if url.is_empty() {
                                        None
                                    } else {
                                        Some((uri.to_string(), url.to_string()))
                                    }
                                })
                                .collect();
                            for h in &mut items {
                                if let Some(url) = h.row["uri"].as_str().and_then(|u| images.get(u))
                                {
                                    h.row["imageUrl"] = json!(url);
                                    h.image = None;
                                }
                            }
                            v.loading = false;
                            v.error.clear();
                            for rows in &mut v.models[2..6] {
                                rows.clear();
                            }
                            for h in &items {
                                if let Some(i) = h.row["kind"].as_str().and_then(type_model) {
                                    v.models[i].push(h.row.clone());
                                }
                            }
                            let rows: Vec<_> = items.iter().map(|h| h.row.clone()).collect();
                            v.models[6] = best_index(&v.query, &rows)
                                .map(|i| vec![rows[i].clone()])
                                .unwrap_or_default();
                            rebuild_filtered(v, &state);
                            update_follow_rows(v, &state.follows);
                        });
                        items
                    }
                    Err(e) => {
                        this.view.send_modify(|v| {
                            v.loading = false;
                            v.error = super::user_message(&e);
                        });
                        Vec::new()
                    }
                }
            };
            this.fill_images(ticket, false, items);
        });
        state.full.task = Some(task.abort_handle());
    }
    pub fn set_type(self: &Arc<Self>, kind: String) {
        if !valid_type(&kind) {
            return;
        }
        let this = self.clone();
        let ticket = {
            let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            self.view.send_modify(|v| {
                v.kind = kind.clone();
                v.has_more = has_more(&state, &kind);
            });
            state.full.generation
        };
        self.env.rt.spawn(async move {
            let state = this.inner.lock().unwrap_or_else(|e| e.into_inner());
            if state.full.generation != ticket {
                return;
            }
            this.view.send_modify(|v| {
                if v.kind == kind {
                    rebuild_filtered(v, &state);
                }
            });
        });
    }
    pub fn more(self: &Arc<Self>) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let (query, loading, more) = {
            let v = self.view.borrow();
            (v.query.clone(), v.loading, v.has_more)
        };
        if loading || !more || state.next_offset >= 1000 {
            return;
        }
        let ticket = state.full.generation;
        let offset = state.next_offset;
        self.view.send_modify(|v| v.loading = true);
        self.launch_full(&mut state, ticket, query, offset);
    }
    fn fill_images(self: &Arc<Self>, ticket: u64, suggest: bool, items: Vec<Hit>) {
        for item in items {
            let Some(url) = item.image else {
                continue;
            };
            let this = self.clone();
            let uri = item.row["uri"].clone();
            self.env.rt.spawn(async move {
                let Ok(_permit) = this.image_permits.clone().acquire_owned().await else {
                    return;
                };
                {
                    let state = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                    if (if suggest {
                        state.suggest.generation
                    } else {
                        state.full.generation
                    }) != ticket
                    {
                        return;
                    }
                }
                let Ok(path) = this.env.covers.thumbnail(&url, 128).await else {
                    return;
                };
                let state = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                if (if suggest {
                    state.suggest.generation
                } else {
                    state.full.generation
                }) != ticket
                {
                    return;
                }
                this.view.send_modify(|v| {
                    let models = if suggest {
                        &mut v.models[..1]
                    } else {
                        &mut v.models[1..]
                    };
                    for rows in models {
                        for row in rows {
                            if row["uri"] == uri {
                                row["imageUrl"] = json!(file_url(&path));
                            }
                        }
                    }
                });
            });
        }
    }
    pub fn play(self: &Arc<Self>, index: i32, best: bool) {
        let row = {
            let v = self.view.borrow();
            if best {
                v.models[6].first()
            } else {
                usize::try_from(index).ok().and_then(|i| v.models[1].get(i))
            }
            .cloned()
        };
        let Some(row) = row else {
            return;
        };
        let Some(uri) = row["uri"]
            .as_str()
            .filter(|u| velo_player::discover::validate_seed(u).is_ok())
            .map(str::to_string)
        else {
            return;
        };
        let kind = row["kind"].as_str().unwrap_or_default().to_string();
        let this = self.clone();
        let generation = self.player.begin_playback();
        self.env.rt.spawn(async move {
            let route = match this.player.prepare_playback().await {
                Ok(Some(route)) if this.player.playback_current(generation) => route,
                Ok(_) => return,
                Err(e) => {
                    this.env.report(&e);
                    return;
                }
            };
            if route == PlayRoute::Velo {
                let Some(engine) = this.env.audio.engine() else {
                    return;
                };
                let target = if kind == "track" {
                    LoadTarget::Tracks {
                        uris: vec![uri],
                        index: 0,
                        shuffle: false,
                    }
                } else {
                    LoadTarget::Context {
                        uri,
                        index: None,
                        shuffle: false,
                    }
                };
                if let Err(e) = engine.load(target) {
                    let _ = this.env.errors.send(super::audio::audio_message(&e));
                }
            } else if let Some(api) = this.api() {
                let call = || {
                    let api = api.clone();
                    let uri = uri.clone();
                    let kind = kind.clone();
                    async move {
                        if kind == "track" {
                            api.play_uris(&[uri], None).await
                        } else {
                            api.play_context(&uri, None).await
                        }
                    }
                };
                let result = match call().await {
                    Err(VeloError::NoActiveDevice) => {
                        match activate_device(&this.env, &*api, true).await {
                            Ok(()) => retry_device_action(call).await,
                            Err(e) => Err(e),
                        }
                    }
                    other => other,
                };
                if let Err(e) = result {
                    this.env.report(&e);
                }
            }
        });
    }
    pub fn inspect_followed(self: &Arc<Self>, id: String) {
        self.follow(id, None);
    }
    pub fn set_followed(self: &Arc<Self>, id: String, on: bool) {
        self.follow(id, Some(on));
    }
    fn follow(self: &Arc<Self>, id: String, on: Option<bool>) {
        let uri = format!("spotify:artist:{id}");
        if velo_player::discover::validate_seed(&uri).is_err() {
            return;
        }
        let api = self.api();
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let epoch = state.account_generation;
        let follow = state.follows.entry(id.clone()).or_default();
        if follow.busy {
            self.view.send_modify(|v| {
                v.followed_artist_id = id.clone();
                update_follow_rows(v, &state.follows);
            });
            return;
        }
        follow.version = follow.version.wrapping_add(1);
        let ticket = follow.version;
        follow.busy = true;
        self.view.send_modify(|v| {
            v.followed_artist_id = id.clone();
            update_follow_rows(v, &state.follows);
        });
        let this = self.clone();
        self.env.rt.spawn(async move {
            {
                let state = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                if state.account_generation != epoch {
                    return;
                }
            }
            let result = match api {
                Some(api) => match on {
                    Some(on) => api.set_saved(&uri, on).await.map(|()| on),
                    None => api.is_saved(&uri).await,
                },
                None => Err(VeloError::NotAuthenticated),
            };
            let mut state = this.inner.lock().unwrap_or_else(|e| e.into_inner());
            if state.account_generation != epoch {
                return;
            }
            let Some(follow) = state.follows.get_mut(&id).filter(|f| f.version == ticket) else {
                return;
            };
            follow.busy = false;
            match result {
                Ok(saved) => follow.known = Some(saved),
                Err(e) => this.env.report(&e),
            }
            this.view
                .send_modify(|v| update_follow_rows(v, &state.follows));
        });
    }
}

fn rebuild_filtered(v: &mut SearchView, state: &Inner) {
    if let Some(model) = type_model(&v.kind) {
        v.models[1] = v.models[model].clone();
    } else {
        let mut rows = Vec::new();
        for i in 0..v.models[2..6].iter().map(Vec::len).max().unwrap_or(0) {
            for group in &v.models[2..6] {
                if let Some(row) = group.get(i) {
                    rows.push(row.clone());
                }
            }
        }
        v.models[1] = rows;
    }
    v.has_more = has_more(state, &v.kind);
}
fn has_more(state: &Inner, kind: &str) -> bool {
    state.next_offset < 1000
        && match kind {
            "track" => state.more[0],
            "artist" => state.more[1],
            "playlist" => state.more[2],
            "album" => state.more[3],
            _ => state.more.iter().any(|x| *x),
        }
}
fn update_follow_rows(v: &mut SearchView, follows: &HashMap<String, Follow>) {
    for rows in &mut v.models {
        for row in rows {
            if row["kind"] != "artist" {
                continue;
            }
            if let Some(f) = row["objectId"].as_str().and_then(|id| follows.get(id)) {
                row["following"] = json!(f.known.unwrap_or(false));
                row["followingKnown"] = json!(f.known.is_some());
                row["followingBusy"] = json!(f.busy);
            }
        }
    }
    if let Some(f) = follows.get(&v.followed_artist_id) {
        v.artist_followed = f.known.unwrap_or(false);
        v.artist_followed_known = f.known.is_some();
        v.artist_follow_busy = f.busy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velo_spotify::{MemoryTokenStore, SpotifyConfig, SpotifyProvider, TokenStore};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    async fn fixture_ctl(
        rt: &tokio::runtime::Runtime,
    ) -> (tempfile::TempDir, MockServer, Arc<SearchCtl>) {
        let (dir, env) = super::super::test_env(rt);
        let server = MockServer::start().await;
        let tokens = Arc::new(MemoryTokenStore::new());
        tokens.save("refresh-fixture").unwrap();
        Mock::given(method("POST"))
            .and(path("/api/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token":"access-fixture","expires_in":3600})),
            )
            .mount(&server)
            .await;
        let api = Arc::new(SpotifyProvider::new(
            SpotifyConfig {
                client_id: "fixture".into(),
                api_base: server.uri(),
                accounts_base: server.uri(),
                redirect_port: 0,
            },
            tokens,
        ));
        let ctl = SearchCtl::new(&env, &PlayerCtl::start(&env));
        *ctl.test_api.lock().unwrap() = Some(api);
        (dir, server, ctl)
    }
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/search.json")).unwrap()
    }
    async fn idle(ctl: &SearchCtl, suggest: bool) {
        let mut view = ctl.view.subscribe();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let busy = {
                    let v = view.borrow_and_update();
                    if suggest { v.suggesting } else { v.loading }
                };
                if !busy {
                    break;
                }
                view.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    async fn filter(ctl: &Arc<SearchCtl>, kind: &str) {
        let mut view = ctl.view.subscribe();
        ctl.set_type(kind.into());
        let model = type_model(kind).unwrap();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let ready = {
                    let v = view.borrow_and_update();
                    v.kind == kind && v.models[1] == v.models[model]
                };
                if ready {
                    break;
                }
                view.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn debounce_only_sends_the_latest_query_and_clear_sends_nothing() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, server, ctl) = fixture_ctl(&rt).await;
            Mock::given(path("/search"))
                .and(query_param("q", "mac"))
                .and(query_param("limit", "10"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
                .expect(1)
                .mount(&server)
                .await;
            ctl.suggest("m".into());
            ctl.suggest("ma".into());
            ctl.suggest("mac".into());
            idle(&ctl, true).await;
            assert_eq!(ctl.view.borrow().models[0].len(), 2);
            ctl.suggest("cancelado".into());
            // U4: while the new query waits, the previous rows stay (the popup does not blink).
            {
                let v = ctl.view.borrow();
                assert_eq!(v.suggestion_query, "cancelado");
                assert!(v.suggesting);
                assert_eq!(v.models[0].len(), 2);
            }
            ctl.suggest("  ".into());
            tokio::time::sleep(DEBOUNCE + Duration::from_millis(50)).await;
            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.url.path() == "/search")
                    .count(),
                1
            );
            assert!(ctl.view.borrow().models[0].is_empty());
        });
    }

    #[test]
    fn cancel_inflight_request_keeps_new_results_after_old_response() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, server, ctl) = fixture_ctl(&rt).await;
            Mock::given(path("/search"))
                .and(query_param("q", "viejo"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(fixture())
                        .set_delay(Duration::from_millis(700)),
                )
                .expect(1)
                .mount(&server)
                .await;
            let mut newer = fixture();
            newer["artists"]["items"][0]["name"] = json!("Nuevo");
            Mock::given(path("/search"))
                .and(query_param("q", "nuevo"))
                .respond_with(ResponseTemplate::new(200).set_body_json(newer))
                .expect(1)
                .mount(&server)
                .await;
            ctl.suggest("viejo".into());
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|r| r.url.query_pairs().any(|(k, v)| k == "q" && v == "viejo"))
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            ctl.suggest("nuevo".into());
            idle(&ctl, true).await;
            tokio::time::sleep(Duration::from_millis(750)).await;
            let v = ctl.view.borrow();
            assert_eq!(v.suggestion_query, "nuevo");
            assert_eq!(v.models[0][1]["title"], "Nuevo");
        });
    }

    #[test]
    fn full_results_filter_without_network_and_use_per_type_pagination() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, server, ctl) = fixture_ctl(&rt).await;
            Mock::given(path("/search"))
                .and(query_param("offset", "0"))
                .and(query_param("limit", "10"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
                .expect(1)
                .mount(&server)
                .await;
            let mut page = fixture();
            page["tracks"]["offset"] = json!(10);
            page["tracks"]["items"][0]["name"] = json!("Página 2");
            page["artists"]["items"] = json!([]);
            page["artists"]["offset"] = json!(10);
            Mock::given(path("/search"))
                .and(query_param("offset", "10"))
                .respond_with(ResponseTemplate::new(200).set_body_json(page))
                .expect(1)
                .mount(&server)
                .await;
            ctl.submit("Mac".into());
            idle(&ctl, false).await;
            assert_eq!(ctl.view.borrow().models[6][0]["kind"], "artist");
            filter(&ctl, "artist").await;
            assert!(!ctl.view.borrow().has_more);
            ctl.more();
            filter(&ctl, "track").await;
            assert!(ctl.view.borrow().has_more);
            ctl.view.send_modify(|v| v.models[2][0]["imageUrl"] = json!("file:///cached.jpg"));
            let previous = ctl.view.borrow().models[2].clone();
            ctl.more();
            idle(&ctl, false).await;
            assert_eq!(ctl.view.borrow().models[2].len(), 2);
            assert!(matches!(row_changes(&ctl.view.borrow().models[2], &previous).as_slice(), [ModelOp::Append(rows)] if rows.len()==1));
            for model in &ctl.view.borrow().models {
                for row in model {
                    for role in SEARCH_ROLES.split(',') {
                        assert!(row.get(role).is_some(), "falta {role}");
                    }
                    let expected = if row["kind"] == "track" { "file:///cached.jpg" } else { "" };
                    assert_eq!(row["imageUrl"], expected);
                }
            }
            ctl.set_type("invalid".into());
            assert_eq!(ctl.view.borrow().kind, "track");
        });
    }

    #[test]
    fn follow_write_is_not_optimistic_and_failure_preserves_previous_state() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, server, ctl) = fixture_ctl(&rt).await;
            let id = "4LLpKhyESsyAXpc4laK94U";
            let uri = format!("spotify:artist:{id}");
            Mock::given(method("GET"))
                .and(path("/me/library/contains"))
                .and(query_param("uris", uri.as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([true])))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("DELETE"))
                .and(path("/me/library"))
                .respond_with(ResponseTemplate::new(403).set_delay(Duration::from_millis(150)))
                .expect(1)
                .mount(&server)
                .await;
            let wait = || async {
                let mut v = ctl.view.subscribe();
                tokio::time::timeout(Duration::from_secs(4), async {
                    loop {
                        if !v.borrow_and_update().artist_follow_busy {
                            break;
                        }
                        v.changed().await.unwrap();
                    }
                })
                .await
                .unwrap();
            };
            ctl.inspect_followed(id.into());
            wait().await;
            assert!(ctl.view.borrow().artist_followed);
            ctl.set_followed(id.into(), false);
            assert!(ctl.view.borrow().artist_follow_busy);
            assert!(ctl.view.borrow().artist_followed);
            wait().await;
            assert!(ctl.view.borrow().artist_followed);
            assert!(ctl.view.borrow().artist_followed_known);
        });
    }

    #[test]
    fn best_result_and_thumbnail_changes_preserve_identity() {
        let mut rows = vec![
            json!({"kind":"track","uri":"t","title":"Mac","subtitle":"x","imageUrl":"","following":false,"followingKnown":false,"followingBusy":false}),
            json!({"kind":"artist","uri":"a","title":"Mac","subtitle":"","imageUrl":"","following":false,"followingKnown":false,"followingBusy":false}),
        ];
        assert_eq!(best_index(" mac ", &rows), Some(1));
        assert_eq!(best_index("", &rows), None);
        let old = rows.clone();
        rows[0]["imageUrl"] = json!("file:///cover.png");
        assert!(matches!(
            row_changes(&rows, &old).as_slice(),
            [ModelOp::SetField {
                row: 0,
                field: "imageUrl",
                ..
            }]
        ));
    }

    #[test]
    fn follow_and_unfollow_update_every_artist_row_only_after_confirmation() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, server, ctl) = fixture_ctl(&rt).await;
            let id = "4LLpKhyESsyAXpc4laK94U";
            Mock::given(path("/search"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
                .expect(1)
                .mount(&server)
                .await;
            for verb in ["PUT", "DELETE"] {
                Mock::given(method(verb))
                    .and(path("/me/library"))
                    .and(query_param("uris", format!("spotify:artist:{id}")))
                    .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_millis(100)))
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            ctl.submit("Mac".into());
            idle(&ctl, false).await;
            for saved in [true, false] {
                ctl.set_followed(id.into(), saved);
                let mut view = ctl.view.subscribe();
                tokio::time::timeout(Duration::from_secs(4), async {
                    loop {
                        if !view.borrow_and_update().artist_follow_busy {
                            break;
                        }
                        view.changed().await.unwrap();
                    }
                })
                .await
                .unwrap();
                let v = ctl.view.borrow();
                assert_eq!(v.artist_followed, saved);
                assert!(v.artist_followed_known);
                for rows in &v.models {
                    for row in rows.iter().filter(|r| r["kind"] == "artist") {
                        assert_eq!(row["following"], saved);
                        assert_eq!(row["followingKnown"], true);
                        assert_eq!(row["followingBusy"], false);
                    }
                }
            }
        });
    }
}
