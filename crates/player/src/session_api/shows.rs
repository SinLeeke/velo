//! Isolated podcast adapter. Collection set names are accepted only after a manual probe;
//! no guessed set is enabled by default. SHOW/EPISODE metadata excludes audiobooks explicitly.
use super::{LibraryEntry, unavailable};
use librespot_core::{Session, SpotifyUri};
use librespot_metadata::{Metadata, episode::Episode, show::Show};
use velo_domain::{ArtistRef, ImageRef, Page, Result, Track, VeloError};

fn images(covers: &librespot_metadata::image::Images) -> Vec<ImageRef> {
    covers
        .iter()
        .filter_map(|c| {
            Some(ImageRef {
                url: format!("https://i.scdn.co/image/{}", c.id.to_base16().ok()?),
                width: u32::try_from(c.width).ok().filter(|n| *n > 0),
                height: u32::try_from(c.height).ok().filter(|n| *n > 0),
            })
        })
        .collect()
}

pub async fn show(session: &Session, id: &str) -> Result<Show> {
    let uri =
        SpotifyUri::from_uri(&format!("spotify:show:{id}")).map_err(|_| VeloError::NotFound)?;
    let show = tokio::time::timeout(std::time::Duration::from_secs(15), Show::get(session, &uri))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
    if show.is_audiobook {
        return Err(VeloError::NotFound);
    }
    Ok(show)
}

pub fn entry(show: &Show, added_at: Option<i64>) -> Result<LibraryEntry> {
    if show.is_audiobook {
        return Err(VeloError::NotFound);
    }
    let uri = show.id.to_uri().map_err(|_| unavailable())?;
    Ok(LibraryEntry {
        kind: "show".into(),
        object_id: uri.trim_start_matches("spotify:show:").into(),
        uri,
        name: show.name.clone(),
        creator: show.publisher.clone(),
        images: images(&show.covers),
        added_at,
    })
}

/// Manual probe also verifies metadata, not just the existence of a set. No secrets logged.
pub async fn probe(session: &Session, set: &str) -> Result<Vec<LibraryEntry>> {
    if set.is_empty()
        || set.len() > 64
        || !set.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(VeloError::NotFound);
    }
    let rows = crate::collection::read_set(session, set)
        .await
        .map_err(|_| unavailable())?;
    let mut out = Vec::new();
    for row in rows {
        let Some(id) = row.uri.strip_prefix("spotify:show:") else {
            continue;
        };
        let s = show(session, id).await?;
        out.push(entry(&s, row.added_at)?);
    }
    Ok(out)
}

pub async fn saved(session: &Session) -> Result<Vec<LibraryEntry>> {
    let set = std::env::var("VELO_SHOWS_SET").map_err(|_| unavailable())?;
    probe(session, &set).await
}

pub async fn episodes(session: &Session, id: &str, offset: u32, limit: u32) -> Result<Page<Track>> {
    let s = show(session, id).await?;
    let total = u32::try_from(s.episodes.len()).unwrap_or(u32::MAX);
    let mut tracks = Vec::new();
    // Sequential and bounded to one page; no task per entire podcast or startup fanout.
    for uri in s.episodes.iter().skip(offset as usize).take(limit as usize) {
        let e = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            Episode::get(session, uri),
        )
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        if e.is_audiobook_chapter {
            continue;
        }
        let uri = e.id.to_uri().map_err(|_| unavailable())?;
        tracks.push(Track {
            id: Some(uri.trim_start_matches("spotify:episode:").into()),
            uri,
            title: e.name,
            artists: vec![ArtistRef {
                id: None,
                name: s.publisher.clone(),
            }],
            album: s.name.clone(),
            album_id: None,
            duration_ms: e.duration.max(0) as u64,
            explicit: e.is_explicit,
            images: images(&e.covers),
            added_at: None,
        });
    }
    Ok(Page {
        items: tracks,
        offset,
        limit,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::Message;
    #[test]
    fn synthetic_show_fixture_excludes_audiobooks_and_keeps_identity() {
        // SYNTHETIC S9 fixture: proves mapping only; never evidence of a live collection set.
        let text = "gid: \"\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\001\" name: \"Synthetic podcast\" publisher: \"Fixture author\" is_audiobook: false";
        let metadata: librespot_protocol::metadata::Show =
            protobuf::text_format::parse_from_str(text).unwrap();
        let s = Show::try_from(&metadata).unwrap();
        let row = entry(&s, Some(42)).unwrap();
        assert_eq!(row.kind, "show");
        assert_eq!(row.creator, "Fixture author");
        assert_eq!(row.added_at, Some(42));
        assert!(row.uri.starts_with("spotify:show:"));
        assert!(!s.is_audiobook);
        let mut audiobook = s.clone();
        audiobook.is_audiobook = true;
        assert!(entry(&audiobook, None).is_err());
        let bytes = metadata.write_to_bytes().unwrap();
        assert!(!bytes.is_empty());
    }
}
