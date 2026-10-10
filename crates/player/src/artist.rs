//! F6: perfil de artista desde ARTIST_V4 de la sesión de audio. Metadatos, nunca audio.
//! Los campos ausentes permanecen ausentes; no se infieren estadísticas de popularidad.
use std::{collections::HashSet, time::Duration};

use librespot_core::SpotifyUri;
use librespot_metadata::image::{Image, ImageSize};
use librespot_metadata::{Album as SpAlbum, Artist as SpArtist, Metadata, Track as SpTrack};
use velo_domain::{Album, Artist, ArtistRef, ImageRef, Track};

use crate::Engine;

const DEADLINE: Duration = Duration::from_secs(55);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
pub const RELEASE_LIMIT: usize = 50;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArtistError {
    #[error("URI de artista inválida")]
    InvalidUri,
    #[error("Spotify no entregó el perfil de artista")]
    Unavailable,
    #[error("El perfil de artista tardó demasiado")]
    Timeout,
}

#[derive(Debug, Clone, Default)]
pub struct ArtistProfile {
    pub biography: String,
    pub photos: Vec<ImageRef>,
    pub popular: Vec<Track>,
    pub related: Vec<Artist>,
    pub discography: Vec<Album>,
    pub appears_on: Vec<Album>,
    pub incomplete: bool,
    // ARTIST_V4 no contiene oyentes mensuales, ciudades ni verificación.
}

/// Solo la selección por país de Spotify (o la global); nunca un país arbitrario.
pub fn popular_uris(artist: &SpArtist, country: &str) -> Vec<SpotifyUri> {
    artist
        .top_tracks
        .for_country(country)
        .iter()
        .take(10)
        .cloned()
        .collect()
}

pub fn validate_artist_uri(uri: &str) -> Result<SpotifyUri, ArtistError> {
    crate::discover::validate_seed(uri).map_err(|_| ArtistError::InvalidUri)?;
    let parsed = SpotifyUri::from_uri(uri).map_err(|_| ArtistError::InvalidUri)?;
    if matches!(parsed, SpotifyUri::Artist { .. }) {
        Ok(parsed)
    } else {
        Err(ArtistError::InvalidUri)
    }
}

/// Conversión pura de los campos inline de ARTIST_V4, sin solicitudes adicionales.
pub fn profile_metadata(artist: &SpArtist) -> ArtistProfile {
    let mut out = ArtistProfile {
        biography: artist
            .biographies
            .iter()
            .find(|b| !b.text.is_empty())
            .map(|b| b.text.clone())
            .unwrap_or_default(),
        // Each portrait group is ONE photo in several sizes (different file ids): keep its
        // largest. `portraits` repeats those photos at one size, so it only counts without groups.
        photos: artist
            .biographies
            .iter()
            .flat_map(|b| -> Vec<&Image> {
                if b.portrait_group.is_empty() {
                    b.portraits.iter().collect()
                } else {
                    b.portrait_group
                        .iter()
                        .filter_map(|g| g.iter().max_by_key(|i| (size_rank(i.size), i.width)))
                        .collect()
                }
            })
            .filter_map(image_ref)
            .collect(),
        related: artist
            .related
            .iter()
            .take(24)
            .filter_map(domain_artist)
            .collect(),
        incomplete: artist.related.len() > 24,
        ..ArtistProfile::default()
    };
    let mut photo_urls = HashSet::new();
    out.photos.retain(|p| photo_urls.insert(p.url.clone()));
    out
}

pub async fn profile(engine: &Engine, uri: &str) -> Result<ArtistProfile, ArtistError> {
    let parsed = validate_artist_uri(uri)?;
    tokio::time::timeout(DEADLINE, async {
        let session = engine.session();
        let artist = tokio::time::timeout(REQUEST_TIMEOUT, SpArtist::get(session, &parsed))
            .await
            .map_err(|_| ArtistError::Timeout)?
            .map_err(|_| ArtistError::Unavailable)?;
        let mut out = profile_metadata(&artist);
        let popular = popular_uris(&artist, &session.country());
        let mut requests = tokio::task::JoinSet::new();
        // Only eight requests active at once, across tracks and releases.
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
        for (index, uri) in popular.into_iter().enumerate() {
            let session = session.clone();
            let semaphore = semaphore.clone();
            requests.spawn(async move {
                let _permit = semaphore.acquire_owned().await.ok()?;
                let t = tokio::time::timeout(REQUEST_TIMEOUT, SpTrack::get(&session, &uri))
                    .await
                    .ok()?
                    .ok()?;
                Some((index, domain_track(&t)?))
            });
        }
        let mut tracks = Vec::new();
        while let Some(result) = requests.join_next().await {
            if let Ok(Some(t)) = result {
                tracks.push(t);
            } else {
                out.incomplete = true;
            }
        }
        tracks.sort_by_key(|(i, _)| *i);
        out.popular = tracks.into_iter().map(|(_, t)| t).collect();
        let groups = [
            (&artist.albums, "album", false),
            (&artist.singles, "single", false),
            (&artist.compilations, "compilation", false),
            (&artist.appears_on_albums, "appears_on", true),
        ];
        let mut requests = tokio::task::JoinSet::new();
        for (group_index, (group, kind, appears)) in groups.into_iter().enumerate() {
            if group.len() > RELEASE_LIMIT {
                out.incomplete = true;
            }
            for (index, uri) in group
                .current_releases()
                .take(RELEASE_LIMIT)
                .cloned()
                .enumerate()
            {
                let session = session.clone();
                let semaphore = semaphore.clone();
                requests.spawn(async move {
                    let _permit = semaphore.acquire_owned().await.ok()?;
                    let album = tokio::time::timeout(REQUEST_TIMEOUT, SpAlbum::get(&session, &uri))
                        .await
                        .ok()?
                        .ok()?;
                    Some((
                        group_index * RELEASE_LIMIT + index,
                        appears,
                        domain_album(&album, kind)?,
                    ))
                });
            }
        }
        let mut albums = Vec::new();
        let budget = tokio::time::Instant::now() + Duration::from_secs(25);
        loop {
            match tokio::time::timeout_at(budget, requests.join_next()).await {
                Ok(Some(Ok(Some(album)))) => albums.push(album),
                Ok(Some(_)) => out.incomplete = true,
                Ok(None) => break,
                Err(_) => {
                    out.incomplete = true;
                    requests.abort_all();
                    break;
                }
            }
        }
        albums.sort_by_key(|(i, _, _)| *i);
        let mut seen = HashSet::new();
        for (_, appears, album) in albums {
            if !seen.insert((appears, album.id.clone())) {
                continue;
            }
            if appears {
                out.appears_on.push(album);
            } else {
                out.discography.push(album);
            }
        }
        Ok(out)
    })
    .await
    .map_err(|_| ArtistError::Timeout)?
}

fn size_rank(size: ImageSize) -> u8 {
    match size {
        ImageSize::SMALL => 0,
        ImageSize::DEFAULT => 1,
        ImageSize::LARGE => 2,
        ImageSize::XLARGE => 3,
    }
}

pub fn image_ref(image: &Image) -> Option<ImageRef> {
    let edge = match image.size {
        ImageSize::SMALL => 64,
        ImageSize::DEFAULT => 300,
        ImageSize::LARGE | ImageSize::XLARGE => 640,
    };
    Some(ImageRef {
        url: format!("https://i.scdn.co/image/{}", image.id.to_base16().ok()?),
        width: Some(
            u32::try_from(image.width)
                .ok()
                .filter(|n| *n > 0)
                .unwrap_or(edge),
        ),
        height: Some(
            u32::try_from(image.height)
                .ok()
                .filter(|n| *n > 0)
                .unwrap_or(edge),
        ),
    })
}

fn unique_images<'a>(images: impl Iterator<Item = &'a Image>) -> Vec<ImageRef> {
    let mut seen = HashSet::new();
    images
        .filter_map(image_ref)
        .filter(|i| seen.insert(i.url.clone()))
        .collect()
}

pub fn domain_artist(a: &SpArtist) -> Option<Artist> {
    Some(Artist {
        id: a.id.to_id().ok()?,
        uri: a.id.to_uri().ok()?,
        name: a.name.clone(),
        images: unique_images(a.portrait_group.iter().chain(a.portraits.iter())),
        genres: vec![],
        followers: None,
    })
}

pub fn domain_album(a: &SpAlbum, kind: &str) -> Option<Album> {
    // Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
    // player.rs:493: ALBUM_V4 distingue EP; no deducirlo del nombre o número de pistas.
    // El filtro «Sencillos y EP» sigue agrupando ambos, pero el rol conserva el tipo real.
    // «Aparece en» es una relación con el artista, no un tipo de lanzamiento.
    let kind = if a.album_type == librespot_metadata::album::AlbumType::EP {
        "ep"
    } else if kind == "appears_on" {
        match a.album_type {
            librespot_metadata::album::AlbumType::SINGLE => "single",
            librespot_metadata::album::AlbumType::COMPILATION => "compilation",
            _ => "album",
        }
    } else {
        kind
    };
    Some(Album {
        id: a.id.to_id().ok()?,
        uri: a.id.to_uri().ok()?,
        name: a.name.clone(),
        artists: a
            .artists
            .iter()
            .map(|a| ArtistRef {
                id: a.id.to_id().ok(),
                name: a.name.clone(),
            })
            .collect(),
        images: unique_images(a.cover_group.iter().chain(a.covers.iter())),
        release_date: format!(
            "{:04}-{:02}-{:02}",
            a.date.year(),
            u8::from(a.date.month()),
            a.date.day()
        ),
        total_tracks: a.tracks().count() as u32,
        album_type: kind.into(),
    })
}

pub fn domain_track(t: &SpTrack) -> Option<Track> {
    Some(Track {
        id: t.id.to_id().ok(),
        uri: t.id.to_uri().ok()?,
        title: t.name.clone(),
        artists: t
            .artists
            .iter()
            .map(|a| ArtistRef {
                id: a.id.to_id().ok(),
                name: a.name.clone(),
            })
            .collect(),
        album: t.album.name.clone(),
        album_id: t.album.id.to_id().ok(),
        duration_ms: u64::try_from(t.duration).unwrap_or_default(),
        explicit: t.is_explicit,
        images: unique_images(t.album.cover_group.iter().chain(t.album.covers.iter())),
        added_at: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::metadata;

    #[test]
    fn only_canonical_artist_uris_reach_spotify() {
        assert!(validate_artist_uri("spotify:artist:4LLpKhyESsyAXpc4laK94U").is_ok());
        for uri in [
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:artist:short",
            "spotify:artist:4LLpKhyESsyAXpc4laK94U?evil=1",
        ] {
            assert_eq!(
                validate_artist_uri(uri).unwrap_err(),
                ArtistError::InvalidUri
            );
        }
    }

    #[test]
    fn portrait_groups_give_one_photo_each_at_the_largest_size() {
        let msg: metadata::Artist = protobuf::text_format::parse_from_str(
            r#"gid: "aaaaaaaaaaaaaaaa"
            biography {
              text: "Bio"
              portrait { file_id: "pppppppppppppppppppp" }
              portrait_group {
                image { file_id: "aaaaaaaaaaaaaaaaaaa1" size: SMALL width: 160 }
                image { file_id: "aaaaaaaaaaaaaaaaaaa2" size: LARGE width: 640 }
                image { file_id: "aaaaaaaaaaaaaaaaaaa3" size: DEFAULT width: 320 }
              }
              portrait_group {
                image { file_id: "bbbbbbbbbbbbbbbbbbb1" size: DEFAULT width: 320 }
                image { file_id: "bbbbbbbbbbbbbbbbbbb2" size: SMALL width: 160 }
              }
            }"#,
        )
        .unwrap();
        let profile = profile_metadata(&SpArtist::try_from(&msg).unwrap());
        let widths: Vec<_> = profile.photos.iter().map(|p| p.width).collect();
        assert_eq!(widths, [Some(640), Some(320)]);
    }

    #[test]
    fn artist_fixture_keeps_country_groups_biography_and_absence() {
        let msg: metadata::Artist = protobuf::text_format::parse_from_str(include_str!(
            "../tests/fixtures/artist_v4.textproto"
        ))
        .unwrap();
        let artist = SpArtist::try_from(&msg).unwrap();
        let profile = profile_metadata(&artist);
        assert_eq!(profile.biography, "Biografía de prueba.");
        assert_eq!(profile.photos.len(), 1);
        assert_eq!(profile.related[0].name, "Relacionado");
        assert_eq!(popular_uris(&artist, "CL").len(), 2);
        assert_eq!(popular_uris(&artist, "US").len(), 1);
        assert_eq!(artist.albums_current().count(), 1);
        assert_eq!(artist.singles_current().count(), 1);
        assert_eq!(artist.compilations_current().count(), 1);
        assert_eq!(artist.appears_on_albums_current().count(), 1);
        assert!(profile.related[0].followers.is_none());
    }

    #[test]
    fn track_fixture_has_album_art_and_preserves_spotify_ep_type() {
        let msg: metadata::Track = protobuf::text_format::parse_from_str(include_str!(
            "../tests/fixtures/track_v4.textproto"
        ))
        .unwrap();
        let track = SpTrack::try_from(&msg).unwrap();
        let row = domain_track(&track).unwrap();
        assert_eq!(row.duration_ms, 234000);
        assert!(row.explicit);
        assert_eq!(row.images.len(), 1);
        assert_eq!(row.artist_line(), "Artista de prueba");
        let album = domain_album(&track.album, "single").unwrap();
        assert_eq!(album.album_type, "ep");
        assert_eq!(
            domain_album(&track.album, "appears_on").unwrap().album_type,
            "ep"
        );
        assert_eq!(album.release_date, "2024-03-01");
        let mut single = track.album.clone();
        single.album_type = librespot_metadata::album::AlbumType::SINGLE;
        assert_eq!(
            domain_album(&single, "single").unwrap().album_type,
            "single"
        );
    }

    #[test]
    fn absent_country_does_not_substitute_another_market() {
        let mut message = metadata::Artist::new();
        message.gid = Some(vec![1; 16]);
        let mut top = metadata::TopTracks::new();
        top.country = Some("US".into());
        message.top_track.push(top);
        let artist = SpArtist::try_from(&message).unwrap();
        assert!(popular_uris(&artist, "CL").is_empty());
        assert!(domain_artist(&artist).unwrap().followers.is_none());
    }

    #[test]
    fn release_groups_pick_one_variant_and_preserve_categories() {
        let mut message = metadata::Artist::new();
        message.gid = Some(vec![1; 16]);
        let mut group = metadata::AlbumGroup::new();
        for id in [2, 3] {
            let mut album = metadata::Album::new();
            album.gid = Some(vec![id; 16]);
            group.album.push(album);
        }
        message.album_group.push(group.clone());
        message.appears_on_group.push(group);
        let artist = SpArtist::try_from(&message).unwrap();
        assert_eq!(artist.albums_current().count(), 1);
        assert_eq!(artist.appears_on_albums_current().count(), 1);
    }
}
