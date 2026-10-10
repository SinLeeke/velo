//! U4: página de perfil de usuario (propio o ajeno) como la de Spotify. Web API en Tokio,
//! publicación por generación (como F6): otra navegación cancela y descarta lo pendiente.
//!
//! - Cabecera: `GET /me` o `GET /users/{id}` (nombre, imagen, seguidores si Spotify los da).
//! - «Playlists públicas»: `GET /users/{id}/playlists`, solo las públicas de ese usuario.
//! - Propio: «Artistas más escuchados este mes» (`/me/top/artists?time_range=short_term`).
//!   «Canciones más escuchadas este mes» reutiliza `Library.topTracks` de Inicio (misma
//!   petición short_term); `LibraryCtl::open_profile` la pide si Inicio aún no la tiene.
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::watch;
use velo_domain::{
    Artist, CoverArtService, MusicProvider, Playlist, PublicProfile, Result, TimeRange, VeloError,
    pick_image,
};

use super::{
    Env,
    images::Generation,
    rows::{ARTIST_ROLES, PLAYLIST_ROLES, THUMB_PX, artist_row, file_url, image_of, playlist_row},
};

pub const PROFILE_MODELS: usize = 2;
/// `profileTopArtists`, `profilePlaylists`.
pub const PROFILE_ROLES: [&str; PROFILE_MODELS] = [ARTIST_ROLES, PLAYLIST_ROLES];
/// Pages of 50 read from `/users/{id}/playlists` (up to 200 playlists, before filtering).
const PLAYLIST_PAGES: u32 = 4;
const PAGE: u32 = 50;
const TOP_ARTISTS: u32 = 10;
/// Avatar edge (px) for the round header portrait.
const AVATAR_PX: u32 = 320;

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileView {
    /// `none`, `loading`, `ready`, `unavailable` (Spotify did not give the profile).
    pub status: String,
    /// Resolved id (`"me"` becomes the account id).
    pub user_id: String,
    pub is_me: bool,
    pub name: String,
    /// `file:///` URL of the cached avatar, empty if none.
    pub image_url: String,
    /// -1 when Spotify does not say (never shown as 0).
    pub followers: i64,
    /// Public playlists found; -1 when they could not be read.
    pub playlist_count: i64,
    pub playlists_available: bool,
    pub models: [Vec<Value>; PROFILE_MODELS],
}

impl Default for ProfileView {
    fn default() -> Self {
        Self {
            status: "none".into(),
            user_id: String::new(),
            is_me: false,
            name: String::new(),
            image_url: String::new(),
            followers: -1,
            playlist_count: -1,
            playlists_available: false,
            models: Default::default(),
        }
    }
}

/// What one profile load gathered from the Web API.
pub struct Loaded {
    pub profile: Result<PublicProfile>,
    pub playlists: Result<Vec<Playlist>>,
    pub top_artists: Vec<Artist>,
}

/// Public playlists owned by `user_id`, following Spotify's raw paging (offset + limit).
async fn public_playlists(api: &dyn MusicProvider, user_id: &str) -> Result<Vec<Playlist>> {
    let mut all = Vec::new();
    let mut offset = 0;
    for _ in 0..PLAYLIST_PAGES {
        let page = api.user_public_playlists(user_id, offset, PAGE).await?;
        all.extend(page.items);
        offset = page.offset + page.limit.max(1);
        if offset >= page.total {
            break;
        }
    }
    Ok(all)
}

/// The three reads of a profile, concurrently. `user_id` is already resolved (never "me").
pub async fn load(api: &dyn MusicProvider, user_id: &str, is_me: bool) -> Loaded {
    let (profile, playlists, top_artists) = tokio::join!(
        api.user_profile(if is_me { "me" } else { user_id }),
        public_playlists(api, user_id),
        async {
            if is_me {
                api.top_artists(TimeRange::ShortTerm, 0, TOP_ARTISTS)
                    .await
                    .map(|p| p.items)
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        }
    );
    Loaded {
        profile,
        playlists,
        top_artists,
    }
}

/// Builds the view of a load. `account` = (display name, avatar file URL) of the signed-in
/// user: the own profile falls back to it when `/me` fails. `me_id` marks playlist ownership.
pub fn view_of(
    loaded: &Loaded,
    user_id: &str,
    is_me: bool,
    me_id: &str,
    account: (&str, &str),
) -> ProfileView {
    let mut view = ProfileView {
        user_id: user_id.into(),
        is_me,
        ..ProfileView::default()
    };
    match &loaded.profile {
        Ok(p) => {
            view.name.clone_from(&p.display_name);
            view.followers = p
                .followers
                .map_or(-1, |n| i64::try_from(n).unwrap_or(i64::MAX));
        }
        Err(_) if is_me && !account.0.is_empty() => {
            view.name = account.0.into();
            view.image_url = account.1.into();
        }
        Err(_) => {
            view.status = "unavailable".into();
            return view;
        }
    }
    view.status = "ready".into();
    if let Ok(playlists) = &loaded.playlists {
        view.playlists_available = true;
        view.playlist_count = playlists.len() as i64;
        view.models[1] = playlists
            .iter()
            .map(|p| playlist_row(p, false, me_id))
            .collect();
    }
    view.models[0] = loaded.top_artists.iter().map(artist_row).collect();
    view
}

pub struct ProfileCtl {
    env: Arc<Env>,
    generation: Arc<Generation>,
    // Publication and generation changes are serialized, never held over await.
    publication: Mutex<()>,
    task: Mutex<Option<tokio::task::AbortHandle>>,
    pub view: watch::Sender<ProfileView>,
}

impl ProfileCtl {
    pub fn new(env: &Arc<Env>) -> Arc<Self> {
        Arc::new(Self {
            env: env.clone(),
            generation: Arc::new(Generation::default()),
            publication: Mutex::new(()),
            task: Mutex::new(None),
            view: watch::channel(ProfileView::default()).0,
        })
    }

    pub fn clear(&self) {
        let _commit = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.bump();
        if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
        if self.view.borrow().status != "none" {
            self.view.send_replace(ProfileView::default());
        }
    }

    /// `"me"` (or the account's own id) opens the own profile. Returns whether it is the own one.
    pub fn open(self: &Arc<Self>, requested: String) -> bool {
        let account = self.env.account.snapshot();
        let is_me =
            requested == "me" || (!account.user_id.is_empty() && requested == account.user_id);
        let user_id = if is_me {
            account.user_id.clone()
        } else {
            requested
        };
        let ticket = {
            let _commit = self.publication.lock().unwrap_or_else(|e| e.into_inner());
            let ticket = self.generation.bump();
            if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
                task.abort();
            }
            self.view.send_replace(ProfileView {
                status: if user_id.is_empty() {
                    "unavailable"
                } else {
                    "loading"
                }
                .into(),
                user_id: user_id.clone(),
                is_me,
                // The own name is known already: the header does not wait for the network.
                name: if is_me {
                    account.display_name.clone()
                } else {
                    String::new()
                },
                image_url: if is_me {
                    account.avatar_url.clone()
                } else {
                    String::new()
                },
                ..ProfileView::default()
            });
            ticket
        };
        if user_id.is_empty() {
            return is_me;
        }
        let this = self.clone();
        let task = self.env.rt.spawn(async move {
            let Some(api) = this.env.account.api() else {
                let _commit = this.publication.lock().unwrap_or_else(|e| e.into_inner());
                if this.generation.current() == ticket {
                    this.view.send_modify(|v| v.status = "unavailable".into());
                }
                return;
            };
            let loaded = load(&*api, &user_id, is_me).await;
            let avatar = loaded
                .profile
                .as_ref()
                .ok()
                .and_then(|p| pick_image(&p.images, AVATAR_PX).map(|i| i.url.clone()));
            let artist_images: Vec<_> = loaded
                .top_artists
                .iter()
                .map(|a| image_of(&a.images))
                .collect();
            let playlist_images: Vec<_> = loaded
                .playlists
                .as_ref()
                .map(|ps| ps.iter().map(|p| image_of(&p.images)).collect())
                .unwrap_or_default();
            {
                let _commit = this.publication.lock().unwrap_or_else(|e| e.into_inner());
                if this.generation.current() != ticket {
                    return;
                }
                if let Err(e) = &loaded.profile {
                    if !matches!(e, VeloError::NotFound | VeloError::Forbidden(_)) {
                        tracing::warn!(error = %e, "profile unavailable");
                    }
                }
                let mut view = view_of(
                    &loaded,
                    &user_id,
                    is_me,
                    &account.user_id,
                    (&account.display_name, &account.avatar_url),
                );
                // Keep the account avatar on screen until the larger one is cached.
                if view.image_url.is_empty() && is_me {
                    view.image_url.clone_from(&account.avatar_url);
                }
                this.view.send_replace(view);
            }
            this.fill_images(ticket, avatar, [artist_images, playlist_images]);
        });
        let _commit = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        if self.generation.current() == ticket {
            *self.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.abort_handle());
        } else {
            task.abort();
        }
        is_me
    }

    fn fill_images(
        self: &Arc<Self>,
        ticket: u64,
        avatar: Option<String>,
        rows: [Vec<Option<String>>; PROFILE_MODELS],
    ) {
        let permits = Arc::new(tokio::sync::Semaphore::new(4));
        let jobs = avatar.map(|url| (None, url, AVATAR_PX)).into_iter().chain(
            rows.into_iter().enumerate().flat_map(|(model, urls)| {
                urls.into_iter()
                    .enumerate()
                    .filter_map(move |(row, url)| url.map(|u| (Some((model, row)), u, THUMB_PX)))
            }),
        );
        for (target, url, px) in jobs {
            let this = self.clone();
            let permits = permits.clone();
            self.env.rt.spawn(async move {
                let Ok(_permit) = permits.acquire_owned().await else {
                    return;
                };
                if this.generation.current() != ticket {
                    return;
                }
                let Ok(path) = this.env.covers.thumbnail(&url, px).await else {
                    return;
                };
                let _commit = this.publication.lock().unwrap_or_else(|e| e.into_inner());
                if this.generation.current() != ticket {
                    return;
                }
                this.view.send_modify(|v| match target {
                    None => v.image_url = file_url(&path),
                    Some((model, row)) => {
                        if let Some(r) = v.models[model].get_mut(row) {
                            r["imageUrl"] = json!(file_url(&path));
                        }
                    }
                });
            });
        }
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

    async fn provider() -> (MockServer, SpotifyProvider) {
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
        let api = SpotifyProvider::new(
            SpotifyConfig {
                client_id: "fixture".into(),
                api_base: server.uri(),
                accounts_base: server.uri(),
                redirect_port: 0,
            },
            tokens,
        );
        (server, api)
    }

    fn playlist(id: &str, owner: &str, public: bool) -> Value {
        json!({"id":id,"name":id,"uri":format!("spotify:playlist:{id}"),"public":public,
               "owner":{"id":owner,"display_name":owner},"items":{"total":3},"images":[]})
    }

    #[test]
    fn own_profile_reads_me_top_artists_and_pages_public_playlists() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (server, api) = provider().await;
            Mock::given(method("GET"))
                .and(path("/me"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"id":"juan","display_name":"Juan","images":[],"followers":{"total":7}}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/me/top/artists"))
                .and(query_param("time_range", "short_term"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "items":[{"id":"a1","name":"Lumen","uri":"spotify:artist:a1","images":[]}],
                    "offset":0,"limit":10,"total":1
                })))
                .expect(1)
                .mount(&server)
                .await;
            // Two raw pages; the second starts where the first one's slots end.
            Mock::given(method("GET"))
                .and(path("/users/juan/playlists"))
                .and(query_param("offset", "0"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "items":[playlist("p1","juan",true), playlist("p2","juan",false), playlist("p3","iria",true)],
                    "offset":0,"limit":50,"total":51
                })))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/users/juan/playlists"))
                .and(query_param("offset", "50"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "items":[playlist("p4","juan",true)],"offset":50,"limit":50,"total":51
                })))
                .expect(1)
                .mount(&server)
                .await;
            let loaded = load(&api, "juan", true).await;
            let view = view_of(&loaded, "juan", true, "juan", ("Juan", ""));
            assert_eq!(view.status, "ready");
            assert!(view.is_me);
            assert_eq!((view.name.as_str(), view.followers), ("Juan", 7));
            assert_eq!(view.playlist_count, 2);
            assert!(view.playlists_available);
            assert_eq!(view.models[1][0]["playlistId"], "p1");
            assert_eq!(view.models[1][1]["playlistId"], "p4");
            assert_eq!(view.models[1][0]["owned"], true);
            assert_eq!(view.models[0][0]["artistId"], "a1");
        });
    }

    #[test]
    fn someone_elses_profile_has_no_top_lists_and_survives_missing_playlists() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (server, api) = provider().await;
            Mock::given(method("GET"))
                .and(path("/users/iria"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"id":"iria","display_name":"Iria","images":[]})),
                )
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/users/iria/playlists"))
                .respond_with(
                    ResponseTemplate::new(403)
                        .set_body_json(json!({"error":{"status":403,"message":"Forbidden"}})),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/me/top/artists"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[]})))
                .expect(0)
                .mount(&server)
                .await;
            let loaded = load(&api, "iria", false).await;
            let view = view_of(&loaded, "iria", false, "juan", ("Juan", ""));
            assert_eq!(view.status, "ready");
            assert!(!view.is_me);
            assert_eq!(view.name, "Iria");
            // Unknown, not zero.
            assert_eq!(view.followers, -1);
            assert_eq!(view.playlist_count, -1);
            assert!(!view.playlists_available);
            assert!(view.models.iter().all(Vec::is_empty));
        });
    }

    #[test]
    fn a_profile_spotify_will_not_give_is_unavailable_but_the_own_one_falls_back() {
        let loaded = || Loaded {
            profile: Err(VeloError::Forbidden("removed".into())),
            playlists: Err(VeloError::NotFound),
            top_artists: Vec::new(),
        };
        let other = view_of(&loaded(), "iria", false, "juan", ("Juan", ""));
        assert_eq!(other.status, "unavailable");
        let own = view_of(&loaded(), "juan", true, "juan", ("Juan", "file:///a.png"));
        assert_eq!(own.status, "ready");
        assert_eq!(
            (own.name.as_str(), own.image_url.as_str()),
            ("Juan", "file:///a.png")
        );
        assert_eq!(own.followers, -1);
    }
}
