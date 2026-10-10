use velo_domain::NowPlayingEvent;

pub(crate) fn spotify_session(aumid: &str) -> bool {
    aumid.to_ascii_lowercase().contains("spotify")
}
pub(crate) fn same_track(previous: Option<&NowPlayingEvent>, next: &NowPlayingEvent) -> bool {
    matches!(next, NowPlayingEvent::TrackChanged { .. }) && previous == Some(next)
}
/// All inputs are WinRT 100 ns ticks; DateTimes share the Windows epoch.
pub(crate) fn position_ms(position: i64, updated: i64, now: i64, playing: bool) -> u64 {
    let elapsed = if playing && updated > 0 {
        now.saturating_sub(updated).max(0)
    } else {
        0
    };
    (position.max(0).saturating_add(elapsed) / 10_000) as u64
}
pub(crate) fn duration_ms(start: i64, end: i64) -> Option<u64> {
    let ticks = end.saturating_sub(start);
    (ticks > 0).then_some((ticks.max(0) / 10_000) as u64)
}
pub(crate) fn root_pid(processes: &[(u32, u32, String)]) -> Option<u32> {
    let spotify: Vec<_> = processes
        .iter()
        .filter(|(_, _, name)| spotify_process(name))
        .collect();
    spotify
        .iter()
        .filter(|(_, parent, _)| !spotify.iter().any(|(pid, _, _)| pid == parent))
        .map(|(pid, _, _)| *pid)
        .min()
}
pub(crate) fn spotify_process(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("spotify")
}
pub(crate) fn refreshed_pid(current: Option<u32>, processes: &[(u32, u32, String)]) -> Option<u32> {
    current
        .filter(|pid| {
            processes
                .iter()
                .any(|(id, _, name)| id == pid && spotify_process(name))
        })
        .or_else(|| root_pid(processes))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;
    #[test]
    fn snapshot_is_ordered_and_retains_latest_position() {
        let mut state = State::default();
        let session = NowPlayingEvent::SessionStarted { pid: Some(42) };
        let track = track("title", "artist", "album");
        let playback = NowPlayingEvent::PlaybackChanged { is_playing: true };
        let position = NowPlayingEvent::Position {
            position_ms: 7000,
            sampled_at: std::time::Instant::now(),
        };
        state.update(session.clone());
        state.update(playback.clone());
        state.update(track.clone());
        state.update(NowPlayingEvent::Position {
            position_ms: 0,
            sampled_at: std::time::Instant::now(),
        });
        state.update(position.clone());
        assert_eq!(state.current(), vec![session, track, playback, position]);
    }
    fn track(title: &str, artist: &str, album: &str) -> NowPlayingEvent {
        NowPlayingEvent::TrackChanged {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration_ms: None,
            thumbnail: None,
        }
    }
    #[test]
    fn session_selection() {
        assert!(spotify_session("Spotify.exe"));
        assert!(spotify_session("package.SPOTIFY_music!App"));
        assert!(!spotify_session("OtherPlayer.exe"));
        assert!(!spotify_session(""));
    }
    #[test]
    fn track_dedupe_uses_all_three_fields() {
        let mut state = State::default();
        assert!(state.update(track("a", "b", "c")));
        assert!(!state.update(track("a", "b", "c")));
        let mut changed_art = track("a", "b", "c");
        // Spotify can supply art and duration in separate, later events.
        for (art, duration) in [
            (Some(vec![1]), None),
            (Some(vec![1]), Some(100)),
            (Some(vec![2]), Some(100)),
            (Some(vec![2]), Some(200)),
            (None, None),
        ] {
            if let NowPlayingEvent::TrackChanged {
                thumbnail,
                duration_ms,
                ..
            } = &mut changed_art
            {
                *thumbnail = art;
                *duration_ms = duration;
            }
            assert!(state.update(changed_art.clone()));
            assert!(!state.update(changed_art.clone()));
            assert_eq!(state.track, Some(changed_art.clone()));
        }
        assert!(state.update(track("x", "b", "c")));
        assert!(state.update(track("x", "y", "c")));
        assert!(state.update(track("x", "y", "z")));
    }
    #[test]
    fn playback_dedupe_and_session_reset() {
        let mut state = State::default();
        assert!(state.update(NowPlayingEvent::PlaybackChanged { is_playing: false }));
        assert!(!state.update(NowPlayingEvent::PlaybackChanged { is_playing: false }));
        assert!(state.update(NowPlayingEvent::PlaybackChanged { is_playing: true }));
        state.update(track("a", "b", "c"));
        state.update(NowPlayingEvent::SessionEnded);
        assert_eq!(state.current(), vec![NowPlayingEvent::SessionEnded]);
        state.update(NowPlayingEvent::SessionStarted { pid: Some(1) });
        assert!(state.update(track("a", "b", "c")));
        assert!(state.update(NowPlayingEvent::PlaybackChanged { is_playing: true }));
    }
    #[test]
    fn position_corrects_only_while_playing() {
        assert_eq!(
            position_ms(50_000_000, 100_000_000, 120_000_000, true),
            7000
        );
        assert_eq!(
            position_ms(50_000_000, 100_000_000, 120_000_000, false),
            5000
        );
        assert_eq!(
            position_ms(50_000_000, 130_000_000, 120_000_000, true),
            5000
        );
        assert_eq!(position_ms(-1, 0, 120_000_000, true), 0);
        assert_eq!(
            position_ms(i64::MAX, 1, i64::MAX, true),
            (i64::MAX / 10_000) as u64
        );
    }
    #[test]
    fn duration_requires_positive_span() {
        assert_eq!(duration_ms(10_000, 30_000), Some(2));
        assert_eq!(duration_ms(30_000, 10_000), None);
        assert_eq!(duration_ms(0, 0), None);
    }
    #[test]
    fn pid_picks_root_not_first_child() {
        let processes = vec![
            (2, 1, "SpotifyHelper.exe".into()),
            (8, 0, "other.exe".into()),
            (1, 8, "SPOTIFYStore.exe".into()),
            (3, 2, "Spotify.exe".into()),
        ];
        assert_eq!(root_pid(&processes), Some(1));
        assert_eq!(root_pid(&[]), None);
        assert_eq!(root_pid(&[(9, 0, "NotSpotify.exe".into())]), None);
        assert_eq!(root_pid(&[(1, 1, "Spotify.exe".into())]), None);
        assert_eq!(
            root_pid(&[(5, 99, "Spotify.exe".into()), (4, 99, "Spotify.exe".into())]),
            Some(4)
        );
    }
    #[test]
    fn pid_recovers_when_unknown_or_dead_and_keeps_live_process() {
        let processes = [
            (2, 1, "SpotifyHelper.exe".into()),
            (1, 0, "SPOTIFY.exe".into()),
        ];
        assert_eq!(refreshed_pid(None, &processes), Some(1));
        assert_eq!(refreshed_pid(Some(99), &processes), Some(1));
        assert_eq!(refreshed_pid(Some(2), &processes), Some(2));
        assert_eq!(refreshed_pid(Some(1), &[]), None);
        assert_eq!(refreshed_pid(Some(1), &[(1, 0, "other.exe".into())]), None);
    }
    #[test]
    fn pid_correction_preserves_snapshot() {
        let mut state = State::default();
        state.update(NowPlayingEvent::SessionStarted { pid: None });
        let track = track("a", "b", "c");
        let playback = NowPlayingEvent::PlaybackChanged { is_playing: true };
        state.update(track.clone());
        state.update(playback.clone());
        let corrected = NowPlayingEvent::SessionStarted { pid: Some(42) };
        assert!(state.update(corrected.clone()));
        assert!(!state.update(corrected.clone()));
        assert_eq!(state.current(), vec![corrected, track, playback]);
    }
}
