//! `Library` engine: playlists, the open list (playlist, liked, search, album, artist), home,
//! saved albums, followed artists, pinning and playlist editing. All Web API work runs on the
//! tokio runtime; rows reach the Qt thread as ordered [`ModelOp`]s.
//!
//! Spotify 2026 restriction: the Web API only lists tracks of playlists the user owns or
//! collaborates on. Other playlists are listed through the audio session (F4,
//! `velo_player::playlist`) once Velo's engine is ready; without it they can only be played as
//! a context (`currentListable` false).
//!
//! F4: the open list can be searched and sorted ([`super::tracklist`]). That works over the
//! rows already loaded: a search or a sort first loads every remaining page (`viewBusy`), then
//! the model is replaced once per change.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;
use velo_domain::{
    Album, Artist, ImageRef, MusicProvider, Playlist, Result, TimeRange, Track, VeloError,
};

use super::images::{Generation, Thumbnailer};
use super::player::PlayerCtl;
use super::rows::{THUMB_PX, album_row, artist_row, image_of, now_ms, playlist_row, track_row_at};
use super::tracklist::{Entry, SortKey, TrackStore, ViewSpec};
use super::{Bus, Env};
use crate::bridge::source::{PlayRoute, activate_device, retry_device_action};
use crate::model::ModelOp;
use velo_player::LoadTarget;
use velo_player::playlist::PlaylistItem;

/// Page sizes: the Web API allows 50 for lists; search is capped at 10 in Development Mode.
const PAGE: u32 = 50;
/// Edge of the header image of the open list.
const HEADER_PX: u32 = 320;
/// Largest `uris` batch sent to start playback of a plain track list.
const MAX_PLAY_URIS: usize = 100;
/// F4: a search or sort loads at most this many rows first (same bound as the queue, Q1).
const MAX_VIEW_ROWS: usize = 10_000;
/// F4: typing in "Buscar en la playlist" filters once the keys pause this long.
const FILTER_DEBOUNCE: Duration = Duration::from_millis(150);

fn descriptor_labels(
    result: velo_player::descriptors::Result<Vec<velo_player::descriptors::Descriptor>>,
) -> velo_player::descriptors::Result<Vec<String>> {
    use velo_player::descriptors::DescriptorError;
    match result {
        Ok(descriptors) => Ok(descriptors
            .into_iter()
            .map(|d| d.text.to_lowercase())
            .collect()),
        Err(
            DescriptorError::Unavailable
            | DescriptorError::Format
            | DescriptorError::Timeout
            | DescriptorError::Empty
            | DescriptorError::InvalidUri,
        ) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// JoinSet aborts outstanding reads when this future is canceled or accept stops.
/// It provides bounded completion-order streaming without an extra dependency.
async fn stream_tag_results<F, T>(
    uris: Vec<String>,
    mut read: impl FnMut(String) -> F,
    mut accept: impl FnMut(String, T) -> bool,
) where
    F: std::future::Future<Output = (String, T)> + Send + 'static,
    T: Send + 'static,
{
    let mut pending = tokio::task::JoinSet::new();
    let mut uris = uris.into_iter();
    loop {
        while pending.len() < 4 {
            let Some(uri) = uris.next() else { break };
            pending.spawn(read(uri));
        }
        match pending.join_next().await {
            Some(Ok((uri, result))) => {
                if !accept(uri, result) {
                    return;
                }
            }
            _ => return,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibModel {
    Playlists = 0,
    Tracks,
    Recent,
    TopArtists,
    FollowedArtists,
    SavedAlbums,
    Albums,
}
const MODELS: usize = 7;

pub type LibEvent = (LibModel, ModelOp);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LibMeta {
    pub recent_source: String,
    pub title: String,
    pub subtitle: String,
    /// `file:///` URL, empty if none.
    pub image_url: String,
    pub kind: String,
    pub listable: bool,
    pub loading: bool,
    pub has_more: bool,
    /// F3: songs in the open playlist / "Tus me gusta" (Spotify's total), 0 if unknown.
    pub track_count: u32,
    /// F3: sum of the durations of the loaded rows.
    pub duration_ms: u64,
    /// F3: every row is loaded, so `duration_ms` is the list's total (else a lower bound).
    pub duration_exact: bool,
    /// F4: how the open list is shown (`filterQuery`, `sortKey`, `sortDescending`,
    /// `compactView`) and whether the remaining pages are being loaded for it (`viewBusy`).
    pub query: String,
    pub sort_key: String,
    pub sort_descending: bool,
    pub compact: bool,
    pub view_busy: bool,
    /// U4: Spotify id of the open playlist's owner (its header links to the profile), else "".
    pub owner_id: String,
    /// Mixear: la playlist abierta está marcada para mezclarse.
    pub mix_current: bool,
    /// Mixear: el contexto que SUENA (playlist o «Tus me gusta») está marcado. Sobrevive a la
    /// navegación: pertenece al reproductor, no a la página abierta.
    pub mix_playing: bool,
    pub track_tags: Vec<String>,
    pub active_tag: String,
}

impl LibMeta {
    fn show_spec(&mut self, spec: &ViewSpec) {
        self.query.clone_from(&spec.query);
        self.sort_key = spec.sort.as_str().into();
        self.sort_descending = spec.descending;
        self.compact = spec.compact;
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Source {
    None,
    /// `foreign`: listed through the audio session (the Web API refuses its items).
    Playlist {
        id: String,
        uri: String,
        foreign: bool,
    },
    Liked,
    Search(String),
    Album {
        id: String,
        uri: String,
    },
    Artist {
        uri: String,
    },
    SavedAlbums,
    Show {
        id: String,
    },
}

struct Nav {
    gen_id: u64,
    source: Source,
    /// Items already loaded of a paged source.
    offset: u32,
    has_more: bool,
    loading: bool,
    /// F4: keep loading pages until the list is complete, then show the view.
    load_all: bool,
    /// Rows of a foreign playlist, read once from the audio session.
    foreign: Option<Arc<Vec<PlaylistItem>>>,
    /// Key under which the list's view is remembered ("" = no search/sort for it).
    list_key: String,
}

enum ViewCommand {
    Filter(String),
    Sort(SortKey),
    Compact(bool),
}

#[derive(Default)]
struct SearchAlbums {
    navigation: Option<(u64, u64)>,
    rows: Vec<Value>,
}

/// Comparar sin reconstruir/copiar listas cuando watch solo cambió propiedades ajenas.
fn same_search_rows<'a>(
    source: &[Value],
    old: impl ExactSizeIterator<Item = &'a Value>,
    roles: &[(&str, &str)],
) -> bool {
    source.len() == old.len()
        && source.iter().zip(old).all(|(new, old)| {
            roles
                .iter()
                .all(|(destination, source)| old[*destination] == new[*source])
        })
}

pub struct LibraryCtl {
    pub(super) env: Arc<Env>,
    pub discover: Arc<super::discover::DiscoverCtl>,
    pub home: Arc<super::home::HomeCtl>,
    pub sidebar: Arc<super::sidebar::SidebarCtl>,
    history: Mutex<Arc<dyn super::sidebar::HistoryReader>>,
    history_task: Mutex<Option<tokio::task::AbortHandle>>,
    pub artists: Arc<super::artists::ArtistsCtl>,
    pub searches: Arc<super::search::SearchCtl>,
    pub profiles: Arc<super::profile::ProfileCtl>,
    player: Arc<PlayerCtl>,
    // Observe the request without starting a real audio session in regression tests.
    #[cfg(test)]
    playback_target: watch::Sender<Option<LoadTarget>>,
    pub meta: watch::Sender<LibMeta>,
    /// Contexto que suena según Velo (uri, título); `Player.contextUri/contextTitle` lo reflejan.
    playing_context: Mutex<(String, String)>,
    pub ui: Bus<LibEvent>,
    nav: Mutex<Nav>,
    // Serializes publication with navigation; never held over an await.
    navigation_write: Mutex<()>,
    requests: Generation,
    playlists: Mutex<Vec<Playlist>>,
    liked: Mutex<HashSet<String>>,
    thumbs: Thumbnailer,
    gens: [Arc<Generation>; MODELS],
    /// Rows of the open list and how they are shown (F4). Every `Tracks` model change is sent
    /// while holding this lock, so row indexes and model ops stay in step.
    store: Arc<Mutex<TrackStore>>,
    /// Views remembered per list for this session (memory only, never on disk).
    views: Mutex<HashMap<String, ViewSpec>>,
    /// Coalesces view changes (typing): only the newest one is applied.
    view_ticket: AtomicU64,
    view_commands: tokio::sync::mpsc::UnboundedSender<(u64, ViewCommand)>,
    /// Cada invocación reserva su lugar antes del spawn, independientemente del scheduler.
    details_tail: Mutex<HashMap<String, tokio::sync::oneshot::Receiver<()>>>,
    search_albums: Mutex<SearchAlbums>,
    tag_cache: Mutex<HashMap<String, Vec<String>>>,
    tag_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    tag_permits: tokio::sync::Semaphore,
}

/// Pinned playlists first, the rest in Spotify's order.
/// Key under which a list's search/sort is remembered; "" for lists without them.
fn list_key(source: &Source) -> String {
    match source {
        Source::Playlist { id, .. } => format!("playlist:{id}"),
        Source::Liked => "liked".into(),
        _ => String::new(),
    }
}

/// Header state with nothing open.
fn blank_meta() -> LibMeta {
    LibMeta {
        kind: "playlist".into(),
        recent_source: "none".into(),
        listable: true,
        sort_key: SortKey::Custom.as_str().into(),
        ..LibMeta::default()
    }
}

/// M1b: the cached playlist takes the new details (true if it was found).
fn apply_details(playlists: &mut [Playlist], id: &str, name: &str, description: &str) -> bool {
    match playlists.iter_mut().find(|p| p.id == id) {
        Some(p) => {
            name.clone_into(&mut p.name);
            description.clone_into(&mut p.description);
            true
        }
        None => false,
    }
}

pub fn recent_source(spotify: bool, local: bool) -> &'static str {
    if spotify {
        "spotify"
    } else if local {
        "local"
    } else {
        "none"
    }
}

pub fn sort_pinned(playlists: &mut [Playlist], pinned: &[String]) {
    playlists.sort_by_key(|p| !pinned.contains(&p.id));
}

/// The owner; QML appends "N canciones, X h Y min" from `track_count` / `duration_ms` (F3).
pub fn playlist_subtitle(p: &Playlist) -> String {
    p.owner.clone()
}

pub fn album_subtitle(a: &Album) -> String {
    let artists = a
        .artists
        .iter()
        .map(|x| x.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let year = super::rows::release_year(&a.release_date);
    if year > 0 {
        format!("{artists} · {year} · {} canciones", a.total_tracks)
    } else {
        format!("{artists} · {} canciones", a.total_tracks)
    }
}

/// `uris` slice that starts playback at row `index` (at most [`MAX_PLAY_URIS`]).
pub fn uris_from(uris: &[String], index: usize) -> Vec<String> {
    uris.iter()
        .skip(index)
        .take(MAX_PLAY_URIS)
        .cloned()
        .collect()
}

/// What to play for the open list.
#[derive(Debug, Clone, PartialEq)]
enum PlayPlan {
    /// The playlist/album/artist itself, from row `index` of its own order.
    Context { uri: String, index: Option<usize> },
    /// An explicit list: the rows as shown (liked songs, search, or a sorted list, F4).
    List { uris: Vec<String>, index: usize },
}

/// F4: a list shown in its own order (even filtered) plays as its context from the entry
/// behind the row; a reordered one, liked songs and search play the rows as shown.
fn play_plan(source: &Source, store: &TrackStore, row: Option<usize>) -> Option<PlayPlan> {
    match source {
        Source::Playlist { uri, .. } | Source::Album { uri, .. } | Source::Artist { uri }
            if store.spec.is_identity() && store.active_tag.is_empty() && row.is_none() =>
        {
            Some(PlayPlan::Context {
                uri: uri.clone(),
                index: row.and_then(|r| store.entry_of(r)),
            })
        }
        Source::Playlist { .. } | Source::Album { .. } | Source::Liked | Source::Search(_) => {
            Some(PlayPlan::List {
                uris: store.shown_uris(),
                index: row.unwrap_or(0),
            })
        }
        Source::Artist { .. } | Source::SavedAlbums | Source::None => None,
        Source::Show { .. } => Some(PlayPlan::List {
            uris: store.shown_uris(),
            index: row.unwrap_or(0),
        }),
    }
}

/// `Library.playContext(uri)`: a Spotify album/playlist/artist from its start (or one song).
/// Anything else (empty, other types, a bare prefix) plays nothing.
fn context_plan(uri: &str) -> Option<PlayPlan> {
    let has_id = |prefix: &str| uri.strip_prefix(prefix).is_some_and(|id| !id.is_empty());
    if ["spotify:album:", "spotify:playlist:", "spotify:artist:"]
        .iter()
        .any(|p| has_id(p))
    {
        Some(PlayPlan::Context {
            uri: uri.to_string(),
            index: None,
        })
    } else if has_id("spotify:track:") {
        Some(PlayPlan::List {
            uris: vec![uri.to_string()],
            index: 0,
        })
    } else {
        None
    }
}

/// Keep the displayed list and its starting row so shuffle can include earlier rows too.
fn velo_target(plan: &PlayPlan, shuffle: bool) -> Option<LoadTarget> {
    match plan {
        PlayPlan::Context { uri, index } => Some(LoadTarget::Context {
            uri: uri.clone(),
            index: index.and_then(|i| u32::try_from(i).ok()),
            shuffle,
        }),
        PlayPlan::List { uris, index } => {
            let index = u32::try_from(*index).ok()?;
            ((index as usize) < uris.len()).then(|| LoadTarget::Tracks {
                uris: uris.clone(),
                index,
                shuffle,
            })
        }
    }
}

/// Recently played often repeats a song; keep the first (newest) of each, up to `max`.
pub fn dedupe_tracks(tracks: Vec<Track>, max: usize) -> Vec<Track> {
    let mut seen = HashSet::new();
    tracks
        .into_iter()
        .filter(|t| seen.insert(t.uri.clone()))
        .take(max)
        .collect()
}

/// Raw slots a Web API page covers (its `limit`), so filtered items still advance the offset.
fn page_advance<T>(page: &velo_domain::Page<T>) -> u32 {
    if page.limit > 0 {
        page.limit
    } else {
        page.items.len() as u32
    }
}

const FOREIGN_NEEDS_AUDIO: &str =
    "Activa la reproducción en Velo para ver las canciones de esta playlist.";
const FOREIGN_FAILED: &str = "Spotify no entregó las canciones de esta playlist.";

fn can_fetch_page(listable: bool, loading: bool, offset: u32, has_more: bool) -> bool {
    listable && !loading && (offset == 0 || has_more)
}

/// Run a Web API call; allow Connect to settle after activating a device.
macro_rules! with_device {
    ($env:expr, $api:expr, $call:expr) => {{
        match $call {
            Err(VeloError::NoActiveDevice) => {
                activate_device($env, &**$api, true).await?;
                retry_device_action(|| async { $call }).await
            }
            other => other,
        }
    }};
}

impl LibraryCtl {
    pub fn start(env: &Arc<Env>, player: &Arc<PlayerCtl>) -> Arc<Self> {
        let (view_commands, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let this = Arc::new(Self {
            env: env.clone(),
            discover: super::discover::DiscoverCtl::start(env),
            home: super::home::HomeCtl::new(env, player),
            sidebar: super::sidebar::SidebarCtl::new(env),
            history: Mutex::new(Arc::new(super::sidebar::EmptyHistory)),
            history_task: Mutex::new(None),
            artists: super::artists::ArtistsCtl::new(env, player),
            searches: super::search::SearchCtl::new(env, player),
            profiles: super::profile::ProfileCtl::new(env),
            player: player.clone(),
            #[cfg(test)]
            playback_target: watch::channel(None).0,
            meta: watch::channel(blank_meta()).0,
            playing_context: Mutex::new((String::new(), String::new())),
            ui: Bus::new(),
            nav: Mutex::new(Nav {
                gen_id: 0,
                source: Source::None,
                offset: 0,
                has_more: false,
                loading: false,
                load_all: false,
                foreign: None,
                list_key: String::new(),
            }),
            navigation_write: Mutex::new(()),
            requests: Generation::default(),
            playlists: Mutex::new(Vec::new()),
            liked: Mutex::new(HashSet::new()),
            thumbs: Thumbnailer::new(env.rt.clone(), env.covers.clone()),
            gens: std::array::from_fn(|_| Arc::new(Generation::default())),
            store: Arc::new(Mutex::new(TrackStore::default())),
            views: Mutex::new(HashMap::new()),
            view_ticket: AtomicU64::new(0),
            view_commands,
            details_tail: Mutex::new(HashMap::new()),
            search_albums: Mutex::new(SearchAlbums::default()),
            tag_cache: Mutex::new(HashMap::new()),
            tag_task: Mutex::new(None),
            tag_permits: tokio::sync::Semaphore::new(4),
        });
        let weak = Arc::downgrade(&this);
        env.rt.spawn(async move {
            while let Some((request, command)) = commands.recv().await {
                let Some(this) = weak.upgrade() else { break };
                match command {
                    ViewCommand::Filter(query) => {
                        this.change_view(request, |s| s.query = query, Some(FILTER_DEBOUNCE))
                    }
                    ViewCommand::Sort(key) => {
                        this.change_view(request, |s| s.choose_sort(key), Some(Duration::ZERO))
                    }
                    ViewCommand::Compact(compact) => {
                        this.change_view(request, |s| s.compact = compact, None)
                    }
                }
            }
        });
        // A playlist of someone else opened before the audio engine was ready can be listed
        // through the session once it is (F4).
        let me = this.clone();
        let mut audio = env.audio.view.subscribe();
        env.rt.spawn(async move {
            let mut ready = false;
            loop {
                let now = audio.borrow_and_update().ready;
                if now && !ready {
                    if me.api().is_some() {
                        me.sidebar.refresh();
                        me.home.load_spotify_shortcuts();
                    }
                    let reopen = match &me.nav().source {
                        Source::Playlist { id, .. } if !me.meta.borrow().listable => {
                            Some(id.clone())
                        }
                        _ => None,
                    };
                    if let Some(id) = reopen {
                        me.open_playlist(id);
                    } else {
                        let gen_id = me.nav().gen_id;
                        me.refresh_tags(gen_id);
                    }
                }
                ready = now;
                if audio.changed().await.is_err() {
                    break;
                }
            }
        });
        let me = this.clone();
        let mut search = this.searches.view.subscribe();
        env.rt.spawn(async move {
            loop {
                let view = search.borrow_and_update().clone();
                me.publish_search_compatibility(&view);
                if search.changed().await.is_err() {
                    break;
                }
            }
        });
        let me = this.clone();
        let mut account = env.account.view.subscribe();
        env.rt.spawn(async move {
            let mut user = String::new();
            loop {
                let (signed_in, id) = {
                    let a = account.borrow_and_update();
                    (a.signed_in, a.user_id.clone())
                };
                if signed_in && !id.is_empty() && id != user {
                    // También limpia si watch coalesció logout + login mientras se cargaban playlists.
                    me.searches.clear();
                    me.home.clear();
                    user = id;
                    me.load_playlists().await;
                    me.sidebar.refresh();
                    me.load_home();
                    let source = me.nav().source.clone();
                    if let Source::Playlist { id, .. } = source {
                        me.open_playlist(id);
                    }
                } else if !signed_in && !user.is_empty() {
                    user.clear();
                    me.reset_all();
                }
                if account.changed().await.is_err() {
                    break;
                }
            }
        });
        this
    }

    fn nav(&self) -> std::sync::MutexGuard<'_, Nav> {
        self.nav.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn api(&self) -> Option<Arc<dyn MusicProvider>> {
        self.env.account.api()
    }

    fn user_id(&self) -> String {
        self.env.account.snapshot().user_id
    }

    // ------------------------------------------------------------------ model helpers

    fn gen_of(&self, m: LibModel) -> &Arc<Generation> {
        &self.gens[m as usize]
    }

    /// Replaces a model and fills its row images in the background.
    fn set_rows(&self, m: LibModel, rows: Vec<Value>, images: Vec<Option<String>>) {
        let gen_id = self.gen_of(m).bump();
        self.ui.send((m, ModelOp::Replace(rows)));
        self.fill_images(m, gen_id, 0, images);
    }

    fn append_rows(
        &self,
        m: LibModel,
        rows: Vec<Value>,
        images: Vec<Option<String>>,
        start: usize,
    ) {
        let gen_id = self.gen_of(m).current();
        self.ui.send((m, ModelOp::Append(rows)));
        self.fill_images(m, gen_id, start, images);
    }

    fn fill_images(&self, m: LibModel, gen_id: u64, start: usize, images: Vec<Option<String>>) {
        let items = images
            .into_iter()
            .enumerate()
            .filter_map(|(i, u)| u.map(|u| (start + i, u)))
            .collect();
        let sender = self.ui.sender();
        self.thumbs.fill(
            self.gen_of(m).clone(),
            gen_id,
            items,
            THUMB_PX,
            move |row, url| {
                let _ = sender.send((
                    m,
                    ModelOp::SetField {
                        row,
                        field: "imageUrl",
                        value: json!(url),
                    },
                ));
            },
        );
    }

    fn set_meta(&self, f: impl FnOnce(&mut LibMeta)) {
        self.meta.send_if_modified(|m| {
            let before = m.clone();
            f(m);
            *m != before
        });
    }

    fn reset_all(&self) {
        self.sidebar.clear();
        if let Some(task) = self
            .history_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
        *self.history.lock().unwrap_or_else(|e| e.into_inner()) =
            Arc::new(super::sidebar::EmptyHistory);
        self.home.clear();
        self.searches.clear();
        self.discover.begin_home();
        let _commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.cancel_tags();
        self.tag_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.artists.clear();
        self.profiles.clear();
        self.requests.bump();
        self.liked.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.reset_tracks();
        self.views.lock().unwrap_or_else(|e| e.into_inner()).clear();
        for m in [
            LibModel::Playlists,
            LibModel::Recent,
            LibModel::TopArtists,
            LibModel::FollowedArtists,
            LibModel::SavedAlbums,
            LibModel::Albums,
        ] {
            self.set_rows(m, Vec::new(), Vec::new());
        }
        self.playlists
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        {
            let mut n = self.nav();
            n.gen_id += 1;
            n.source = Source::None;
            n.offset = 0;
            n.has_more = false;
            n.loading = false;
            n.load_all = false;
            n.foreign = None;
            n.list_key.clear();
        }
        let mix_playing = self.meta.borrow().mix_playing;
        self.meta.send_replace(LibMeta {
            mix_playing,
            ..blank_meta()
        });
    }

    fn liked_flag(&self, uri: &str) -> bool {
        self.liked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(uri)
    }

    fn track_rows(&self, tracks: &[Track]) -> (Vec<Value>, Vec<Option<String>>) {
        let now = now_ms();
        (
            tracks
                .iter()
                .map(|t| track_row_at(t, self.liked_flag(&t.uri), now))
                .collect(),
            tracks.iter().map(|t| image_of(&t.images)).collect(),
        )
    }

    fn store(&self) -> std::sync::MutexGuard<'_, TrackStore> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Empties the open list (rows, view, pending thumbnails).
    fn reset_tracks(&self) {
        let mut st = self.store();
        st.clear();
        self.gen_of(LibModel::Tracks).bump();
        self.ui
            .send((LibModel::Tracks, ModelOp::Replace(Vec::new())));
    }

    /// Shrinks the covers of entries `items` (entry index, remote URL) of the current rows and
    /// puts each on the row that shows that entry, if any.
    fn fill_track_images(&self, st: &TrackStore, items: Vec<(usize, String)>) {
        let generation = st.generation;
        let store = self.store.clone();
        let sender = self.ui.sender();
        let generations = self.gen_of(LibModel::Tracks);
        self.thumbs.fill(
            generations.clone(),
            generations.current(),
            items,
            THUMB_PX,
            move |i, url| {
                let mut st = store.lock().unwrap_or_else(|e| e.into_inner());
                if st.generation != generation {
                    return;
                }
                let Some(entry) = st.entries.get_mut(i) else {
                    return;
                };
                entry.row["imageUrl"] = json!(url);
                entry.image = None;
                if let Some(row) = st.shown_row(i) {
                    let _ = sender.send((
                        LibModel::Tracks,
                        ModelOp::SetField {
                            row,
                            field: "imageUrl",
                            value: json!(url),
                        },
                    ));
                }
            },
        );
    }

    /// Re-requests the covers still missing after the entries moved (a removal).
    fn refill_track_images(&self, st: &mut TrackStore) {
        st.generation += 1;
        self.gen_of(LibModel::Tracks).bump();
        let items = st
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.image.clone().map(|u| (i, u)))
            .collect();
        self.fill_track_images(st, items);
    }

    /// Starts a new open list: invalidates older loads, clears `tracks`, publishes the header.
    // The arguments mirror the header contract and are committed as one navigation update.
    #[allow(clippy::too_many_arguments)]
    fn begin_nav(
        self: &Arc<Self>,
        source: Source,
        kind: &str,
        title: String,
        subtitle: String,
        listable: bool,
        image: Option<&[ImageRef]>,
    ) -> u64 {
        let _commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.artists.clear();
        self.profiles.clear();
        self.home.leave();
        self.searches.leave();
        self.requests.bump();
        let key = list_key(&source);
        self.cancel_tags();
        let spec = self
            .views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned()
            .unwrap_or_else(|| ViewSpec {
                // No session override: start from the global `Settings.compactTracks`.
                compact: self.default_compact(),
                ..ViewSpec::default()
            });
        let gen_id = {
            let mut n = self.nav();
            n.gen_id += 1;
            n.source = source;
            n.offset = 0;
            n.has_more = false;
            n.loading = false;
            // A remembered search or sort needs every page before it can be shown.
            n.load_all = !spec.is_identity();
            n.foreign = None;
            n.list_key = key;
            n.gen_id
        };
        self.view_ticket.fetch_add(1, Ordering::SeqCst);
        self.reset_tracks();
        self.store().spec = spec.clone();
        let mut meta = LibMeta {
            title,
            subtitle,
            image_url: String::new(),
            kind: kind.into(),
            listable,
            mix_playing: self.meta.borrow().mix_playing,
            ..LibMeta::default()
        };
        meta.show_spec(&spec);
        self.meta.send_replace(meta);
        if let Some(url) = image.and_then(image_of) {
            let this = self.clone();
            self.env.rt.spawn(async move {
                if let Some(file) = this.thumbs.one(&url, HEADER_PX).await {
                    if let Some(_commit) = this.commit_navigation(gen_id) {
                        this.set_meta(|m| m.image_url = file);
                    }
                }
            });
        }
        gen_id
    }

    // ------------------------------------------------------------------ loading

    pub(super) async fn load_playlists(self: &Arc<Self>) {
        let Some(api) = self.api() else {
            return;
        };
        let mut all: Vec<Playlist> = Vec::new();
        let mut offset = 0;
        loop {
            match api.my_playlists(offset, PAGE).await {
                Ok(page) => {
                    let more = page.has_more() && !page.items.is_empty();
                    offset += page.items.len() as u32;
                    all.extend(page.items);
                    if !more {
                        break;
                    }
                }
                Err(e) => {
                    self.env.report(&e);
                    break;
                }
            }
        }
        self.publish_playlists(all);
    }

    fn publish_playlists(self: &Arc<Self>, mut all: Vec<Playlist>) {
        let user_id = self.user_id();
        self.sidebar.set_playlists(&all, &user_id);
        let pinned = self.sidebar.pinned_playlists();
        sort_pinned(&mut all, &pinned);
        let rows = all
            .iter()
            .map(|p| playlist_row(p, pinned.contains(&p.id), &user_id))
            .collect();
        let images = all.iter().map(|p| image_of(&p.images)).collect();
        self.home.set_playlists(self.user_id(), all.clone());
        *self.playlists.lock().unwrap_or_else(|e| e.into_inner()) = all;
        self.set_rows(LibModel::Playlists, rows, images);
    }

    /// Loads the next page of the open list (or the first, right after `begin_nav`).
    fn load_page(self: &Arc<Self>) {
        let _commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (gen_id, source, offset, load_all) = {
            let mut n = self.nav();
            // Followed, non-collaborative playlists may be played as contexts only.
            if !can_fetch_page(self.meta.borrow().listable, n.loading, n.offset, n.has_more) {
                return;
            }
            n.loading = true;
            (n.gen_id, n.source.clone(), n.offset, n.load_all)
        };
        self.publish_tags(HashMap::new());
        self.set_meta(|m| {
            m.loading = true;
            m.view_busy = load_all;
        });
        self.cancel_tags();
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result = this.fetch_page(&source, offset, gen_id).await;
            let next = {
                let Some(_commit) = this.commit_navigation(gen_id) else {
                    return;
                };
                match result {
                    // F4: a search or sort waits for the whole list; fetch the next page.
                    Ok(()) => {
                        let n = this.nav();
                        n.load_all && n.has_more
                    }
                    Err(e) => {
                        this.env.report(&e);
                        {
                            let mut n = this.nav();
                            n.loading = false;
                            n.load_all = false;
                        }
                        this.set_meta(|m| {
                            m.loading = false;
                            m.view_busy = false;
                        });
                        // Show the view over what did load.
                        this.show_view();
                        false
                    }
                }
            };
            if next {
                this.load_page();
            } else {
                this.refresh_tags(gen_id);
            }
        });
    }

    fn commit_navigation(&self, gen_id: u64) -> Option<std::sync::MutexGuard<'_, ()>> {
        let commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (self.nav().gen_id == gen_id).then_some(commit)
    }

    async fn fetch_page(self: &Arc<Self>, source: &Source, offset: u32, gen_id: u64) -> Result<()> {
        let api = self.api().ok_or(VeloError::NotAuthenticated)?;
        match source {
            Source::Playlist {
                uri, foreign: true, ..
            } => self.fetch_foreign_page(uri, offset, gen_id).await?,
            Source::Playlist { id, .. } => {
                let page = api.playlist_tracks(id, offset, PAGE).await?;
                let Some(_commit) = self.commit_navigation(gen_id) else {
                    return Ok(());
                };
                let more = page.has_more();
                let next = offset + page_advance(&page);
                self.accept_tracks(offset, page.items, page.total, more, next);
            }
            Source::Liked => {
                let page = api.saved_tracks(offset, PAGE).await?;
                let Some(_commit) = self.commit_navigation(gen_id) else {
                    return Ok(());
                };
                {
                    let mut liked = self.liked.lock().unwrap_or_else(|e| e.into_inner());
                    liked.extend(page.items.iter().map(|t| t.uri.clone()));
                }
                let more = page.has_more();
                let next = offset + page_advance(&page);
                self.accept_tracks(offset, page.items, page.total, more, next);
            }
            Source::Album { id, .. } => {
                let page = api.album_tracks(id, offset, PAGE).await?;
                let Some(_commit) = self.commit_navigation(gen_id) else {
                    return Ok(());
                };
                let more = page.has_more();
                let next = offset + page_advance(&page);
                self.accept_tracks(offset, page.items, page.total, more, next);
            }
            Source::Show { id } => {
                let engine = self
                    .env
                    .audio
                    .engine()
                    .filter(|_| self.env.audio.is_ready())
                    .ok_or(VeloError::NoActiveDevice)?;
                let page =
                    velo_player::session_api::shows::episodes(engine.session(), id, offset, PAGE)
                        .await?;
                let Some(_commit) = self.commit_navigation(gen_id) else {
                    return Ok(());
                };
                let next = offset.saturating_add(PAGE).min(page.total);
                let more = next < page.total;
                self.accept_tracks(offset, page.items, page.total, more, next);
            }
            Source::SavedAlbums => {
                let page = api.saved_albums(offset, PAGE).await?;
                let Some(_commit) = self.commit_navigation(gen_id) else {
                    return Ok(());
                };
                let count = page.items.len() as u32;
                let rows = page.items.iter().map(album_row).collect();
                let images = page.items.iter().map(|a| image_of(&a.images)).collect();
                if offset == 0 {
                    self.set_rows(LibModel::SavedAlbums, rows, images);
                } else {
                    self.append_rows(LibModel::SavedAlbums, rows, images, offset as usize);
                }
                let more = page.has_more() && count > 0;
                {
                    let mut n = self.nav();
                    n.offset += count.max(1);
                    n.has_more = more;
                    n.loading = false;
                }
                self.set_meta(|m| {
                    m.loading = false;
                    m.has_more = more;
                });
            }
            Source::Artist { .. } | Source::Search(_) | Source::None => {
                let Some(_commit) = self.commit_navigation(gen_id) else {
                    return Ok(());
                };
                self.nav().loading = false;
                self.set_meta(|m| m.loading = false);
            }
        }
        Ok(())
    }

    /// A page of a foreign playlist, through the audio session (F4).
    async fn fetch_foreign_page(&self, uri: &str, offset: u32, gen_id: u64) -> Result<()> {
        let engine = self
            .env
            .audio
            .engine()
            .filter(|_| self.env.audio.is_ready())
            .ok_or_else(|| VeloError::Other(FOREIGN_NEEDS_AUDIO.into()))?;
        let known = self.nav().foreign.clone();
        let items = match known {
            Some(items) => items,
            None => {
                let items = velo_player::playlist::playlist_items(&engine, uri)
                    .await
                    .map_err(|_| VeloError::Other(FOREIGN_FAILED.into()))?;
                let items = Arc::new(items);
                let Some(_commit) = self.commit_navigation(gen_id) else {
                    return Ok(());
                };
                self.nav().foreign = Some(items.clone());
                items
            }
        };
        let start = (offset as usize).min(items.len());
        let end = (start + PAGE as usize).min(items.len());
        let tracks = velo_player::playlist::tracks(&engine, &items[start..end]).await;
        let Some(_commit) = self.commit_navigation(gen_id) else {
            return Ok(());
        };
        self.accept_tracks(
            offset,
            tracks,
            items.len() as u32,
            end < items.len(),
            end as u32,
        );
        Ok(())
    }

    /// Adds a page of the open list. `next` is the offset of the following page (raw Spotify
    /// slots: skipped episodes and songs without metadata still advance it).
    fn accept_tracks(&self, offset: u32, items: Vec<Track>, total: u32, more: bool, next: u32) {
        let (rows, images) = self.track_rows(&items);
        let more = more && next > offset;
        let entries: Vec<Entry> = items
            .iter()
            .zip(rows.iter().cloned())
            .zip(images.iter().cloned())
            .map(|((t, row), image)| Entry::new(t, row, image))
            .collect();
        let load_all = {
            let mut n = self.nav();
            n.offset = next.max(offset + 1);
            n.has_more = more;
            n.loading = false;
            n.load_all
        };
        let (finished, duration_ms) = {
            let mut st = self.store();
            if offset == 0 {
                st.clear();
                self.gen_of(LibModel::Tracks).bump();
            }
            let start = st.entries.len();
            st.entries.extend(entries);
            // F4: with a search or sort, wait for the rest of the list (bounded), then replace
            // the model once.
            let waiting = load_all && more && st.entries.len() < MAX_VIEW_ROWS;
            if st.view.is_none() && ((st.spec.is_identity() && st.active_tag.is_empty()) || waiting)
            {
                // The model shows the list in its order: keep it growing (a pending search
                // or sort is applied once the last page arrives).
                let op = if offset == 0 {
                    ModelOp::Replace(rows)
                } else {
                    ModelOp::Append(rows)
                };
                self.ui.send((LibModel::Tracks, op));
            } else if !waiting {
                let rows = st.apply();
                self.ui.send((LibModel::Tracks, ModelOp::Replace(rows)));
            }
            let items = images
                .into_iter()
                .enumerate()
                .filter_map(|(i, u)| u.map(|u| (start + i, u)))
                .collect();
            self.fill_track_images(&st, items);
            (!waiting, st.duration_ms())
        };
        if finished {
            self.nav().load_all = false;
        }
        self.set_meta(|m| {
            m.loading = false;
            m.has_more = more;
            m.view_busy = !finished;
            if matches!(m.kind.as_str(), "playlist" | "liked") {
                m.track_count = total;
            }
            m.duration_ms = duration_ms;
            m.duration_exact = !more && self.store().entries.len() as u32 == total;
        });
    }

    // ------------------------------------------------------------------ search and sort (F4)

    fn cancel_tags(&self) {
        if let Some(task) = self
            .tag_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
    }

    /// One cancelable worker per navigation, four queries in flight. A completed query
    /// frees a slot immediately; chips publish progressively without waiting for the list.
    fn refresh_tags(self: &Arc<Self>, gen_id: u64) {
        let Some(_commit) = self.commit_navigation(gen_id) else {
            return;
        };
        let eligible = {
            let n = self.nav();
            !n.loading && matches!(n.source, Source::Playlist { .. } | Source::Liked)
        };
        if !eligible {
            return;
        }
        self.cancel_tags();
        self.set_meta(|m| m.track_tags.clear());
        let Some(engine) = self
            .env
            .audio
            .engine()
            .filter(|_| self.env.audio.is_ready())
        else {
            self.publish_tags(HashMap::new());
            return;
        };
        let uris: Vec<_> = {
            let st = self.store();
            let mut seen = HashSet::new();
            st.entries
                .iter()
                .take(MAX_VIEW_ROWS)
                .map(|e| e.uri.clone())
                .filter(|u| seen.insert(u.clone()))
                .collect()
        };
        let this = self.clone();
        let task = self.env.rt.spawn(async move {
            let mut tags = HashMap::new();
            let mut published_at = std::time::Instant::now();
            stream_tag_results(
                uris,
                |uri| {
                    let this = this.clone();
                    let engine = engine.clone();
                    async move {
                        let cached = this
                            .tag_cache
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&uri)
                            .cloned();
                        let result = if let Some(tags) = cached {
                            Ok(tags)
                        } else if let Ok(_permit) = this.tag_permits.acquire().await {
                            descriptor_labels(
                                velo_player::descriptors::track_descriptors(engine.session(), &uri)
                                    .await,
                            )
                        } else {
                            Err(velo_player::descriptors::DescriptorError::Session)
                        };
                        (uri, result)
                    }
                },
                |uri, result| {
                    let Some(_commit) = this.commit_navigation(gen_id) else {
                        return false;
                    };
                    let Ok(labels) = result else {
                        // Preserve all successful rows on a session/transport failure.
                        this.publish_tags(tags.clone());
                        return false;
                    };
                    {
                        let mut cache = this.tag_cache.lock().unwrap_or_else(|e| e.into_inner());
                        if cache.len() >= MAX_VIEW_ROWS * 2 {
                            cache.clear();
                        }
                        cache.insert(uri.clone(), labels.clone());
                    }
                    tags.insert(uri, labels);
                    if tags.len() == 1
                        || tags.len() % 32 == 0
                        || published_at.elapsed() >= Duration::from_millis(250)
                    {
                        this.publish_tags(tags.clone());
                        published_at = std::time::Instant::now();
                    }
                    true
                },
            )
            .await;
            let Some(_commit) = this.commit_navigation(gen_id) else {
                return;
            };
            this.publish_tags(tags);
        });
        *self.tag_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
    }

    /// Caller holds navigation_write. Empty data clears both chips and their filter.
    fn publish_tags(&self, tags: HashMap<String, Vec<String>>) {
        let (track_tags, active_tag) = {
            let mut st = self.store();
            st.tags = tags;
            let track_tags = st.top_tags();
            if !st.tags.values().any(|tags| tags.contains(&st.active_tag)) {
                st.active_tag.clear();
            }
            let active_tag = st.active_tag.clone();
            let before = st.view.clone();
            let rows = st.apply();
            if before != st.view {
                self.ui.send((LibModel::Tracks, ModelOp::Replace(rows)));
            }
            (track_tags, active_tag)
        };
        self.set_meta(|m| {
            m.track_tags = track_tags;
            m.active_tag = active_tag;
        });
    }

    pub fn filter_tag(self: &Arc<Self>, tag: String) {
        let _commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.nav().list_key.is_empty()
            || (!tag.is_empty() && !self.meta.borrow().track_tags.contains(&tag))
        {
            return;
        }
        let active_tag = {
            let mut st = self.store();
            st.toggle_tag(&tag);
            let rows = st.apply();
            self.ui.send((LibModel::Tracks, ModelOp::Replace(rows)));
            st.active_tag.clone()
        };
        self.set_meta(|m| m.active_tag = active_tag);
        drop(_commit);
        self.refresh_view();
    }

    pub fn set_filter(self: &Arc<Self>, query: String) {
        let _ = self
            .view_commands
            .send((self.requests.current(), ViewCommand::Filter(query)));
    }

    pub fn set_sort(self: &Arc<Self>, key: String) {
        if let Some(key) = SortKey::parse(&key) {
            let _ = self
                .view_commands
                .send((self.requests.current(), ViewCommand::Sort(key)));
        }
    }

    /// `Settings.compactTracks` (C1 owns the key; read schema-neutrally, false when absent).
    fn default_compact(&self) -> bool {
        let v = serde_json::to_value(self.env.prefs.settings()).unwrap_or_default();
        ["appearance", "library", "playback"]
            .iter()
            .find_map(|section| v[section]["compact_tracks"].as_bool())
            .or_else(|| v["compact_tracks"].as_bool())
            .unwrap_or(false)
    }

    /// "Ver como": only QML changes (rows and their order stay).
    pub fn set_compact(self: &Arc<Self>, compact: bool) {
        let _ = self
            .view_commands
            .send((self.requests.current(), ViewCommand::Compact(compact)));
    }

    /// Updates the open list's view, remembers it for the session and applies it on the
    /// runtime (a search waits for the keys to pause; only the newest change is applied).
    fn change_view(
        self: &Arc<Self>,
        request: u64,
        edit: impl FnOnce(&mut ViewSpec),
        apply: Option<Duration>,
    ) {
        let _commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.requests.current() != request {
            return;
        }
        let key = self.nav().list_key.clone();
        if key.is_empty() {
            return;
        }
        let spec = {
            let mut st = self.store();
            edit(&mut st.spec);
            st.spec.clone()
        };
        self.views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, spec.clone());
        self.set_meta(|m| m.show_spec(&spec));
        let Some(delay) = apply else {
            return;
        };
        let ticket = self.view_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        let this = self.clone();
        self.env.rt.spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if this.view_ticket.load(Ordering::SeqCst) == ticket {
                this.refresh_view();
            }
        });
    }

    /// Shows the current view, first loading every remaining page when it needs them.
    fn refresh_view(self: &Arc<Self>) {
        // Serialize navigation with publication, but release before starting another page.
        let commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let needs_all = {
            let st = self.store();
            !st.spec.is_identity() || !st.active_tag.is_empty()
        };
        let capped = self.store().entries.len() >= MAX_VIEW_ROWS;
        let (wait, fetch) = {
            let mut n = self.nav();
            // A page in flight (or the first one) counts: its arrival continues the chain.
            let pending = n.has_more || n.loading;
            n.load_all = needs_all && pending && !capped;
            (n.load_all, n.load_all && n.has_more && !n.loading)
        };
        if wait {
            self.set_meta(|m| m.view_busy = true);
            drop(commit);
            if fetch {
                self.load_page();
            }
            return;
        }
        self.set_meta(|m| m.view_busy = false);
        self.show_view();
    }

    /// Replaces the model with the rows of the current view, unless they are already shown.
    fn show_view(&self) {
        let mut st = self.store();
        let before = st.view.clone();
        let rows = st.apply();
        if st.view != before {
            self.ui.send((LibModel::Tracks, ModelOp::Replace(rows)));
        }
    }

    // ------------------------------------------------------------------ navigation

    pub fn open_playlist(self: &Arc<Self>, id: String) {
        let request = self.requests.bump();
        let this = self.clone();
        self.env.rt.spawn(async move {
            let cached = this
                .playlists
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|p| p.id == id)
                .cloned();
            let cached = cached.or_else(|| this.home.playlist(&id));
            let playlist = match cached {
                Some(p) => p,
                None => {
                    let Some(api) = this.api() else { return };
                    match api.playlist(&id).await {
                        Ok(p) => p,
                        Err(e) => return this.env.report(&e),
                    }
                }
            };
            // F4: playlists the Web API will not list are read through the audio session.
            let foreign = !playlist.items_listable_by(&this.user_id());
            let listable = !foreign || this.env.audio.is_ready();
            if this.requests.current() != request {
                return;
            }
            this.begin_nav(
                Source::Playlist {
                    id: playlist.id.clone(),
                    uri: playlist.uri.clone(),
                    foreign,
                },
                "playlist",
                playlist.name.clone(),
                playlist_subtitle(&playlist),
                listable,
                Some(&playlist.images),
            );
            this.sidebar.request_image(playlist.uri.clone());
            let mixed = this.env.prefs.extras().mixed_playlists;
            this.set_meta(|m| {
                m.track_count = playlist.track_count;
                m.owner_id.clone_from(&playlist.owner_id);
                m.mix_current = mixed.contains(&playlist.id);
            });
            if listable {
                this.load_page();
            }
        });
    }

    pub fn open_liked(self: &Arc<Self>) {
        self.begin_nav(
            Source::Liked,
            "liked",
            "Tus me gusta".into(),
            String::new(),
            true,
            None,
        );
        let mixed = self.env.prefs.extras().mixed_playlists;
        self.set_meta(|m| m.mix_current = mixed.iter().any(|k| k == super::prefs::LIKED_MIX_KEY));
        self.load_page();
    }

    pub fn search(self: &Arc<Self>, query: String) {
        let query = query.trim().to_string();
        let generation = self.begin_nav(
            Source::Search(query.clone()),
            "search",
            query.clone(),
            String::new(),
            true,
            None,
        );
        if let Some(_commit) = self.commit_navigation(generation) {
            self.searches.submit(query);
        }
    }

    pub fn fetch_more(self: &Arc<Self>) {
        if matches!(self.nav().source, Source::Search(_)) {
            self.searches.more();
        } else {
            self.load_page();
        }
    }

    /// Conserva tracks/albums/loading para la UI existente, con la misma única petición.
    fn publish_search_compatibility(&self, view: &super::search::SearchView) {
        let _commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut nav = self.nav();
        if !self.searches.is_current(view)
            || !matches!(&nav.source, Source::Search(q) if *q == view.query)
        {
            return;
        }
        let key = (nav.gen_id, view.generation);
        nav.loading = view.loading;
        nav.has_more = view.has_more;
        drop(nav);
        // F4 keeps the rows (and their play order) in the track store, not in Nav.
        {
            let mut st = self.store();
            let unchanged = same_search_rows(
                &view.models[2],
                st.entries.iter().map(|e| &e.row),
                &[
                    ("trackId", "objectId"),
                    ("uri", "uri"),
                    ("title", "title"),
                    ("artist", "subtitle"),
                    ("album", "album"),
                    ("durationMs", "durationMs"),
                    ("imageUrl", "imageUrl"),
                    ("artistId", "artistId"),
                    ("albumId", "albumId"),
                ],
            ) && st
                .entries
                .iter()
                .all(|e| e.row["liked"] == false && e.row["addedLabel"] == "");
            if !unchanged {
                let tracks: Vec<Value> = view.models[2].iter().map(|r|json!({
                "trackId":r["objectId"],"uri":r["uri"],"title":r["title"],"artist":r["subtitle"],
                "album":r["album"],"durationMs":r["durationMs"],"imageUrl":r["imageUrl"],"liked":false,
                "artistId":r["artistId"],"albumId":r["albumId"],"addedLabel":""
            })).collect();
                let old: Vec<_> = st.entries.iter().map(|e| e.row.clone()).collect();
                let changes = super::search::row_changes_for(
                    &tracks,
                    &old,
                    &[
                        "trackId",
                        "uri",
                        "title",
                        "artist",
                        "album",
                        "durationMs",
                        "liked",
                        "artistId",
                        "albumId",
                        "addedLabel",
                    ],
                    &["imageUrl"],
                );
                for op in changes {
                    match &op {
                        ModelOp::Replace(rows) => {
                            st.clear();
                            self.gen_of(LibModel::Tracks).bump();
                            st.entries = rows.iter().cloned().map(Entry::from_row).collect();
                        }
                        ModelOp::Append(rows) => {
                            st.entries.extend(rows.iter().cloned().map(Entry::from_row))
                        }
                        ModelOp::SetField { row, field, value } => {
                            st.entries[*row].row[*field] = value.clone()
                        }
                        _ => unreachable!("row_changes solo produce Replace/Append/SetField"),
                    }
                    self.ui.send((LibModel::Tracks, op));
                }
            }
        }
        let mut previous = self.search_albums.lock().unwrap_or_else(|e| e.into_inner());
        let unchanged = previous.navigation == Some(key)
            && same_search_rows(
                &view.models[5],
                previous.rows.iter(),
                &[
                    ("albumId", "objectId"),
                    ("name", "title"),
                    ("artist", "subtitle"),
                    ("imageUrl", "imageUrl"),
                    ("year", "year"),
                ],
            );
        if !unchanged {
            let albums: Vec<Value> = view.models[5]
                .iter()
                .map(|r| {
                    json!({"albumId":r["objectId"],"name":r["title"],
            "artist":r["subtitle"],"imageUrl":r["imageUrl"],"year":r["year"]})
                })
                .collect();
            let changes = if previous.navigation == Some(key) {
                super::search::row_changes_for(
                    &albums,
                    &previous.rows,
                    &["albumId", "name", "artist", "year"],
                    &["imageUrl"],
                )
            } else {
                vec![ModelOp::Replace(albums.clone())]
            };
            for op in changes {
                self.ui.send((LibModel::Albums, op));
            }
            *previous = SearchAlbums {
                navigation: Some(key),
                rows: albums,
            };
        }
        self.set_meta(|m| {
            m.loading = view.loading;
            m.has_more = view.has_more;
        });
    }

    pub fn open_album(self: &Arc<Self>, id: String) {
        let request = self.requests.bump();
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            match api.album(&id).await {
                Ok(album) => {
                    if this.requests.current() != request {
                        return;
                    }
                    this.begin_nav(
                        Source::Album {
                            id: album.id.clone(),
                            uri: album.uri.clone(),
                        },
                        "album",
                        album.name.clone(),
                        album_subtitle(&album),
                        true,
                        Some(&album.images),
                    );
                    this.load_page();
                }
                Err(e) => this.env.report(&e),
            }
        });
    }

    pub fn open_artist(self: &Arc<Self>, id: String) {
        let request = self.requests.bump();
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            let artist: Artist = match api.artist(&id).await {
                Ok(a) => a,
                Err(e) => return this.env.report(&e),
            };
            if this.requests.current() != request {
                return;
            }
            let gen_id = this.begin_nav(
                Source::Artist {
                    uri: artist.uri.clone(),
                },
                "artist",
                artist.name.clone(),
                artist.genres.join(", "),
                false,
                Some(&artist.images),
            );
            {
                let Some(_commit) = this.commit_navigation(gen_id) else {
                    return;
                };
                this.artists.open(artist.uri.clone());
            }
            let mut albums: Vec<Album> = Vec::new();
            let mut offset = 0;
            // The endpoint pages by 10 at most: cap the discography at 50 (5 requests).
            while albums.len() < 50 {
                match api.artist_albums(&id, offset, PAGE).await {
                    Ok(page) => {
                        let more = page.has_more() && !page.items.is_empty();
                        offset += page.items.len() as u32;
                        albums.extend(page.items);
                        if !more {
                            break;
                        }
                    }
                    Err(e) => {
                        this.env.report(&e);
                        break;
                    }
                }
            }
            let rows = albums.iter().map(album_row).collect();
            let images = albums.iter().map(|a| image_of(&a.images)).collect();
            let Some(_commit) = this.commit_navigation(gen_id) else {
                return;
            };
            this.set_rows(LibModel::Albums, rows, images);
        });
    }

    /// U4: profile page of a user ("me" = the signed-in account). The header and lists come
    /// from `profiles`; the own one also shows `topTracks` of Inicio, read here if missing.
    pub fn open_profile(self: &Arc<Self>, user_id: String) {
        let gen_id = self.begin_nav(
            Source::None,
            "profile",
            String::new(),
            String::new(),
            true,
            None,
        );
        let is_me = {
            let Some(_commit) = self.commit_navigation(gen_id) else {
                return;
            };
            self.profiles.open(user_id)
        };
        if !is_me || self.home.has_top() {
            return;
        }
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            let Ok(page) = api.top_tracks(TimeRange::ShortTerm, 0, 20).await else {
                return;
            };
            let Some(_commit) = this.commit_navigation(gen_id) else {
                return;
            };
            let (rows, images) = this.track_rows(&page.items);
            this.home.adopt_top(page.items, rows, images);
        });
    }

    pub fn open_saved_albums(self: &Arc<Self>) {
        self.begin_nav(
            Source::SavedAlbums,
            "albums",
            "Álbumes".into(),
            String::new(),
            true,
            None,
        );
        self.set_rows(LibModel::SavedAlbums, Vec::new(), Vec::new());
        self.load_page();
    }

    pub fn open_followed_artists(self: &Arc<Self>) {
        let gen_id = self.begin_nav(
            Source::None,
            "artists",
            "Artistas".into(),
            String::new(),
            true,
            None,
        );
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            match api.followed_artists(10_000).await {
                Ok(artists) => {
                    let Some(_commit) = this.commit_navigation(gen_id) else {
                        return;
                    };
                    let rows = artists.iter().map(artist_row).collect();
                    let images = artists.iter().map(|a| image_of(&a.images)).collect();
                    this.set_rows(LibModel::FollowedArtists, rows, images);
                }
                Err(e) => this.env.report(&e),
            }
        });
    }

    // Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
    pub fn load_home(self: &Arc<Self>) {
        self.history_changed();
        let discovery = self.discover.begin_home();
        let gen_id = self.begin_nav(
            Source::None,
            "home",
            "Inicio".into(),
            String::new(),
            true,
            None,
        );
        let Some(api) = self.api() else { return };
        let home_ticket = self.home.begin(api.clone());
        self.home.load_spotify_shortcuts();
        let this = self.clone();
        self.env.rt.spawn(async move {
            let (recent, top, top_tracks) = super::home::home_reads(&*api).await;
            let Some(_commit) = this.commit_navigation(gen_id) else {
                return;
            };
            // G3: Spotify gives no top list through the session and the Web API may be limited
            // (429): rank the local history by plays instead; fewer than 3 hides the section.
            let top_tracks = match top_tracks {
                Ok(page) if !page.items.is_empty() => Ok(page),
                other => {
                    let reader = this
                        .history
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    let local = reader.top(10);
                    if local.len() >= 3 {
                        let total = local.len() as u32;
                        Ok(velo_domain::Page {
                            items: local,
                            offset: 0,
                            limit: total,
                            total,
                        })
                    } else {
                        other
                    }
                }
            };
            let recommended_seeds: Vec<String> = top_tracks
                .as_ref()
                .map(|p| p.items.iter().take(3).map(|t| t.uri.clone()).collect())
                .unwrap_or_default();
            let top_uris: std::collections::HashSet<String> = top_tracks
                .as_ref()
                .map(|p| p.items.iter().map(|t| t.uri.clone()).collect())
                .unwrap_or_default();
            this.discover.set_home_seeds(
                discovery,
                super::discover::home_seeds(
                    top_tracks
                        .as_ref()
                        .map(|p| p.items.as_slice())
                        .unwrap_or_default(),
                    top.as_ref().map(|p| p.items.as_slice()).unwrap_or_default(),
                ),
            );
            let mut seeds = recommended_seeds;
            if let Ok(tracks) = &recent {
                seeds.extend(tracks.iter().take(3).map(|t| t.uri.clone()));
            }
            this.load_recommended(home_ticket, seeds, top_uris);
            match recent {
                Ok(tracks) => {
                    let tracks = dedupe_tracks(tracks, 20);
                    this.home.set_recent(home_ticket, tracks.clone());
                    this.set_meta(|m| {
                        m.recent_source = recent_source(!tracks.is_empty(), false).into()
                    });
                    let (rows, images) = this.track_rows(&tracks);
                    this.set_rows(LibModel::Recent, rows, images);
                }
                Err(e) => {
                    this.set_meta(|m| m.recent_source = "local".into());
                    this.history_changed();
                    this.env.report(&e);
                }
            }
            match top_tracks {
                Ok(page) => {
                    let (rows, images) = this.track_rows(&page.items);
                    this.home.set_top(home_ticket, page.items, rows, images);
                }
                Err(e) => {
                    this.home
                        .set_top(home_ticket, Vec::new(), Vec::new(), Vec::new());
                    this.env.report(&e);
                }
            }
            match top {
                Ok(page) => {
                    let rows = page.items.iter().map(artist_row).collect();
                    let images = page.items.iter().map(|a| image_of(&a.images)).collect();
                    this.set_rows(LibModel::TopArtists, rows, images);
                }
                Err(e) => this.env.report(&e),
            }
        });
    }

    /// G3: "Recomendado para ti" = radios of the seed tracks through the audio session. Any
    /// failure leaves the list empty and the section hidden.
    fn load_recommended(
        self: &Arc<Self>,
        ticket: u64,
        seeds: Vec<String>,
        exclude: std::collections::HashSet<String>,
    ) {
        let Some(engine) = self.env.audio.engine() else {
            return;
        };
        if seeds.is_empty() {
            return;
        }
        let this = self.clone();
        self.env.rt.spawn(async move {
            let tracks =
                velo_player::discover::recommended_tracks(engine.session(), &seeds, &exclude, 20)
                    .await;
            if tracks.is_empty() {
                return;
            }
            let (rows, images) = this.track_rows(&tracks);
            this.home.set_recommended(ticket, tracks, rows, images);
        });
    }

    // ------------------------------------------------------------------ playback of the open list

    /// `row`: displayed row of `tracks` (F4: with a search or sort it is mapped to the list).
    fn play_target(self: &Arc<Self>, row: Option<usize>, shuffle: bool) {
        self.play_plan_with(shuffle, move |this| {
            let source = this.nav().source.clone();
            // Mixear: la playlist abierta decide si el fundido cruzado queda activo.
            let (uri, title) = match &source {
                Source::Playlist { uri, .. } | Source::Album { uri, .. } => {
                    (uri.as_str(), this.meta.borrow().title.clone())
                }
                Source::Liked => (
                    super::prefs::LIKED_CONTEXT,
                    this.meta.borrow().title.clone(),
                ),
                _ => ("", String::new()),
            };
            this.set_playing_context(uri, &title);
            this.apply_mix_context(uri);
            play_plan(&source, &this.store(), row)
        });
    }

    /// `Library.playContext(uri)`: same path as a page's play button, in the active device.
    pub fn play_context(self: &Arc<Self>, uri: &str) {
        if let Some(plan) = context_plan(uri) {
            let uri = uri.to_string();
            self.play_plan_with(self.player.view.borrow().shuffle, move |this| {
                this.set_playing_context(&uri, "");
                this.apply_mix_context(&uri);
                Some(plan)
            });
        }
    }

    /// `plan` runs under the navigation lock, so a page and its rows are read consistently.
    fn play_plan_with(
        self: &Arc<Self>,
        shuffle: bool,
        plan: impl FnOnce(&Arc<Self>) -> Option<PlayPlan> + Send + 'static,
    ) {
        let Some(api) = self.api() else { return };
        let this = self.clone();
        let generation = self.player.begin_playback();
        self.env.rt.spawn(async move {
            let plan = {
                let _commit = this
                    .navigation_write
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let Some(plan) = plan(&this) else {
                    return;
                };
                plan
            };
            // Velo is the default device: with its engine running and nothing else in charge,
            // the list plays here through Spirc (no Web API round trip, no device to wake).
            let Some(target) = velo_target(&plan, shuffle) else {
                return;
            };
            #[cfg(test)]
            this.playback_target.send_replace(Some(target.clone()));
            let route = match this.player.prepare_playback().await {
                Ok(Some(route)) if this.player.playback_current(generation) => route,
                Ok(_) => return,
                Err(e) => {
                    this.env.report(&e);
                    return;
                }
            };
            if route == PlayRoute::Velo {
                if let Some(engine) = this.env.audio.engine() {
                    match engine.load(target) {
                        Ok(()) => return,
                        Err(e) => {
                            let _ = this.env.errors.send(super::audio::audio_message(&e));
                            return;
                        }
                    }
                }
            }
            let result: Result<()> = async {
                let env = &this.env;
                match &plan {
                    PlayPlan::Context { uri, index } => {
                        let offset = index.map(|i| i as u32);
                        with_device!(
                            env,
                            &api,
                            async {
                                api.set_shuffle(shuffle).await?;
                                api.play_context(uri, offset).await
                            }
                            .await
                        )
                    }
                    PlayPlan::List { uris, index } => {
                        let list = uris_from(uris, *index);
                        if list.is_empty() {
                            return Ok(());
                        }
                        with_device!(
                            env,
                            &api,
                            async {
                                api.set_shuffle(shuffle).await?;
                                api.play_uris(&list, Some(0)).await
                            }
                            .await
                        )
                    }
                }
            }
            .await;
            match result {
                Ok(()) => this.player.resync(),
                Err(e) => this.env.report(&e),
            }
        });
    }

    pub fn play_index(self: &Arc<Self>, index: usize) {
        self.play_target(Some(index), self.player.view.borrow().shuffle);
    }

    /// Mixear: reevalúa el fundido cruzado para el contexto que va a sonar.
    fn apply_mix_context(&self, uri: &str) {
        let extras = self.env.prefs.extras();
        // Guardar solo la marca por playlist: el ajuste automático se lee en caliente en AudioCtl.
        self.env.audio.set_mix_context(super::prefs::mix_wanted(
            uri,
            &extras.mixed_playlists,
            false,
        ));
    }

    /// Contexto que suena: lo publica en `Player.contextUri/contextTitle` y recalcula
    /// `mix_playing`. Título vacío = se desconoce.
    fn set_playing_context(&self, uri: &str, title: &str) {
        *self
            .playing_context
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = (uri.to_string(), title.to_string());
        self.player.view.send_if_modified(|v| {
            if v.context_uri == uri && v.context_title == title {
                return false;
            }
            v.context_uri = uri.to_string();
            v.context_title = title.to_string();
            true
        });
        let marked = self.is_mix_marked(uri);
        self.set_meta(|m| m.mix_playing = marked);
    }

    fn is_mix_marked(&self, context_uri: &str) -> bool {
        super::prefs::mix_key(context_uri).is_some_and(|key| {
            self.env
                .prefs
                .extras()
                .mixed_playlists
                .iter()
                .any(|m| m == key)
        })
    }

    /// Marca o desmarca una clave de Mixear; devuelve el estado nuevo.
    fn flip_mix_key(&self, key: &str) -> bool {
        let mut now = false;
        self.env.prefs.update_extras(|e| {
            if let Some(pos) = e.mixed_playlists.iter().position(|m| m == key) {
                e.mixed_playlists.remove(pos);
            } else {
                e.mixed_playlists.push(key.to_string());
                now = true;
            }
        });
        now
    }

    /// Contexto que suena ahora: el que Velo lanzó o, si no, el de la sesión.
    fn playing_context_uri(&self) -> String {
        let own = self
            .playing_context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .0
            .clone();
        if !own.is_empty() {
            return own;
        }
        self.env
            .account
            .session_queue_snapshot()
            .map(|s| s.context_uri)
            .unwrap_or_default()
    }

    /// Marca o desmarca la playlist abierta (o «Tus me gusta») para Mixear; si es la que suena,
    /// aplica al instante.
    pub fn toggle_mix_current(self: &Arc<Self>) {
        let this = self.clone();
        self.env.rt.spawn(async move {
            let key = match this.nav().source.clone() {
                Source::Playlist { id, .. } => id,
                Source::Liked => super::prefs::LIKED_MIX_KEY.to_string(),
                _ => return,
            };
            let now = this.flip_mix_key(&key);
            this.set_meta(|m| m.mix_current = now);
            let playing = this.playing_context_uri();
            if super::prefs::mix_key(&playing) == Some(key.as_str()) {
                this.set_meta(|m| m.mix_playing = now);
                this.apply_mix_context(&playing);
            }
        });
    }

    /// Marca o desmarca el contexto que SUENA (playlist o «Tus me gusta») para Mixear.
    pub fn toggle_mix_playing(self: &Arc<Self>) {
        let this = self.clone();
        self.env.rt.spawn(async move {
            this.toggle_mix_playing_now();
        });
    }

    fn toggle_mix_playing_now(&self) -> Option<bool> {
        let uri = self.playing_context_uri();
        let key = super::prefs::mix_key(&uri)?.to_string();
        let now = self.flip_mix_key(&key);
        self.set_meta(|m| m.mix_playing = now);
        let open_key = match &self.nav().source {
            Source::Playlist { id, .. } => Some(id.clone()),
            Source::Liked => Some(super::prefs::LIKED_MIX_KEY.to_string()),
            _ => None,
        };
        if open_key.as_deref() == Some(key.as_str()) {
            self.set_meta(|m| m.mix_current = now);
        }
        self.apply_mix_context(&uri);
        Some(now)
    }

    pub fn play_current(self: &Arc<Self>) {
        self.play_target(None, self.player.view.borrow().shuffle);
    }

    pub fn shuffle_current(self: &Arc<Self>) {
        self.play_target(None, true);
    }

    // ------------------------------------------------------------------ pins, queue, editing

    /// Event-driven C3 hook. The account-scoped reader notifies on append and clear.
    pub fn install_history_reader(
        self: &Arc<Self>,
        reader: Arc<dyn super::sidebar::HistoryReader>,
        mut changes: watch::Receiver<u64>,
    ) {
        *self.history.lock().unwrap_or_else(|e| e.into_inner()) = reader;
        let mut slot = self.history_task.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = slot.take() {
            task.abort();
        }
        let weak = Arc::downgrade(self);
        *slot = Some(
            self.env
                .rt
                .spawn(async move {
                    loop {
                        changes.borrow_and_update();
                        let Some(this) = weak.upgrade() else { break };
                        this.history_changed();
                        drop(this);
                        if changes.changed().await.is_err() {
                            break;
                        }
                    }
                })
                .abort_handle(),
        );
    }
    fn history_changed(&self) {
        let reader = self
            .history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.sidebar.history_changed(reader.last_played_contexts());
        let tracks = dedupe_tracks(reader.recent(50), 20);
        if !tracks.is_empty() || self.meta.borrow().recent_source == "local" {
            self.set_meta(|m| m.recent_source = recent_source(false, !tracks.is_empty()).into());
            let (rows, images) = self.track_rows(&tracks);
            self.set_rows(LibModel::Recent, rows, images);
        }
    }
    pub fn toggle_library_pin(self: &Arc<Self>, uri: String) {
        self.sidebar.toggle_pin(uri);
        let all = self
            .playlists
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.publish_playlists(all);
    }
    pub fn refresh_sidebar(self: &Arc<Self>) {
        let this = self.clone();
        self.env.rt.spawn(async move {
            this.load_playlists().await;
            this.sidebar.refresh();
        });
    }
    pub fn open_show(self: &Arc<Self>, id: String) {
        let gen_id = self.begin_nav(
            Source::Show { id: id.clone() },
            "show",
            String::new(),
            String::new(),
            true,
            None,
        );
        self.set_meta(|m| m.loading = true);
        let this = self.clone();
        self.env.rt.spawn(async move {
            let Some(engine) = this
                .env
                .audio
                .engine()
                .filter(|_| this.env.audio.is_ready())
            else {
                let Some(_commit) = this.commit_navigation(gen_id) else {
                    return;
                };
                this.set_meta(|m| {
                    m.loading = false;
                    m.listable = false;
                });
                return;
            };
            match velo_player::session_api::shows::show(engine.session(), &id).await {
                Ok(show) => {
                    let Ok(entry) = velo_player::session_api::shows::entry(&show, None) else {
                        return;
                    };
                    let Some(commit) = this.commit_navigation(gen_id) else {
                        return;
                    };
                    this.set_meta(|m| {
                        m.title = entry.name;
                        m.subtitle = entry.creator;
                        m.loading = false;
                    });
                    drop(commit);
                    this.load_page();
                    if let Some(url) = image_of(&entry.images) {
                        if let Some(file) = this.thumbs.one(&url, HEADER_PX).await {
                            if let Some(_commit) = this.commit_navigation(gen_id) {
                                this.set_meta(|m| m.image_url = file);
                            }
                        }
                    }
                }
                Err(_) => {
                    let Some(_commit) = this.commit_navigation(gen_id) else {
                        return;
                    };
                    this.set_meta(|m| {
                        m.loading = false;
                        m.listable = false;
                    });
                }
            }
        });
    }

    pub fn toggle_pin(self: &Arc<Self>, playlist_id: String) {
        let this = self.clone();
        self.env
            .rt
            .spawn(async move { this.update_pin(playlist_id) });
    }

    fn update_pin(self: &Arc<Self>, playlist_id: String) {
        self.toggle_library_pin(format!("spotify:playlist:{playlist_id}"));
    }

    /// M1: reload the sidebar after following/unfollowing a playlist from a menu.
    pub fn refresh_playlists(self: &Arc<Self>) {
        let this = self.clone();
        self.env
            .rt
            .spawn(async move { this.load_playlists().await });
    }

    pub fn add_to_queue(self: &Arc<Self>, uri: String) {
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            let result: Result<()> = async {
                let env = &this.env;
                with_device!(env, &api, api.add_to_queue(&uri).await)
            }
            .await;
            match result {
                Ok(()) => this.player.refresh_queue(),
                Err(e) => this.env.report(&e),
            }
        });
    }

    pub fn create_playlist(self: &Arc<Self>, name: String) {
        let name = name.trim().to_string();
        let Some(api) = self.api() else { return };
        if name.is_empty() {
            return;
        }
        let this = self.clone();
        self.env.rt.spawn(async move {
            match api.create_playlist(&name, "", false).await {
                Ok(_) => this.load_playlists().await,
                Err(e) => this.env.report(&e),
            }
        });
    }

    /// M1b "Editar detalles": renames / redescribes an own playlist. On success the sidebar row
    /// and, when it is the open list, the header change in place (no reload: `/me/playlists`
    /// may still answer with the old name for a moment).
    pub fn update_playlist_details(
        self: &Arc<Self>,
        playlist_id: String,
        name: String,
        description: String,
    ) {
        let name = name.trim().to_string();
        let description = description.trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some((api, ticket)) = self.env.account.api_ticket() else {
            return;
        };
        let (done, tail) = tokio::sync::oneshot::channel();
        let previous = {
            let mut tails = self.details_tail.lock().unwrap_or_else(|e| e.into_inner());
            tails.retain(|_, tail| {
                matches!(
                    tail.try_recv(),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                )
            });
            tails.insert(playlist_id.clone(), tail)
        };
        let this = self.clone();
        self.env.rt.spawn(async move {
            // Drop despierta al siguiente incluso ante error o cancelación de la tarea.
            let _done = done;
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            if !this.env.account.commit_if_current(ticket, || {}) {
                return;
            }
            let result = api
                .update_playlist_details(&playlist_id, &name, &description)
                .await;
            this.env.account.commit_if_current(ticket, || {
                if let Err(e) = result {
                    return this.env.report(&e);
                }
                let all = {
                    let mut cached = this.playlists.lock().unwrap_or_else(|e| e.into_inner());
                    apply_details(&mut cached, &playlist_id, &name, &description);
                    cached.clone()
                };
                this.publish_playlists(all);
                let _commit = this
                    .navigation_write
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let is_open =
                    matches!(&this.nav().source, Source::Playlist { id, .. } if *id == playlist_id);
                if is_open {
                    this.set_meta(|m| m.title.clone_from(&name));
                }
            });
        });
    }

    pub fn add_to_playlist(self: &Arc<Self>, playlist_id: String, uri: String) {
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            if let Err(e) = api.add_to_playlist(&playlist_id, &[uri]).await {
                this.env.report(&e);
            }
        });
    }

    pub fn remove_from_playlist(self: &Arc<Self>, playlist_id: String, uri: String) {
        let Some(api) = self.api() else { return };
        let this = self.clone();
        self.env.rt.spawn(async move {
            match api
                .remove_from_playlist(&playlist_id, std::slice::from_ref(&uri))
                .await
            {
                Ok(()) => this.drop_rows_of(&playlist_id, &uri),
                Err(e) => this.env.report(&e),
            }
        });
    }

    /// Removes the rows of `uri` from the open playlist (the API removes every occurrence).
    fn drop_rows_of(&self, playlist_id: &str, uri: &str) {
        let _commit = self
            .navigation_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let n = self.nav();
            let is_open = matches!(&n.source, Source::Playlist { id, .. } if id == playlist_id);
            if !is_open {
                return;
            }
        }
        let (removed, duration_ms) = {
            let mut st = self.store();
            let entries: Vec<usize> = (0..st.entries.len())
                .filter(|&i| st.entries[i].uri == uri)
                .collect();
            if entries.is_empty() {
                return;
            }
            let mut rows: Vec<usize> = entries.iter().filter_map(|&i| st.shown_row(i)).collect();
            rows.sort_unstable();
            st.entries.retain(|e| e.uri != uri);
            if st.view.is_none() {
                for row in rows.into_iter().rev() {
                    self.ui.send((LibModel::Tracks, ModelOp::Remove(row)));
                }
            } else {
                let rows = st.apply();
                self.ui.send((LibModel::Tracks, ModelOp::Replace(rows)));
            }
            // Entry indexes moved: covers still on their way are requested again.
            self.refill_track_images(&mut st);
            (entries.len() as u32, st.duration_ms())
        };
        {
            let mut n = self.nav();
            n.offset = n.offset.saturating_sub(removed);
        }
        self.set_meta(|m| {
            m.track_count = m.track_count.saturating_sub(removed);
            m.duration_ms = duration_ms;
        });
        let tags = self.store().tags.clone();
        self.publish_tags(tags);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mix_auto_can_be_disabled_after_loading_an_unmarked_context() {
        let rt = super::super::r4b_test_support::runtime();
        let (_dir, env) = super::super::test_env(&rt);
        let player = PlayerCtl::for_test(&env);
        let ctl = LibraryCtl::start(&env, &player);
        env.prefs
            .update_extras(|e| e.mixed_playlists.push("marked".into()));
        env.prefs.update_settings(|s| s.playback.mix_auto = true);
        for uri in ["spotify:playlist:unmarked", "spotify:album:a", ""] {
            ctl.apply_mix_context(uri);
            assert!(env.audio.mix_config(&env.prefs.settings()).enabled);
            env.prefs.update_settings(|s| s.playback.mix_auto = false);
            assert!(
                !env.audio.mix_config(&env.prefs.settings()).enabled,
                "{uri}"
            );
            env.prefs.update_settings(|s| s.playback.mix_auto = true);
        }
        ctl.apply_mix_context("spotify:playlist:marked");
        env.prefs.update_settings(|s| s.playback.mix_auto = false);
        assert!(env.audio.mix_config(&env.prefs.settings()).enabled);
        ctl.apply_mix_context("spotify:playlist:unmarked");
        assert!(!env.audio.mix_config(&env.prefs.settings()).enabled);
    }

    #[test]
    fn local_history_notifications_update_recent_and_context_dates_then_clear() {
        struct Reader(Mutex<Vec<Track>>);
        impl super::super::sidebar::HistoryReader for Reader {
            fn recent(&self, _: usize) -> Vec<Track> {
                self.0.lock().unwrap().clone()
            }
            fn last_played_contexts(&self) -> HashMap<String, i64> {
                if self.0.lock().unwrap().is_empty() {
                    HashMap::new()
                } else {
                    HashMap::from([("spotify:playlist:a".into(), 42)])
                }
            }
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_dir, env) = super::super::test_env(&rt);
            let player = PlayerCtl::start(&env);
            let library = LibraryCtl::start(&env, &player);
            library
                .sidebar
                .set_playlists(&[playlist("a"), playlist("b")], "juan");
            let reader = Arc::new(Reader(Mutex::new(vec![track("spotify:track:test")])));
            let (changes, rx) = watch::channel(0);
            library.install_history_reader(reader.clone(), rx);
            tokio::time::timeout(Duration::from_secs(2), async {
                while library.meta.borrow().recent_source != "local" {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let rows = library.sidebar.view.borrow().rows.clone();
            assert_eq!(
                rows.iter()
                    .find(|r| r["uri"] == "spotify:playlist:a")
                    .unwrap()["lastPlayedAtMs"],
                42
            );
            assert_eq!(
                rows.iter()
                    .find(|r| r["uri"] == "spotify:playlist:b")
                    .unwrap()["lastPlayedAtMs"],
                -1
            );
            reader.0.lock().unwrap().clear();
            changes.send_replace(1);
            tokio::time::timeout(Duration::from_secs(2), async {
                while library.meta.borrow().recent_source != "none" {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(
                library
                    .sidebar
                    .view
                    .borrow()
                    .rows
                    .iter()
                    .all(|r| r["lastPlayedAtMs"] == -1)
            );
        });
    }

    #[test]
    fn recent_source_distinguishes_429_local_fallback_and_empty() {
        assert_eq!(super::recent_source(true, false), "spotify");
        assert_eq!(super::recent_source(false, true), "local");
        assert_eq!(super::recent_source(false, false), "none");
    }

    use super::super::r4b_test_support::{ManualTime, Server, runtime, settle};
    use super::*;
    use velo_domain::ArtistRef;

    #[test]
    fn f5_row_errors_are_cacheable_empty_labels_and_session_errors_abort() {
        use velo_player::descriptors::{Descriptor, DescriptorError, DescriptorKind};
        for error in [
            DescriptorError::Unavailable,
            DescriptorError::Format,
            DescriptorError::Timeout,
            DescriptorError::Empty,
            DescriptorError::InvalidUri,
        ] {
            assert_eq!(descriptor_labels(Err(error)).unwrap(), Vec::<String>::new());
        }
        assert_eq!(
            descriptor_labels(Err(DescriptorError::Session)),
            Err(DescriptorError::Session)
        );
        assert_eq!(
            descriptor_labels(Ok(vec![Descriptor {
                text: "Rock".into(),
                kind: DescriptorKind::Genre,
                weight: 1.0
            }]))
            .unwrap(),
            ["rock"]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn f5_stream_replenishes_slots_before_slowest_and_publishes_progressively() {
        use std::sync::atomic::AtomicUsize;
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let mut order = Vec::new();
        stream_tag_results(
            (0..8).map(|i| i.to_string()).collect(),
            |uri| {
                let active = active.clone();
                let peak = peak.clone();
                let completed = completed.clone();
                async move {
                    let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    let index: u64 = uri.parse().unwrap();
                    tokio::time::sleep(Duration::from_millis(if index == 0 { 100 } else { 1 }))
                        .await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    completed.fetch_add(1, Ordering::SeqCst);
                    (uri, index)
                }
            },
            |_, index| {
                if order.is_empty() {
                    assert!(completed.load(Ordering::SeqCst) < 8);
                }
                order.push(index);
                true
            },
        )
        .await;
        assert_eq!(peak.load(Ordering::SeqCst), 4);
        assert_eq!(order.last(), Some(&0));
        assert!(
            order.iter().position(|i| *i == 4).unwrap()
                < order.iter().position(|i| *i == 0).unwrap()
        );
        assert_eq!(order.len(), 8);
    }

    #[test]
    fn f5_stale_refresh_does_not_clear_new_tags_and_load_page_clears_filter() {
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        let ctl = LibraryCtl::start(&env, &PlayerCtl::for_test(&env));
        ctl.begin_nav(
            Source::Liked,
            "liked",
            "Likes".into(),
            String::new(),
            true,
            None,
        );
        ctl.accept_tracks(0, vec![track("a"), track("b")], 2, false, 2);
        let gen_id = ctl.nav().gen_id;
        {
            let _commit = ctl.navigation_write.lock().unwrap();
            ctl.publish_tags(HashMap::from([("a".into(), vec!["rock".into()])]));
        }
        ctl.filter_tag("rock".into());
        ctl.refresh_tags(gen_id.wrapping_sub(1));
        assert_eq!(ctl.meta.borrow().track_tags, ["rock"]);
        assert_eq!(ctl.store().active_tag, "rock");
        ctl.nav().has_more = true;
        ctl.load_page();
        assert!(ctl.store().tags.is_empty() && ctl.store().active_tag.is_empty());
        assert!(ctl.meta.borrow().track_tags.is_empty() && ctl.meta.borrow().active_tag.is_empty());
        assert_eq!(ctl.store().shown_uris(), ["a", "b"]);
    }

    #[test]
    fn f5_bridge_state_filters_toggles_clears_on_failure_and_navigation() {
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        let ctl = LibraryCtl::start(&env, &PlayerCtl::for_test(&env));
        ctl.begin_nav(
            Source::Liked,
            "liked",
            "Likes".into(),
            String::new(),
            true,
            None,
        );
        ctl.accept_tracks(0, vec![track("a"), track("b"), track("a")], 3, false, 3);
        {
            let _commit = ctl.navigation_write.lock().unwrap();
            ctl.publish_tags(HashMap::from([
                ("a".into(), vec!["rock".into()]),
                ("b".into(), vec!["calm".into()]),
            ]));
        }
        assert_eq!(ctl.meta.borrow().track_tags, ["rock", "calm"]);
        ctl.filter_tag("rock".into());
        assert_eq!(ctl.meta.borrow().active_tag, "rock");
        assert_eq!(ctl.store().shown_uris(), ["a", "a"]);
        assert_eq!(
            play_plan(
                &Source::Playlist {
                    id: "p".into(),
                    uri: "spotify:playlist:p".into(),
                    foreign: false
                },
                &ctl.store(),
                None
            ),
            Some(PlayPlan::List {
                uris: vec!["a".into(), "a".into()],
                index: 0
            })
        );
        ctl.filter_tag("rock".into());
        assert_eq!(ctl.store().shown_uris(), ["a", "b", "a"]);
        ctl.filter_tag("calm".into());
        {
            let _commit = ctl.navigation_write.lock().unwrap();
            ctl.publish_tags(HashMap::new());
        }
        assert!(ctl.meta.borrow().track_tags.is_empty());
        assert!(ctl.meta.borrow().active_tag.is_empty());
        assert_eq!(ctl.store().shown_uris(), ["a", "b", "a"]);
        let old = ctl.nav().gen_id;
        ctl.tag_cache
            .lock()
            .unwrap()
            .insert("a".into(), vec!["rock".into()]);
        ctl.begin_nav(
            Source::Album {
                id: "album".into(),
                uri: "spotify:album:album".into(),
            },
            "album",
            "Album".into(),
            String::new(),
            true,
            None,
        );
        assert!(ctl.commit_navigation(old).is_none());
        assert!(ctl.meta.borrow().track_tags.is_empty() && ctl.meta.borrow().active_tag.is_empty());
        ctl.filter_tag("rock".into());
        assert!(ctl.meta.borrow().active_tag.is_empty());
        ctl.reset_all();
        assert!(ctl.tag_cache.lock().unwrap().is_empty());
    }

    #[test]
    fn toggle_mix_playing_marks_playlist_and_liked_contexts() {
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        rt.block_on(async {
            let ctl = LibraryCtl::start(&env, &PlayerCtl::for_test(&env));
            // Sin contexto que suene no hay nada que marcar.
            assert_eq!(ctl.toggle_mix_playing_now(), None);
            // Álbum: Mixear no aplica.
            ctl.set_playing_context("spotify:album:a", "Álbum");
            assert_eq!(ctl.toggle_mix_playing_now(), None);
            assert!(env.prefs.extras().mixed_playlists.is_empty());
            // Playlist que suena.
            ctl.set_playing_context("spotify:playlist:p", "Mi lista");
            assert!(!ctl.meta.borrow().mix_playing);
            assert_eq!(ctl.toggle_mix_playing_now(), Some(true));
            assert!(ctl.meta.borrow().mix_playing);
            assert_eq!(env.prefs.extras().mixed_playlists, vec!["p"]);
            // Volver a darle play la reconoce como marcada.
            ctl.set_playing_context("spotify:playlist:p", "Mi lista");
            assert!(ctl.meta.borrow().mix_playing);
            // «Tus me gusta».
            ctl.set_playing_context(super::super::prefs::LIKED_CONTEXT, "Tus me gusta");
            assert!(!ctl.meta.borrow().mix_playing);
            assert_eq!(ctl.toggle_mix_playing_now(), Some(true));
            assert!(
                env.prefs
                    .extras()
                    .mixed_playlists
                    .contains(&"liked".to_string())
            );
            assert_eq!(ctl.toggle_mix_playing_now(), Some(false));
            assert_eq!(env.prefs.extras().mixed_playlists, vec!["p"]);
            assert_eq!(ctl.player.view.borrow().context_title, "Tus me gusta");
        });
    }

    async fn details_finished(ctl: &LibraryCtl, id: &str) {
        let tail = ctl.details_tail.lock().unwrap().remove(id).unwrap();
        assert!(tail.await.is_err());
    }

    #[test]
    fn r4b_details_keep_invocation_order_and_ignore_logout_answers() {
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        rt.block_on(async {
            let _time = ManualTime::new();
            let mut server = Server::new().await;
            env.account.set_test_provider(server.provider());
            let ctl = LibraryCtl::start(&env, &PlayerCtl::for_test(&env));
            ctl.publish_playlists(vec![playlist("p")]);
            ctl.begin_nav(
                Source::Playlist {
                    id: "p".into(),
                    uri: "spotify:playlist:p".into(),
                    foreign: false,
                },
                "playlist",
                "p".into(),
                String::new(),
                true,
                None,
            );
            ctl.update_playlist_details("p".into(), "A".into(), "uno".into());
            ctl.update_playlist_details("p".into(), "B".into(), "dos".into());
            let a = server.next().await;
            assert_eq!(
                (a.method.as_str(), a.path.as_str()),
                ("PUT", "/playlists/p")
            );
            assert_eq!(a.body, json!({"name":"A","description":"uno"}));
            // Barrera HTTP: A permanece pendiente. Otra playlist sí puede avanzar.
            ctl.update_playlist_details("other".into(), "C".into(), "".into());
            let c = server.next().await;
            assert_eq!(
                c.path, "/playlists/other",
                "B no puede adelantarse a la respuesta de A"
            );
            c.respond(204, Value::Null);
            details_finished(&ctl, "other").await;
            tokio::time::advance(Duration::from_secs(1)).await;
            assert_eq!(ctl.meta.borrow().title, "p");
            a.respond(204, Value::Null);
            let b = server.next().await;
            assert_eq!(b.body, json!({"name":"B","description":"dos"}));
            assert_eq!(ctl.meta.borrow().title, "A");
            b.respond(204, Value::Null);
            details_finished(&ctl, "p").await;
            assert_eq!(ctl.meta.borrow().title, "B");
            assert_eq!(ctl.playlists.lock().unwrap()[0].description, "dos");

            // Logout invalida inmediatamente aunque su tarea y la respuesta no se hayan sondeado.
            ctl.update_playlist_details("p".into(), "Viejo".into(), "".into());
            let old = server.next().await;
            ctl.update_playlist_details("p".into(), "Pendiente".into(), "".into());
            env.account.sign_out();
            ctl.reset_all();
            old.respond(204, Value::Null);
            details_finished(&ctl, "p").await;
            settle().await;
            assert!(ctl.playlists.lock().unwrap().is_empty());
            assert!(ctl.meta.borrow().title.is_empty());
            assert!(
                server.requests.try_recv().is_err(),
                "no iniciar la escritura de la cuenta anterior"
            );

            // Incluso el mismo ID de usuario / provider nuevo debe invalidar la generación vieja.
            env.account.set_test_provider(server.provider());
            ctl.publish_playlists(vec![playlist("p")]);
            ctl.update_playlist_details("p".into(), "Otra sesión".into(), "".into());
            let old = server.next().await;
            env.account.set_test_provider(server.provider());
            old.respond(204, Value::Null);
            details_finished(&ctl, "p").await;
            assert_eq!(ctl.playlists.lock().unwrap()[0].name, "p");
        });
    }

    #[test]
    fn r4b_search_compatibility_only_changes_relevant_rows_and_keeps_play_order() {
        let rt = runtime();
        let (_dir, env) = super::super::test_env(&rt);
        let ctl = LibraryCtl::start(&env, &PlayerCtl::for_test(&env));
        ctl.begin_nav(
            Source::Search("q".into()),
            "search",
            "q".into(),
            String::new(),
            true,
            None,
        );
        let mut view = ctl.searches.view.borrow().clone();
        view.query = "q".into();
        view.generation = ctl.searches.view.borrow().generation;
        // leave() publica la vista vacía pero incrementa la generación interna.
        ctl.searches.submit(String::new());
        view.generation = ctl.searches.view.borrow().generation;
        view.models[2] = vec![
            json!({"objectId":"t1","uri":"spotify:track:t1","title":"Uno",
            "subtitle":"Artista","album":"Álbum","durationMs":1000,"imageUrl":"", "artistId":"a", "albumId":"b"}),
        ];
        view.models[5] = vec![
            json!({"objectId":"b","title":"Álbum","subtitle":"Artista","year":2026,"imageUrl":""}),
        ];
        ctl.publish_search_compatibility(&view);
        let mut ui = ctl.ui.take().unwrap();
        while ui.try_recv().is_ok() {}
        let generation = ctl.store().generation;
        view.models[0].push(json!({"title":"Sugerencia"}));
        view.models[3].push(json!({"objectId":"a","following":true}));
        view.artist_follow_busy = true;
        ctl.publish_search_compatibility(&view);
        assert!(
            ui.try_recv().is_err(),
            "sugerencias/seguimiento no deben tocar la compatibilidad"
        );
        assert_eq!(ctl.store().generation, generation);

        view.models[2][0]["imageUrl"] = json!("file:///track.png");
        view.models[5][0]["imageUrl"] = json!("file:///album.png");
        ctl.publish_search_compatibility(&view);
        assert_eq!(
            ui.try_recv().unwrap(),
            (
                LibModel::Tracks,
                ModelOp::SetField {
                    row: 0,
                    field: "imageUrl",
                    value: json!("file:///track.png")
                }
            )
        );
        assert_eq!(
            ui.try_recv().unwrap(),
            (
                LibModel::Albums,
                ModelOp::SetField {
                    row: 0,
                    field: "imageUrl",
                    value: json!("file:///album.png")
                }
            )
        );
        assert!(ui.try_recv().is_err());
        assert_eq!(ctl.store().entries[0].row["imageUrl"], "file:///track.png");
        assert_eq!(ctl.store().generation, generation);
        let mut second = view.models[2][0].clone();
        second["uri"] = json!("spotify:track:t2");
        second["objectId"] = json!("t2");
        view.models[2].push(second);
        ctl.publish_search_compatibility(&view);
        assert!(
            matches!(ui.try_recv().unwrap(), (LibModel::Tracks, ModelOp::Append(rows)) if rows.len()==1)
        );
        assert_eq!(
            ctl.store().shown_uris(),
            ["spotify:track:t1", "spotify:track:t2"]
        );
        view.models[2].swap(0, 1);
        ctl.publish_search_compatibility(&view);
        assert!(
            matches!(ui.try_recv().unwrap(), (LibModel::Tracks, ModelOp::Replace(rows)) if rows.len()==2)
        );
        assert_eq!(
            ctl.store().shown_uris(),
            ["spotify:track:t2", "spotify:track:t1"]
        );
        ctl.begin_nav(
            Source::Liked,
            "liked",
            "Nuevo".into(),
            String::new(),
            true,
            None,
        );
        while ui.try_recv().is_ok() {}
        ctl.publish_search_compatibility(&view);
        assert!(ui.try_recv().is_err());
        assert!(ctl.store().entries.is_empty());
    }

    fn playlist(id: &str) -> Playlist {
        Playlist {
            id: id.into(),
            uri: format!("spotify:playlist:{id}"),
            name: id.into(),
            description: String::new(),
            owner: "Juan".into(),
            owner_id: "juan".into(),
            collaborative: false,
            images: vec![],
            track_count: 3,
        }
    }

    fn track(uri: &str) -> Track {
        Track {
            id: None,
            uri: uri.into(),
            title: uri.into(),
            artists: vec![ArtistRef {
                id: None,
                name: "A".into(),
            }],
            album: "B".into(),
            album_id: None,
            duration_ms: 1,
            explicit: false,
            images: vec![],
            added_at: None,
        }
    }

    #[test]
    fn pinned_playlists_go_first_and_keep_order() {
        let mut list = vec![playlist("a"), playlist("b"), playlist("c"), playlist("d")];
        sort_pinned(&mut list, &["d".into(), "b".into()]);
        let ids: Vec<_> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["b", "d", "a", "c"]);
    }

    #[test]
    fn subtitles_are_built_from_the_domain() {
        assert_eq!(playlist_subtitle(&playlist("a")), "Juan");
        let album = Album {
            id: "x".into(),
            uri: "u".into(),
            name: "N".into(),
            artists: vec![ArtistRef {
                id: None,
                name: "Lumen".into(),
            }],
            images: vec![],
            release_date: "2024-01-02".into(),
            total_tracks: 9,
            album_type: "album".into(),
        };
        assert_eq!(album_subtitle(&album), "Lumen · 2024 · 9 canciones");
    }

    #[test]
    fn play_context_accepts_contexts_and_tracks_only() {
        for uri in ["spotify:album:a", "spotify:playlist:p", "spotify:artist:r"] {
            assert_eq!(
                context_plan(uri),
                Some(PlayPlan::Context {
                    uri: uri.into(),
                    index: None
                })
            );
        }
        assert_eq!(
            context_plan("spotify:track:t"),
            Some(PlayPlan::List {
                uris: vec!["spotify:track:t".into()],
                index: 0
            })
        );
        for uri in [
            "",
            "spotify:album:",
            "spotify:show:s",
            "https://x",
            "album:a",
        ] {
            assert_eq!(context_plan(uri), None, "{uri}");
        }
    }

    #[test]
    fn play_uris_start_at_the_row_and_are_capped() {
        let uris: Vec<String> = (0..250).map(|i| format!("u{i}")).collect();
        let list = uris_from(&uris, 10);
        assert_eq!(list.len(), MAX_PLAY_URIS);
        assert_eq!(list[0], "u10");
        assert!(uris_from(&uris, 400).is_empty());
    }

    fn store_of(uris: &[String]) -> TrackStore {
        let mut store = TrackStore::default();
        for uri in uris {
            let t = track(uri);
            store
                .entries
                .push(Entry::new(&t, json!({ "uri": uri }), None));
        }
        store.apply();
        store
    }

    #[test]
    fn play_index_preserves_player_shuffle_and_clicked_row() {
        for shuffle in [true, false] {
            let rt = runtime();
            let (_dir, env) = super::super::test_env(&rt);
            rt.block_on(async {
                let _time = ManualTime::new();
                let mut server = Server::new().await;
                env.account.set_test_provider(server.provider());
                let player = PlayerCtl::for_test(&env);
                let ctl = LibraryCtl::start(&env, &player);
                ctl.nav().source = Source::Search("recommended".into());
                let uris: Vec<String> = (0..5).map(|i| format!("spotify:track:{i}")).collect();
                *ctl.store() = store_of(&uris);
                player.view.send_modify(|v| v.shuffle = shuffle);
                let mut targets = ctl.playback_target.subscribe();

                ctl.play_index(2);
                targets.changed().await.unwrap();
                assert_eq!(
                    *targets.borrow(),
                    Some(LoadTarget::Tracks {
                        uris: uris.clone(),
                        index: 2,
                        shuffle
                    })
                );
                let devices = server.next().await;
                assert_eq!(devices.path, "/me/player/devices");
                devices.respond(
                    200,
                    json!({"devices": [{"id": "phone", "name": "Phone",
                    "type": "Smartphone", "is_active": true}]}),
                );
                let request = server.next().await;
                assert_eq!(request.method, "PUT");
                assert_eq!(request.path, format!("/me/player/shuffle?state={shuffle}"));
                request.respond(204, Value::Null);
                let request = server.next().await;
                assert_eq!(request.path, "/me/player/play");
                assert_eq!(
                    request.body,
                    json!({"uris": uris_from(&uris, 2),
                    "offset": {"position": 0}})
                );
                request.respond(204, Value::Null);
                settle().await;
            });
        }
    }

    #[test]
    fn context_and_current_playback_set_player_shuffle_in_both_directions() {
        for shuffle in [true, false] {
            for current in [false, true] {
                let rt = runtime();
                let (_dir, env) = super::super::test_env(&rt);
                rt.block_on(async {
                    let _time = ManualTime::new();
                    let mut server = Server::new().await;
                    env.account.set_test_provider(server.provider());
                    let player = PlayerCtl::for_test(&env);
                    let ctl = LibraryCtl::start(&env, &player);
                    ctl.nav().source = Source::Album {
                        id: "a".into(),
                        uri: "spotify:album:a".into(),
                    };
                    player.view.send_modify(|v| v.shuffle = shuffle);
                    let mut targets = ctl.playback_target.subscribe();
                    if current {
                        ctl.play_current();
                    } else {
                        ctl.play_context("spotify:album:a");
                    }
                    targets.changed().await.unwrap();
                    assert_eq!(
                        *targets.borrow(),
                        Some(LoadTarget::Context {
                            uri: "spotify:album:a".into(),
                            index: None,
                            shuffle
                        })
                    );
                    let devices = server.next().await;
                    assert_eq!(devices.path, "/me/player/devices");
                    devices.respond(
                        200,
                        json!({"devices": [{"id": "phone", "name": "Phone",
                        "type": "Smartphone", "is_active": true}]}),
                    );
                    let request = server.next().await;
                    assert_eq!(request.path, format!("/me/player/shuffle?state={shuffle}"));
                    request.respond(204, Value::Null);
                    let request = server.next().await;
                    assert_eq!(request.path, "/me/player/play");
                    assert_eq!(request.body, json!({"context_uri": "spotify:album:a"}));
                    request.respond(204, Value::Null);
                    settle().await;
                });
            }
        }
    }

    #[test]
    fn velo_engine_keeps_the_displayed_rows_and_starting_index() {
        let playlist = Source::Playlist {
            id: "p".into(),
            uri: "spotify:playlist:p".into(),
            foreign: false,
        };
        let empty = TrackStore::default();
        let five: Vec<String> = (0..5).map(|i| format!("u{i}")).collect();
        let plan = play_plan(&playlist, &store_of(&five), Some(4)).unwrap();
        assert_eq!(
            velo_target(&plan, false),
            Some(LoadTarget::Tracks {
                uris: five,
                index: 4,
                shuffle: false
            })
        );
        let artist = Source::Artist {
            uri: "spotify:artist:a".into(),
        };
        assert!(matches!(
            velo_target(&play_plan(&artist, &empty, None).unwrap(), true),
            Some(LoadTarget::Context {
                index: None,
                shuffle: true,
                ..
            })
        ));
        let uris: Vec<String> = (0..250).map(|i| format!("spotify:track:{i}")).collect();
        let plan = play_plan(&Source::Liked, &store_of(&uris), Some(10)).unwrap();
        match velo_target(&plan, false) {
            Some(LoadTarget::Tracks {
                uris: list, index, ..
            }) => {
                assert_eq!(list, uris);
                assert_eq!(index, 10);
            }
            other => panic!("{other:?}"),
        }
        let plan = play_plan(&Source::Search("x".into()), &empty, Some(0)).unwrap();
        assert_eq!(velo_target(&plan, false), None);
        assert_eq!(
            play_plan(&Source::SavedAlbums, &store_of(&uris), None),
            None
        );
        assert_eq!(play_plan(&Source::None, &store_of(&uris), None), None);
    }

    #[test]
    fn filtered_rows_play_their_song_and_sorted_rows_play_in_the_shown_order() {
        let playlist = Source::Playlist {
            id: "p".into(),
            uri: "spotify:playlist:p".into(),
            foreign: true,
        };
        let uris: Vec<String> = ["xc", "xa", "xb"]
            .iter()
            .map(|u| (*u).to_string())
            .collect();
        let mut store = store_of(&uris);
        // Explicit URIs avoid wrong context indexes after omitted episodes/null metadata.
        store.spec.query = "xb".into();
        store.apply();
        assert_eq!(
            play_plan(&playlist, &store, Some(0)),
            Some(PlayPlan::List {
                uris: vec!["xb".into()],
                index: 0
            })
        );
        // Sorted by title: an explicit list in the order shown, starting at the row.
        store.spec.query.clear();
        store.spec.choose_sort(SortKey::Title);
        store.apply();
        assert_eq!(
            play_plan(&playlist, &store, Some(1)),
            Some(PlayPlan::List {
                uris: vec!["xa".into(), "xb".into(), "xc".into()],
                index: 1
            })
        );
        assert_eq!(
            velo_target(&play_plan(&playlist, &store, Some(1)).unwrap(), false),
            Some(LoadTarget::Tracks {
                uris: vec!["xa".into(), "xb".into(), "xc".into()],
                index: 1,
                shuffle: false
            })
        );
        // "Tus me gusta" filtered: the rows shown.
        store.spec = ViewSpec {
            query: "xa".into(),
            ..ViewSpec::default()
        };
        store.apply();
        assert_eq!(
            play_plan(&Source::Liked, &store, None),
            Some(PlayPlan::List {
                uris: vec!["xa".into()],
                index: 0
            })
        );
    }

    #[test]
    fn remembered_views_are_per_playlist_and_liked_only() {
        let p = |id: &str| Source::Playlist {
            id: id.into(),
            uri: String::new(),
            foreign: false,
        };
        assert_eq!(list_key(&p("x")), "playlist:x");
        assert_ne!(list_key(&p("x")), list_key(&p("y")));
        assert_eq!(list_key(&Source::Liked), "liked");
        assert_eq!(list_key(&Source::Search("q".into())), "");
        let album = Source::Album {
            id: "a".into(),
            uri: String::new(),
        };
        assert_eq!(list_key(&album), "");
    }

    #[test]
    fn pages_advance_by_raw_slots() {
        let page = velo_domain::Page {
            items: vec![track("a")],
            offset: 0,
            limit: 50,
            total: 120,
        };
        assert_eq!(page_advance(&page), 50);
        let page = velo_domain::Page {
            items: vec![track("a"), track("b")],
            offset: 0,
            limit: 0,
            total: 2,
        };
        assert_eq!(page_advance(&page), 2);
    }

    #[test]
    fn recent_tracks_are_deduplicated_newest_first() {
        let out = dedupe_tracks(vec![track("a"), track("b"), track("a"), track("c")], 2);
        let uris: Vec<_> = out.iter().map(|t| t.uri.as_str()).collect();
        assert_eq!(uris, ["a", "b"]);
    }

    #[test]
    fn edited_details_replace_only_that_playlist() {
        let mut all = vec![playlist("a"), playlist("b")];
        assert!(apply_details(&mut all, "b", "Nuevo", "Para la tarde"));
        assert_eq!(all[1].name, "Nuevo");
        assert_eq!(all[1].description, "Para la tarde");
        assert_eq!(all[0], playlist("a"));
        assert!(!apply_details(&mut all, "zz", "x", ""));
    }

    #[test]
    fn non_owned_playlists_are_not_listable() {
        let mut p = playlist("a");
        assert!(p.items_listable_by("juan"));
        assert!(!p.items_listable_by("otro"));
        p.collaborative = true;
        assert!(p.items_listable_by("otro"));
    }

    #[test]
    fn pagination_blocks_foreign_lists_duplicates_and_exhausted_pages() {
        assert!(can_fetch_page(true, false, 0, false));
        assert!(can_fetch_page(true, false, 50, true));
        assert!(!can_fetch_page(false, false, 0, true));
        assert!(!can_fetch_page(true, true, 0, true));
        assert!(!can_fetch_page(true, false, 50, false));
    }
}
