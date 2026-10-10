//! F5: géneros/ánimos reales por canción, si la sesión autoriza TRACK_DESCRIPTOR.
//! No son todavía los chips personalizados de Me gusta del cliente oficial.
use std::{collections::HashSet, time::Duration};

use librespot_core::Session;
use librespot_protocol::{
    extended_metadata::{
        BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionQuery,
    },
    extension_kind::ExtensionKind,
};
use protobuf::{EnumOrUnknown, Message};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/track_descriptors/mod.rs"));
}
use generated::track_descriptors::{ExtensionDescriptorData, ExtensionDescriptorType};

use crate::parse_track_uri;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorKind {
    Genre,
    Mood,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Descriptor {
    pub text: String,
    pub kind: DescriptorKind,
    /// Peso de Spotify, sin transformarlo a BPM, energía o confianza.
    pub weight: f32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DescriptorError {
    #[error("Se requiere una URI canónica de canción")]
    InvalidUri,
    #[error("Spotify no entregó esta extensión")]
    Unavailable,
    #[error("Spotify tardó demasiado")]
    Timeout,
    #[error("Schema de descriptores no reconocido")]
    Format,
    #[error("No hay géneros o ánimos en esta respuesta")]
    Empty,
    #[error("La sesión o el transporte de metadatos no está disponible")]
    Session,
}

pub type Result<T> = std::result::Result<T, DescriptorError>;

pub async fn track_descriptors(session: &Session, uri: &str) -> Result<Vec<Descriptor>> {
    parse_track_uri(uri).map_err(|_| DescriptorError::InvalidUri)?;
    let response = tokio::time::timeout(
        Duration::from_secs(15),
        session
            .spclient()
            .get_extended_metadata(descriptor_request(uri)),
    )
    .await
    .map_err(|_| DescriptorError::Timeout)?
    .map_err(|error| match error.kind {
        librespot_core::error::ErrorKind::NotFound => DescriptorError::Unavailable,
        librespot_core::error::ErrorKind::DeadlineExceeded => DescriptorError::Timeout,
        _ => DescriptorError::Session,
    })?;
    parse_response(&response, uri)
}

fn descriptor_request(uri: &str) -> BatchedEntityRequest {
    BatchedEntityRequest {
        entity_request: vec![EntityRequest {
            entity_uri: uri.into(),
            query: vec![ExtensionQuery {
                extension_kind: EnumOrUnknown::new(ExtensionKind::TRACK_DESCRIPTOR),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn parse_response(response: &BatchedExtensionResponse, uri: &str) -> Result<Vec<Descriptor>> {
    let mut found = false;
    let mut descriptors = Vec::new();
    for array in &response.extended_metadata {
        if array.extension_kind.enum_value_or_default() != ExtensionKind::TRACK_DESCRIPTOR {
            continue;
        }
        if let Some(header) = array
            .header
            .as_ref()
            .filter(|h| h.provider_error_status != 0)
        {
            return Err(descriptor_status(header.provider_error_status));
        }
        for entity in &array.extension_data {
            if entity.entity_uri != uri {
                continue;
            }
            found = true;
            if entity
                .header
                .as_ref()
                .is_some_and(|h| h.status_code != 0 && h.status_code != 200)
            {
                return Err(descriptor_status(entity.header.status_code));
            }
            let data = entity
                .extension_data
                .as_ref()
                .ok_or(DescriptorError::Unavailable)?;
            // Un protobuf de schema distinto puede decodificar sin error y dar etiquetas
            // falsas. Verificar Any.type_url antes de interpretar sus bytes.
            if data.type_url.rsplit('/').next()
                != Some("spotify.descriptorextension.ExtensionDescriptorData")
                || data.value.len() > 1024 * 1024
            {
                return Err(DescriptorError::Format);
            }
            let parsed = ExtensionDescriptorData::parse_from_bytes(&data.value)
                .map_err(|_| DescriptorError::Format)?;
            descriptors.extend(normalize(&parsed));
        }
    }
    if !found {
        return Err(DescriptorError::Unavailable);
    }
    let mut seen = HashSet::new();
    descriptors.retain(|d| seen.insert((d.text.to_lowercase(), d.kind)));
    descriptors.truncate(100);
    if descriptors.is_empty() {
        Err(DescriptorError::Empty)
    } else {
        Ok(descriptors)
    }
}

fn descriptor_status(status: i32) -> DescriptorError {
    match status {
        408 | 504 => DescriptorError::Timeout,
        401 | 429 | 500..=599 => DescriptorError::Session,
        _ => DescriptorError::Unavailable,
    }
}

fn normalize(data: &ExtensionDescriptorData) -> Vec<Descriptor> {
    let mut result = Vec::new();
    for item in data.descriptors.iter().take(100) {
        let text = item.text.trim();
        if text.is_empty() || text.len() > 256 || !item.weight.is_finite() || item.weight < 0.0 {
            continue;
        }
        for kind in &item.types {
            let kind = match kind.enum_value_or_default() {
                ExtensionDescriptorType::GENRE => DescriptorKind::Genre,
                ExtensionDescriptorType::MOOD => DescriptorKind::Mood,
                _ => continue,
            };
            result.push(Descriptor {
                text: text.into(),
                kind,
                weight: item.weight,
            });
        }
    }
    result.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    result
}

#[cfg(test)]
mod tests {
    use super::generated::track_descriptors::ExtensionDescriptor;
    use super::*;
    use librespot_protocol::{
        entity_extension_data::{EntityExtensionData, EntityExtensionDataHeader},
        extended_metadata::EntityExtensionDataArray,
    };
    use protobuf::{MessageField, well_known_types::any::Any};

    const URI: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    fn fixture(type_url: &str, status: i32) -> BatchedExtensionResponse {
        let mut data = ExtensionDescriptorData::new();
        for (text, weight, types) in [
            (" Rock ", 0.7, vec![ExtensionDescriptorType::GENRE]),
            ("Chill", 0.8, vec![ExtensionDescriptorType::MOOD]),
            ("rock", 0.3, vec![ExtensionDescriptorType::GENRE]),
            ("guitar", 0.9, vec![ExtensionDescriptorType::INSTRUMENT]),
            ("bad", f32::NAN, vec![ExtensionDescriptorType::GENRE]),
        ] {
            data.descriptors.push(ExtensionDescriptor {
                text: text.into(),
                weight,
                types: types.into_iter().map(EnumOrUnknown::new).collect(),
                ..Default::default()
            });
        }
        BatchedExtensionResponse {
            extended_metadata: vec![EntityExtensionDataArray {
                extension_kind: EnumOrUnknown::new(ExtensionKind::TRACK_DESCRIPTOR),
                extension_data: vec![EntityExtensionData {
                    entity_uri: URI.into(),
                    header: MessageField::some(EntityExtensionDataHeader {
                        status_code: status,
                        ..Default::default()
                    }),
                    extension_data: MessageField::some(Any {
                        type_url: type_url.into(),
                        value: data.write_to_bytes().unwrap(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn decodifica_fixture_binario_de_upstream_y_deduplica() {
        let response = fixture(
            "type.googleapis.com/spotify.descriptorextension.ExtensionDescriptorData",
            200,
        );
        let bytes = response.write_to_bytes().unwrap();
        let roundtrip = BatchedExtensionResponse::parse_from_bytes(&bytes).unwrap();
        let result = parse_response(&roundtrip, URI).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "Chill");
        assert_eq!(result[0].kind, DescriptorKind::Mood);
        assert_eq!(result[1].text, "Rock");
    }

    #[test]
    fn distingue_rechazo_schema_desconocido_y_respuesta_para_otra_cancion() {
        assert_eq!(
            parse_response(&fixture("unknown", 200), URI),
            Err(DescriptorError::Format)
        );
        assert_eq!(
            parse_response(&fixture("unknown", 403), URI),
            Err(DescriptorError::Unavailable)
        );
        assert_eq!(
            parse_response(&fixture("unknown", 200), "other"),
            Err(DescriptorError::Unavailable)
        );
    }

    #[test]
    fn session_statuses_abort_but_per_entity_errors_stay_local() {
        for status in [401, 429, 500, 503] {
            assert_eq!(
                parse_response(&fixture("unknown", status), URI),
                Err(DescriptorError::Session)
            );
        }
        for status in [408, 504] {
            assert_eq!(
                parse_response(&fixture("unknown", status), URI),
                Err(DescriptorError::Timeout)
            );
        }
        assert_eq!(
            parse_response(&fixture("unknown", 404), URI),
            Err(DescriptorError::Unavailable)
        );
        for (status, expected) in [
            (404, DescriptorError::Unavailable),
            (401, DescriptorError::Session),
            (429, DescriptorError::Session),
        ] {
            let mut response = fixture("unknown", 200);
            response.extended_metadata[0]
                .header
                .mut_or_insert_default()
                .provider_error_status = status;
            assert_eq!(parse_response(&response, URI), Err(expected));
        }
    }

    #[tokio::test]
    async fn no_hay_red_con_uri_invalida() {
        let session = Session::new(Default::default(), None);
        assert_eq!(
            track_descriptors(&session, "spotify:album:bad").await,
            Err(DescriptorError::InvalidUri)
        );
    }
}
