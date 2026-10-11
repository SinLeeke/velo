//! `Library` singleton (docs/CONTRATO.md, including the wave-2 additions). A thin view of
//! `engine::library`: lists are `QAbstractListModel`s (`VeloListModel`) filled with ordered row
//! operations; every invokable returns immediately and works on the tokio runtime.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_QString = cxx_qt_lib::QList<QString>;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
        include!("QtCore/QAbstractListModel");
        type QAbstractListModel = crate::model::ffi::QAbstractListModel;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(*mut QAbstractListModel, playlists, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, tracks, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, recent, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, top_artists, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, followed_artists, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, saved_albums, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, albums, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, for_you, READ, CONSTANT)]
        #[qproperty(bool, for_you_available, READ, NOTIFY)]
        #[qproperty(QString, current_title, READ, NOTIFY)]
        #[qproperty(QString, current_subtitle, READ, NOTIFY)]
        #[qproperty(QUrl, current_image_url, READ, NOTIFY)]
        #[qproperty(QString, current_kind, READ, NOTIFY)]
        #[qproperty(bool, current_listable, READ, NOTIFY)]
        #[qproperty(bool, loading, READ, NOTIFY)]
        #[qproperty(bool, has_more, READ, NOTIFY)]
        #[qproperty(QList_QString, track_tags, READ, NOTIFY)]
        #[qproperty(QString, active_tag, READ, NOTIFY)]
        #[qproperty(i32, current_track_count, READ, NOTIFY)]
        #[qproperty(f64, current_duration_ms, READ, NOTIFY)]
        #[qproperty(bool, current_duration_exact, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, artist_popular, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, artist_related, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, artist_discography, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, artist_appears_on, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, artist_photos, READ, CONSTANT)]
        #[qproperty(QString, artist_status, READ, NOTIFY)]
        #[qproperty(QString, artist_biography, READ, NOTIFY)]
        #[qproperty(i32, artist_monthly_listeners, READ, NOTIFY)]
        #[qproperty(i32, artist_followers, READ, NOTIFY)]
        #[qproperty(bool, artist_verified, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, artist_top_cities, READ, CONSTANT)]
        #[qproperty(QUrl, artist_header_image_url, READ, NOTIFY)]
        #[qproperty(QString, artist_discography_type, READ, NOTIFY)]
        #[qproperty(bool, artist_incomplete, READ, NOTIFY)]
        #[qproperty(QString, filter_query, READ, NOTIFY)]
        #[qproperty(QString, sort_key, READ, NOTIFY)]
        #[qproperty(bool, sort_descending, READ, NOTIFY)]
        #[qproperty(bool, compact_view, READ, NOTIFY)]
        #[qproperty(bool, view_busy, READ, NOTIFY)]
        #[qproperty(bool, mix_current, READ, NOTIFY)]
        #[qproperty(bool, mix_playing, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, search_suggestions, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, search_results, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, search_tracks, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, search_artists, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, search_playlists, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, search_albums, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, search_best_result, READ, CONSTANT)]
        #[qproperty(bool, artist_follow_busy, READ, NOTIFY)]
        #[qproperty(bool, artist_followed, READ, NOTIFY)]
        #[qproperty(bool, artist_followed_known, READ, NOTIFY)]
        #[qproperty(QString, followed_artist_id, READ, NOTIFY)]
        #[qproperty(QString, search_error, READ, NOTIFY)]
        #[qproperty(bool, search_has_more, READ, NOTIFY)]
        #[qproperty(bool, search_loading, READ, NOTIFY)]
        #[qproperty(QString, search_query, READ, NOTIFY)]
        #[qproperty(bool, search_suggesting, READ, NOTIFY)]
        #[qproperty(QString, search_suggestion_error, READ, NOTIFY)]
        #[qproperty(QString, search_suggestion_query, READ, NOTIFY)]
        #[qproperty(QString, search_type, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, made_for_you, READ, CONSTANT)]
        #[qproperty(QString, made_for_you_status, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, quick_access, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, top_tracks, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, recommended_tracks, READ, CONSTANT)]
        #[qproperty(QString, current_owner_id, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, profile_top_artists, READ, CONSTANT)]
        #[qproperty(*mut QAbstractListModel, profile_playlists, READ, CONSTANT)]
        #[qproperty(QString, profile_status, READ, NOTIFY)]
        #[qproperty(QString, profile_user_id, READ, NOTIFY)]
        #[qproperty(QString, profile_name, READ, NOTIFY)]
        #[qproperty(QUrl, profile_image_url, READ, NOTIFY)]
        #[qproperty(i32, profile_followers, READ, NOTIFY)]
        #[qproperty(i32, profile_playlist_count, READ, NOTIFY)]
        #[qproperty(bool, profile_is_me, READ, NOTIFY)]
        #[qproperty(bool, profile_playlists_available, READ, NOTIFY)]
        #[qproperty(*mut QAbstractListModel, sidebar_items, READ, CONSTANT)]
        #[qproperty(QString, sidebar_filter, READ, NOTIFY)]
        #[qproperty(QString, sidebar_query, READ, NOTIFY)]
        #[qproperty(QString, sidebar_status, READ, NOTIFY)]
        #[qproperty(QString, sidebar_error, READ, NOTIFY)]
        #[qproperty(QString, shows_status, READ, NOTIFY)]
        #[qproperty(QString, recent_source, READ, NOTIFY)]
        type Library = super::LibraryRust;
        #[qinvokable]
        fn set_sidebar_filter(self: &Library, kind: &QString);
        #[qinvokable]
        fn search_library(self: &Library, text: &QString);
        #[qinvokable]
        fn refresh_sidebar(self: &Library);
        #[qinvokable]
        fn toggle_library_pin(self: &Library, uri: &QString);
        #[qinvokable]
        fn open_show(self: &Library, id: &QString);
        #[qinvokable]
        fn request_sidebar_image(self: &Library, uri: &QString);

        #[qinvokable]
        fn open_link(self: &Library, text: &QString);
        #[qsignal]
        fn navigate_requested(self: Pin<&mut Library>, kind: QString, id: QString);

        #[qinvokable]
        fn open_playlist(self: &Library, id: &QString);
        #[qinvokable]
        fn open_liked(self: &Library);
        #[qinvokable]
        fn search(self: &Library, q: &QString);
        #[qinvokable]
        fn suggest_search(self: &Library, query: &QString);
        #[qinvokable]
        fn cancel_search_suggestions(self: &Library);
        #[qinvokable]
        fn set_search_type(self: &Library, kind: &QString);
        #[qinvokable]
        fn fetch_more_search(self: &Library);
        #[qinvokable]
        fn play_search_result(self: &Library, index: i32);
        #[qinvokable]
        fn play_best_search_result(self: &Library);
        #[qinvokable]
        fn inspect_artist_followed(self: &Library, id: &QString);
        #[qinvokable]
        fn set_artist_followed(self: &Library, id: &QString, followed: bool);
        #[qinvokable]
        fn fetch_more(self: &Library);
        #[qinvokable]
        fn play_index(self: &Library, i: i32);
        #[qinvokable]
        fn play_context(self: &Library, uri: &QString);
        #[qinvokable]
        fn play_current(self: &Library);
        #[qinvokable]
        fn shuffle_current(self: &Library);
        #[qinvokable]
        fn toggle_mix_current(self: &Library);
        #[qinvokable]
        fn toggle_mix_playing(self: &Library);
        #[qinvokable]
        fn toggle_pin(self: &Library, playlist_id: &QString);
        #[qinvokable]
        fn add_to_queue(self: &Library, uri: &QString);
        #[qinvokable]
        fn load_home(self: &Library);
        #[qinvokable]
        fn play_top_track(self: &Library, index: i32);
        #[qinvokable]
        fn play_recommended_track(self: &Library, index: i32);
        #[qinvokable]
        fn open_album(self: &Library, id: &QString);
        #[qinvokable]
        fn open_artist(self: &Library, id: &QString);
        #[qinvokable]
        fn set_artist_discography_type(self: &Library, kind: &QString);
        #[qinvokable]
        fn play_artist_popular(self: &Library, index: i32);
        #[qinvokable]
        fn open_profile(self: &Library, user_id: &QString);
        #[qinvokable]
        fn open_saved_albums(self: &Library);
        #[qinvokable]
        fn open_followed_artists(self: &Library);
        #[qinvokable]
        fn create_playlist(self: &Library, name: &QString);
        #[qinvokable]
        fn update_playlist_details(
            self: &Library,
            playlist_id: &QString,
            name: &QString,
            description: &QString,
        );
        #[qinvokable]
        fn add_to_playlist(self: &Library, playlist_id: &QString, uri: &QString);
        #[qinvokable]
        fn remove_from_playlist(self: &Library, playlist_id: &QString, uri: &QString);
        #[qinvokable]
        fn play_mix(self: &Library, mix_id: &QString);
        #[qinvokable]
        fn save_radio_as_playlist(self: &Library, seed_uri: &QString);
        #[qinvokable]
        fn set_filter(self: &Library, query: &QString);
        #[qinvokable]
        fn filter_tag(self: &Library, tag: &QString);
        #[qinvokable]
        fn set_sort(self: &Library, key: &QString);
        #[qinvokable]
        fn set_compact(self: &Library, compact: bool);
    }

    impl cxx_qt::Threading for Library {}
    impl cxx_qt::Initialize for Library {}
}

use core::pin::Pin;

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString, QUrl};

use crate::engine::core;
use crate::engine::library::{LibEvent, LibMeta, LibModel};
use crate::engine::rows::{ALBUM_ROLES, ARTIST_ROLES, PLAYLIST_ROLES, TRACK_ROLES};
use crate::model::ListModel;

fn count_i32(n: u32) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// Navigation may contend with the row-store worker; never wait for it on Qt.
fn dispatch(f: impl FnOnce(&std::sync::Arc<crate::engine::library::LibraryCtl>) + Send + 'static) {
    let c = core();
    let library = c.library.clone();
    c.env.rt.spawn(async move { f(&library) });
}

pub struct LibraryRust {
    sidebar_items: *mut crate::model::ffi::QAbstractListModel,
    sidebar_model: Option<ListModel>,
    sidebar_view: crate::engine::sidebar::SidebarView,
    sidebar_filter: QString,
    sidebar_query: QString,
    sidebar_status: QString,
    sidebar_error: QString,
    shows_status: QString,
    recent_source: QString,
    track_tags: QList<QString>,
    active_tag: QString,
    current_owner_id: QString,
    profile_top_artists: *mut crate::model::ffi::QAbstractListModel,
    profile_playlists: *mut crate::model::ffi::QAbstractListModel,
    profile_status: QString,
    profile_user_id: QString,
    profile_name: QString,
    profile_image_url: QUrl,
    profile_followers: i32,
    profile_playlist_count: i32,
    profile_is_me: bool,
    profile_playlists_available: bool,
    profile_models: Vec<ListModel>,
    profile_view: crate::engine::profile::ProfileView,
    made_for_you: *mut crate::model::ffi::QAbstractListModel,
    made_for_you_status: QString,
    quick_access: *mut crate::model::ffi::QAbstractListModel,
    top_tracks: *mut crate::model::ffi::QAbstractListModel,
    recommended_tracks: *mut crate::model::ffi::QAbstractListModel,
    home_models: Vec<ListModel>,
    home_view: crate::engine::home::HomeView,
    artist_popular: *mut crate::model::ffi::QAbstractListModel,
    artist_related: *mut crate::model::ffi::QAbstractListModel,
    artist_discography: *mut crate::model::ffi::QAbstractListModel,
    artist_appears_on: *mut crate::model::ffi::QAbstractListModel,
    artist_photos: *mut crate::model::ffi::QAbstractListModel,
    artist_status: QString,
    artist_biography: QString,
    artist_monthly_listeners: i32,
    artist_followers: i32,
    artist_verified: bool,
    artist_top_cities: *mut crate::model::ffi::QAbstractListModel,
    artist_header_image_url: QUrl,
    artist_discography_type: QString,
    artist_incomplete: bool,
    artist_models: Vec<ListModel>,
    artist_view: crate::engine::artists::ArtistView,
    search_suggestions: *mut crate::model::ffi::QAbstractListModel,
    search_results: *mut crate::model::ffi::QAbstractListModel,
    search_tracks: *mut crate::model::ffi::QAbstractListModel,
    search_artists: *mut crate::model::ffi::QAbstractListModel,
    search_playlists: *mut crate::model::ffi::QAbstractListModel,
    search_albums: *mut crate::model::ffi::QAbstractListModel,
    search_best_result: *mut crate::model::ffi::QAbstractListModel,
    artist_follow_busy: bool,
    artist_followed: bool,
    artist_followed_known: bool,
    followed_artist_id: QString,
    search_error: QString,
    search_has_more: bool,
    search_loading: bool,
    search_query: QString,
    search_suggesting: bool,
    search_suggestion_error: QString,
    search_suggestion_query: QString,
    search_type: QString,
    search_models: Vec<ListModel>,
    search_view: crate::engine::search::SearchView,

    for_you: *mut crate::model::ffi::QAbstractListModel,
    for_you_available: bool,
    for_you_model: Option<ListModel>,
    discover_view: crate::engine::discover::DiscoverView,
    playlists: *mut crate::model::ffi::QAbstractListModel,
    tracks: *mut crate::model::ffi::QAbstractListModel,
    recent: *mut crate::model::ffi::QAbstractListModel,
    top_artists: *mut crate::model::ffi::QAbstractListModel,
    followed_artists: *mut crate::model::ffi::QAbstractListModel,
    saved_albums: *mut crate::model::ffi::QAbstractListModel,
    albums: *mut crate::model::ffi::QAbstractListModel,
    current_title: QString,
    current_subtitle: QString,
    current_image_url: QUrl,
    current_kind: QString,
    current_listable: bool,
    loading: bool,
    has_more: bool,
    current_track_count: i32,
    current_duration_ms: f64,
    current_duration_exact: bool,
    filter_query: QString,
    sort_key: QString,
    sort_descending: bool,
    compact_view: bool,
    mix_current: bool,
    mix_playing: bool,
    view_busy: bool,
    /// Indexed by `LibModel as usize`.
    models: Vec<ListModel>,
}

impl Default for LibraryRust {
    fn default() -> Self {
        let m = core().library.meta.borrow().clone();
        Self {
            sidebar_items: std::ptr::null_mut(),
            sidebar_model: None,
            sidebar_view: crate::engine::sidebar::SidebarView::default(),
            sidebar_filter: QString::from("all"),
            sidebar_query: QString::default(),
            sidebar_status: QString::from("none"),
            sidebar_error: QString::default(),
            shows_status: QString::from("none"),
            recent_source: QString::from(m.recent_source.as_str()),
            track_tags: m
                .track_tags
                .iter()
                .map(|s| QString::from(s.as_str()))
                .collect(),
            active_tag: QString::from(m.active_tag.as_str()),
            current_owner_id: QString::from(m.owner_id.as_str()),
            profile_top_artists: std::ptr::null_mut(),
            profile_playlists: std::ptr::null_mut(),
            profile_status: QString::from("none"),
            profile_user_id: QString::default(),
            profile_name: QString::default(),
            profile_image_url: QUrl::default(),
            profile_followers: -1,
            profile_playlist_count: -1,
            profile_is_me: false,
            profile_playlists_available: false,
            profile_models: Vec::new(),
            profile_view: crate::engine::profile::ProfileView::default(),
            made_for_you: std::ptr::null_mut(),
            made_for_you_status: QString::from("none"),
            quick_access: std::ptr::null_mut(),
            top_tracks: std::ptr::null_mut(),
            recommended_tracks: std::ptr::null_mut(),
            home_models: Vec::new(),
            home_view: crate::engine::home::HomeView::default(),
            artist_popular: std::ptr::null_mut(),
            artist_related: std::ptr::null_mut(),
            artist_discography: std::ptr::null_mut(),
            artist_appears_on: std::ptr::null_mut(),
            artist_photos: std::ptr::null_mut(),
            artist_status: QString::from("none"),
            artist_biography: QString::default(),
            artist_monthly_listeners: -1,
            artist_followers: -1,
            artist_verified: false,
            artist_top_cities: std::ptr::null_mut(),
            artist_header_image_url: QUrl::default(),
            artist_discography_type: QString::from("all"),
            artist_incomplete: false,
            artist_models: Vec::new(),
            artist_view: crate::engine::artists::ArtistView::default(),
            search_suggestions: std::ptr::null_mut(),
            search_results: std::ptr::null_mut(),
            search_tracks: std::ptr::null_mut(),
            search_artists: std::ptr::null_mut(),
            search_playlists: std::ptr::null_mut(),
            search_albums: std::ptr::null_mut(),
            search_best_result: std::ptr::null_mut(),
            artist_follow_busy: false,
            artist_followed: false,
            artist_followed_known: false,
            followed_artist_id: QString::default(),
            search_error: QString::default(),
            search_has_more: false,
            search_loading: false,
            search_query: QString::default(),
            search_suggesting: false,
            search_suggestion_error: QString::default(),
            search_suggestion_query: QString::default(),
            search_type: QString::from("all"),
            search_models: Vec::new(),
            search_view: crate::engine::search::SearchView::default(),
            for_you: std::ptr::null_mut(),
            for_you_available: false,
            for_you_model: None,
            discover_view: crate::engine::discover::DiscoverView::default(),
            playlists: std::ptr::null_mut(),
            tracks: std::ptr::null_mut(),
            recent: std::ptr::null_mut(),
            top_artists: std::ptr::null_mut(),
            followed_artists: std::ptr::null_mut(),
            saved_albums: std::ptr::null_mut(),
            albums: std::ptr::null_mut(),
            current_title: QString::from(m.title.as_str()),
            current_subtitle: QString::from(m.subtitle.as_str()),
            current_image_url: QUrl::from(m.image_url.as_str()),
            current_kind: QString::from(m.kind.as_str()),
            current_listable: m.listable,
            loading: m.loading,
            has_more: m.has_more,
            current_track_count: count_i32(m.track_count),
            current_duration_ms: m.duration_ms as f64,
            current_duration_exact: m.duration_exact,
            filter_query: QString::from(m.query.as_str()),
            sort_key: QString::from(m.sort_key.as_str()),
            sort_descending: m.sort_descending,
            compact_view: m.compact,
            mix_current: m.mix_current,
            mix_playing: m.mix_playing,
            view_busy: m.view_busy,
            models: Vec::new(),
        }
    }
}

/// Roles of each model, in `LibModel` order.
const ROLES: [&str; 7] = [
    PLAYLIST_ROLES, // Playlists
    TRACK_ROLES,    // Tracks
    TRACK_ROLES,    // Recent
    ARTIST_ROLES,   // TopArtists
    ARTIST_ROLES,   // FollowedArtists
    ALBUM_ROLES,    // SavedAlbums
    ALBUM_ROLES,    // Albums
];

impl cxx_qt::Initialize for ffi::Library {
    fn initialize(mut self: Pin<&mut Self>) {
        if let Some(mut events) = crate::engine::links::navigation().take() {
            let thread = self.qt_thread();
            core().env.rt.spawn(async move {
                while let Some(link) = events.recv().await {
                    if thread
                        .queue(move |q| {
                            q.navigate_requested(QString::from(link.kind), QString::from(link.id))
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        // Upcast the QObject itself, not the `Pin` (the blanket `Upcast<T> for T` would
        // otherwise hand Qt the address of this stack variable as the parent).
        let parent = std::ptr::from_ref::<cxx_qt::QObject>((*self).upcast())
            .cast_mut()
            .cast();
        let sidebar_model = ListModel::new(crate::engine::sidebar::ROLES, parent);
        let models: Vec<ListModel> = ROLES.iter().map(|r| ListModel::new(r, parent)).collect();
        let artist_models: Vec<_> = crate::engine::artists::ARTIST_MODEL_ROLES
            .iter()
            .map(|r| ListModel::new(r, parent))
            .collect();
        let search_models: Vec<_> = (0..crate::engine::search::MODELS)
            .map(|_| ListModel::new(crate::engine::search::SEARCH_ROLES, parent))
            .collect();
        let home_models: Vec<_> = crate::engine::home::HOME_ROLES
            .iter()
            .map(|r| ListModel::new(r, parent))
            .collect();
        let profile_models: Vec<_> = crate::engine::profile::PROFILE_ROLES
            .iter()
            .map(|r| ListModel::new(r, parent))
            .collect();
        let for_you = ListModel::new(crate::engine::discover::MIX_ROLES, parent);
        let variants: Vec<_> = models.iter().map(ListModel::as_model).collect();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.sidebar_items = sidebar_model.as_model();
            rust.sidebar_model = Some(sidebar_model);
            rust.made_for_you = home_models[0].as_model();
            rust.quick_access = home_models[1].as_model();
            rust.top_tracks = home_models[2].as_model();
            rust.recommended_tracks = home_models[3].as_model();
            rust.home_models = home_models;
            rust.profile_top_artists = profile_models[0].as_model();
            rust.profile_playlists = profile_models[1].as_model();
            rust.profile_models = profile_models;
            rust.artist_popular = artist_models[0].as_model();
            rust.artist_related = artist_models[1].as_model();
            rust.artist_discography = artist_models[2].as_model();
            rust.artist_appears_on = artist_models[3].as_model();
            rust.artist_photos = artist_models[4].as_model();
            rust.artist_top_cities = artist_models[5].as_model();
            rust.artist_models = artist_models;
            rust.search_suggestions = search_models[0].as_model();
            rust.search_results = search_models[1].as_model();
            rust.search_tracks = search_models[2].as_model();
            rust.search_artists = search_models[3].as_model();
            rust.search_playlists = search_models[4].as_model();
            rust.search_albums = search_models[5].as_model();
            rust.search_best_result = search_models[6].as_model();
            rust.search_models = search_models;
            rust.for_you = for_you.as_model();
            rust.for_you_model = Some(for_you);
            rust.playlists = variants[LibModel::Playlists as usize];
            rust.tracks = variants[LibModel::Tracks as usize];
            rust.recent = variants[LibModel::Recent as usize];
            rust.top_artists = variants[LibModel::TopArtists as usize];
            rust.followed_artists = variants[LibModel::FollowedArtists as usize];
            rust.saved_albums = variants[LibModel::SavedAlbums as usize];
            rust.albums = variants[LibModel::Albums as usize];
            rust.models = models;
        }

        let c = core();
        let thread = self.qt_thread();
        let mut sidebar = c.library.sidebar.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let view = sidebar.borrow_and_update().clone();
                if thread.queue(move |q| q.apply_sidebar(view)).is_err() {
                    break;
                }
                if sidebar.changed().await.is_err() {
                    break;
                }
            }
        });
        let thread = self.qt_thread();
        let mut home = c.library.home.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let view = home.borrow_and_update().clone();
                if thread.queue(move |q| q.apply_home(view)).is_err() {
                    break;
                }
                if home.changed().await.is_err() {
                    break;
                }
            }
        });
        let thread = self.qt_thread();
        let mut profile = c.library.profiles.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let view = profile.borrow_and_update().clone();
                if thread.queue(move |q| q.apply_profile(view)).is_err() {
                    break;
                }
                if profile.changed().await.is_err() {
                    break;
                }
            }
        });
        let thread = self.qt_thread();
        let mut artist = c.library.artists.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let view = artist.borrow_and_update().clone();
                if thread.queue(move |q| q.apply_artist(view)).is_err() {
                    break;
                }
                if artist.changed().await.is_err() {
                    break;
                }
            }
        });
        let thread = self.qt_thread();
        let mut search = c.library.searches.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let view = search.borrow_and_update().clone();
                if thread.queue(move |q| q.apply_search(view)).is_err() {
                    break;
                }
                if search.changed().await.is_err() {
                    break;
                }
            }
        });
        let thread = self.qt_thread();
        let mut discover = c.library.discover.view.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let view = discover.borrow_and_update().clone();
                if thread.queue(move |q| q.apply_discover(view)).is_err() {
                    break;
                }
                if discover.changed().await.is_err() {
                    break;
                }
            }
        });
        let thread = self.qt_thread();
        let mut meta = c.library.meta.subscribe();
        c.env.rt.spawn(async move {
            loop {
                let m = meta.borrow_and_update().clone();
                if thread.queue(move |q| q.apply_meta(&m)).is_err() {
                    break;
                }
                if meta.changed().await.is_err() {
                    break;
                }
            }
        });

        if let Some(mut events) = c.library.ui.take() {
            let thread = self.qt_thread();
            c.env.rt.spawn(async move {
                while let Some(event) = events.recv().await {
                    if thread.queue(move |q| q.apply_event(event)).is_err() {
                        break;
                    }
                }
            });
        }
    }
}

impl ffi::Library {
    fn apply_sidebar(mut self: Pin<&mut Self>, view: crate::engine::sidebar::SidebarView) {
        if let Some(model) = &self.sidebar_model {
            for op in crate::engine::artists::row_changes(&view.rows, &self.sidebar_view.rows) {
                model.apply(op);
            }
        }
        set_ro!(
            self,
            sidebar_filter,
            sidebar_filter_changed,
            QString::from(view.filter.as_str())
        );
        set_ro!(
            self,
            sidebar_query,
            sidebar_query_changed,
            QString::from(view.query.as_str())
        );
        set_ro!(
            self,
            sidebar_error,
            sidebar_error_changed,
            QString::from(view.error.as_str())
        );
        set_ro!(
            self,
            shows_status,
            shows_status_changed,
            QString::from(view.shows_status.as_str())
        );
        set_ro!(
            self,
            sidebar_status,
            sidebar_status_changed,
            QString::from(view.status.as_str())
        );
        self.as_mut().rust_mut().sidebar_view = view;
    }
    pub fn set_sidebar_filter(&self, kind: &QString) {
        let kind = kind.to_string();
        dispatch(move |l| l.sidebar.filter(kind));
    }
    pub fn search_library(&self, text: &QString) {
        let text = text.to_string();
        dispatch(move |l| l.sidebar.search(text));
    }
    pub fn refresh_sidebar(&self) {
        dispatch(|l| l.refresh_sidebar());
    }
    pub fn toggle_library_pin(&self, uri: &QString) {
        let uri = uri.to_string();
        dispatch(move |l| l.toggle_library_pin(uri));
    }
    pub fn request_sidebar_image(&self, uri: &QString) {
        let uri = uri.to_string();
        dispatch(move |l| l.sidebar.request_image(uri));
    }
    pub fn open_show(&self, id: &QString) {
        let id = id.to_string();
        dispatch(move |l| l.open_show(id));
    }

    fn apply_home(mut self: Pin<&mut Self>, view: crate::engine::home::HomeView) {
        for (index, model) in self.home_models.iter().enumerate() {
            for op in crate::engine::artists::row_changes(
                &view.models[index],
                &self.home_view.models[index],
            ) {
                model.apply(op);
            }
        }
        set_ro!(
            self,
            made_for_you_status,
            made_for_you_status_changed,
            QString::from(view.status.as_str())
        );
        self.as_mut().rust_mut().home_view = view;
    }
    fn apply_profile(mut self: Pin<&mut Self>, view: crate::engine::profile::ProfileView) {
        for (index, model) in self.profile_models.iter().enumerate() {
            for op in crate::engine::artists::row_changes(
                &view.models[index],
                &self.profile_view.models[index],
            ) {
                model.apply(op);
            }
        }
        let clamp = |n: i64| i32::try_from(n).unwrap_or(if n < 0 { -1 } else { i32::MAX });
        set_ro!(
            self,
            profile_user_id,
            profile_user_id_changed,
            QString::from(view.user_id.as_str())
        );
        set_ro!(
            self,
            profile_name,
            profile_name_changed,
            QString::from(view.name.as_str())
        );
        set_ro!(
            self,
            profile_image_url,
            profile_image_url_changed,
            QUrl::from(view.image_url.as_str())
        );
        set_ro!(
            self,
            profile_followers,
            profile_followers_changed,
            clamp(view.followers)
        );
        set_ro!(
            self,
            profile_playlist_count,
            profile_playlist_count_changed,
            clamp(view.playlist_count)
        );
        set_ro!(self, profile_is_me, profile_is_me_changed, view.is_me);
        set_ro!(
            self,
            profile_playlists_available,
            profile_playlists_available_changed,
            view.playlists_available
        );
        // Last: QML reads the rest of the profile when the status changes.
        set_ro!(
            self,
            profile_status,
            profile_status_changed,
            QString::from(view.status.as_str())
        );
        self.as_mut().rust_mut().profile_view = view;
    }
    pub fn open_profile(&self, user_id: &QString) {
        let id = user_id.to_string();
        dispatch(move |library| library.open_profile(id));
    }
    pub fn play_recommended_track(&self, index: i32) {
        core().library.home.play_recommended(index);
    }
    pub fn play_top_track(&self, index: i32) {
        core().library.home.play_top(index);
    }
    pub fn open_link(&self, text: &QString) {
        crate::engine::links::open(text.to_string());
    }
    fn apply_artist(mut self: Pin<&mut Self>, view: crate::engine::artists::ArtistView) {
        for (index, model) in self.artist_models.iter().enumerate() {
            for op in crate::engine::artists::row_changes(
                &view.models[index],
                &self.artist_view.models[index],
            ) {
                model.apply(op);
            }
        }
        set_ro!(
            self,
            artist_monthly_listeners,
            artist_monthly_listeners_changed,
            view.monthly_listeners
        );
        set_ro!(
            self,
            artist_followers,
            artist_followers_changed,
            view.followers
        );
        set_ro!(
            self,
            artist_verified,
            artist_verified_changed,
            view.verified
        );
        set_ro!(
            self,
            artist_header_image_url,
            artist_header_image_url_changed,
            QUrl::from(view.header_image_url.as_str())
        );
        set_ro!(
            self,
            artist_biography,
            artist_biography_changed,
            QString::from(view.biography.as_str())
        );
        set_ro!(
            self,
            artist_discography_type,
            artist_discography_type_changed,
            QString::from(view.discography_type.as_str())
        );
        set_ro!(
            self,
            artist_incomplete,
            artist_incomplete_changed,
            view.incomplete
        );
        // Publish readiness after every field/model of this navigation generation.
        set_ro!(
            self,
            artist_status,
            artist_status_changed,
            QString::from(view.status.as_str())
        );
        self.as_mut().rust_mut().artist_view = view;
    }
    pub fn set_artist_discography_type(&self, kind: &QString) {
        core()
            .library
            .artists
            .set_discography_type(kind.to_string());
    }
    pub fn play_artist_popular(&self, index: i32) {
        core().library.artists.play_popular(index);
    }
    fn apply_search(mut self: Pin<&mut Self>, view: crate::engine::search::SearchView) {
        for (index, model) in self.search_models.iter().enumerate() {
            for op in crate::engine::search::row_changes(
                &view.models[index],
                &self.search_view.models[index],
            ) {
                model.apply(op);
            }
        }
        set_ro!(
            self,
            artist_follow_busy,
            artist_follow_busy_changed,
            view.artist_follow_busy
        );
        set_ro!(
            self,
            artist_followed,
            artist_followed_changed,
            view.artist_followed
        );
        set_ro!(
            self,
            artist_followed_known,
            artist_followed_known_changed,
            view.artist_followed_known
        );
        set_ro!(
            self,
            followed_artist_id,
            followed_artist_id_changed,
            QString::from(view.followed_artist_id.as_str())
        );
        set_ro!(
            self,
            search_error,
            search_error_changed,
            QString::from(view.error.as_str())
        );
        set_ro!(
            self,
            search_has_more,
            search_has_more_changed,
            view.has_more
        );
        set_ro!(self, search_loading, search_loading_changed, view.loading);
        set_ro!(
            self,
            search_query,
            search_query_changed,
            QString::from(view.query.as_str())
        );
        set_ro!(
            self,
            search_suggesting,
            search_suggesting_changed,
            view.suggesting
        );
        set_ro!(
            self,
            search_suggestion_error,
            search_suggestion_error_changed,
            QString::from(view.suggestion_error.as_str())
        );
        set_ro!(
            self,
            search_suggestion_query,
            search_suggestion_query_changed,
            QString::from(view.suggestion_query.as_str())
        );
        set_ro!(
            self,
            search_type,
            search_type_changed,
            QString::from(view.kind.as_str())
        );
        self.as_mut().rust_mut().search_view = view;
    }
    pub fn suggest_search(&self, query: &QString) {
        core().library.searches.suggest(query.to_string());
    }
    pub fn cancel_search_suggestions(&self) {
        core().library.searches.cancel_suggestions();
    }
    pub fn set_search_type(&self, kind: &QString) {
        core().library.searches.set_type(kind.to_string());
    }
    pub fn fetch_more_search(&self) {
        core().library.searches.more();
    }
    pub fn play_search_result(&self, index: i32) {
        core().library.searches.play(index, false);
    }
    pub fn play_best_search_result(&self) {
        core().library.searches.play(0, true);
    }
    pub fn inspect_artist_followed(&self, id: &QString) {
        core().library.searches.inspect_followed(id.to_string());
    }
    pub fn set_artist_followed(&self, id: &QString, followed: bool) {
        core()
            .library
            .searches
            .set_followed(id.to_string(), followed);
    }
    fn apply_discover(mut self: Pin<&mut Self>, view: crate::engine::discover::DiscoverView) {
        if let Some(model) = &self.for_you_model {
            for op in view.row_changes(&self.discover_view) {
                model.apply(op);
            }
        }
        set_ro!(
            self,
            for_you_available,
            for_you_available_changed,
            view.available && core().env.audio.is_ready()
        );
        self.as_mut().rust_mut().discover_view = view;
    }

    pub fn play_mix(&self, mix_id: &QString) {
        core().library.discover.play_mix(mix_id.to_string());
    }

    pub fn save_radio_as_playlist(&self, seed_uri: &QString) {
        core().library.save_radio_as_playlist(seed_uri.to_string());
    }

    fn apply_meta(mut self: Pin<&mut Self>, m: &LibMeta) {
        set_ro!(
            self,
            recent_source,
            recent_source_changed,
            QString::from(m.recent_source.as_str())
        );
        set_ro!(
            self,
            track_tags,
            track_tags_changed,
            m.track_tags
                .iter()
                .map(|s| QString::from(s.as_str()))
                .collect::<QList<QString>>()
        );
        set_ro!(
            self,
            active_tag,
            active_tag_changed,
            QString::from(m.active_tag.as_str())
        );
        set_ro!(
            self,
            current_title,
            current_title_changed,
            QString::from(m.title.as_str())
        );
        set_ro!(
            self,
            current_subtitle,
            current_subtitle_changed,
            QString::from(m.subtitle.as_str())
        );
        set_ro!(
            self,
            current_image_url,
            current_image_url_changed,
            QUrl::from(m.image_url.as_str())
        );
        set_ro!(
            self,
            current_kind,
            current_kind_changed,
            QString::from(m.kind.as_str())
        );
        set_ro!(self, current_listable, current_listable_changed, m.listable);
        set_ro!(self, loading, loading_changed, m.loading);
        set_ro!(self, has_more, has_more_changed, m.has_more);
        set_ro!(
            self,
            current_track_count,
            current_track_count_changed,
            count_i32(m.track_count)
        );
        set_ro!(
            self,
            current_duration_ms,
            current_duration_ms_changed,
            m.duration_ms as f64
        );
        set_ro!(
            self,
            current_duration_exact,
            current_duration_exact_changed,
            m.duration_exact
        );
        set_ro!(
            self,
            filter_query,
            filter_query_changed,
            QString::from(m.query.as_str())
        );
        set_ro!(
            self,
            sort_key,
            sort_key_changed,
            QString::from(m.sort_key.as_str())
        );
        set_ro!(
            self,
            sort_descending,
            sort_descending_changed,
            m.sort_descending
        );
        set_ro!(self, compact_view, compact_view_changed, m.compact);
        set_ro!(self, view_busy, view_busy_changed, m.view_busy);
        set_ro!(self, mix_current, mix_current_changed, m.mix_current);
        set_ro!(self, mix_playing, mix_playing_changed, m.mix_playing);
        set_ro!(
            self,
            current_owner_id,
            current_owner_id_changed,
            QString::from(m.owner_id.as_str())
        );
    }

    pub fn set_filter(&self, query: &QString) {
        core().library.set_filter(query.to_string());
    }

    pub fn filter_tag(&self, tag: &QString) {
        let tag = tag.to_string();
        dispatch(move |library| library.filter_tag(tag));
    }

    pub fn set_sort(&self, key: &QString) {
        core().library.set_sort(key.to_string());
    }

    pub fn set_compact(&self, compact: bool) {
        core().library.set_compact(compact);
    }

    fn apply_event(self: Pin<&mut Self>, event: LibEvent) {
        let (model, op) = event;
        if let Some(m) = self.models.get(model as usize) {
            m.apply(op);
        }
    }

    pub fn open_playlist(&self, id: &QString) {
        core().library.open_playlist(id.to_string());
    }

    pub fn open_liked(&self) {
        dispatch(|library| library.open_liked());
    }

    pub fn search(&self, q: &QString) {
        let query = q.to_string();
        dispatch(move |library| library.search(query));
    }

    pub fn fetch_more(&self) {
        dispatch(|library| library.fetch_more());
    }

    pub fn play_index(&self, i: i32) {
        if let Ok(i) = usize::try_from(i) {
            core().library.play_index(i);
        }
    }

    pub fn play_context(&self, uri: &QString) {
        core().library.play_context(&uri.to_string());
    }

    pub fn play_current(&self) {
        core().library.play_current();
    }

    pub fn toggle_mix_current(&self) {
        core().library.toggle_mix_current();
    }

    pub fn toggle_mix_playing(&self) {
        core().library.toggle_mix_playing();
    }

    pub fn shuffle_current(&self) {
        core().library.shuffle_current();
    }

    pub fn toggle_pin(&self, playlist_id: &QString) {
        core().library.toggle_pin(playlist_id.to_string());
    }

    pub fn add_to_queue(&self, uri: &QString) {
        core().library.add_to_queue(uri.to_string());
    }

    pub fn load_home(&self) {
        dispatch(|library| library.load_home());
    }

    pub fn open_album(&self, id: &QString) {
        core().library.open_album(id.to_string());
    }

    pub fn open_artist(&self, id: &QString) {
        core().library.open_artist(id.to_string());
    }

    pub fn open_saved_albums(&self) {
        dispatch(|library| library.open_saved_albums());
    }

    pub fn open_followed_artists(&self) {
        dispatch(|library| library.open_followed_artists());
    }

    pub fn create_playlist(&self, name: &QString) {
        core().library.create_playlist(name.to_string());
    }

    pub fn update_playlist_details(
        &self,
        playlist_id: &QString,
        name: &QString,
        description: &QString,
    ) {
        core().library.update_playlist_details(
            playlist_id.to_string(),
            name.to_string(),
            description.to_string(),
        );
    }

    pub fn add_to_playlist(&self, playlist_id: &QString, uri: &QString) {
        core()
            .library
            .add_to_playlist(playlist_id.to_string(), uri.to_string());
    }

    pub fn remove_from_playlist(&self, playlist_id: &QString, uri: &QString) {
        core()
            .library
            .remove_from_playlist(playlist_id.to_string(), uri.to_string());
    }
}
