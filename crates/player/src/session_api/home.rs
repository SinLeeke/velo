//! Inicio for [`super::SessionApi`]: recently played and the user's tops, through the session.
//! `home.json`'s "Recently played" contains playlists/a station, not tracks, and "Your top
//! mixes" contains playlists. recently_played reads spclient `recently-played/v3`; top_tracks
//! has no session source (G3 probe) and keeps the Web API fallback (see [`super::catalog`]). top_artists (L1) reads the artist section from pathfinder `home`:
//! Spotify's own pick of the artists the user listens to most, without a TimeRange, which is
//! what Inicio shows; any range gets the same list.

use librespot_core::Session;
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;
use velo_domain::{Artist, ImageRef, Page, Result, TimeRange, Track, VeloError};

use super::catalog::not_migrated;

const HOME: (&str, &str) = (
    "home",
    "5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1",
);

/// Track URIs from spclient `recently-played/v3` (one `lastPlayedTrackUri` per recent
/// context), newest first, deduplicated. Pure so it is tested on the captured fixture.
pub fn parse_recent_track_uris(body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    value
        .get("playContexts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("lastPlayedTrackUri").and_then(Value::as_str))
        .filter(|u| u.starts_with("spotify:track:") && seen.insert(*u))
        .map(str::to_string)
        .collect()
}

/// Recently played track URIs through the session (empty on any failure).
pub async fn recent_track_uris(session: &Session, limit: u32) -> Result<Vec<String>> {
    let path = format!(
        "/recently-played/v3/user/{}/recently-played?format=json&limit={}",
        session.username(),
        limit.clamp(1, 50)
    );
    let body = session
        .spclient()
        .request_as_json(&http::Method::GET, &path, None, None)
        .await
        .map_err(|_| super::unavailable())?;
    Ok(parse_recent_track_uris(&body))
}

pub async fn recently_played(session: &Session, limit: u32) -> Result<Vec<Track>> {
    let uris = recent_track_uris(session, limit).await?;
    let items: Vec<_> = uris
        .into_iter()
        .map(|uri| crate::playlist::PlaylistItem {
            uri,
            added_at: None,
        })
        .collect();
    Ok(crate::playlist::tracks_in(session, &items).await)
}

pub async fn top_tracks(
    _session: &Session,
    _range: TimeRange,
    _offset: u32,
    _limit: u32,
) -> Result<Page<Track>> {
    Err(not_migrated())
}

pub async fn top_artists(
    session: &Session,
    _range: TimeRange,
    offset: u32,
    limit: u32,
) -> Result<Page<Artist>> {
    let data = home_data(session).await?;
    let all = favorite_artists(&data)?;
    let total = u32::try_from(all.len()).unwrap_or(u32::MAX);
    Ok(Page {
        items: all
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect(),
        offset,
        limit,
        total,
    })
}

/// Single-flight cache shared by shortcuts and favorite artists. No polling.
pub async fn home_data(session: &Session) -> Result<Arc<Value>> {
    type Cache = Option<(String, Instant, Arc<Value>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cached = CACHE.get_or_init(|| Mutex::new(None)).lock().await;
    let key = format!("{}:{}", session.username(), session.device_id());
    if let Some((owner, at, data)) = &*cached {
        if owner == &key && at.elapsed() < Duration::from_secs(60) {
            return Ok(data.clone());
        }
    }
    let variables = json!({
        "homeEndUserIntegration": "INTEGRATION_WEB_PLAYER",
        "timeZone": "UTC",
        "sp_t": "",
        "facet": "",
        "sectionItemsLimit": 20,
        "includeEpisodeContentRatingsV2": false,
    });
    let data = Arc::new(super::pathfinder::query(session, HOME.0, &variables, HOME.1).await?);

    *cached = Some((key, Instant::now(), data.clone()));
    Ok(data)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub kind: String,
    pub object_id: String,
    pub uri: String,
    pub name: String,
    pub images: Vec<ImageRef>,
}

pub async fn shortcuts(session: &Session) -> Result<Vec<Shortcut>> {
    let data = home_data(session).await?;
    quick_access(&data)
}

/// Select by typename, never by translated title. UnknownType collections use canonical URI.
pub fn quick_access(data: &Value) -> Result<Vec<Shortcut>> {
    let section = data["home"]["sectionContainer"]["sections"]["items"]
        .as_array()
        .and_then(|s| {
            s.iter()
                .find(|s| s["data"]["__typename"] == "HomeShortsSectionData")
        })
        .ok_or(VeloError::NotFound)?;
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for item in section["sectionItems"]["items"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let content = &item["content"];
        let v = &content["data"];
        let uri = v["uri"]
            .as_str()
            .or_else(|| content["uri"].as_str())
            .unwrap_or_default();
        let (kind, name, sources) = match v["__typename"].as_str() {
            Some("Artist") => (
                "artist",
                v["profile"]["name"].as_str(),
                &v["visuals"]["avatarImage"]["sources"],
            ),
            Some("Playlist") => (
                "playlist",
                v["name"].as_str(),
                &v["images"]["items"][0]["sources"],
            ),
            Some("Album") => ("album", v["name"].as_str(), &v["coverArt"]["sources"]),
            _ if content["__typename"] == "UnknownType"
                && uri.starts_with("spotify:user:")
                && uri.ends_with(":collection") =>
            {
                ("liked", Some(""), &Value::Null)
            }
            _ => continue,
        };
        let object_id = if kind == "liked" {
            ""
        } else {
            let Some(id) = uri.strip_prefix(&format!("spotify:{kind}:")) else {
                continue;
            };
            if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
                continue;
            }
            id
        };
        let Some(name) = name else { continue };
        if !seen.insert(uri.to_owned()) {
            continue;
        }
        let images = sources
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some(ImageRef {
                    url: s["url"]
                        .as_str()
                        .filter(|u| u.starts_with("https://"))?
                        .to_owned(),
                    width: s["width"].as_u64().and_then(|n| n.try_into().ok()),
                    height: s["height"].as_u64().and_then(|n| n.try_into().ok()),
                })
            })
            .collect();
        rows.push(Shortcut {
            kind: kind.into(),
            object_id: object_id.into(),
            uri: uri.into(),
            name: name.into(),
            images,
        });
        if rows.len() == 8 {
            break;
        }
    }
    Ok(rows)
}

/// The artists of the favorite-artists section of a `home` answer (`data`).
pub fn favorite_artists(data: &Value) -> Result<Vec<Artist>> {
    let section = data["home"]["sectionContainer"]["sections"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["data"]["__typename"] == "HomeFavoriteArtistsSectionData")
        .or_else(|| {
            data["home"]["sectionContainer"]["sections"]["items"]
                .as_array()?
                .iter()
                .find(|s| {
                    s["sectionItems"]["items"].as_array().is_some_and(|items| {
                        !items.is_empty()
                            && items.iter().all(|item| {
                                item["content"]["__typename"] == "ArtistResponseWrapper"
                                    || item["content"]["data"]["__typename"] == "Artist"
                            })
                    })
                })
        })
        .ok_or(VeloError::NotFound)?;
    let artists: Vec<Artist> = section["sectionItems"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| home_artist(&item["content"]["data"]))
        .collect();
    if artists.is_empty() {
        return Err(VeloError::NotFound);
    }
    Ok(artists)
}

fn home_artist(v: &Value) -> Option<Artist> {
    if v["__typename"] != "Artist" {
        return None;
    }
    let uri = v["uri"].as_str()?;
    let id = uri.strip_prefix("spotify:artist:")?;
    if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let images = v["visuals"]["avatarImage"]["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let url = s["url"].as_str().filter(|u| u.starts_with("https://"))?;
            Some(ImageRef {
                url: url.to_string(),
                width: s["width"].as_u64().and_then(|w| u32::try_from(w).ok()),
                height: s["height"].as_u64().and_then(|h| u32::try_from(h).ok()),
            })
        })
        .collect();
    Some(Artist {
        id: id.to_string(),
        uri: uri.to_string(),
        name: v["profile"]["name"].as_str()?.to_string(),
        images,
        genres: Vec::new(),
        followers: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcuts_preserve_spotify_order_liked_and_dedupe() {
        let mut v: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/home.json"))
                .unwrap();
        let rows = super::quick_access(&v["data"]).unwrap();
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[0].kind, "artist");
        assert!(rows.iter().any(|r| r.kind == "liked"));
        let section = v["data"]["home"]["sectionContainer"]["sections"]["items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|s| s["data"]["__typename"] == "HomeShortsSectionData")
            .unwrap();
        section["data"]["title"] = serde_json::json!({"translatedBaseText":"changed"});
        let items = section["sectionItems"]["items"].as_array_mut().unwrap();
        assert_eq!(items.len(), 10);
        items.insert(0, items[0].clone());
        assert_eq!(super::quick_access(&v["data"]).unwrap(), rows);
        assert_eq!(
            rows.iter()
                .map(|r| &r.uri)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            8
        );
    }

    #[test]
    fn favorite_artists_come_from_their_section_with_https_images() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/home.json"))
                .unwrap();
        let artists = super::favorite_artists(&v["data"]).unwrap();
        assert_eq!(artists.len(), 9);
        assert!(
            artists
                .iter()
                .all(|a| a.uri == format!("spotify:artist:{}", a.id)
                    && !a.name.is_empty()
                    && a.images.iter().all(|i| i.url.starts_with("https://")))
        );
        assert!(super::favorite_artists(&serde_json::json!({"home": {}})).is_err());
    }

    #[test]
    fn favorite_artists_ignore_changing_section_uris() {
        let mut v: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/home.json"))
                .unwrap();
        let before = super::favorite_artists(&v["data"]).unwrap();
        for section in v["data"]["home"]["sectionContainer"]["sections"]["items"]
            .as_array_mut()
            .unwrap()
        {
            section["uri"] = serde_json::json!("spotify:section:changing-per-request");
            section["data"]["__typename"] = serde_json::json!("HomeGenericSectionData");
            section["data"]["title"] = serde_json::json!({"translatedBaseText":"Vos artistes préférés", "transformedLabel":"お気に入り"});
        }
        assert_eq!(super::favorite_artists(&v["data"]).unwrap(), before);
    }

    #[test]
    fn home_fixture_has_context_history_and_no_time_range_tops() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pathfinder/home.json"))
                .unwrap();
        let sections = v["data"]["home"]["sectionContainer"]["sections"]["items"]
            .as_array()
            .unwrap();
        let recent = sections
            .iter()
            .find(|s| s["data"]["title"]["translatedBaseText"] == "Recently played")
            .unwrap();
        let items = recent["sectionItems"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 10);
        assert_eq!(
            items
                .iter()
                .filter(|i| i["content"]["data"]["__typename"] == "Playlist")
                .count(),
            9
        );
        assert_eq!(
            items[0]["content"]["uri"],
            "spotify:station:artist:3Sz7ZnJQBIHsXLUSo0OQtM"
        );
        assert!(
            items
                .iter()
                .all(|i| i["content"]["data"]["__typename"] != "Track")
        );
        assert!(sections.iter().all(|s| s["data"]["timeRange"].is_null()));
        let favorites = sections
            .iter()
            .find(|s| s["data"]["title"]["translatedBaseText"] == "Your favorite artists")
            .unwrap();
        assert_eq!(
            favorites["sectionItems"]["items"].as_array().unwrap().len(),
            9
        );
    }
}

#[cfg(test)]
mod recent_tests {
    use super::parse_recent_track_uris;

    #[test]
    fn recent_tracks_come_from_play_contexts_in_order_without_duplicates() {
        let uris = parse_recent_track_uris(include_bytes!(
            "../../tests/fixtures/spclient/recently_played.json"
        ));
        assert_eq!(uris.len(), 12);
        assert!(uris.iter().all(|u| u.starts_with("spotify:track:")));
        assert!(parse_recent_track_uris(b"<html>").is_empty());
        let twice = br#"{"playContexts":[{"lastPlayedTrackUri":"spotify:track:a"},{"lastPlayedTrackUri":"spotify:track:a"},{"lastPlayedTrackUri":"spotify:episode:e"}]}"#;
        assert_eq!(parse_recent_track_uris(twice), ["spotify:track:a"]);
    }
}
