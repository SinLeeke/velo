//! Tolerant Spotify JSON adapters, independent of HTTP and authentication.
use serde_json::Value;
use std::time::Instant;
use velo_domain::*;

fn string(v: &Value, field: &str) -> String {
    v[field].as_str().unwrap_or_default().to_owned()
}

fn optional_string(v: &Value, field: &str) -> Option<String> {
    v[field]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn number(v: &Value, field: &str) -> u32 {
    v[field].as_u64().unwrap_or_default().min(u32::MAX as u64) as u32
}

pub(crate) fn list<T>(v: &Value, map: impl Fn(&Value) -> Option<T>) -> Vec<T> {
    v.as_array().into_iter().flatten().filter_map(map).collect()
}

// Pagination metadata describes Spotify's raw slots, including skipped items.
// Consumers must advance by offset + limit, not the filtered items.len(). The
// domain Page::has_more can overestimate the last page after filtering.
pub(crate) fn page<T>(v: &Value, map: impl Fn(&Value) -> Option<T>) -> Page<T> {
    Page {
        items: list(&v["items"], map),
        offset: number(v, "offset"),
        limit: number(v, "limit"),
        total: number(v, "total"),
    }
}

fn images(v: &Value) -> Vec<ImageRef> {
    list(&v["images"], |i| {
        Some(ImageRef {
            url: i["url"].as_str()?.to_owned(),
            width: i["width"].as_u64().and_then(|n| n.try_into().ok()),
            height: i["height"].as_u64().and_then(|n| n.try_into().ok()),
        })
    })
}

fn artists(v: &Value) -> Vec<ArtistRef> {
    list(&v["artists"], |a| {
        Some(ArtistRef {
            id: optional_string(a, "id"),
            name: a["name"].as_str()?.to_owned(),
        })
    })
}

pub(crate) fn track(v: &Value) -> Option<Track> {
    if v.is_null() || v["type"].as_str().is_some_and(|kind| kind != "track") {
        return None;
    }
    let uri = v["uri"].as_str()?;
    if !uri.starts_with("spotify:track:") && !uri.starts_with("spotify:local:") {
        return None;
    }
    Some(Track {
        id: if v["is_local"].as_bool() == Some(true) || uri.starts_with("spotify:local:") {
            None
        } else {
            optional_string(v, "id")
        },
        uri: uri.to_owned(),
        title: v["name"].as_str()?.to_owned(),
        artists: artists(v),
        album: string(&v["album"], "name"),
        album_id: optional_string(&v["album"], "id"),
        duration_ms: v["duration_ms"].as_u64().unwrap_or_default(),
        explicit: v["explicit"].as_bool().unwrap_or(false),
        images: images(&v["album"]),
        added_at: None,
    })
}

/// A playlist or saved-tracks item: the track under `key` plus the item's `added_at` (F3).
pub(crate) fn dated_track(v: &Value, key: &str) -> Option<Track> {
    let mut t = track(&v[key])?;
    t.added_at = v["added_at"].as_str().and_then(timestamp_ms);
    Some(t)
}

/// `"2026-03-01T12:00:00Z"` (ISO 8601 in UTC, optional fraction or `+00:00`) → Unix ms.
/// `None` for anything else, including Spotify's `1970-01-01T00:00:00Z` placeholder of
/// very old playlists.
pub(crate) fn timestamp_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let field = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let rest = &s[19..];
    let rest = rest
        .strip_prefix('.')
        .map(|r| r.trim_start_matches(|c: char| c.is_ascii_digit()))
        .unwrap_or(rest);
    if !matches!(rest, "Z" | "z" | "+00:00" | "") {
        return None;
    }
    let ms =
        (days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second) * 1000;
    (ms > 0).then_some(ms)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn playlist(v: &Value) -> Option<Playlist> {
    Some(Playlist {
        id: v["id"].as_str()?.to_owned(),
        uri: string(v, "uri"),
        name: v["name"].as_str()?.to_owned(),
        description: string(v, "description"),
        owner: optional_string(&v["owner"], "display_name")
            .unwrap_or_else(|| string(&v["owner"], "id")),
        owner_id: string(&v["owner"], "id"),
        collaborative: v["collaborative"].as_bool().unwrap_or(false),
        images: images(v),
        track_count: number(v.get("items").unwrap_or(&v["tracks"]), "total"),
    })
}

pub(crate) fn album(v: &Value) -> Option<Album> {
    Some(Album {
        id: v["id"].as_str()?.to_owned(),
        uri: string(v, "uri"),
        name: v["name"].as_str()?.to_owned(),
        artists: artists(v),
        images: images(v),
        release_date: string(v, "release_date"),
        total_tracks: number(v, "total_tracks"),
        album_type: string(v, "album_type"),
    })
}

pub(crate) fn artist(v: &Value) -> Option<Artist> {
    Some(Artist {
        id: v["id"].as_str()?.to_owned(),
        uri: string(v, "uri"),
        name: v["name"].as_str()?.to_owned(),
        images: images(v),
        genres: list(&v["genres"], |g| g.as_str().map(str::to_owned)),
        followers: v["followers"]["total"].as_u64(),
    })
}

pub(crate) fn user(v: &Value) -> Option<UserProfile> {
    let id = v["id"].as_str()?.to_owned();
    Some(UserProfile {
        display_name: optional_string(v, "display_name").unwrap_or_else(|| id.clone()),
        id,
        // Development Mode can omit product; do not infer Premium from its absence.
        product: string(v, "product"),
        images: images(v),
    })
}

/// U4: `GET /me` / `GET /users/{id}` for the profile page.
pub(crate) fn public_profile(v: &Value) -> Option<PublicProfile> {
    let id = v["id"].as_str()?.to_owned();
    Some(PublicProfile {
        display_name: optional_string(v, "display_name").unwrap_or_else(|| id.clone()),
        id,
        images: images(v),
        followers: v["followers"]["total"].as_u64(),
    })
}

/// U4: a playlist the profile of `user_id` shows: owned by that user and not private
/// (`public: false`). `public: null` (unknown) is kept, as Spotify's own profile would.
pub(crate) fn profile_playlist(v: &Value, user_id: &str) -> Option<Playlist> {
    if v["public"].as_bool() == Some(false) || v["owner"]["id"].as_str() != Some(user_id) {
        return None;
    }
    playlist(v)
}

pub(crate) fn device(v: &Value) -> Option<Device> {
    Some(Device {
        id: optional_string(v, "id"),
        name: v["name"].as_str()?.to_owned(),
        kind: string(v, "type"),
        is_active: v["is_active"].as_bool().unwrap_or(false),
        volume_percent: v["volume_percent"].as_u64().map(|n| n.min(100) as u8),
        supports_volume: v["supports_volume"].as_bool().unwrap_or(false),
    })
}

pub(crate) fn playback(v: &Value) -> PlaybackState {
    PlaybackState {
        track: track(&v["item"]),
        is_playing: v["is_playing"].as_bool().unwrap_or(false),
        progress_ms: v["progress_ms"].as_u64().unwrap_or_default(),
        sampled_at: Instant::now(),
        shuffle: v["shuffle_state"].as_bool().unwrap_or(false),
        repeat: match v["repeat_state"].as_str() {
            Some("context") => RepeatMode::Context,
            Some("track") => RepeatMode::Track,
            _ => RepeatMode::Off,
        },
        device: device(&v["device"]),
        context_uri: optional_string(&v["context"], "uri"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mixed_playlist_items_preserve_local_tracks_and_skip_nulls_and_episodes() {
        let v = json!({"offset": 5, "limit": 4, "total": 20, "items": [
            {"added_at":"2026-03-01T12:00:00Z", "item": {
                "type":"track", "id":"song", "uri":"spotify:track:song", "name":"Song",
                "duration_ms":231000, "explicit":true, "artists":[{"id":"a","name":"Artist"}],
                "album":{"id":"album","name":"Album","images":[{"url":"https://i.scdn.co/image/cover","width":640,"height":640}]}
            }},
            {"item":null},
            {"item":{"type":"episode","name":"Podcast","uri":"spotify:episode:episode"}},
            {"item":{"type":"track","is_local":true,"id":null,"uri":"spotify:local:Artist:Album:Local:60","name":"Local","album":{"name":"Album","images":[]},"artists":[{"id":null,"name":"Artist"}],"duration_ms":60000}}
        ]});
        let result = page(&v, |i| track(&i["item"]));
        assert_eq!((result.offset, result.limit, result.total), (5, 4, 20));
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].images[0].width, Some(640));
        assert!(result.items[0].explicit);
        assert_eq!(result.items[0].artist_line(), "Artist");
        assert_eq!(result.items[1].id, None);
    }

    #[test]
    fn items_keep_their_added_at_and_bad_dates_are_none() {
        let v = json!({"offset": 0, "limit": 2, "total": 2, "items": [
            {"added_at":"2026-03-01T12:00:00Z", "track": {
                "type":"track", "id":"song", "uri":"spotify:track:song", "name":"Song",
                "artists":[], "album":{"name":"Album"}
            }},
            {"added_at":"1970-01-01T00:00:00Z", "track": {
                "type":"track", "id":"old", "uri":"spotify:track:old", "name":"Old",
                "artists":[], "album":{"name":"Album"}
            }}
        ]});
        let result = page(&v, |i| dated_track(i, "track"));
        assert_eq!(result.items[0].added_at, Some(1_772_366_400_000));
        assert_eq!(result.items[1].added_at, None);
        assert_eq!(track(&v["items"][0]["track"]).unwrap().added_at, None);
        assert_eq!(
            timestamp_ms("2000-02-29T00:00:01.250Z"),
            Some(951_782_401_000)
        );
        assert_eq!(
            timestamp_ms("2024-12-31T23:59:59+00:00"),
            Some(1_735_689_599_000)
        );
        for bad in [
            "",
            "2024-13-01T00:00:00Z",
            "2024-01-01",
            "2024-01-01T00:00:00+02:00",
            "abcd-01-01T00:00:00Z",
        ] {
            assert_eq!(timestamp_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn playlist_ownership_and_current_item_count() {
        let p = playlist(&json!({"id":"p","name":"Mix","owner":{"id":"owner","display_name":null},"collaborative":true,"items":{"total":42},"images":null})).unwrap();
        assert_eq!(p.owner_id, "owner");
        assert_eq!(p.owner, "owner");
        assert_eq!(p.track_count, 42);
        assert!(p.items_listable_by("collaborator"));
    }

    #[test]
    fn episode_playback_and_missing_optional_metadata_are_safe() {
        let p = playback(
            &json!({"item":{"type":"episode"},"is_playing":true,"progress_ms":null,"device":{"id":null,"name":"PC","type":"Computer","volume_percent":null}}),
        );
        assert!(p.track.is_none());
        assert_eq!(p.progress_ms, 0);
        assert_eq!(p.device.unwrap().volume_percent, None);
        assert_eq!(
            artist(&json!({"id":"a","name":"Artist"}))
                .unwrap()
                .followers,
            None
        );
        assert_eq!(
            user(&json!({"id":"u","display_name":null}))
                .unwrap()
                .display_name,
            "u"
        );
    }
}
