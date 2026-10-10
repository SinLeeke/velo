//! Source (0): Velo's own engine, the Spotify Connect device "Velo" (librespot).
//!
//! State is pushed by librespot's player events through `engine::audio` (no polling, no Web
//! API). Actions go straight to Spirc. Active only while "Velo" is the account's active Connect
//! device; then it outranks the other sources.

use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::watch;
use velo_domain::{Result, VeloError};
use velo_player::EngineState;

use super::source::{PlaybackSource, Refresh, SourceAction, SourceState, SourceTrack};
use crate::engine::Env;
use crate::engine::audio::audio_message;

static LIVI_THUMB: LazyLock<Arc<Vec<u8>>> =
    LazyLock::new(|| Arc::new(include_bytes!("../assets/livi.png").to_vec()));

pub struct VeloSource {
    env: Arc<Env>,
    tx: watch::Sender<SourceState>,
}

/// A transfer is asynchronous in Spirc; play sent before activation is ignored.
async fn wait_for_session(mut state: watch::Receiver<SourceState>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            {
                let st = state.borrow_and_update();
                if st.active && st.track.is_some() {
                    return Ok(());
                }
            }
            state
                .changed()
                .await
                .map_err(|_| VeloError::NoActiveDevice)?;
        }
    })
    .await
    .map_err(|_| VeloError::NoActiveDevice)?
}

/// The engine's state as a playback source. `name` is the Connect device name ("Velo").
pub fn state_of(s: &EngineState, name: &str) -> SourceState {
    let speech = s.dj.speech.as_ref().filter(|_| s.dj.active);
    SourceState {
        active: s.active,
        track: speech
            .map(|speech| SourceTrack {
                title: if speech.title.trim().is_empty() {
                    "DJ".into()
                } else {
                    speech.title.clone()
                },
                artist: "DJ Livi".into(),
                album: speech.album.clone(),
                duration_ms: speech.duration_ms,
                uri: None,
                thumbnail: Some(LIVI_THUMB.clone()),
                cover_url: speech.image_url.clone(),
            })
            .or_else(|| {
                s.track.as_ref().map(|t| SourceTrack {
                    title: t.title.clone(),
                    artist: t.artist.clone(),
                    album: t.album.clone(),
                    duration_ms: t.duration_ms,
                    uri: Some(t.uri.clone()),
                    thumbnail: None,
                    cover_url: t.cover_url.clone(),
                })
            }),
        is_playing: s.active && speech.map_or(s.playing, |speech| speech.playing),
        position_ms: speech.map_or(s.position_ms, |speech| speech.position_ms),
        position_at: speech.map_or(s.position_at, |speech| speech.position_at),
        volume: Some(s.volume),
        shuffle: s.shuffle,
        repeat: s.repeat,
        device_name: name.to_string(),
        controllable: s.active,
        device_available: true,
        // The engine's PCM feeds the visualizer (E3); there is no process to loop back.
        pid: None,
        remote: false,
        own_engine: true,
        dj: s.dj.clone(),
    }
}

impl VeloSource {
    pub fn start(env: &Arc<Env>) -> Arc<Self> {
        let this = Arc::new(Self {
            env: env.clone(),
            tx: watch::channel(SourceState {
                own_engine: true,
                ..SourceState::default()
            })
            .0,
        });
        let me = this.clone();
        let mut engine = env.audio.state.subscribe();
        env.rt.spawn(async move {
            loop {
                let state = state_of(&engine.borrow_and_update(), crate::APP_NAME);
                me.tx.send_replace(state);
                if engine.changed().await.is_err() {
                    break;
                }
            }
        });
        this
    }
}

#[async_trait]
impl PlaybackSource for VeloSource {
    fn name(&self) -> &'static str {
        "velo-engine"
    }

    fn state(&self) -> watch::Receiver<SourceState> {
        self.tx.subscribe()
    }

    async fn act(&self, action: SourceAction) -> Result<()> {
        let engine = self.env.audio.engine().ok_or(VeloError::NoActiveDevice)?;
        let result = match action {
            SourceAction::Play => {
                if !self.tx.borrow().active {
                    let state = self.tx.subscribe();
                    engine
                        .transfer_here()
                        .map_err(|e| VeloError::Other(audio_message(&e)))?;
                    wait_for_session(state).await?;
                }
                engine.play()
            }
            SourceAction::Pause => engine.pause(),
            SourceAction::Next => engine.next(),
            SourceAction::Previous => engine.previous(),
            SourceAction::Seek(ms) => engine.seek(ms),
            SourceAction::Volume(v) => engine.set_volume(v),
            SourceAction::Shuffle(on) => engine.set_shuffle(on),
            SourceAction::Repeat(mode) => engine.set_repeat(mode),
        };
        result.map_err(|e| VeloError::Other(audio_message(&e)))
    }

    /// Nothing to re-read: librespot pushes every change.
    fn refresh(&self, _why: Refresh) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use velo_domain::RepeatMode;
    use velo_player::EngineTrack;

    #[tokio::test]
    async fn resuming_waits_for_transfer_activation_and_a_track() {
        let (tx, rx) = watch::channel(SourceState::default());
        let waiter = tokio::spawn(wait_for_session(rx));
        tx.send_modify(|s| s.active = true);
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        tx.send_replace(state_of(
            &EngineState {
                active: true,
                track: Some(EngineTrack {
                    uri: "spotify:track:x".into(),
                    title: "T".into(),
                    artist: "A".into(),
                    album: "B".into(),
                    duration_ms: 1000,
                    cover_url: None,
                }),
                ..EngineState::new(50, Instant::now())
            },
            "Velo",
        ));
        waiter.await.unwrap().unwrap();
    }

    #[test]
    fn livi_metadata_and_timeline_replace_the_song_then_return_without_losing_it() {
        let at = Instant::now();
        let mut state = EngineState {
            active: true,
            playing: false,
            track: Some(EngineTrack {
                uri: "spotify:track:x".into(),
                title: "Canción".into(),
                artist: "Artista".into(),
                album: "Álbum".into(),
                duration_ms: 200_000,
                cover_url: Some("https://i.scdn.co/image/song".into()),
            }),
            ..Default::default()
        };
        state.dj.active = true;
        state.dj.speech = Some(velo_player::DjSpeech {
            title: String::new(),
            artist: "Otro dato del servidor".into(),
            album: "DJ".into(),
            image_url: Some("https://i.scdn.co/image/livi".into()),
            line: "Hola".into(),
            duration_ms: 5000,
            position_ms: 1000,
            position_at: at,
            playing: true,
        });
        let voice = state_of(&state, "Velo");
        assert!(voice.is_playing && voice.controllable);
        let track = voice.track.unwrap();
        assert_eq!(
            (track.title.as_str(), track.artist.as_str()),
            ("DJ", "DJ Livi")
        );
        assert!(track.uri.is_none() && track.thumbnail.is_some());
        assert_eq!(
            track.cover_url.as_deref(),
            Some("https://i.scdn.co/image/livi")
        );
        assert_eq!(
            (voice.position_ms, voice.position_at, track.duration_ms),
            (1000, at, 5000)
        );
        state.dj.speech.as_mut().unwrap().playing = false;
        assert!(!state_of(&state, "Velo").is_playing);
        state.dj.speech = None;
        state.playing = true;
        let song = state_of(&state, "Velo");
        assert!(song.is_playing);
        assert_eq!(song.track.unwrap().title, "Canción");
    }

    #[test]
    fn engine_state_maps_to_a_local_spirc_controlled_source() {
        let at = Instant::now();
        let mut s = EngineState::new(64, at);
        let idle = state_of(&s, "Velo");
        assert!(!idle.active && !idle.controllable && !idle.is_playing);
        assert!(idle.own_engine && !idle.remote && idle.pid.is_none());

        s.active = true;
        s.playing = true;
        s.position_ms = 42_000;
        s.shuffle = true;
        s.repeat = RepeatMode::Track;
        s.track = Some(EngineTrack {
            uri: "spotify:track:x".into(),
            title: "Marea".into(),
            artist: "Lumen Fiel".into(),
            album: "Atlas".into(),
            duration_ms: 200_000,
            cover_url: Some("https://i.scdn.co/image/x".into()),
        });
        let st = state_of(&s, "Velo");
        assert!(st.active && st.controllable && st.is_playing);
        assert_eq!(st.device_name, "Velo");
        assert_eq!((st.position_ms, st.position_at), (42_000, at));
        assert_eq!(st.volume, Some(64));
        assert!(st.shuffle);
        assert_eq!(st.repeat, RepeatMode::Track);
        let t = st.track.unwrap();
        assert_eq!(t.uri.as_deref(), Some("spotify:track:x"));
        assert_eq!(t.cover_url.as_deref(), Some("https://i.scdn.co/image/x"));
        assert_eq!(t.duration_ms, 200_000);

        // Another device took over: the last track stays, but nothing plays here.
        s.active = false;
        let st = state_of(&s, "Velo");
        assert!(!st.active && !st.is_playing && !st.controllable);
    }
}
