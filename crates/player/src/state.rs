//! Engine state as librespot pushes it: a pure reducer over `PlayerEvent`s (no polling, no I/O).

use std::time::Instant;

use librespot_metadata::audio::item::{AudioItem, UniqueFields};
use librespot_playback::player::PlayerEvent;
use velo_domain::RepeatMode;

/// The song the engine is playing (or has loaded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineTrack {
    pub uri: String,
    pub title: String,
    /// Artists joined with ", " (show name for podcast episodes).
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    /// Remote cover (i.scdn.co), the largest one up to [`COVER_MAX_PX`].
    pub cover_url: Option<String>,
}

/// Covers above this width are skipped: the app downscales to 600 px anyway.
pub const COVER_MAX_PX: i32 = 640;

/// Latest engine state. The position is a measurement: `position_ms` at `position_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineState {
    /// This Connect device ("Velo") is the active one of the account.
    pub active: bool,
    /// Playback request identity; unchanged by seek, pause and volume.
    pub occurrence: u64,
    pub track: Option<EngineTrack>,
    pub playing: bool,
    pub position_ms: u64,
    pub position_at: Instant,
    /// 0–100.
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Bumped each time the audio output could not be opened or written.
    pub audio_errors: u32,
    /// Bumped each time Spotify refused an upcoming track while preloading it (Spirc drops it
    /// from the queue). Failures of the song being played go to `failures`.
    pub unavailable: u32,
    /// Bumped each time the song being played failed (Spirc already skipped to the next one);
    /// `last_failure` says which one and whether a retry makes sense.
    pub failures: u32,
    pub last_failure: Option<TrackFailure>,
    /// The song librespot started loading last, and when (for [`TrackFailure::transient`]).
    pub loading: Option<(String, Instant)>,
    /// E5b: DJ Livi en este motor (voz que suena, sesión activa).
    pub dj: crate::dj::DjState,
    /// Contexto DJ resuelto en esta sesión; false mientras es desconocido.
    pub dj_available: bool,
    /// Errores del controlador, para el canal existente de errores de la app.
    pub dj_errors: u32,
    pub dj_error: Option<crate::dj::DjError>,
}

/// A song that could not be played to its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackFailure {
    pub uri: String,
    /// Where it stopped: 0 when it never started.
    pub position_ms: u64,
    /// Likely temporary: the load waited for Spotify longer than its 1.5 s audio-key timeout
    /// ("Audio key response timeout" → decoding without key fails), or the stream broke in the
    /// middle of the song. A refusal that comes back at once (region, removed) is not.
    pub transient: bool,
}

/// A load that fails after this long waited on Spotify (the audio key times out at 1.5 s).
pub const SLOW_LOAD: std::time::Duration = std::time::Duration::from_millis(1400);
/// An end of track this far before the song's duration is a broken stream, not the end.
const END_TOLERANCE_MS: u64 = 5_000;

impl EngineState {
    pub fn new(volume: u8, now: Instant) -> Self {
        Self {
            active: false,
            occurrence: 0,
            track: None,
            playing: false,
            position_ms: 0,
            position_at: now,
            volume: volume.min(100),
            shuffle: false,
            repeat: RepeatMode::Off,
            audio_errors: 0,
            unavailable: 0,
            failures: 0,
            last_failure: None,
            loading: None,
            dj: crate::dj::DjState::default(),
            dj_available: false,
            dj_errors: 0,
            dj_error: None,
        }
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new(50, Instant::now())
    }
}

/// librespot volume (0..=65535) to percent, rounded.
pub fn volume_to_percent(volume: u16) -> u8 {
    ((u32::from(volume) * 100 + 32_767) / 65_535) as u8
}

/// Percent (clamped to 100) to librespot volume.
pub fn percent_to_volume(percent: u8) -> u16 {
    (u32::from(percent.min(100)) * 65_535 / 100) as u16
}

/// Spotify's two repeat switches to Velo's three-state mode.
pub fn repeat_mode(context: bool, track: bool) -> RepeatMode {
    if track {
        RepeatMode::Track
    } else if context {
        RepeatMode::Context
    } else {
        RepeatMode::Off
    }
}

/// The Spirc switches for a mode: `(repeat_context, repeat_track)`. Track repeat keeps the
/// context switch as it is (Spotify's own cycle goes off → context → track).
pub fn repeat_switches(mode: RepeatMode) -> (Option<bool>, bool) {
    match mode {
        RepeatMode::Off => (Some(false), false),
        RepeatMode::Context => (Some(true), false),
        RepeatMode::Track => (None, true),
    }
}

pub(crate) fn track_of(item: &AudioItem) -> EngineTrack {
    let (artist, album) = match &item.unique_fields {
        UniqueFields::Track { artists, album, .. } => (
            artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            album.clone(),
        ),
        UniqueFields::Episode { show_name, .. } => (show_name.clone(), show_name.clone()),
        UniqueFields::Local { artists, album, .. } => (
            artists.clone().unwrap_or_default(),
            album.clone().unwrap_or_default(),
        ),
    };
    let cover_url = item
        .covers
        .iter()
        .filter(|c| c.width <= COVER_MAX_PX)
        .max_by_key(|c| c.width)
        .or_else(|| item.covers.first())
        .map(|c| c.url.clone());
    EngineTrack {
        uri: item.uri.clone(),
        title: item.name.clone(),
        artist,
        album,
        duration_ms: u64::from(item.duration_ms),
        cover_url,
    }
}

/// Applies one librespot event; returns whether anything visible changed.
pub fn apply_event(state: &mut EngineState, event: &PlayerEvent, now: Instant) -> bool {
    let before = state.clone();
    match event {
        PlayerEvent::SessionConnected { .. } => state.active = true,
        PlayerEvent::SessionDisconnected { .. } => {
            state.active = false;
            state.playing = false;
        }
        PlayerEvent::TrackChanged { audio_item } => {
            let track = track_of(audio_item);
            if state.track.as_ref().is_none_or(|t| t.uri != track.uri) {
                state.position_ms = 0;
                state.position_at = now;
            }
            state.track = Some(track);
        }
        PlayerEvent::Playing { position_ms, .. } => {
            state.loading = None;
            state.playing = true;
            state.position_ms = u64::from(*position_ms);
            state.position_at = now;
        }
        PlayerEvent::Paused { position_ms, .. } => {
            state.loading = None;
            state.playing = false;
            state.position_ms = u64::from(*position_ms);
            state.position_at = now;
        }
        PlayerEvent::Loading {
            play_request_id,
            position_ms,
            track_id,
            ..
        } => {
            state.position_ms = u64::from(*position_ms);
            state.position_at = now;
            state.occurrence = *play_request_id;
            state.loading = track_id.to_uri().ok().map(|uri| (uri, now));
        }
        PlayerEvent::Seeked { position_ms, .. }
        | PlayerEvent::PositionCorrection { position_ms, .. }
        | PlayerEvent::PositionChanged { position_ms, .. } => {
            state.position_ms = u64::from(*position_ms);
            state.position_at = now;
        }
        PlayerEvent::Stopped { .. } => {
            state.loading = None;
            state.playing = false;
            state.position_ms = 0;
            state.position_at = now;
        }
        PlayerEvent::Unavailable { track_id, .. } => match track_id.to_uri().ok() {
            // La clave puede fallar antes de TrackChanged: Loading identifica la pista pedida.
            Some(uri)
                if current_uri(state) == Some(uri.as_str())
                    || state.loading.as_ref().is_some_and(|(u, _)| *u == uri) =>
            {
                let transient = state.loading.as_ref().is_some_and(|(u, at)| {
                    *u == uri && now.saturating_duration_since(*at) >= SLOW_LOAD
                });
                state.loading = None;
                fail(state, uri, 0, transient);
            }
            _ => state.unavailable = state.unavailable.wrapping_add(1),
        },
        PlayerEvent::EndOfTrack { track_id, .. } => {
            let uri = track_id.to_uri().ok();
            if let Some(t) = state
                .track
                .as_ref()
                .filter(|t| Some(&t.uri) == uri.as_ref())
            {
                let position = crate::queue::position_now(state, now);
                if t.duration_ms > END_TOLERANCE_MS
                    && position.saturating_add(END_TOLERANCE_MS) < t.duration_ms
                {
                    let uri = t.uri.clone();
                    fail(state, uri, position, true);
                }
            }
        }
        PlayerEvent::VolumeChanged { volume } => state.volume = volume_to_percent(*volume),
        PlayerEvent::ShuffleChanged { shuffle } => state.shuffle = *shuffle,
        PlayerEvent::RepeatChanged { context, track } => {
            state.repeat = repeat_mode(*context, *track)
        }
        _ => return false,
    }
    *state != before
}

fn current_uri(state: &EngineState) -> Option<&str> {
    state.track.as_ref().map(|t| t.uri.as_str())
}

fn fail(state: &mut EngineState, uri: String, position_ms: u64, transient: bool) {
    state.failures = state.failures.wrapping_add(1);
    state.last_failure = Some(TrackFailure {
        uri,
        position_ms,
        transient,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_track_uri;
    use librespot_metadata::{
        artist::{ArtistRole, ArtistWithRole, ArtistsWithRole},
        audio::item::CoverImage,
        image::ImageSize,
    };
    use std::time::Duration;

    const URI: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    #[test]
    fn loading_failure_before_metadata_is_retried_even_without_a_current_track() {
        let now = Instant::now();
        let id = parse_track_uri(URI).unwrap();
        for previous in [None, Some("spotify:track:0000000000000000000001")] {
            let mut s = EngineState::new(50, now);
            if let Some(previous) = previous {
                s.track = Some(track_of(&item(previous, &[])));
            }
            // Orden real de librespot: la clave falla antes de start_playback/TrackChanged.
            apply_event(
                &mut s,
                &PlayerEvent::Loading {
                    play_request_id: 2,
                    track_id: id.clone(),
                    position_ms: 0,
                },
                now,
            );
            apply_event(
                &mut s,
                &PlayerEvent::Unavailable {
                    play_request_id: 2,
                    track_id: id.clone(),
                },
                now + Duration::from_millis(1600),
            );
            assert_eq!((s.failures, s.unavailable), (1, 0));
            assert_eq!(s.last_failure.as_ref().unwrap().uri, URI);
            assert!(s.last_failure.as_ref().unwrap().transient);
            assert!(s.loading.is_none());
        }
    }

    #[test]
    fn successful_playback_clears_loading_and_does_not_retry_a_later_preload() {
        let now = Instant::now();
        let id = parse_track_uri(URI).unwrap();
        let mut s = EngineState::new(50, now);
        apply_event(
            &mut s,
            &PlayerEvent::Loading {
                play_request_id: 1,
                track_id: id.clone(),
                position_ms: 0,
            },
            now,
        );
        apply_event(
            &mut s,
            &PlayerEvent::Playing {
                play_request_id: 1,
                track_id: id.clone(),
                position_ms: 0,
            },
            now + Duration::from_secs(2),
        );
        assert!(s.loading.is_none());
        s.track = Some(track_of(&item("spotify:track:0000000000000000000001", &[])));
        apply_event(
            &mut s,
            &PlayerEvent::Unavailable {
                play_request_id: 3,
                track_id: id,
            },
            now + Duration::from_secs(3),
        );
        assert_eq!((s.failures, s.unavailable), (0, 1));
    }

    #[test]
    fn slow_load_failures_are_transient_quick_refusals_and_preloads_are_not() {
        let now = Instant::now();
        let id = parse_track_uri(URI).unwrap();
        let mut s = EngineState::new(50, now);
        let loading = PlayerEvent::Loading {
            play_request_id: 1,
            track_id: id.clone(),
            position_ms: 0,
        };
        apply_event(&mut s, &loading, now);
        apply_event(
            &mut s,
            &PlayerEvent::TrackChanged {
                audio_item: item(URI, &[]),
            },
            now,
        );
        let unavailable = PlayerEvent::Unavailable {
            play_request_id: 1,
            track_id: id.clone(),
        };
        // Refused at once (region, removed): reported, never retried.
        apply_event(&mut s, &unavailable, now + Duration::from_millis(200));
        assert_eq!((s.failures, s.unavailable), (1, 0));
        assert!(!s.last_failure.as_ref().unwrap().transient);
        // After the 1.5 s audio-key timeout: worth retrying.
        apply_event(&mut s, &loading, now);
        apply_event(&mut s, &unavailable, now + Duration::from_millis(1600));
        assert_eq!(s.failures, 2);
        assert_eq!(
            s.last_failure,
            Some(TrackFailure {
                uri: URI.into(),
                position_ms: 0,
                transient: true
            })
        );
        // An upcoming song failing its preload is only counted.
        let other = parse_track_uri("spotify:track:0000000000000000000001").unwrap();
        apply_event(
            &mut s,
            &PlayerEvent::Unavailable {
                play_request_id: 1,
                track_id: other,
            },
            now,
        );
        assert_eq!((s.failures, s.unavailable), (2, 1));
    }

    #[test]
    fn premature_end_is_a_transient_failure_at_its_position_but_a_normal_end_is_not() {
        let now = Instant::now();
        let id = parse_track_uri(URI).unwrap();
        let mut s = EngineState::new(50, now);
        apply_event(
            &mut s,
            &PlayerEvent::TrackChanged {
                audio_item: item(URI, &[]),
            },
            now,
        );
        let end = PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: id.clone(),
        };
        s.position_ms = 30_000;
        apply_event(&mut s, &end, now);
        assert_eq!(s.failures, 1);
        assert_eq!(
            s.last_failure,
            Some(TrackFailure {
                uri: URI.into(),
                position_ms: 30_000,
                transient: true
            })
        );
        // Natural end (duration 123 s), by measurement or by elapsed playing time.
        s.position_ms = 120_000;
        apply_event(&mut s, &end, now);
        s.playing = true;
        s.position_ms = 0;
        apply_event(&mut s, &end, now + Duration::from_secs(123));
        assert_eq!(s.failures, 1);
        // An end event for another song (stale) is ignored.
        s.track.as_mut().unwrap().uri = "spotify:track:other".into();
        apply_event(&mut s, &end, now);
        assert_eq!(s.failures, 1);
    }

    fn item(uri: &str, covers: &[(i32, &str)]) -> Box<AudioItem> {
        let id = parse_track_uri(uri).unwrap();
        Box::new(AudioItem {
            track_id: id.clone(),
            uri: uri.into(),
            files: Default::default(),
            name: "Fixture song".into(),
            covers: covers
                .iter()
                .map(|(w, url)| CoverImage {
                    url: (*url).into(),
                    size: ImageSize::DEFAULT,
                    width: *w,
                    height: *w,
                })
                .collect(),
            language: vec![],
            duration_ms: 123_000,
            is_explicit: false,
            availability: Ok(()),
            alternatives: None,
            unique_fields: UniqueFields::Track {
                artists: ArtistsWithRole(
                    ["Lumen Fiel", "Iria Sol"]
                        .iter()
                        .map(|name| ArtistWithRole {
                            id: id.clone(),
                            name: (*name).into(),
                            role: ArtistRole::ARTIST_ROLE_MAIN_ARTIST,
                        })
                        .collect(),
                ),
                album: "Atlas de agua".into(),
                album_artists: vec![],
                popularity: 0,
                number: 1,
                disc_number: 1,
            },
        })
    }

    #[test]
    fn track_metadata_joins_artists_and_picks_a_cover_up_to_640() {
        let t = track_of(&item(
            URI,
            &[(64, "s"), (640, "l"), (1280, "xl"), (300, "m")],
        ));
        assert_eq!(t.uri, URI);
        assert_eq!(t.title, "Fixture song");
        assert_eq!(t.artist, "Lumen Fiel, Iria Sol");
        assert_eq!(t.album, "Atlas de agua");
        assert_eq!(t.duration_ms, 123_000);
        assert_eq!(t.cover_url.as_deref(), Some("l"));
        // Only oversized covers: still better than none.
        assert_eq!(
            track_of(&item(URI, &[(1280, "xl")])).cover_url.as_deref(),
            Some("xl")
        );
        assert_eq!(track_of(&item(URI, &[])).cover_url, None);
    }

    #[test]
    fn transport_events_anchor_the_position_measurement() {
        let t0 = Instant::now();
        let mut s = EngineState::new(50, t0);
        let id = parse_track_uri(URI).unwrap();
        assert!(apply_event(
            &mut s,
            &PlayerEvent::SessionConnected {
                connection_id: "c".into(),
                user_name: "u".into()
            },
            t0
        ));
        assert!(s.active);
        assert!(apply_event(
            &mut s,
            &PlayerEvent::TrackChanged {
                audio_item: item(URI, &[])
            },
            t0
        ));
        let t1 = t0 + Duration::from_secs(1);
        assert!(apply_event(
            &mut s,
            &PlayerEvent::Playing {
                play_request_id: 1,
                track_id: id.clone(),
                position_ms: 1_500
            },
            t1
        ));
        assert!(s.playing);
        assert_eq!((s.position_ms, s.position_at), (1_500, t1));
        let t2 = t1 + Duration::from_secs(2);
        apply_event(
            &mut s,
            &PlayerEvent::Seeked {
                play_request_id: 1,
                track_id: id.clone(),
                position_ms: 60_000,
            },
            t2,
        );
        assert_eq!(
            (s.position_ms, s.position_at, s.playing),
            (60_000, t2, true)
        );
        apply_event(
            &mut s,
            &PlayerEvent::Paused {
                play_request_id: 1,
                track_id: id.clone(),
                position_ms: 61_000,
            },
            t2,
        );
        assert!(!s.playing);
        assert_eq!(s.position_ms, 61_000);
        // Same track metadata again (gapless preload): position untouched.
        apply_event(
            &mut s,
            &PlayerEvent::TrackChanged {
                audio_item: item(URI, &[]),
            },
            t2,
        );
        assert_eq!(s.position_ms, 61_000);
        apply_event(
            &mut s,
            &PlayerEvent::SessionDisconnected {
                connection_id: "c".into(),
                user_name: "u".into(),
            },
            t2,
        );
        assert!(!s.active && !s.playing);
    }

    #[test]
    fn connect_switches_and_volume_are_mirrored() {
        let mut s = EngineState::default();
        assert!(apply_event(
            &mut s,
            &PlayerEvent::ShuffleChanged { shuffle: true },
            Instant::now()
        ));
        assert!(s.shuffle);
        apply_event(
            &mut s,
            &PlayerEvent::RepeatChanged {
                context: true,
                track: false,
            },
            Instant::now(),
        );
        assert_eq!(s.repeat, RepeatMode::Context);
        apply_event(
            &mut s,
            &PlayerEvent::VolumeChanged { volume: 65_535 },
            Instant::now(),
        );
        assert_eq!(s.volume, 100);
        // Unchanged values and events Velo ignores report no change.
        assert!(!apply_event(
            &mut s,
            &PlayerEvent::VolumeChanged { volume: 65_535 },
            Instant::now()
        ));
        assert!(!apply_event(
            &mut s,
            &PlayerEvent::AutoPlayChanged { auto_play: true },
            Instant::now()
        ));
        assert!(!apply_event(
            &mut s,
            &PlayerEvent::PlayRequestIdChanged { play_request_id: 9 },
            Instant::now()
        ));
    }

    #[test]
    fn repeat_and_volume_conversions_round_trip() {
        for (context, track, mode) in [
            (false, false, RepeatMode::Off),
            (true, false, RepeatMode::Context),
            (true, true, RepeatMode::Track),
            (false, true, RepeatMode::Track),
        ] {
            assert_eq!(repeat_mode(context, track), mode);
        }
        assert_eq!(repeat_switches(RepeatMode::Off), (Some(false), false));
        assert_eq!(repeat_switches(RepeatMode::Context), (Some(true), false));
        assert_eq!(repeat_switches(RepeatMode::Track), (None, true));
        for p in 0..=100u8 {
            assert_eq!(volume_to_percent(percent_to_volume(p)), p);
        }
        assert_eq!(percent_to_volume(200), 65_535);
        assert_eq!(volume_to_percent(32_768), 50);
    }
}
