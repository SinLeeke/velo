//! One-shot home discovery and explicit radio actions, all on Tokio workers.
//! No public recommendations endpoint, no timers/polling; audio readiness is pushed by watch.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::{sync::watch, task::AbortHandle};
use velo_domain::{Artist, CoverArtService, Track, VeloError};
use velo_player::{
    Engine, LoadTarget, PlayerError,
    discover::{Discover, Mix, Seed},
};

use super::{
    Env,
    audio::audio_message,
    library::LibraryCtl,
    rows::{THUMB_PX, file_url, image_of},
};
use crate::model::ModelOp;

pub const MIX_ROLES: &str = "mixId,name,subtitle,imageUrl";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiscoverView {
    pub available: bool,
    pub rows: Vec<Value>,
}

impl DiscoverView {
    /// Thumbnail completion changes a role rather than resetting the list's focus/scroll.
    pub fn row_changes(&self, previous: &Self) -> Vec<ModelOp> {
        if self.rows.len() != previous.rows.len()
            || self.rows.iter().zip(&previous.rows).any(|(a, b)| {
                ["mixId", "name", "subtitle"]
                    .iter()
                    .any(|key| a[*key] != b[*key])
            })
        {
            return vec![ModelOp::Replace(self.rows.clone())];
        }
        self.rows
            .iter()
            .zip(&previous.rows)
            .enumerate()
            .filter(|(_, (a, b))| a["imageUrl"] != b["imageUrl"])
            .map(|(row, (a, _))| ModelOp::SetField {
                row,
                field: "imageUrl",
                value: a["imageUrl"].clone(),
            })
            .collect()
    }
}

#[derive(Default)]
struct Inner {
    generation: u64,
    home_generation: u64,
    seeds: Vec<Seed>,
    mixes: Vec<Mix>,
    task: Option<AbortHandle>,
}

pub struct DiscoverCtl {
    env: Arc<Env>,
    pub view: watch::Sender<DiscoverView>,
    inner: Mutex<Inner>,
}

/// Alternate favorite tracks and artists, so a handful of tracks cannot crowd out artists.
pub fn home_seeds(tracks: &[Track], artists: &[Artist]) -> Vec<Seed> {
    let mut seeds = Vec::new();
    for index in 0..velo_player::discover::MAX_MIXES {
        if let Some(track) = tracks.get(index) {
            seeds.push(Seed {
                uri: track.uri.clone(),
                title: track.title.clone(),
                subtitle: track.artist_line(),
                image_url: image_of(&track.images),
            });
        }
        if let Some(artist) = artists.get(index) {
            seeds.push(Seed {
                uri: artist.uri.clone(),
                title: artist.name.clone(),
                subtitle: String::new(),
                image_url: image_of(&artist.images),
            });
        }
    }
    let mut seen = std::collections::HashSet::new();
    seeds
        .into_iter()
        .filter(|s| {
            velo_player::discover::validate_seed(&s.uri).is_ok() && seen.insert(s.uri.clone())
        })
        .take(velo_player::discover::MAX_MIXES)
        .collect()
}

pub fn mix_row(mix: &Mix, local_image: &str) -> Value {
    json!({ "mixId": mix.id, "name": mix.title, "subtitle": mix.subtitle, "imageUrl": local_image })
}

impl DiscoverCtl {
    pub fn start(env: &Arc<Env>) -> Arc<Self> {
        let this = Arc::new(Self {
            env: env.clone(),
            view: watch::channel(DiscoverView::default()).0,
            inner: Mutex::new(Inner::default()),
        });
        let me = this.clone();
        let mut audio = env.audio.view.subscribe();
        env.rt.spawn(async move {
            let mut ready = false;
            loop {
                let now = audio.borrow_and_update().ready;
                if now != ready {
                    ready = now;
                    me.reload();
                }
                if audio.changed().await.is_err() {
                    break;
                }
            }
        });
        this
    }

    /// Invalidates a previous home request immediately, including in-flight internal requests.
    pub fn begin_home(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::invalidate(&mut inner);
        inner.home_generation += 1;
        inner.seeds.clear();
        self.view.send_replace(DiscoverView::default());
        inner.home_generation
    }

    fn invalidate(inner: &mut Inner) {
        inner.generation += 1;
        inner.mixes.clear();
        if let Some(task) = inner.task.take() {
            task.abort();
        }
    }

    pub fn set_home_seeds(self: &Arc<Self>, generation: u64, seeds: Vec<Seed>) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.home_generation != generation {
                return;
            }
            inner.seeds = seeds;
        }
        self.reload();
    }

    fn reload(self: &Arc<Self>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::invalidate(&mut inner);
        self.view.send_replace(DiscoverView::default());
        let Some(engine) = self
            .env
            .audio
            .engine()
            .filter(|_| self.env.audio.is_ready())
        else {
            return;
        };
        if inner.seeds.is_empty() {
            return;
        }
        let generation = inner.generation;
        let seeds = inner.seeds.clone();
        let this = self.clone();
        let task = self.env.rt.spawn(async move {
            let mixes = match Discover::new(&engine, &seeds).for_you().await {
                Ok(mixes) => mixes,
                Err(error) => {
                    // Expected on accounts where internal endpoints are unavailable; hide the row.
                    tracing::debug!(%error, "home discovery unavailable");
                    return;
                }
            };
            let rows = mixes.iter().map(|m| mix_row(m, "")).collect();
            if !this.publish(generation, &engine, mixes.clone(), rows) {
                return;
            }
            // Only disk cache URLs reach QML. Covers come from Web API seed metadata.
            for (index, mix) in mixes.iter().enumerate() {
                if let Some(url) = &mix.image_url {
                    if let Ok(path) = this.env.covers.thumbnail(url, THUMB_PX).await {
                        let inner = this.inner.lock().unwrap_or_else(|e| e.into_inner());
                        if inner.generation != generation {
                            return;
                        }
                        this.view.send_modify(|v| {
                            if let Some(row) = v.rows.get_mut(index) {
                                row["imageUrl"] = json!(file_url(&path));
                            }
                        });
                    }
                }
            }
        });
        inner.task = Some(task.abort_handle());
    }

    fn publish(
        &self,
        generation: u64,
        engine: &Arc<Engine>,
        mixes: Vec<Mix>,
        rows: Vec<Value>,
    ) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != generation
            || !self.env.audio.is_ready()
            || !self
                .env
                .audio
                .engine()
                .is_some_and(|current| Arc::ptr_eq(&current, engine))
        {
            return false;
        }
        inner.mixes = mixes;
        self.view.send_replace(DiscoverView {
            available: !rows.is_empty(),
            rows,
        });
        true
    }

    pub fn play_mix(self: &Arc<Self>, id: String) {
        let mix = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mixes
            .iter()
            .find(|m| m.id == id)
            .cloned();
        let this = self.clone();
        self.env.rt.spawn(async move {
            let Some(engine) = ready_engine(&this.env).await else {
                return;
            };
            let Some(mix) = mix else { return };
            match mix.load_target() {
                Ok(target) => load_on_velo(&this.env, &engine, target),
                Err(_) => report_discovery(&this.env),
            }
        });
    }
}

async fn ready_engine(env: &Env) -> Option<Arc<Engine>> {
    // E2b: the first radio or mix with Velo's audio off opens the one-time audio login
    // instead of failing; a cancelled or failed login already showed its own message.
    if !env.audio.is_ready() && env.account.snapshot().signed_in && !env.audio.ensure_ready().await
    {
        return None;
    }
    match env.audio.engine().filter(|_| env.audio.is_ready()) {
        Some(engine) => Some(engine),
        None => {
            report_audio(env, &PlayerError::NotSignedIn);
            None
        }
    }
}

fn report_audio(env: &Env, error: &PlayerError) {
    let _ = env.errors.send(audio_message(error));
}

fn load_on_velo(env: &Env, engine: &Arc<Engine>, target: LoadTarget) {
    // A radio request can outlive sign-out or reconnection; never load the replaced engine.
    if !env.audio.is_ready()
        || !env
            .audio
            .engine()
            .is_some_and(|current| Arc::ptr_eq(&current, engine))
    {
        report_audio(env, &PlayerError::NotSignedIn);
        return;
    }
    match engine.load(target) {
        Ok(()) => env.audio.set_chosen_device(None),
        Err(error) => report_audio(env, &error),
    }
}

fn report_discovery(env: &Env) {
    env.report(&VeloError::Other(
        "No se pudo cargar la radio de Spotify. Vuelve a intentarlo.".into(),
    ));
}

pub fn start_radio(env: &Arc<Env>, seed_uri: String) {
    let env = env.clone();
    env.rt.clone().spawn(async move {
        let Some(engine) = ready_engine(&env).await else {
            return;
        };
        match Discover::new(&engine, &[])
            .radio(&seed_uri)
            .await
            .and_then(|s| s.load_target())
        {
            Ok(target) => load_on_velo(&env, &engine, target),
            Err(_) => report_discovery(&env),
        }
    });
}

impl LibraryCtl {
    pub fn save_radio_as_playlist(self: &Arc<Self>, seed_uri: String) {
        let this = self.clone();
        self.env.rt.spawn(async move {
            let Some(engine) = ready_engine(&this.env).await else {
                return;
            };
            let Some(api) = this.env.account.api() else {
                return;
            };
            let user = this.env.account.snapshot().user_id;
            let station = match Discover::new(&engine, &[]).radio(&seed_uri).await {
                Ok(station) => station,
                Err(_) => return report_discovery(&this.env),
            };
            // Do not create a playlist under a replacement account after a slow radio request.
            if this.env.account.snapshot().user_id != user || !this.env.account.snapshot().signed_in
            {
                return;
            }
            let title = if station.title.is_empty() {
                "Radio de Spotify"
            } else {
                &station.title
            };
            match api
                .create_playlist(title, "Radio guardada desde Velo", false)
                .await
            {
                Ok(playlist) => {
                    if let Err(error) = api.add_to_playlist(&playlist.id, &station.tracks).await {
                        // Creation already succeeded; refresh even when the add failed.
                        this.env.report(&error);
                    }
                    this.load_playlists().await;
                }
                Err(error) => this.env.report(&error),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mix_rows_use_stable_ids_and_only_cached_images() {
        let mix = Mix {
            id: "seed".into(),
            title: "Radio".into(),
            subtitle: "Artist".into(),
            image_url: Some("https://remote/image".into()),
            play_uri: None,
            tracks: vec![],
        };
        let row = mix_row(&mix, "file:///cached.png");
        assert_eq!(row["mixId"], "seed");
        assert_eq!(row["imageUrl"], "file:///cached.png");
        assert_eq!(mix_row(&mix, "")["imageUrl"], "");
        let before = DiscoverView {
            available: true,
            rows: vec![mix_row(&mix, "")],
        };
        let after = DiscoverView {
            available: true,
            rows: vec![row],
        };
        assert_eq!(
            after.row_changes(&before),
            vec![ModelOp::SetField {
                row: 0,
                field: "imageUrl",
                value: json!("file:///cached.png")
            }]
        );
        assert_eq!(
            after.row_changes(&DiscoverView::default()),
            vec![ModelOp::Replace(after.rows.clone())]
        );
    }
    #[test]
    fn home_seeds_reject_unknown_uris_and_keep_artist_metadata() {
        let artist = Artist {
            id: "a".into(),
            uri: "spotify:artist:4uLU6hMCjMI75M1A2tKUQC".into(),
            name: "Artist".into(),
            images: vec![],
            genres: vec![],
            followers: None,
        };
        let mut invalid = artist.clone();
        invalid.uri = "spotify:artist:bad".into();
        let seeds = home_seeds(&[], &[invalid, artist.clone(), artist]);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].title, "Artist");
    }
}
