//! F6: perfil adicional de Library. Trabajo en Tokio, publicación por generación.
use super::{
    Env,
    images::Generation,
    rows::{album_row, artist_row, file_url, image_of, track_row},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use velo_domain::{Album, CoverArtService};
use velo_player::{LoadTarget, artist::ArtistProfile};

pub const ARTIST_MODELS: usize = 6;
pub const ARTIST_MODEL_ROLES: [&str; ARTIST_MODELS] = [
    super::rows::TRACK_ROLES,
    super::rows::ARTIST_ROLES,
    "albumId,name,artist,imageUrl,year,albumType",
    super::rows::ALBUM_ROLES,
    "imageUrl",
    "city,country,listeners",
];

#[derive(Clone, Debug, PartialEq)]
pub struct ArtistView {
    pub status: String,
    pub biography: String,
    pub monthly_listeners: i32,
    pub followers: i32,
    pub verified: bool,
    pub header_image_url: String,
    pub models: [Vec<Value>; ARTIST_MODELS],
    pub discography_type: String,
    pub incomplete: bool,
}
impl Default for ArtistView {
    fn default() -> Self {
        Self {
            status: "none".into(),
            biography: String::new(),
            monthly_listeners: -1,
            followers: -1,
            verified: false,
            header_image_url: String::new(),
            models: Default::default(),
            discography_type: "all".into(),
            incomplete: false,
        }
    }
}

pub struct ArtistsCtl {
    env: Arc<Env>,
    player: Arc<super::player::PlayerCtl>,
    generation: Arc<Generation>,
    // Publication and generation changes are serialized, never held over await.
    publication: Mutex<()>,
    profile: Mutex<Option<ArtistProfile>>,
    task: Mutex<Option<tokio::task::AbortHandle>>,
    pub view: watch::Sender<ArtistView>,
}

pub fn discography_rows(albums: &[Album], kind: &str) -> Vec<Value> {
    albums
        .iter()
        .filter(|a| matches_discography_type(&a.album_type, kind))
        .map(|a| {
            let mut row = album_row(a);
            row["albumType"] = json!(a.album_type);
            row
        })
        .collect()
}

/// El chip existente «Sencillos y EP» incluye ambos tipos sin perder la etiqueta EP.
fn matches_discography_type(album_type: &str, kind: &str) -> bool {
    kind == "all" || album_type == kind || (kind == "single" && album_type == "ep")
}

/// Las miniaturas actualizan un rol sin reiniciar el scroll ni la selección de Qt.
pub fn row_changes(rows: &[Value], previous: &[Value]) -> Vec<crate::model::ModelOp> {
    use crate::model::ModelOp;
    if rows.len() != previous.len()
        || rows.iter().zip(previous).any(|(a, b)| {
            let mut a = a.clone();
            let mut b = b.clone();
            a["imageUrl"] = Value::Null;
            b["imageUrl"] = Value::Null;
            a != b
        })
    {
        return vec![ModelOp::Replace(rows.to_vec())];
    }
    rows.iter()
        .zip(previous)
        .enumerate()
        .filter(|(_, (a, b))| a["imageUrl"] != b["imageUrl"])
        .map(|(row, (a, _))| ModelOp::SetField {
            row,
            field: "imageUrl",
            value: a["imageUrl"].clone(),
        })
        .collect()
}

impl ArtistsCtl {
    pub fn new(env: &Arc<Env>, player: &Arc<super::player::PlayerCtl>) -> Arc<Self> {
        Arc::new(Self {
            env: env.clone(),
            player: player.clone(),
            generation: Arc::new(Generation::default()),
            publication: Mutex::new(()),
            profile: Mutex::new(None),
            task: Mutex::new(None),
            view: watch::channel(ArtistView::default()).0,
        })
    }
    pub fn clear(&self) {
        let _commit = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.bump();
        if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
        *self.profile.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.view.send_replace(ArtistView::default());
    }
    pub fn open(self: &Arc<Self>, uri: String) {
        let ticket = {
            let _commit = self.publication.lock().unwrap_or_else(|e| e.into_inner());
            let ticket = self.generation.bump();
            if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
                task.abort();
            }
            *self.profile.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.view.send_replace(ArtistView {
                status: "loading".into(),
                ..ArtistView::default()
            });
            ticket
        };
        let this = self.clone();
        let task = self.env.rt.spawn(async move {
            let (profile, overview) = match this
                .env
                .audio
                .engine()
                .filter(|_| this.env.audio.is_ready())
            {
                Some(engine) => {
                    let (profile, overview) = tokio::join!(
                        velo_player::artist::profile(&engine, &uri),
                        velo_player::session_api::catalog::artist_overview(engine.session(), &uri)
                    );
                    (profile.ok(), overview.ok())
                }
                None => (None, None),
            };
            let (profile, header_source) = {
                let _commit = this.publication.lock().unwrap_or_else(|e| e.into_inner());
                if this.generation.current() != ticket {
                    return;
                }
                if profile.is_none() && overview.is_none() {
                    this.view.send_replace(ArtistView {
                        status: "unavailable".into(),
                        ..ArtistView::default()
                    });
                    return;
                }
                let metadata_missing = profile.is_none();
                let profile = profile.unwrap_or_default();
                *this.profile.lock().unwrap_or_else(|e| e.into_inner()) = Some(profile.clone());
                let mut view = ArtistView {
                    status: "ready".into(),
                    biography: velo_player::session_api::catalog::biography_plain_text(
                        &profile.biography,
                    ),
                    incomplete: profile.incomplete || metadata_missing,
                    ..ArtistView::default()
                };
                view.models[0] = profile
                    .popular
                    .iter()
                    .map(|t| track_row(t, false))
                    .collect();
                view.models[1] = profile.related.iter().map(artist_row).collect();
                view.models[2] = discography_rows(&profile.discography, "all");
                view.models[3] = profile.appears_on.iter().map(album_row).collect();
                view.models[4] = profile
                    .photos
                    .iter()
                    .take(16)
                    .map(|_| json!({"imageUrl": ""}))
                    .collect();
                let header_source = if let Some(overview) = overview {
                    view.monthly_listeners = overview.monthly_listeners;
                    view.followers = overview.followers;
                    view.verified = overview.verified;
                    view.biography = overview.biography;
                    view.models[5] = overview.top_cities;
                    overview.header_image_url
                } else {
                    String::new()
                };
                this.view.send_replace(view);
                (profile, header_source)
            };
            this.fill_images(ticket, &profile, "all");
            if !header_source.is_empty() {
                let this = this.clone();
                let result = this.env.covers.thumbnail(&header_source, 600).await;
                if let Ok(path) = result {
                    let _commit = this.publication.lock().unwrap_or_else(|e| e.into_inner());
                    if this.generation.current() == ticket {
                        this.view
                            .send_modify(|v| v.header_image_url = file_url(&path));
                    }
                }
            }
        });
        let _commit = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        if self.generation.current() == ticket {
            *self.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.abort_handle());
        } else {
            task.abort();
        }
    }
    fn fill_images(self: &Arc<Self>, ticket: u64, profile: &ArtistProfile, kind: &str) {
        let images: [Vec<Option<String>>; ARTIST_MODELS] = [
            profile
                .popular
                .iter()
                .map(|t| image_of(&t.images))
                .collect(),
            profile
                .related
                .iter()
                .map(|a| image_of(&a.images))
                .collect(),
            profile
                .discography
                .iter()
                .filter(|a| matches_discography_type(&a.album_type, kind))
                .map(|a| image_of(&a.images))
                .collect(),
            profile
                .appears_on
                .iter()
                .map(|a| image_of(&a.images))
                .collect(),
            profile
                .photos
                .iter()
                .take(16)
                .map(|i| Some(i.url.clone()))
                .collect(),
            Vec::new(),
        ];
        let permits = Arc::new(tokio::sync::Semaphore::new(4));
        for (model, urls) in images.into_iter().enumerate() {
            for (row, url) in urls.into_iter().enumerate() {
                let Some(url) = url else { continue };
                let this = self.clone();
                let permits = permits.clone();
                let kind = kind.to_string();
                self.env.rt.spawn(async move {
                    let Ok(_permit) = permits.acquire_owned().await else {
                        return;
                    };
                    if this.generation.current() != ticket {
                        return;
                    }
                    if let Ok(path) = this
                        .env
                        .covers
                        .thumbnail(&url, if model == 4 { 600 } else { 128 })
                        .await
                    {
                        let _commit = this.publication.lock().unwrap_or_else(|e| e.into_inner());
                        if this.generation.current() != ticket {
                            return;
                        }
                        this.view.send_modify(|v| {
                            if model == 2 && v.discography_type != kind {
                                return;
                            }
                            if let Some(r) = v.models[model].get_mut(row) {
                                r["imageUrl"] = json!(file_url(&path));
                            }
                        });
                    }
                });
            }
        }
    }
    pub fn set_discography_type(self: &Arc<Self>, kind: String) {
        if !matches!(kind.as_str(), "all" | "album" | "single" | "compilation") {
            return;
        }
        let this = self.clone();
        self.env.rt.spawn(async move {
            let _commit = this.publication.lock().unwrap_or_else(|e| e.into_inner());
            let profile = this
                .profile
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let Some(profile) = profile else { return };
            let ticket = this.generation.current();
            this.view.send_modify(|v| {
                v.discography_type = kind.clone();
                v.models[2] = discography_rows(&profile.discography, &kind);
            });
            drop(_commit);
            this.fill_images(ticket, &profile, &kind);
        });
    }
    pub fn play_popular(self: &Arc<Self>, index: i32) {
        let this = self.clone();
        let generation = self.player.begin_playback();
        self.env.rt.spawn(async move {
            let Some(uris) = this
                .profile
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|p| p.popular.iter().map(|t| t.uri.clone()).collect::<Vec<_>>())
            else {
                return;
            };
            let Ok(index) = u32::try_from(index) else {
                return;
            };
            if index as usize >= uris.len() {
                return;
            }
            let route = match this.player.prepare_playback().await {
                Ok(Some(route)) if this.player.playback_current(generation) => route,
                Ok(_) => return,
                Err(e) => {
                    this.env.report(&e);
                    return;
                }
            };
            if route == crate::bridge::source::PlayRoute::Velo {
                let Some(engine) = this.env.audio.engine() else {
                    return;
                };
                if let Err(e) = engine.load(LoadTarget::Tracks {
                    uris,
                    index,
                    shuffle: false,
                }) {
                    let _ = this.env.errors.send(super::audio::audio_message(&e));
                }
            } else if let Some(api) = this.env.account.api() {
                if let Err(e) = api.play_uris(&uris, Some(index)).await {
                    this.env.report(&e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absence_is_not_zero_listeners_or_a_fabricated_biography() {
        let view = ArtistView::default();
        assert_eq!(view.status, "none");
        assert!(view.biography.is_empty());
        assert_eq!((view.monthly_listeners, view.followers), (-1, -1));
        assert!(!view.verified && view.header_image_url.is_empty());
        assert!(view.models.iter().all(Vec::is_empty));
    }
    #[test]
    fn type_filters_keep_album_kind_in_the_contract() {
        let album = |kind: &str| Album {
            id: kind.into(),
            uri: String::new(),
            name: kind.into(),
            artists: vec![],
            images: vec![],
            release_date: "2025".into(),
            total_tracks: 1,
            album_type: kind.into(),
        };
        let albums = vec![
            album("album"),
            album("single"),
            album("ep"),
            album("compilation"),
        ];
        assert_eq!(discography_rows(&albums, "all").len(), 4);
        let rows = discography_rows(&albums, "single");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["albumType"], "single");
        assert_eq!(rows[1]["albumType"], "ep");
        assert!(discography_rows(&albums, "unknown").is_empty());
    }
}
