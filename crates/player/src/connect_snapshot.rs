//! Read-only dealer observer. It adds a subscriber, never takes Spirc's command handler.
//! Cluster/next_tracks for the local device still require a live probe; no context shuffle.
use librespot_core::{Session, dealer::protocol::Message};
use librespot_protocol::connect::ClusterUpdate;
use std::{future::poll_fn, time::Duration};
use tokio::sync::watch;
use velo_domain::{ArtistRef, Track};

/// Shared live state of the existing pre-registration observer. Weak entries do not keep
/// sessions/tasks alive; replacing an engine replaces its channel and drops the old state.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterState {
    pub timestamp: i64,
    pub local_device_id: String,
    pub devices: Vec<velo_domain::Device>,
    pub playback: Option<velo_domain::PlaybackState>,
    pub queue: Option<Snapshot>,
}
type ClusterChannel = watch::Sender<Option<ClusterState>>;
fn channels()
-> &'static std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<ClusterChannel>>> {
    static CHANNELS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<ClusterChannel>>>,
    > = std::sync::OnceLock::new();
    CHANNELS.get_or_init(Default::default)
}
pub(crate) fn register(session: &Session, channel: &std::sync::Arc<ClusterChannel>) {
    let mut entries = channels().lock().unwrap_or_else(|e| e.into_inner());
    entries.retain(|_, value| value.strong_count() > 0);
    entries.insert(
        session.device_id().to_owned(),
        std::sync::Arc::downgrade(channel),
    );
}
pub fn cluster_updates(session: &Session) -> Option<watch::Receiver<Option<ClusterState>>> {
    if session.is_invalid() {
        return None;
    }
    channels()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session.device_id())?
        .upgrade()
        .map(|tx| tx.subscribe())
}
pub fn cluster_state(session: &Session) -> Option<ClusterState> {
    cluster_updates(session)?.borrow().clone()
}

pub(crate) fn publish_cluster(tx: &ClusterChannel, update: &ClusterUpdate, local_id: &str) {
    let Some(cluster) = update.cluster.as_ref() else {
        return;
    };
    tx.send_if_modified(|old| {
        if old
            .as_ref()
            .is_some_and(|s| s.timestamp > cluster.changed_timestamp_ms)
        {
            return false;
        }
        let mut devices = crate::connect_devices::cluster_devices(update).unwrap_or_default();
        group_devices(&mut devices, Some(local_id));
        let mut playback = playback_state(update, std::time::SystemTime::now());
        let mut queue = decode(update, &cluster.active_device_id);
        if let Some(previous) = old.as_ref() {
            let same_device = previous
                .playback
                .as_ref()
                .and_then(|p| p.device.as_ref())
                .and_then(|d| d.id.as_deref())
                == Some(cluster.active_device_id.as_str());
            let same_track = cluster
                .player_state
                .as_ref()
                .and_then(|p| p.track.as_ref())
                .is_none_or(|t| {
                    previous
                        .playback
                        .as_ref()
                        .and_then(|p| p.track.as_ref())
                        .is_some_and(|p| p.uri == t.uri)
                });
            if playback.is_none() && same_device && same_track {
                playback = previous.playback.clone();
                if let Some(p) = playback.as_mut() {
                    p.device = devices.iter().find(|d| d.is_active).cloned();
                }
            }
            if queue.is_none() {
                queue = reconcile(previous.queue.as_ref(), update, &cluster.active_device_id);
            }
        }
        let next = ClusterState {
            timestamp: cluster.changed_timestamp_ms,
            local_device_id: local_id.to_owned(),
            devices,
            playback,
            queue,
        };
        if old.as_ref() == Some(&next) {
            return false;
        }
        *old = Some(next);
        true
    });
}

/// Active endpoint, this computer, then the remaining cluster endpoints; stable ties by ID.
pub fn group_devices(rows: &mut [velo_domain::Device], local_id: Option<&str>) {
    rows.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| {
                (b.id.as_deref().is_some() && b.id.as_deref() == local_id)
                    .cmp(&(a.id.as_deref().is_some() && a.id.as_deref() == local_id))
            })
            .then_with(|| b.id.is_some().cmp(&a.id.is_some()))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Timestamp is Spotify's Unix milliseconds. Rebase once onto Instant, then interpolate
/// with the existing monotonic position clock (never advance paused/buffering playback).
pub fn playback_state(
    update: &ClusterUpdate,
    now: std::time::SystemTime,
) -> Option<velo_domain::PlaybackState> {
    use velo_domain::{PlaybackState, RepeatMode};
    let cluster = update.cluster.as_ref()?;
    if cluster.need_full_player_state || cluster.active_device_id.is_empty() {
        return None;
    }
    let state = cluster.player_state.as_ref()?;
    let device = crate::connect_devices::cluster_devices(update)?
        .into_iter()
        .find(|d| d.is_active)?;
    let playing = state.is_playing && !state.is_paused && !state.is_buffering;
    let now_ms = now
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .min(i64::MAX as u128) as i64;
    let elapsed = if playing && state.timestamp > 0 {
        now_ms.saturating_sub(state.timestamp).max(0) as u64
    } else {
        0
    };
    let duration = state.duration.max(0) as u64;
    let progress = (state.position_as_of_timestamp.max(0) as u64).saturating_add(elapsed);
    let options = state.options.as_ref();
    Some(PlaybackState {
        track: state
            .track
            .as_ref()
            .filter(|t| playable(&t.uri))
            .map(|t| provided_track(t, duration)),
        is_playing: playing,
        progress_ms: if duration > 0 {
            progress.min(duration)
        } else {
            progress
        },
        sampled_at: std::time::Instant::now(),
        shuffle: options.is_some_and(|o| o.shuffling_context),
        repeat: if options.is_some_and(|o| o.repeating_track) {
            RepeatMode::Track
        } else if options.is_some_and(|o| o.repeating_context) {
            RepeatMode::Context
        } else {
            RepeatMode::Off
        },
        device: Some(device),
        context_uri: (!state.context_uri.is_empty()).then(|| state.context_uri.clone()),
    })
}
fn provided_track(t: &librespot_protocol::player::ProvidedTrack, duration_ms: u64) -> Track {
    let metadata = |key: &str| t.metadata.get(key).cloned().unwrap_or_default();
    Track {
        id: t.uri.rsplit(':').next().map(str::to_owned),
        uri: t.uri.clone(),
        title: metadata("title"),
        artists: if metadata("artist_name").is_empty() {
            Vec::new()
        } else {
            vec![ArtistRef {
                id: t
                    .artist_uri
                    .strip_prefix("spotify:artist:")
                    .map(str::to_owned),
                name: metadata("artist_name"),
            }]
        },
        album: metadata("album_title"),
        album_id: t
            .album_uri
            .strip_prefix("spotify:album:")
            .map(str::to_owned),
        duration_ms,
        explicit: metadata("is_explicit") == "true",
        images: Vec::new(),
        added_at: None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Occurrence {
    pub uri: String,
    pub uid: String,
    pub queued: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Snapshot {
    pub current: String,
    pub occurrence: String,
    pub context_uri: String,
    pub shuffle: bool,
    pub revision: String,
    pub timestamp: i64,
    pub tracks: Vec<Occurrence>,
}
impl Snapshot {
    pub fn same_queue(&self, other: &Self) -> bool {
        self.current == other.current
            && self.occurrence == other.occurrence
            && self.context_uri == other.context_uri
            && self.shuffle == other.shuffle
            && self.revision == other.revision
            && self.tracks == other.tracks
    }
    /// Context rows represented by the visible prefix. Manually queued duplicates must
    /// never consume matching occurrences from the original album/playlist tail.
    pub fn context_prefix(&self, visible: &[String]) -> Result<Vec<String>, crate::PlayerError> {
        if visible.len() > self.tracks.len()
            || self
                .tracks
                .iter()
                .zip(visible)
                .any(|(row, uri)| row.uri != *uri)
        {
            return Err(crate::PlayerError::QueueChanged);
        }
        Ok(self
            .tracks
            .iter()
            .take(visible.len())
            .filter(|row| !row.queued)
            .map(|row| row.uri.clone())
            .collect())
    }
}

pub fn decode(update: &ClusterUpdate, device_id: &str) -> Option<Snapshot> {
    let cluster = update.cluster.as_ref()?;
    if cluster.active_device_id != device_id || cluster.need_full_player_state {
        return None;
    }
    let state = cluster.player_state.as_ref()?;
    let track = state.track.as_ref()?;
    if !playable(&track.uri) || state.playback_id.is_empty() || state.options.is_none() {
        return None;
    }
    if state.next_tracks.len() > 10_000 || state.next_tracks.iter().any(|t| !playable(&t.uri)) {
        return None;
    }
    Some(Snapshot {
        current: track.uri.clone(),
        occurrence: state.playback_id.clone(),
        context_uri: state.context_uri.clone(),
        shuffle: state.options.as_ref()?.shuffling_context,
        revision: state.queue_revision.clone(),
        timestamp: cluster.changed_timestamp_ms,
        tracks: state
            .next_tracks
            .iter()
            .map(|t| Occurrence {
                uri: t.uri.clone(),
                uid: t.uid.clone(),
                queued: t.provider == "queue"
                    || t.metadata.get("is_queued").is_some_and(|v| v == "true"),
            })
            .collect(),
    })
}
fn playable(uri: &str) -> bool {
    librespot_core::SpotifyUri::from_uri(uri).is_ok()
        && (uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:"))
}

/// Metadata failure preserves the row and its position; it never turns a real queue empty.
pub async fn describe(session: &Session, snapshot: &Snapshot) -> Vec<Track> {
    let mut by_uri = std::collections::HashMap::new();
    let items = snapshot
        .tracks
        .iter()
        .take(MAX_ROWS)
        .filter(|t| by_uri.insert(t.uri.clone(), ()).is_none())
        .map(|t| crate::playlist::PlaylistItem {
            uri: t.uri.clone(),
            added_at: None,
        })
        .collect::<Vec<_>>();
    let metadata = crate::playlist::tracks_in(session, &items)
        .await
        .into_iter()
        .map(|t| (t.uri.clone(), t))
        .collect::<std::collections::HashMap<_, _>>();
    rows(snapshot, &metadata)
}

/// Metadata is only fetched for the first `MAX_ROWS` URIs, so only those rows are returned
/// (never empty placeholders for rows whose metadata was not requested).
const MAX_ROWS: usize = 50;
fn rows(snapshot: &Snapshot, metadata: &std::collections::HashMap<String, Track>) -> Vec<Track> {
    snapshot
        .tracks
        .iter()
        .take(MAX_ROWS)
        .map(|t| {
            metadata.get(&t.uri).cloned().unwrap_or_else(|| Track {
                id: t.uri.rsplit(':').next().map(str::to_owned),
                uri: t.uri.clone(),
                title: String::new(),
                artists: Vec::<ArtistRef>::new(),
                album: String::new(),
                album_id: None,
                duration_ms: 0,
                explicit: false,
                images: Vec::new(),
                added_at: None,
            })
        })
        .collect()
}

/// A cluster update that `decode` rejects keeps the previous good snapshot, unless the
/// current track or the active device changed (then the snapshot is really stale).
pub fn reconcile(
    old: Option<&Snapshot>,
    update: &ClusterUpdate,
    device_id: &str,
) -> Option<Snapshot> {
    if let Some(next) = decode(update, device_id) {
        return Some(next);
    }
    let old = old?;
    let cluster = update.cluster.as_ref()?;
    if cluster.active_device_id != device_id {
        return None;
    }
    if let Some(state) = cluster.player_state.as_ref() {
        if state.track.as_ref().is_some_and(|t| t.uri != old.current)
            || (!state.playback_id.is_empty() && state.playback_id != old.occurrence)
            || (!state.queue_revision.is_empty() && state.queue_revision != old.revision)
            || state
                .options
                .as_ref()
                .is_some_and(|o| o.shuffling_context != old.shuffle)
        {
            return None;
        }
    }
    let mut retained = old.clone();
    retained.timestamp = cluster.changed_timestamp_ms;
    Some(retained)
}

fn publish_snapshot(tx: &watch::Sender<Option<Snapshot>>, next: Option<Snapshot>) {
    tx.send_if_modified(|old| {
        if let (Some(old), Some(new)) = (old.as_ref(), next.as_ref()) {
            if new.timestamp < old.timestamp {
                return false;
            }
        }
        let same = match (old.as_ref(), next.as_ref()) {
            (Some(a), Some(b)) => a.same_queue(b),
            (None, None) => true,
            _ => false,
        };
        // Retain the newest watermark even if only transport/timestamp changed; no UI wake.
        *old = next;
        !same
    });
}

pub async fn observe(
    session: &Session,
    tx: &watch::Sender<Option<Snapshot>>,
) -> Result<(), librespot_core::Error> {
    // The engine's observer is already installed before registration: preserve its initial
    // local queue instead of waiting for another dealer update after audio becomes ready.
    if let Some(mut cluster) = cluster_updates(session) {
        loop {
            let next = cluster.borrow_and_update().as_ref().and_then(|s| {
                s.devices
                    .iter()
                    .any(|d| d.is_active && d.id.as_deref() == Some(session.device_id()))
                    .then(|| s.queue.clone())
                    .flatten()
            });
            publish_snapshot(tx, next);
            if cluster.changed().await.is_err() {
                break;
            }
        }
        tx.send_replace(None);
        return Ok(());
    }
    let mut stream = session.dealer().listen_for(
        "hm://connect-state/v1/cluster",
        Message::from_raw::<ClusterUpdate>,
    )?;
    while let Some(update) = poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
        let Ok(update) = update else { continue };
        let next = reconcile(tx.borrow().as_ref(), &update, session.device_id());
        publish_snapshot(tx, next);
    }
    tx.send_replace(None);
    Ok(())
}

/// Bounded live probe for the existing queue_probe example's `shuffle` branch.
/// Prints only structure/Spotify URIs, never headers, credentials, device/account IDs or audio.
pub async fn shuffle_probe(engine: &crate::Engine) -> Result<(), crate::PlayerError> {
    let tx = watch::channel(None).0;
    let mut rx = tx.subscribe();
    let session = engine.session().clone();
    let task = tokio::spawn(async move { observe(&session, &tx).await });
    let result = async {
        engine.set_shuffle(true)?;
        engine.next()?;
        tokio::time::timeout(Duration::from_secs(30), rx.wait_for(|s| s.as_ref().is_some_and(|s| s.shuffle)))
            .await.map_err(|_| crate::PlayerError::QueueContext)?.map_err(|_| crate::PlayerError::Stopped)?;
        let snapshot = rx.borrow().clone().ok_or(crate::PlayerError::QueueContext)?;
        let rows = snapshot.tracks.iter().enumerate().map(|(i,t)| serde_json::json!({"uri":t.uri,"uid":format!("row-{i}")})).collect::<Vec<_>>();
        println!("{}", serde_json::json!({"synthetic":false,"current":snapshot.current,"occurrence":"live-occurrence","shuffle":snapshot.shuffle,"next_tracks":rows}));
        Ok(())
    }.await;
    task.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::{
        connect::Cluster,
        player::{ContextPlayerOptions, PlayerState, ProvidedTrack},
    };
    const A: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
    const B: &str = "spotify:track:6rqhFgbbKwnb9MLmUQDhG6";
    // S9 synthetic fixture, constructed from the upstream protobuf schema; NOT a live recording.
    fn synthetic() -> ClusterUpdate {
        let mut state = PlayerState::new();
        state.track = Some(ProvidedTrack {
            uri: A.into(),
            uid: "a-1".into(),
            ..Default::default()
        })
        .into();
        state.playback_id = "play-1".into();
        state.queue_revision = "rev-1".into();
        state.options = Some(ContextPlayerOptions {
            shuffling_context: true,
            ..Default::default()
        })
        .into();
        state.next_tracks = [(B, "b-1"), (B, "b-2"), (A, "a-2")]
            .map(|(uri, uid)| ProvidedTrack {
                uri: uri.into(),
                uid: uid.into(),
                ..Default::default()
            })
            .to_vec();
        let mut c = Cluster::new();
        c.active_device_id = "local".into();
        c.player_state = Some(state).into();
        ClusterUpdate {
            cluster: Some(c).into(),
            ..Default::default()
        }
    }
    #[test]
    fn cluster_transport_rebases_progress_and_keeps_pause_options_and_duplicates() {
        use librespot_protocol::{connect::DeviceInfo, devices::DeviceType};
        let mut u = synthetic();
        let c = u.cluster.as_mut().unwrap();
        c.device.insert(
            "local".into(),
            DeviceInfo {
                name: "Velo".into(),
                device_type: DeviceType::COMPUTER.into(),
                ..Default::default()
            },
        );
        let p = c.player_state.as_mut().unwrap();
        p.timestamp = 10_000;
        p.position_as_of_timestamp = 1_200;
        p.duration = 240_000;
        p.is_playing = true;
        p.options.as_mut().unwrap().repeating_context = true;
        let now = std::time::UNIX_EPOCH + Duration::from_millis(12_000);
        let state = playback_state(&u, now).unwrap();
        assert_eq!(state.progress_ms, 3_200);
        assert!(state.is_playing && state.shuffle);
        assert_eq!(state.repeat, velo_domain::RepeatMode::Context);
        assert!(state.device.unwrap().is_active);
        assert_eq!(decode(&u, "local").unwrap().tracks.len(), 3);
        let p = u.cluster.as_mut().unwrap().player_state.as_mut().unwrap();
        p.is_paused = true;
        p.options.as_mut().unwrap().repeating_track = true;
        let paused = playback_state(&u, now).unwrap();
        assert_eq!(paused.progress_ms, 1_200);
        assert!(!paused.is_playing);
        assert_eq!(paused.repeat, velo_domain::RepeatMode::Track);
        let p = u.cluster.as_mut().unwrap().player_state.as_mut().unwrap();
        p.is_paused = false;
        p.is_buffering = true;
        assert!(!playback_state(&u, now).unwrap().is_playing);
        u.cluster.as_mut().unwrap().need_full_player_state = true;
        assert!(playback_state(&u, now).is_none());
    }

    #[test]
    fn live_cluster_groups_local_second_rejects_stale_and_preserves_partial_transport() {
        use librespot_protocol::{connect::DeviceInfo, devices::DeviceType};
        let mut u = synthetic();
        let c = u.cluster.as_mut().unwrap();
        c.changed_timestamp_ms = 100;
        c.active_device_id = "phone".into();
        for (id, name, kind) in [
            ("phone", "Pixel", DeviceType::SMARTPHONE),
            ("local", "Z Velo", DeviceType::COMPUTER),
            ("speaker", "A speaker", DeviceType::SPEAKER),
        ] {
            c.device.insert(
                id.into(),
                DeviceInfo {
                    name: name.into(),
                    device_type: kind.into(),
                    ..Default::default()
                },
            );
        }
        let tx = watch::channel(None).0;
        let rx = tx.subscribe();
        publish_cluster(&tx, &u, "local");
        let s = rx.borrow().clone().unwrap();
        assert_eq!(
            s.devices
                .iter()
                .map(|d| d.id.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["phone", "local", "speaker"]
        );
        assert_eq!(s.queue.as_ref().unwrap().tracks.len(), 3);
        let c = u.cluster.as_mut().unwrap();
        c.changed_timestamp_ms = 90;
        c.device.clear();
        publish_cluster(&tx, &u, "local");
        assert_eq!(rx.borrow().as_ref().unwrap().devices.len(), 3);
        let mut partial = synthetic();
        let c = partial.cluster.as_mut().unwrap();
        c.changed_timestamp_ms = 110;
        c.active_device_id = "phone".into();
        c.need_full_player_state = true;
        c.device = u.cluster.as_ref().unwrap().device.clone();
        // Restore device metadata, as a partial transport still contains a device list.
        c.device = s
            .devices
            .iter()
            .map(|d| {
                (
                    d.id.clone().unwrap(),
                    DeviceInfo {
                        name: d.name.clone(),
                        ..Default::default()
                    },
                )
            })
            .collect();
        publish_cluster(&tx, &partial, "local");
        assert_eq!(
            rx.borrow()
                .as_ref()
                .unwrap()
                .playback
                .as_ref()
                .unwrap()
                .track
                .as_ref()
                .unwrap()
                .uri,
            A
        );
        partial.cluster.as_mut().unwrap().active_device_id = "speaker".into();
        publish_cluster(&tx, &partial, "local");
        assert!(rx.borrow().as_ref().unwrap().playback.is_none());
    }

    #[tokio::test]
    #[ignore = "Manual live probe: saved keyring login and a playing context required"]
    async fn s9_queue_probe() {
        let engine = crate::Engine::from_keyring(crate::EngineConfig::velo("Velo queue probe"))
            .await
            .unwrap();
        let result = shuffle_probe(&engine).await;
        engine.shutdown().await;
        result.unwrap();
    }
    #[test]
    fn rejected_update_keeps_previous_snapshot_unless_track_or_device_changes() {
        let good = decode(&synthetic(), "local").unwrap();
        let mut partial = synthetic();
        partial.cluster.as_mut().unwrap().need_full_player_state = true;
        assert_eq!(
            reconcile(Some(&good), &partial, "local"),
            Some(good.clone())
        );
        let mut moved = partial.clone();
        moved.cluster.as_mut().unwrap().active_device_id = "other".into();
        assert_eq!(reconcile(Some(&good), &moved, "local"), None);
        let mut changed = partial.clone();
        changed
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .track = Some(ProvidedTrack {
            uri: B.into(),
            ..Default::default()
        })
        .into();
        assert_eq!(reconcile(Some(&good), &changed, "local"), None);
        assert_eq!(reconcile(None, &partial, "local"), None);
    }
    #[test]
    fn rows_are_capped_to_the_fetched_metadata_window() {
        let mut s = decode(&synthetic(), "local").unwrap();
        s.tracks = (0..80)
            .map(|i| Occurrence {
                uri: A.into(),
                uid: format!("u{i}"),
                queued: false,
            })
            .collect();
        assert_eq!(rows(&s, &Default::default()).len(), MAX_ROWS);
    }
    #[test]
    fn partial_queue_change_or_new_duplicate_occurrence_invalidates_old_rows() {
        let good = decode(&synthetic(), "local").unwrap();
        for field in ["occurrence", "revision", "shuffle"] {
            let mut u = synthetic();
            let c = u.cluster.as_mut().unwrap();
            c.need_full_player_state = true;
            let p = c.player_state.as_mut().unwrap();
            match field {
                "occurrence" => p.playback_id = "play-2".into(),
                "revision" => p.queue_revision = "rev-2".into(),
                _ => p.options.as_mut().unwrap().shuffling_context = false,
            }
            assert_eq!(reconcile(Some(&good), &u, "local"), None, "{field}");
        }
    }
    #[test]
    fn v92_transport_update_advances_watermark_without_waking_queue() {
        let mut s = decode(&synthetic(), "local").unwrap();
        s.timestamp = 100;
        let tx = watch::channel(Some(s.clone())).0;
        let rx = tx.subscribe();
        s.timestamp = 300;
        publish_snapshot(&tx, Some(s.clone()));
        assert_eq!(tx.borrow().as_ref().unwrap().timestamp, 300);
        assert!(!rx.has_changed().unwrap());
        s.timestamp = 200;
        s.tracks.clear();
        publish_snapshot(&tx, Some(s));
        assert_eq!(tx.borrow().as_ref().unwrap().tracks.len(), 3);
        assert!(!rx.has_changed().unwrap());
    }
    #[test]
    fn v92_queued_duplicate_does_not_consume_the_context_tail_when_edited() {
        let mut u = synthetic();
        let p = u.cluster.as_mut().unwrap().player_state.as_mut().unwrap();
        p.options.as_mut().unwrap().shuffling_context = false;
        p.next_tracks[0].provider = "queue".into();
        p.next_tracks[1].provider = "context".into();
        let s = decode(&u, "local").unwrap();
        let context = vec![A.to_owned(), B.to_owned(), A.to_owned()];
        let visible = vec![B.to_owned()]; // Only the manually queued occurrence fits.
        let prefix = s.context_prefix(&visible).unwrap();
        assert!(prefix.is_empty());
        assert_eq!(
            super::super::tail_after(&context, A, &prefix, false, Some(0)).unwrap(),
            [B, A]
        );
        let prefix = s.context_prefix(&[B.to_owned(), B.to_owned()]).unwrap();
        assert_eq!(prefix, [B]);
        assert_eq!(
            super::super::tail_after(&context, A, &prefix, false, Some(0)).unwrap(),
            [A]
        );
        assert!(
            s.context_prefix(&[A.to_owned()]).is_err(),
            "stale rows are refused"
        );
        let p = u.cluster.as_mut().unwrap().player_state.as_mut().unwrap();
        p.next_tracks[0].provider.clear();
        p.next_tracks[0]
            .metadata
            .insert("is_queued".into(), "true".into());
        assert!(decode(&u, "local").unwrap().tracks[0].queued);
    }
    #[test]
    fn synthetic_shuffle_duplicates_advance_and_empty_status() {
        let mut u = synthetic();
        let s = decode(&u, "local").unwrap();
        assert!(s.shuffle);
        assert_eq!(s.tracks.len(), 3);
        assert_ne!(s.tracks[0].uid, s.tracks[1].uid);
        let p = u.cluster.as_mut().unwrap().player_state.as_mut().unwrap();
        p.track = Some(p.next_tracks.remove(0)).into();
        p.playback_id = "play-2".into();
        let n = decode(&u, "local").unwrap();
        assert_eq!(n.tracks.len(), 2);
        assert_ne!(s.occurrence, n.occurrence);
        u.cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .next_tracks
            .clear();
        assert!(decode(&u, "local").unwrap().tracks.is_empty());
        assert!(decode(&u, "remote").is_none());
        u.cluster.as_mut().unwrap().need_full_player_state = true;
        assert!(decode(&u, "local").is_none());
    }
}
