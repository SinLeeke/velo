//! Pure adapters for the `data` returned by captured pathfinder operations.

use serde_json::Value;
use velo_domain::{
    Album, Artist, ArtistRef, ImageRef, Page, Playlist, PublicProfile, Result, SearchResults, Track,
};

fn malformed() -> velo_domain::VeloError {
    super::super::unavailable()
}

fn profile_images(v: &Value) -> Result<Vec<ImageRef>> {
    let url = text(&v["image_url"])?;
    Ok(if url.starts_with("https://") {
        vec![ImageRef {
            url: url.into(),
            width: None,
            height: None,
        }]
    } else {
        Vec::new()
    })
}

pub fn parse_public_profile(v: &Value, user_id: &str) -> Result<PublicProfile> {
    object(v)?;
    // A missing name is not evidence of a profile (e.g. an upstream error object).
    let name = v["name"].as_str().ok_or_else(malformed)?;
    Ok(PublicProfile {
        id: user_id.into(),
        display_name: if name.is_empty() { user_id } else { name }.into(),
        images: profile_images(v)?,
        followers: number(&v["followers_count"])?,
    })
}

pub fn parse_public_playlists(
    v: &Value,
    user_id: &str,
    offset: u32,
    limit: u32,
) -> Result<Page<Playlist>> {
    object(v)?;
    let rows = array(&v["public_playlists"])?;
    let mut items = Vec::new();
    for row in rows.iter().take(limit as usize) {
        let Ok(playlist_id) = id(row, "playlist") else {
            continue;
        };
        let Some(owner_uri) = row["owner_uri"].as_str() else {
            continue;
        };
        let Some(owner_name) = row["owner_name"].as_str() else {
            continue;
        };
        let owner_id = owner_uri.strip_prefix("spotify:user:").unwrap_or_default();
        // The endpoint lists only public playlists; never guess missing ownership.
        if owner_id != user_id || row["public"] == false {
            continue;
        }
        let parsed = (|| -> Result<Playlist> {
            Ok(Playlist {
                id: playlist_id,
                uri: text(&row["uri"])?.into(),
                name: text(&row["name"])?.into(),
                description: String::new(),
                owner: owner_name.into(),
                owner_id: owner_id.into(),
                collaborative: false,
                images: profile_images(row)?,
                // followers_count is not a song count.
                track_count: count(&row["tracks_count"], 0)?,
            })
        })();
        if let Ok(playlist) = parsed {
            items.push(playlist);
        }
    }
    // The documented payload may omit totals. A full raw page needs one more read;
    // a short page is exhausted, even when ownership filtering removed rows.
    let consumed = u32::try_from(rows.len().min(limit as usize)).unwrap_or(limit);
    let inferred = offset
        .saturating_add(consumed)
        .saturating_add(u32::from(consumed == limit));
    // Page::has_more uses retained rows. Without an explicit total, a short final
    // page terminates even when rows were filtered. Preserve real totals when supplied.
    let inferred = if consumed < limit {
        offset.saturating_add(items.len() as u32)
    } else {
        inferred
    };
    let total = count(&v["total_public_playlists_count"], inferred as usize)?;
    Ok(Page {
        items,
        offset,
        limit,
        total,
    })
}

pub fn parse_public_playlists_prefix(
    mut v: Value,
    user_id: &str,
    offset: u32,
    limit: u32,
) -> Result<Page<Playlist>> {
    let rows = v["public_playlists"].as_array_mut().ok_or_else(malformed)?;
    let start = (offset as usize).min(rows.len());
    rows.drain(..start);
    rows.truncate(limit as usize);
    parse_public_playlists(&v, user_id, offset, limit)
}

fn object(v: &Value) -> Result<&Value> {
    v.as_object().ok_or_else(malformed)?;
    Ok(v)
}

fn optional_objects(v: &Value, fields: &[&str]) -> Result<()> {
    for field in fields {
        if !v[field].is_null() {
            object(&v[field])?;
        }
    }
    Ok(())
}

fn entity(v: &Value, typename: &str, containers: &[&str]) -> Result<()> {
    object(v)?;
    if !v["__typename"].is_null() && text(&v["__typename"])? != typename {
        return Err(malformed());
    }
    optional_objects(v, containers)
}

fn text(v: &Value) -> Result<&str> {
    if v.is_null() {
        Ok("")
    } else {
        v.as_str().ok_or_else(malformed)
    }
}

fn number(v: &Value) -> Result<Option<u64>> {
    if v.is_null() {
        Ok(None)
    } else {
        v.as_u64().map(Some).ok_or_else(malformed)
    }
}

fn count(v: &Value, fallback: usize) -> Result<u32> {
    Ok(number(v)?.unwrap_or(fallback as u64).min(u32::MAX as u64) as u32)
}

fn array(v: &Value) -> Result<&[Value]> {
    v.as_array().map(Vec::as_slice).ok_or_else(malformed)
}

fn id(v: &Value, kind: &str) -> Result<String> {
    let prefix = format!("spotify:{kind}:");
    let id = text(&v["uri"])?
        .strip_prefix(&prefix)
        .ok_or_else(malformed)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(malformed());
    }
    Ok(id.to_owned())
}

fn images(v: &Value) -> Result<Vec<ImageRef>> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    let mut images = Vec::new();
    for source in array(v)? {
        object(source)?;
        let url = text(&source["url"])?;
        let dimension = |field| -> Result<Option<u32>> {
            number(&source[field])?
                .map(|n| n.try_into().map_err(|_| malformed()))
                .transpose()
        };
        let width = dimension("width")?;
        let height = dimension("height")?;
        if url.starts_with("https://")
            && width.is_none_or(|n| n <= 640)
            && height.is_none_or(|n| n <= 640)
        {
            images.push(ImageRef {
                url: url.to_owned(),
                width,
                height,
            });
        }
    }
    // One cover: largest usable source, never request >640 px.
    Ok(images
        .into_iter()
        .max_by_key(|i| i.width.unwrap_or(0))
        .into_iter()
        .collect())
}

fn artist_refs(v: &Value) -> Result<Vec<ArtistRef>> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    array(&v["items"])?
        .iter()
        .map(|a| {
            object(a)?;
            optional_objects(a, &["profile"])?;
            Ok(ArtistRef {
                id: if a["uri"].is_null() {
                    None
                } else {
                    Some(id(a, "artist")?)
                },
                name: text(&a["profile"]["name"])?.to_owned(),
            })
        })
        .collect()
}

fn release_date(v: &Value) -> Result<String> {
    let iso = text(&v["isoString"])?;
    if !iso.is_empty() {
        let date = iso.split('T').next().unwrap_or_default();
        let precision = text(&v["precision"])?;
        return Ok(match precision {
            "YEAR" => date.get(..4),
            "MONTH" => date.get(..7),
            _ => Some(date),
        }
        .unwrap_or_default()
        .to_owned());
    }
    Ok(number(&v["year"])?
        .map(|y| y.to_string())
        .unwrap_or_default())
}

fn album(v: &Value) -> Result<Album> {
    entity(
        v,
        "Album",
        &["artists", "date", "coverArt", "tracksV2", "tracks"],
    )?;
    let tracks = v.get("tracksV2").unwrap_or(&v["tracks"]);
    Ok(Album {
        id: id(v, "album")?,
        uri: text(&v["uri"])?.to_owned(),
        name: text(&v["name"])?.to_owned(),
        artists: artist_refs(&v["artists"])?,
        images: images(&v["coverArt"]["sources"])?,
        release_date: release_date(&v["date"])?,
        total_tracks: count(
            &tracks["totalCount"],
            tracks["items"].as_array().map_or(0, Vec::len),
        )?,
        album_type: text(&v["type"])?.to_ascii_lowercase(),
    })
}

fn artist(v: &Value) -> Result<Artist> {
    entity(v, "Artist", &["profile", "visuals", "stats"])?;
    optional_objects(&v["visuals"], &["avatarImage"])?;
    let genres = if v["genres"].is_null() {
        Vec::new()
    } else {
        array(&v["genres"])?
            .iter()
            .map(|g| text(g).map(str::to_owned))
            .collect::<Result<_>>()?
    };
    Ok(Artist {
        id: id(v, "artist")?,
        uri: text(&v["uri"])?.to_owned(),
        name: text(&v["profile"]["name"])?.to_owned(),
        images: images(&v["visuals"]["avatarImage"]["sources"])?,
        genres,
        followers: v["stats"]["followers"].as_u64(),
    })
}

fn track(v: &Value) -> Result<Track> {
    entity(
        v,
        "Track",
        &["artists", "albumOfTrack", "duration", "contentRating"],
    )?;
    let a = &v["albumOfTrack"];
    optional_objects(a, &["coverArt"])?;
    Ok(Track {
        id: Some(id(v, "track")?),
        uri: text(&v["uri"])?.to_owned(),
        title: text(&v["name"])?.to_owned(),
        artists: artist_refs(&v["artists"])?,
        album: text(&a["name"])?.to_owned(),
        album_id: if a.is_null() {
            None
        } else {
            Some(id(a, "album")?)
        },
        duration_ms: number(&v["duration"]["totalMilliseconds"])?.unwrap_or_default(),
        explicit: text(&v["contentRating"]["label"])? == "EXPLICIT",
        images: images(&a["coverArt"]["sources"])?,
        added_at: None,
    })
}

fn playlist(v: &Value) -> Result<Playlist> {
    entity(v, "Playlist", &["ownerV2", "images", "content"])?;
    optional_objects(&v["ownerV2"], &["data"])?;
    let owner = &v["ownerV2"]["data"];
    let owner_id = text(&owner["username"])?;
    let owner_id = if owner_id.is_empty() {
        text(&owner["uri"])?
            .strip_prefix("spotify:user:")
            .unwrap_or_default()
    } else {
        owner_id
    };
    let name = text(&owner["name"])?;
    let mut sources = Vec::new();
    if !v["images"].is_null() {
        for image in array(&v["images"]["items"])? {
            sources.extend_from_slice(array(&image["sources"])?);
        }
    }
    Ok(Playlist {
        id: id(v, "playlist")?,
        uri: text(&v["uri"])?.to_owned(),
        name: text(&v["name"])?.to_owned(),
        description: text(&v["description"])?.to_owned(),
        owner: if name.is_empty() { owner_id } else { name }.to_owned(),
        owner_id: owner_id.to_owned(),
        collaborative: if v["collaborative"].is_null() {
            false
        } else {
            v["collaborative"].as_bool().ok_or_else(malformed)?
        },
        images: images(&Value::Array(sources))?,
        track_count: count(&v["content"]["totalCount"], 0)?,
    })
}

fn page<T>(v: &Value, parse: impl Fn(&Value) -> Result<T>) -> Result<Page<T>> {
    object(v)?;
    let rows = array(&v["items"])?;
    Ok(Page {
        items: rows.iter().map(parse).collect::<Result<_>>()?,
        offset: 0,
        limit: count(&Value::Null, rows.len())?,
        total: count(&v["totalCount"], rows.len())?,
    })
}

pub(super) fn parse_search(v: &Value) -> Result<SearchResults> {
    let s = object(&v["searchV2"])?;
    Ok(SearchResults {
        tracks: page(&s["tracksV2"], |i| track(&i["item"]["data"]))?,
        albums: page(&s["albumsV2"], |i| album(&i["data"]))?,
        artists: page(&s["artists"], |i| artist(&i["data"]))?,
        playlists: page(&s["playlists"], |i| playlist(&i["data"]))?,
    })
}

pub(super) fn parse_album(v: &Value) -> Result<Album> {
    album(&v["albumUnion"])
}

pub(super) fn parse_album_tracks(v: &Value) -> Result<Page<Track>> {
    let a = parse_album(v)?;
    page(&v["albumUnion"]["tracksV2"], |i| {
        let mut t = track(&i["track"])?;
        t.album.clone_from(&a.name);
        t.album_id = Some(a.id.clone());
        t.images.clone_from(&a.images);
        Ok(t)
    })
}

pub(super) fn parse_artist(v: &Value) -> Result<Artist> {
    artist(&v["artistUnion"])
}

/// Pure F6 adapter. Optional fields fail independently: missing/bad counts are
/// unknown (-1), never zero; city order is the upstream order.
pub fn artist_overview_value(v: &Value) -> Result<super::ArtistOverview> {
    let artist = parse_artist(v)?;
    let a = &v["artistUnion"];
    let known_count = |v: &Value| {
        v.as_u64()
            .map(|n| n.min(i32::MAX as u64) as i32)
            .unwrap_or(-1)
    };
    let top_cities = a["stats"]["topCities"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .take(5)
        .filter_map(|row| {
            let city = row["city"].as_str().filter(|s| !s.is_empty())?;
            Some(serde_json::json!({"city": city,
                "country": row["country"].as_str().unwrap_or_default(),
                "listeners": known_count(&row["numberOfListeners"])}))
        })
        .collect();
    let header_image_url = a["headerImage"]["data"]["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| {
            s["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("https://"))
        })
        .min_by_key(|s| s["maxWidth"].as_u64().unwrap_or(u64::MAX))
        .and_then(|s| s["url"].as_str())
        .unwrap_or_default()
        .to_owned();
    Ok(super::ArtistOverview {
        followers: artist
            .followers
            .map(|n| n.min(i32::MAX as u64) as i32)
            .unwrap_or(-1),
        artist,
        monthly_listeners: known_count(&a["stats"]["monthlyListeners"]),
        verified: a["onPlatformReputationTrait"]["verification"]["isVerified"]
            .as_bool()
            .unwrap_or(false),
        top_cities,
        biography: biography_plain_text(
            a["profile"]["biography"]["text"]
                .as_str()
                .unwrap_or_default(),
        ),
        header_image_url,
    })
}

/// Strip markup before entity decoding, so escaped '<' remains literal text.
/// Paragraphs and existing line breaks survive; script/style contents are omitted.
pub fn biography_plain_text(html: &str) -> String {
    let mut plain = String::new();
    let mut rest = html;
    let mut suppressed: Option<String> = None;
    while let Some(start) = rest.find('<') {
        if suppressed.is_none() {
            plain.push_str(&rest[..start]);
        }
        rest = &rest[start + 1..];
        if !rest.starts_with(['/', '!', '?'])
            && !rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        {
            if suppressed.is_none() {
                plain.push('<');
            }
            continue;
        }
        if rest.starts_with("!--") {
            let Some(end) = rest.find("-->") else {
                rest = "";
                break;
            };
            rest = &rest[end + 3..];
            continue;
        }
        // Respect quoted '>' in tag attributes.
        let mut quote = None;
        let end = rest.char_indices().find_map(|(i, c)| {
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if c == '\'' || c == '"' {
                quote = Some(c);
            } else if c == '>' {
                return Some(i);
            }
            None
        });
        let Some(end) = end else {
            rest = "";
            break;
        };
        let tag = rest[..end].trim().to_ascii_lowercase();
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default();
        if matches!(name, "script" | "style") {
            if tag.starts_with('/') {
                if suppressed.as_deref() == Some(name) {
                    suppressed = None;
                }
            } else {
                suppressed = Some(name.into());
            }
        } else if suppressed.is_none() {
            if name == "br" {
                plain.push('\n');
            } else if matches!(name, "p" | "div" | "li")
                && !plain.is_empty()
                && !plain.ends_with("\n\n")
            {
                if !plain.ends_with('\n') {
                    plain.push('\n');
                }
                plain.push('\n');
            }
        }
        rest = &rest[end + 1..];
    }
    if suppressed.is_none() {
        plain.push_str(rest);
    }
    decode_entities(&plain.replace("\r\n", "\n").replace('\r', "\n"))
        .trim()
        .to_owned()
}

fn decode_entities(mut text: &str) -> String {
    let mut decoded = String::new();
    while let Some(start) = text.find('&') {
        decoded.push_str(&text[..start]);
        text = &text[start..];
        if let Some(end) = text.find(';').filter(|end| *end <= 32) {
            let name = &text[1..end];
            let numeric = name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|n| u32::from_str_radix(n, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|n| n.parse().ok()))
                .and_then(char::from_u32);
            if let Some(c) = numeric {
                decoded.push(c);
                text = &text[end + 1..];
                continue;
            }
            if let Ok(i) = HTML_ENTITIES.binary_search_by_key(&name, |(key, _)| *key) {
                decoded.push_str(HTML_ENTITIES[i].1);
                text = &text[end + 1..];
                continue;
            }
        }
        decoded.push('&');
        text = &text[1..];
    }
    decoded.push_str(text);
    decoded
}

pub(super) fn parse_artist_albums(v: &Value) -> Result<Page<Album>> {
    entity(&v["artistUnion"], "Artist", &["discography"])?;
    // Pagination counts release groups. Keep the first edition of each group, so
    // additional editions do not inflate the page past its offset/total.
    page(&v["artistUnion"]["discography"]["all"], |group| {
        let releases = array(&group["releases"]["items"])?;
        album(releases.first().ok_or_else(malformed)?)
    })
}

// HTML named character references (HTML 4 plus apos), generated from System.Net.WebUtility.
const HTML_ENTITIES: &[(&str, &str)] = &[
    ("AElig", "\u{c6}"),
    ("Aacute", "\u{c1}"),
    ("Acirc", "\u{c2}"),
    ("Agrave", "\u{c0}"),
    ("Alpha", "\u{391}"),
    ("Aring", "\u{c5}"),
    ("Atilde", "\u{c3}"),
    ("Auml", "\u{c4}"),
    ("Beta", "\u{392}"),
    ("Ccedil", "\u{c7}"),
    ("Chi", "\u{3a7}"),
    ("Dagger", "\u{2021}"),
    ("Delta", "\u{394}"),
    ("ETH", "\u{d0}"),
    ("Eacute", "\u{c9}"),
    ("Ecirc", "\u{ca}"),
    ("Egrave", "\u{c8}"),
    ("Epsilon", "\u{395}"),
    ("Eta", "\u{397}"),
    ("Euml", "\u{cb}"),
    ("Gamma", "\u{393}"),
    ("Iacute", "\u{cd}"),
    ("Icirc", "\u{ce}"),
    ("Igrave", "\u{cc}"),
    ("Iota", "\u{399}"),
    ("Iuml", "\u{cf}"),
    ("Kappa", "\u{39a}"),
    ("Lambda", "\u{39b}"),
    ("Mu", "\u{39c}"),
    ("Ntilde", "\u{d1}"),
    ("Nu", "\u{39d}"),
    ("OElig", "\u{152}"),
    ("Oacute", "\u{d3}"),
    ("Ocirc", "\u{d4}"),
    ("Ograve", "\u{d2}"),
    ("Omega", "\u{3a9}"),
    ("Omicron", "\u{39f}"),
    ("Oslash", "\u{d8}"),
    ("Otilde", "\u{d5}"),
    ("Ouml", "\u{d6}"),
    ("Phi", "\u{3a6}"),
    ("Pi", "\u{3a0}"),
    ("Prime", "\u{2033}"),
    ("Psi", "\u{3a8}"),
    ("Rho", "\u{3a1}"),
    ("Scaron", "\u{160}"),
    ("Sigma", "\u{3a3}"),
    ("THORN", "\u{de}"),
    ("Tau", "\u{3a4}"),
    ("Theta", "\u{398}"),
    ("Uacute", "\u{da}"),
    ("Ucirc", "\u{db}"),
    ("Ugrave", "\u{d9}"),
    ("Upsilon", "\u{3a5}"),
    ("Uuml", "\u{dc}"),
    ("Xi", "\u{39e}"),
    ("Yacute", "\u{dd}"),
    ("Yuml", "\u{178}"),
    ("Zeta", "\u{396}"),
    ("aacute", "\u{e1}"),
    ("acirc", "\u{e2}"),
    ("acute", "\u{b4}"),
    ("aelig", "\u{e6}"),
    ("agrave", "\u{e0}"),
    ("alefsym", "\u{2135}"),
    ("alpha", "\u{3b1}"),
    ("amp", "&"),
    ("and", "\u{2227}"),
    ("ang", "\u{2220}"),
    ("apos", "'"),
    ("aring", "\u{e5}"),
    ("asymp", "\u{2248}"),
    ("atilde", "\u{e3}"),
    ("auml", "\u{e4}"),
    ("bdquo", "\u{201e}"),
    ("beta", "\u{3b2}"),
    ("brvbar", "\u{a6}"),
    ("bull", "\u{2022}"),
    ("cap", "\u{2229}"),
    ("ccedil", "\u{e7}"),
    ("cedil", "\u{b8}"),
    ("cent", "\u{a2}"),
    ("chi", "\u{3c7}"),
    ("circ", "\u{2c6}"),
    ("clubs", "\u{2663}"),
    ("cong", "\u{2245}"),
    ("copy", "\u{a9}"),
    ("crarr", "\u{21b5}"),
    ("cup", "\u{222a}"),
    ("curren", "\u{a4}"),
    ("dArr", "\u{21d3}"),
    ("dagger", "\u{2020}"),
    ("darr", "\u{2193}"),
    ("deg", "\u{b0}"),
    ("delta", "\u{3b4}"),
    ("diams", "\u{2666}"),
    ("divide", "\u{f7}"),
    ("eacute", "\u{e9}"),
    ("ecirc", "\u{ea}"),
    ("egrave", "\u{e8}"),
    ("empty", "\u{2205}"),
    ("emsp", "\u{2003}"),
    ("ensp", "\u{2002}"),
    ("epsilon", "\u{3b5}"),
    ("equiv", "\u{2261}"),
    ("eta", "\u{3b7}"),
    ("eth", "\u{f0}"),
    ("euml", "\u{eb}"),
    ("euro", "\u{20ac}"),
    ("exist", "\u{2203}"),
    ("fnof", "\u{192}"),
    ("forall", "\u{2200}"),
    ("frac12", "\u{bd}"),
    ("frac14", "\u{bc}"),
    ("frac34", "\u{be}"),
    ("frasl", "\u{2044}"),
    ("gamma", "\u{3b3}"),
    ("ge", "\u{2265}"),
    ("gt", ">"),
    ("hArr", "\u{21d4}"),
    ("harr", "\u{2194}"),
    ("hearts", "\u{2665}"),
    ("hellip", "\u{2026}"),
    ("iacute", "\u{ed}"),
    ("icirc", "\u{ee}"),
    ("iexcl", "\u{a1}"),
    ("igrave", "\u{ec}"),
    ("image", "\u{2111}"),
    ("infin", "\u{221e}"),
    ("int", "\u{222b}"),
    ("iota", "\u{3b9}"),
    ("iquest", "\u{bf}"),
    ("isin", "\u{2208}"),
    ("iuml", "\u{ef}"),
    ("kappa", "\u{3ba}"),
    ("lArr", "\u{21d0}"),
    ("lambda", "\u{3bb}"),
    ("lang", "\u{2329}"),
    ("laquo", "\u{ab}"),
    ("larr", "\u{2190}"),
    ("lceil", "\u{2308}"),
    ("ldquo", "\u{201c}"),
    ("le", "\u{2264}"),
    ("lfloor", "\u{230a}"),
    ("lowast", "\u{2217}"),
    ("loz", "\u{25ca}"),
    ("lrm", "\u{200e}"),
    ("lsaquo", "\u{2039}"),
    ("lsquo", "\u{2018}"),
    ("lt", "<"),
    ("macr", "\u{af}"),
    ("mdash", "\u{2014}"),
    ("micro", "\u{b5}"),
    ("middot", "\u{b7}"),
    ("minus", "\u{2212}"),
    ("mu", "\u{3bc}"),
    ("nabla", "\u{2207}"),
    ("nbsp", "\u{a0}"),
    ("ndash", "\u{2013}"),
    ("ne", "\u{2260}"),
    ("ni", "\u{220b}"),
    ("not", "\u{ac}"),
    ("notin", "\u{2209}"),
    ("nsub", "\u{2284}"),
    ("ntilde", "\u{f1}"),
    ("nu", "\u{3bd}"),
    ("oacute", "\u{f3}"),
    ("ocirc", "\u{f4}"),
    ("oelig", "\u{153}"),
    ("ograve", "\u{f2}"),
    ("oline", "\u{203e}"),
    ("omega", "\u{3c9}"),
    ("omicron", "\u{3bf}"),
    ("oplus", "\u{2295}"),
    ("or", "\u{2228}"),
    ("ordf", "\u{aa}"),
    ("ordm", "\u{ba}"),
    ("oslash", "\u{f8}"),
    ("otilde", "\u{f5}"),
    ("otimes", "\u{2297}"),
    ("ouml", "\u{f6}"),
    ("para", "\u{b6}"),
    ("part", "\u{2202}"),
    ("permil", "\u{2030}"),
    ("perp", "\u{22a5}"),
    ("phi", "\u{3c6}"),
    ("pi", "\u{3c0}"),
    ("piv", "\u{3d6}"),
    ("plusmn", "\u{b1}"),
    ("pound", "\u{a3}"),
    ("prime", "\u{2032}"),
    ("prod", "\u{220f}"),
    ("prop", "\u{221d}"),
    ("psi", "\u{3c8}"),
    ("quot", "\""),
    ("rArr", "\u{21d2}"),
    ("radic", "\u{221a}"),
    ("rang", "\u{232a}"),
    ("raquo", "\u{bb}"),
    ("rarr", "\u{2192}"),
    ("rceil", "\u{2309}"),
    ("rdquo", "\u{201d}"),
    ("real", "\u{211c}"),
    ("reg", "\u{ae}"),
    ("rfloor", "\u{230b}"),
    ("rho", "\u{3c1}"),
    ("rlm", "\u{200f}"),
    ("rsaquo", "\u{203a}"),
    ("rsquo", "\u{2019}"),
    ("sbquo", "\u{201a}"),
    ("scaron", "\u{161}"),
    ("sdot", "\u{22c5}"),
    ("sect", "\u{a7}"),
    ("shy", "\u{ad}"),
    ("sigma", "\u{3c3}"),
    ("sigmaf", "\u{3c2}"),
    ("sim", "\u{223c}"),
    ("spades", "\u{2660}"),
    ("sub", "\u{2282}"),
    ("sube", "\u{2286}"),
    ("sum", "\u{2211}"),
    ("sup", "\u{2283}"),
    ("sup1", "\u{b9}"),
    ("sup2", "\u{b2}"),
    ("sup3", "\u{b3}"),
    ("supe", "\u{2287}"),
    ("szlig", "\u{df}"),
    ("tau", "\u{3c4}"),
    ("there4", "\u{2234}"),
    ("theta", "\u{3b8}"),
    ("thetasym", "\u{3d1}"),
    ("thinsp", "\u{2009}"),
    ("thorn", "\u{fe}"),
    ("tilde", "\u{2dc}"),
    ("times", "\u{d7}"),
    ("trade", "\u{2122}"),
    ("uArr", "\u{21d1}"),
    ("uacute", "\u{fa}"),
    ("uarr", "\u{2191}"),
    ("ucirc", "\u{fb}"),
    ("ugrave", "\u{f9}"),
    ("uml", "\u{a8}"),
    ("upsih", "\u{3d2}"),
    ("upsilon", "\u{3c5}"),
    ("uuml", "\u{fc}"),
    ("weierp", "\u{2118}"),
    ("xi", "\u{3be}"),
    ("yacute", "\u{fd}"),
    ("yen", "\u{a5}"),
    ("yuml", "\u{ff}"),
    ("zeta", "\u{3b6}"),
    ("zwj", "\u{200d}"),
    ("zwnj", "\u{200c}"),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artist_overview_preserves_real_statistics_and_complete_biography() {
        let v = data(ARTIST);
        let a = artist_overview_value(&v).unwrap();
        assert_eq!(
            (a.monthly_listeners, a.followers, a.verified),
            (51691719, 58691539, true)
        );
        assert_eq!(a.top_cities.len(), 5);
        assert_eq!(
            a.top_cities[0],
            json!({"city":"Mexico City","country":"MX","listeners":1075446})
        );
        assert_eq!(a.top_cities[3]["city"], "Santiago");
        // Independent strip/decode of the captured text has 11,068 bytes; no truncation.
        assert_eq!(a.biography.len(), 11068);
        assert!(!a.biography.contains("<a"));
        assert!(a.biography.ends_with("~ Stephen Thomas Erlewine"));
        assert!(a.biography.contains("\n\n"));
        assert_eq!(
            a.header_image_url,
            "https://image-cdn-fa.spotifycdn.com/image/ab6761860000660e25119771314a74f431bd3bb1"
        );
    }

    #[test]
    fn missing_artist_fields_stay_unknown_and_cities_are_bounded_in_order() {
        let mut v = data(ARTIST);
        v["artistUnion"]["stats"] = json!({"topCities":{"items":(0..8).map(|i| json!({"city":format!("City {i}")})).collect::<Vec<_>>()}});
        v["artistUnion"]["profile"]["biography"] = Value::Null;
        v["artistUnion"]["headerImage"] = Value::Null;
        v["artistUnion"]["onPlatformReputationTrait"] = Value::Null;
        let a = artist_overview_value(&v).unwrap();
        assert_eq!(
            (a.monthly_listeners, a.followers, a.verified),
            (-1, -1, false)
        );
        assert!(a.biography.is_empty() && a.header_image_url.is_empty());
        assert_eq!(a.top_cities.len(), 5);
        assert_eq!(a.top_cities[4]["city"], "City 4");
        assert_eq!(a.top_cities[0]["listeners"], -1);
        v["artistUnion"]["stats"]["monthlyListeners"] = json!(u64::MAX);
        assert_eq!(
            artist_overview_value(&v).unwrap().monthly_listeners,
            i32::MAX
        );
        assert!(artist_overview_value(&json!({"artistUnion":{"__typename":"NotFound"}})).is_err());
    }

    #[test]
    fn malformed_optional_counts_do_not_discard_biography_or_other_statistics() {
        let mut v = data(ARTIST);
        v["artistUnion"]["stats"]["followers"] = json!("unknown");
        v["artistUnion"]["stats"]["topCities"]["items"][0]["numberOfListeners"] = json!(-1);
        let a = artist_overview_value(&v).unwrap();
        assert_eq!(a.followers, -1);
        assert_eq!(a.monthly_listeners, 51691719);
        assert_eq!(a.top_cities[0]["listeners"], -1);
        assert_eq!(a.biography.len(), 11068);
    }

    #[test]
    fn biography_strips_markup_decodes_entities_and_keeps_paragraphs() {
        assert_eq!(
            biography_plain_text(
                "<p>A &amp; B&nbsp;&eacute; &#x1F3B5;</p><p>Next<br/>line &#39; &quot;x&quot; &lt;img&gt;</p><script>bad()</script><style>bad</style>"
            ),
            "A & B\u{a0}é 🎵\n\nNext\nline ' \"x\" <img>"
        );
        assert_eq!(
            biography_plain_text("<a href='x>y'>all</a>\r\n\r\nlast &apos; &unknown;"),
            "all\n\nlast ' &unknown;"
        );
        assert_eq!(biography_plain_text("<!--ignore--><script>unfinished"), "");
        assert_eq!(
            biography_plain_text("Plain 2 < 3, all words remain."),
            "Plain 2 < 3, all words remain."
        );
        assert!(HTML_ENTITIES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }
    #[test]
    fn synthetic_spclient_profile_and_public_playlists() {
        // Synthetic documented shape; the leader will replace it with a real probe capture.
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/spclient/profile.json"))
                .unwrap();
        let p = parse_public_profile(&v, "fixture.user").unwrap();
        assert_eq!(
            (p.display_name.as_str(), p.followers),
            ("Fixture Listener", Some(42))
        );
        assert_eq!(p.images.len(), 1);
        let page = parse_public_playlists(&v, "fixture.user", 0, 3).unwrap();
        assert_eq!(
            page.items.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["fixtureA", "fixtureB"]
        );
        assert_eq!(page.items[0].track_count, 0);
        assert_eq!(page.items[0].owner_id, "fixture.user");
        assert!(page.has_more());
        let minimal = serde_json::json!({"name":"Fixture", "public_playlists":[]});
        assert_eq!(
            parse_public_profile(&minimal, "fixture").unwrap().followers,
            None
        );
        assert!(
            !parse_public_playlists(&minimal, "fixture", 50, 50)
                .unwrap()
                .has_more()
        );
        assert!(parse_public_profile(&serde_json::json!({"error":404}), "fixture").is_err());
        assert!(parse_public_playlists(&serde_json::json!({}), "fixture", 0, 50).is_err());
    }

    #[test]
    fn public_playlist_pagination_uses_raw_page_and_rejects_bad_shapes() {
        let row = serde_json::json!({"uri":"spotify:playlist:p", "owner_uri":"spotify:user:u", "owner_name":"User"});
        let full = serde_json::json!({"public_playlists":[row.clone(), row.clone()]});
        let page = parse_public_playlists(&full, "u", 8, 2).unwrap();
        assert_eq!((page.offset, page.limit, page.total), (8, 2, 11));
        assert!(page.has_more());
        let short = serde_json::json!({"public_playlists":[row]});
        assert!(
            !parse_public_playlists(&short, "u", 10, 2)
                .unwrap()
                .has_more()
        );
        assert!(
            !parse_public_playlists(&short, "other", 10, 2)
                .unwrap()
                .has_more()
        );
        assert!(parse_public_playlists(&json!({"public_playlists":null}), "u", 0, 2).is_err());
    }

    #[test]
    fn public_playlist_bad_rows_do_not_discard_neighbors_or_raw_pagination() {
        let good =
            json!({"uri":"spotify:playlist:p", "owner_uri":"spotify:user:u", "owner_name":"U"});
        let mut no_owner = good.clone();
        no_owner.as_object_mut().unwrap().remove("owner_name");
        let value = json!({"public_playlists":[good.clone(), {"uri":"spotify:track:wrong"}, no_owner, 7, good]});
        let page = parse_public_playlists(&value, "u", 10, 5).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!((page.offset, page.limit, page.total), (10, 5, 16));
        assert!(page.has_more());
        let bad = parse_public_playlists(&json!({"public_playlists":[{"uri":"wrong"}]}), "u", 0, 2)
            .unwrap();
        assert!(bad.items.is_empty() && !bad.has_more());
    }

    #[test]
    fn profile_playlist_prefix_applies_offset_once_and_keeps_total() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/spclient/profile.json"))
                .unwrap();
        let page = parse_public_playlists_prefix(v.clone(), "fixture.user", 1, 1).unwrap();
        assert_eq!(page.items[0].id, "fixtureB");
        assert_eq!((page.offset, page.limit, page.total), (1, 1, 3));
        let end = parse_public_playlists_prefix(v, "fixture.user", 3, 2).unwrap();
        assert!(end.items.is_empty() && !end.has_more());
    }
    use serde_json::json;

    const SEARCH: &str = include_str!("../../tests/fixtures/pathfinder/search.json");
    const ALBUM: &str = include_str!("../../tests/fixtures/pathfinder/album.json");
    const ARTIST: &str = include_str!("../../tests/fixtures/pathfinder/artist.json");
    const DISCOGRAPHY: &str =
        include_str!("../../tests/fixtures/pathfinder/artist_discography.json");

    fn data(fixture: &str) -> Value {
        serde_json::from_str::<Value>(fixture).unwrap()["data"].clone()
    }

    #[test]
    fn search_fixture_maps_all_four_categories() {
        let s = parse_search(&data(SEARCH)).unwrap();
        assert_eq!((s.tracks.items.len(), s.tracks.total), (10, 15));
        assert_eq!((s.albums.items.len(), s.albums.total), (5, 20));
        assert_eq!((s.artists.items.len(), s.artists.total), (10, 17));
        assert_eq!((s.playlists.items.len(), s.playlists.total), (10, 14));
        let t = &s.tracks.items[0];
        assert_eq!(t.title, "Bohemian Rhapsody");
        assert_eq!(t.id.as_deref(), Some("2JiDi0qAXsPwhPqA2qaKGt"));
        assert_eq!(t.artist_line(), "Queen");
        assert_eq!(t.album, "A Night At The Opera");
        assert_eq!(t.album_id.as_deref(), Some("1TkbyIkf6GSrO5e7gWS4AM"));
        assert_eq!(t.duration_ms, 355154);
        assert_eq!(
            t.images[0].url,
            "https://i.scdn.co/image/ab67616d0000b273fdab4a163ab9f6db72c952ee"
        );
        assert_eq!(s.albums.items[0].name, "Sheer Heart Attack");
        assert_eq!(s.albums.items[0].release_date, "1974");
        assert_eq!(
            s.albums.items[0].images[0].url,
            "https://i.scdn.co/image/ab67616d0000b2734d09b89253dcbbe1d944e198"
        );
        assert_eq!(s.artists.items[0].name, "Queen");
        assert_eq!(s.artists.items[0].followers, None);
        assert_eq!(
            s.artists.items[0].images[0].url,
            "https://i.scdn.co/image/ab6761610000e5eb73e4d22612ac8d944b5789b4"
        );
        let p = &s.playlists.items[0];
        assert_eq!(p.name, "This Is Queen");
        assert_eq!((&*p.owner, &*p.owner_id), ("Spotify", "spotify"));
        assert_eq!(
            p.images[0].url,
            "https://pickasso.spotifycdn.com/image/ab67c0de0000deef/dt/v1/img/thisisv3/1dfeR4HaWDbWqFHLkxsg1d/en"
        );
    }

    #[test]
    fn album_fixture_maps_metadata_and_art() {
        let a = parse_album(&data(ALBUM)).unwrap();
        assert_eq!(a.name, "Bohemian Rhapsody (The Original Soundtrack)");
        assert_eq!(a.id, "6i6folBtxKV28WX3msQ4FE");
        assert_eq!(a.uri, "spotify:album:6i6folBtxKV28WX3msQ4FE");
        assert_eq!(a.total_tracks, 22);
        assert_eq!(a.release_date, "2018-10-19");
        assert_eq!(a.album_type, "compilation");
        assert_eq!(a.artists[0].name, "Queen");
        assert_eq!(
            a.images[0].url,
            "https://i.scdn.co/image/ab67616d0000b273e8b066f70c206551210d902b"
        );
        assert_eq!(a.images[0].width, Some(640));
    }

    #[test]
    fn album_tracks_fixture_inherits_album_without_losing_tracks() {
        let v = data(ALBUM);
        let a = parse_album(&v).unwrap();
        let p = parse_album_tracks(&v).unwrap();
        assert_eq!((p.items.len(), p.total, p.offset, p.limit), (22, 22, 0, 22));
        assert_eq!(p.items[0].title, "20th Century Fox Fanfare");
        assert_eq!(p.items[0].duration_ms, 25186);
        assert_eq!(p.items[0].id.as_deref(), Some("3YM9o7X3ar7Dm39zXadQSz"));
        assert_eq!(p.items[2].artist_line(), "Smile");
        assert!(p.items.iter().all(|t| t.album == a.name
            && t.album_id.as_ref() == Some(&a.id)
            && t.images == a.images
            && t.added_at.is_none()));
    }

    #[test]
    fn artist_fixture_preserves_real_followers() {
        let a = parse_artist(&data(ARTIST)).unwrap();
        assert_eq!(a.name, "Queen");
        assert_eq!(a.id, "1dfeR4HaWDbWqFHLkxsg1d");
        assert_eq!(a.followers, Some(58691539));
        assert!(a.genres.is_empty());
        assert_eq!(
            a.images[0].url,
            "https://i.scdn.co/image/ab6761610000e5eb73e4d22612ac8d944b5789b4"
        );
    }

    #[test]
    fn discography_fixture_keeps_release_group_total() {
        let p = parse_artist_albums(&data(DISCOGRAPHY)).unwrap();
        assert_eq!((p.items.len(), p.total, p.offset, p.limit), (20, 88, 0, 20));
        assert!(p.has_more());
        let a = &p.items[0];
        assert_eq!(a.name, "Don't Stop Me Now / In Only Seven Days");
        assert_eq!(a.id, "1LTJ0KNMXcbQnewKg8P4qf");
        assert_eq!(a.total_tracks, 2);
        assert_eq!(a.album_type, "single");
        assert_eq!(a.release_date, "2026-09-25");
        assert_eq!(
            a.images[0].url,
            "https://i.scdn.co/image/ab67616d0000b273d1567333c2b5d84544c53518"
        );
        let mut v = data(DISCOGRAPHY);
        let duplicate =
            v["artistUnion"]["discography"]["all"]["items"][0]["releases"]["items"][0].clone();
        v["artistUnion"]["discography"]["all"]["items"][0]["releases"]["items"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert_eq!(parse_artist_albums(&v).unwrap(), p);
    }

    #[test]
    fn malformed_roots_and_rows_fail_for_web_fallback() {
        for v in [
            Value::Null,
            json!({}),
            json!([]),
            json!({"albumUnion": {"__typename":"NotFound"}}),
        ] {
            assert!(parse_album(&v).is_err());
            assert!(parse_album_tracks(&v).is_err());
            assert!(parse_artist(&v).is_err());
            assert!(parse_artist_albums(&v).is_err());
            assert!(parse_search(&v).is_err());
        }
        let mut v = data(SEARCH);
        v["searchV2"]["tracksV2"]["items"][0]["item"]["data"] = Value::Null;
        assert!(parse_search(&v).is_err());
        let mut v = data(ALBUM);
        v["albumUnion"]["tracksV2"]["items"] = json!({});
        assert!(parse_album_tracks(&v).is_err());
        let mut v = data(DISCOGRAPHY);
        v["artistUnion"]["discography"]["all"]["totalCount"] = json!("88");
        assert!(parse_artist_albums(&v).is_err());
        let mut v = data(ARTIST);
        v["artistUnion"]["uri"] = json!("spotify:album:wrongKind");
        assert!(parse_artist(&v).is_err());
    }

    #[test]
    fn missing_optional_fields_have_honest_defaults() {
        let a = parse_album(&json!({"albumUnion":{"uri":"spotify:album:abc"}})).unwrap();
        assert!(a.name.is_empty() && a.artists.is_empty() && a.images.is_empty());
        assert_eq!(a.total_tracks, 0);
        let a = parse_artist(&json!({"artistUnion":{"uri":"spotify:artist:abc"}})).unwrap();
        assert_eq!(a.followers, None);
        let t = track(&json!({"uri":"spotify:track:abc","contentRating":{"label":"EXPLICIT"}}))
            .unwrap();
        assert!(t.explicit);
        assert_eq!(t.duration_ms, 0);
        assert_eq!(t.album_id, None);
        assert_eq!(
            release_date(&json!({"isoString":"2018-10-19T00:00:00Z","precision":"MONTH"})).unwrap(),
            "2018-10"
        );
    }

    #[test]
    fn covers_require_https_and_both_dimensions_at_most_640() {
        let v = json!([
            {"url":"http://unsafe","width":640,"height":640},
            {"url":"https://large","width":900,"height":900},
            {"url":"https://tall","width":640,"height":800},
            {"url":"https://small","width":300,"height":300},
            {"url":"https://chosen","width":640,"height":640}
        ]);
        assert_eq!(images(&v).unwrap()[0].url, "https://chosen");
        assert!(images(&json!({})).is_err());
        assert!(images(&json!([{"url":"https://bad","width":"640"}])).is_err());
        assert!(images(&json!([{"url":"https://overflow","width":u64::MAX}])).is_err());
    }

    #[test]
    fn wrong_optional_container_types_and_union_variants_are_errors() {
        let mut v = data(ALBUM);
        v["albumUnion"]["date"] = json!([]);
        assert!(parse_album(&v).is_err());
        let mut v = data(ARTIST);
        v["artistUnion"]["visuals"]["avatarImage"] = json!(42);
        assert!(parse_artist(&v).is_err());
        let mut v = data(SEARCH);
        v["searchV2"]["tracksV2"]["items"][0]["item"]["data"]["__typename"] = json!("Episode");
        assert!(parse_search(&v).is_err());
        let mut v = data(ALBUM);
        v["albumUnion"]["tracksV2"]["items"][0]["track"]["duration"] = json!("invalid");
        assert!(parse_album_tracks(&v).is_err());
    }
}
