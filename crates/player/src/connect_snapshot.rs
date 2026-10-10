//! Read-only dealer observer. It adds a subscriber, never takes Spirc's command handler.
//! Cluster/next_tracks for the local device still require a live probe; no context shuffle.
use librespot_core::{Session, dealer::protocol::Message};
use librespot_protocol::connect::ClusterUpdate;
use std::{future::poll_fn, time::Duration};
use tokio::sync::watch;
use velo_domain::{ArtistRef, Track};

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
