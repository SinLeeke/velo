//! Domain types → model rows (JSON objects whose keys are the QML role names).
//! Pure functions; images start empty and are filled asynchronously with `file:///` URLs.

use std::path::Path;

use serde_json::{Value, json};
use velo_domain::{
    Album, Artist, Device, ImageRef, LyricLine, Lyrics, Playlist, Track, pick_image,
};

pub const TRACK_ROLES: &str =
    "trackId,uri,title,artist,album,durationMs,imageUrl,liked,artistId,albumId,addedLabel";
pub const PLAYLIST_ROLES: &str =
    "playlistId,name,owner,imageUrl,trackCount,pinned,owned,description";
pub const ARTIST_ROLES: &str = "artistId,name,imageUrl";
pub const ALBUM_ROLES: &str = "albumId,name,artist,imageUrl,year";
pub const DEVICE_ROLES: &str = "deviceId,name,kind,isActive";
pub const LYRIC_ROLES: &str = "text,timeMs";

/// Thumbnail edge (px) for list rows and cards; 2x the largest row image at 100 % scale.
pub const THUMB_PX: u32 = 128;

pub fn track_row(t: &Track, liked: bool) -> Value {
    track_row_at(t, liked, now_ms())
}

/// [`track_row`] with an explicit "now" (Unix ms) for the relative `addedLabel`.
pub fn track_row_at(t: &Track, liked: bool, now_ms: i64) -> Value {
    json!({
        "trackId": t.id.clone().unwrap_or_default(),
        "uri": t.uri,
        "title": t.title,
        "artist": t.artist_line(),
        "album": t.album,
        "durationMs": t.duration_ms,
        "imageUrl": "",
        "liked": liked,
        "artistId": t.artists.first().and_then(|a| a.id.clone()).unwrap_or_default(),
        "albumId": t.album_id.clone().unwrap_or_default(),
        "addedLabel": t.added_at.map(|at| added_label(at, now_ms)).unwrap_or_default(),
    })
}

/// Current Unix time in ms (0 if the clock is before 1970).
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// "Fecha en que se agregó" like the official app: relative for the last four weeks
/// ("hace 2 días"), then the date ("12 sept 2025"). Computed once when the rows are built
/// (no timer refreshes it). Dates are in UTC (no time zone database).
pub fn added_label(added_ms: i64, now_ms: i64) -> String {
    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    let ago = now_ms - added_ms;
    let plural = |n: i64, one: &str, many: &str| {
        if n == 1 {
            format!("hace 1 {one}")
        } else {
            format!("hace {n} {many}")
        }
    };
    if ago < MIN {
        // Includes a slightly fast server clock (a date in the near future).
        "hace un momento".into()
    } else if ago < HOUR {
        plural(ago / MIN, "minuto", "minutos")
    } else if ago < DAY {
        plural(ago / HOUR, "hora", "horas")
    } else if ago < 7 * DAY {
        plural(ago / DAY, "día", "días")
    } else if ago < 29 * DAY {
        plural(ago / (7 * DAY), "semana", "semanas")
    } else {
        const MONTHS: [&str; 12] = [
            "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
        ];
        let (y, m, d) = civil_from_days(added_ms.div_euclid(DAY));
        format!("{d} {} {y}", MONTHS[(m - 1) as usize])
    }
}

/// Proleptic Gregorian (year, month 1-12, day) of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `user_id`: the signed-in account; `owned` (M1b) says whether the menu may edit or delete it.
pub fn playlist_row(p: &Playlist, pinned: bool, user_id: &str) -> Value {
    json!({
        "playlistId": p.id,
        "name": p.name,
        "owner": p.owner,
        "imageUrl": "",
        "trackCount": p.track_count,
        "pinned": pinned,
        "owned": !user_id.is_empty() && p.owner_id == user_id,
        "description": plain_text(&p.description),
    })
}

/// Spotify returns playlist descriptions HTML-escaped (`&#x27;`, `&amp;`…). QML shows them as
/// plain text and "Editar detalles" sends them back, so the entities are decoded once here.
pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let c = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn artist_row(a: &Artist) -> Value {
    json!({ "artistId": a.id, "name": a.name, "imageUrl": "" })
}

pub fn album_row(a: &Album) -> Value {
    json!({
        "albumId": a.id,
        "name": a.name,
        "artist": a.artists.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", "),
        "imageUrl": "",
        "year": release_year(&a.release_date),
    })
}

pub fn device_row(d: &Device) -> Value {
    json!({
        "deviceId": d.id.clone().unwrap_or_default(),
        "name": d.name,
        "kind": d.kind,
        "isActive": d.is_active,
    })
}

pub fn lyric_rows(lines: &[LyricLine]) -> Vec<Value> {
    lines
        .iter()
        .map(|l| json!({ "text": l.text, "timeMs": l.time_ms }))
        .collect()
}

/// `"2024-05-17"` / `"2024"` → 2024; 0 when unknown.
pub fn release_year(date: &str) -> u32 {
    date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
}

/// URL of the image to downscale for a row (the smallest one that is at least `THUMB_PX`).
pub fn image_of(images: &[ImageRef]) -> Option<String> {
    pick_image(images, THUMB_PX).map(|i| i.url.clone())
}

/// `file:///C:/…` URL for QML; empty when the path cannot be expressed as a URL.
pub fn file_url(path: &Path) -> String {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .unwrap_or_default()
}

/// Lyrics status string of the contract.
pub fn lyrics_status(l: &Lyrics) -> &'static str {
    if l.instrumental {
        "instrumental"
    } else if !l.synced.is_empty() {
        "synced"
    } else if l.plain.as_deref().is_some_and(|p| !p.trim().is_empty()) {
        "plain"
    } else {
        "none"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velo_domain::ArtistRef;

    fn track() -> Track {
        Track {
            id: Some("abc".into()),
            uri: "spotify:track:abc".into(),
            title: "Marea".into(),
            artists: vec![
                ArtistRef {
                    id: None,
                    name: "Lumen".into(),
                },
                ArtistRef {
                    id: None,
                    name: "Iria".into(),
                },
            ],
            album: "Atlas".into(),
            album_id: None,
            duration_ms: 261_000,
            explicit: false,
            images: vec![],
            added_at: None,
        }
    }

    #[test]
    fn track_row_has_every_role() {
        let row = track_row(&track(), true);
        for role in TRACK_ROLES.split(',') {
            assert!(row.get(role).is_some(), "missing {role}");
        }
        assert_eq!(row["artist"], "Lumen, Iria");
        assert_eq!(row["durationMs"], 261_000);
        assert_eq!(row["liked"], true);
        assert_eq!(row["imageUrl"], "");
    }

    fn assert_roles(row: &Value, roles: &str) {
        let expected: std::collections::BTreeSet<_> = roles.split(',').collect();
        let actual: std::collections::BTreeSet<_> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn playlist_and_artist_rows_match_the_contract_without_remote_images() {
        let image = ImageRef {
            url: "https://example.invalid/cover.jpg".into(),
            width: Some(640),
            height: Some(640),
        };
        let p = Playlist {
            id: "p".into(),
            uri: "spotify:playlist:p".into(),
            name: "Mix".into(),
            description: String::new(),
            owner: "Juan".into(),
            owner_id: "juan".into(),
            collaborative: false,
            images: vec![image.clone()],
            track_count: 12,
        };
        let row = playlist_row(&p, true, "juan");
        assert_roles(&row, PLAYLIST_ROLES);
        assert_eq!(row["pinned"], true);
        assert_eq!(row["trackCount"], 12);
        assert_eq!(row["imageUrl"], "");
        assert_eq!(row["owned"], true);
        assert_eq!(playlist_row(&p, false, "otro")["owned"], false);
        // Signed out (no user id yet): nothing is editable.
        let mut nobody = p.clone();
        nobody.owner_id = String::new();
        assert_eq!(playlist_row(&nobody, false, "")["owned"], false);
        let a = Artist {
            id: "a".into(),
            uri: "spotify:artist:a".into(),
            name: "Lumen".into(),
            images: vec![image],
            genres: vec![],
            followers: None,
        };
        let row = artist_row(&a);
        assert_roles(&row, ARTIST_ROLES);
        assert_eq!(row["imageUrl"], "");
        let mut t = track();
        t.id = None;
        assert_eq!(track_row(&t, false)["trackId"], "");
        assert_roles(&track_row(&t, false), TRACK_ROLES);
    }

    #[test]
    fn thumbnail_uses_smallest_sufficient_image_and_escapes_local_urls() {
        let images: Vec<_> = [640, 64, 300]
            .into_iter()
            .map(|width| ImageRef {
                url: width.to_string(),
                width: Some(width),
                height: Some(width),
            })
            .collect();
        assert_eq!(image_of(&images).as_deref(), Some("300"));
        assert_eq!(image_of(&[]), None);
        let p = std::env::temp_dir().join("cover #1.png");
        let u = file_url(&p);
        assert!(u.contains("cover%20%231.png"));
        assert_eq!(url::Url::parse(&u).unwrap().to_file_path().unwrap(), p);
    }

    #[test]
    fn album_and_device_rows_have_every_role() {
        let album = Album {
            id: "al1".into(),
            uri: "spotify:album:al1".into(),
            name: "Atlas".into(),
            artists: vec![ArtistRef {
                id: None,
                name: "Lumen".into(),
            }],
            images: vec![],
            release_date: "2024-05-17".into(),
            total_tracks: 9,
            album_type: "album".into(),
        };
        let row = album_row(&album);
        for role in ALBUM_ROLES.split(',') {
            assert!(row.get(role).is_some(), "missing {role}");
        }
        assert_eq!(row["year"], 2024);
        let dev = Device {
            id: None,
            name: "PC".into(),
            kind: "Computer".into(),
            is_active: true,
            volume_percent: None,
            supports_volume: true,
        };
        let row = device_row(&dev);
        for role in DEVICE_ROLES.split(',') {
            assert!(row.get(role).is_some(), "missing {role}");
        }
    }

    #[test]
    fn track_rows_carry_ids_and_the_added_label() {
        let mut t = track();
        t.artists[0].id = Some("ar".into());
        t.album_id = Some("al".into());
        let now = 1_772_366_400_000; // 2026-03-01T12:00:00Z
        t.added_at = Some(now - 2 * 86_400_000);
        let row = track_row_at(&t, false, now);
        assert_roles(&row, TRACK_ROLES);
        assert_eq!(row["artistId"], "ar");
        assert_eq!(row["albumId"], "al");
        assert_eq!(row["addedLabel"], "hace 2 días");
        t.added_at = None;
        assert_eq!(track_row_at(&t, false, now)["addedLabel"], "");
    }

    #[test]
    fn added_labels_are_relative_for_four_weeks_then_a_date() {
        let now = 1_772_366_400_000; // 2026-03-01T12:00:00Z
        let ago = |ms: i64| added_label(now - ms, now);
        const MIN: i64 = 60_000;
        assert_eq!(ago(10_000), "hace un momento");
        assert_eq!(ago(-5 * MIN), "hace un momento");
        assert_eq!(ago(MIN), "hace 1 minuto");
        assert_eq!(ago(59 * MIN), "hace 59 minutos");
        assert_eq!(ago(60 * MIN), "hace 1 hora");
        assert_eq!(ago(23 * 60 * MIN), "hace 23 horas");
        assert_eq!(ago(24 * 60 * MIN), "hace 1 día");
        assert_eq!(ago(6 * 24 * 60 * MIN), "hace 6 días");
        assert_eq!(ago(7 * 24 * 60 * MIN), "hace 1 semana");
        assert_eq!(ago(28 * 24 * 60 * MIN), "hace 4 semanas");
        assert_eq!(ago(29 * 24 * 60 * MIN), "31 ene 2026");
        // 2025-09-12T08:00:00Z
        assert_eq!(added_label(1_757_664_000_000, now), "12 sept 2025");
        assert_eq!(added_label(951_782_401_000, now), "29 feb 2000");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn year_parsing_is_forgiving() {
        assert_eq!(release_year("1999"), 1999);
        assert_eq!(release_year(""), 0);
        assert_eq!(release_year("xx"), 0);
    }

    #[test]
    fn lyrics_status_follows_the_contract() {
        let mut l = Lyrics::default();
        assert_eq!(lyrics_status(&l), "none");
        l.plain = Some("la la".into());
        assert_eq!(lyrics_status(&l), "plain");
        l.synced = vec![LyricLine {
            time_ms: 1,
            text: "a".into(),
        }];
        assert_eq!(lyrics_status(&l), "synced");
        l.instrumental = true;
        assert_eq!(lyrics_status(&l), "instrumental");
    }

    #[test]
    fn lyric_rows_use_role_names() {
        let rows = lyric_rows(&[LyricLine {
            time_ms: 5,
            text: "hi".into(),
        }]);
        assert_eq!(rows[0]["text"], "hi");
        assert_eq!(rows[0]["timeMs"], 5);
        assert_eq!(LYRIC_ROLES, "text,timeMs");
    }

    #[test]
    fn file_urls_are_absolute() {
        let p = std::env::temp_dir().join("velo-x.jpg");
        assert!(file_url(&p).starts_with("file:///"));
        assert_eq!(file_url(Path::new("relative.jpg")), "");
    }

    #[test]
    fn playlist_descriptions_are_decoded_to_plain_text() {
        assert_eq!(plain_text("Rock &amp; roll"), "Rock & roll");
        assert_eq!(
            plain_text("It&#x27;s &quot;mine&quot; &#47; &lt;3"),
            "It's \"mine\" / <3"
        );
        assert_eq!(
            plain_text("a & b &unknown; &#xZZ; &"),
            "a & b &unknown; &#xZZ; &"
        );
        assert_eq!(plain_text("sin entidades"), "sin entidades");
    }
}
