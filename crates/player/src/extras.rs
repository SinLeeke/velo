//! E5/E6: consultas internas y arranque experimental de DJ en otro dispositivo.
//! La aceptación HTTP no prueba reproducción ni paridad con el cliente oficial.
//! Ejecutar en Tokio; nunca registrar errores upstream, cuerpos o credenciales.

use std::time::Duration;

use bytes::Bytes;
use http::{Method, Request};
use http_body_util::BodyExt;
use librespot_core::Session;
use librespot_metadata::{Metadata, Track, artist::ArtistRole};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::social_connect_v2::SessionUpdate;
use serde_json::{Value, json};

use crate::parse_track_uri;

pub const DJ_LIVI_URI: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExtrasError {
    #[error("URI o dispositivo no válido")]
    InvalidInput,
    #[error("Spotify no entregó los datos o rechazó la acción experimental")]
    Unavailable,
    #[error("Spotify tardó demasiado")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, ExtrasError>;

/// Resumen de eventos Jam, sin IDs, nombres, tokens ni enlaces de invitación.
pub fn jam_summary(update: &SessionUpdate) -> Value {
    let Some(session) = update.session.as_ref() else {
        return json!({"hasSession": false, "reason": update.reason.value()});
    };
    json!({"hasSession": true, "reason": update.reason.value(),
        "members": session.session_members.len(), "type": session.initial_session_type.value(),
        "isOwner": session.is_session_owner, "isListening": session.is_listening,
        "isControlling": session.is_controlling})
}

/// Créditos ampliados; si la consulta o su schema fallan, conserva TRACK_V4.
pub async fn credits(session: &Session, uri: &str) -> Result<Value> {
    let id = parse_track_uri(uri).map_err(|_| ExtrasError::InvalidInput)?;
    prefer_complete_credits(track_credits(session, uri), async {
        let track = tokio::time::timeout(TIMEOUT, Track::get(session, &id))
            .await
            .map_err(|_| ExtrasError::Timeout)?
            .map_err(|_| ExtrasError::Unavailable)?;
        Ok(credits_value(&track))
    })
    .await
}

async fn prefer_complete_credits(
    complete: impl std::future::Future<Output = Result<Value>>,
    fallback: impl std::future::Future<Output = Result<Value>>,
) -> Result<Value> {
    match complete.await {
        Ok(value) => Ok(value),
        Err(_) => fallback.await,
    }
}

/// Endpoint JSON completo, también utilizado por el probe de captura.
pub async fn track_credits(session: &Session, uri: &str) -> Result<Value> {
    let id = parse_track_uri(uri).map_err(|_| ExtrasError::InvalidInput)?;
    let id = id.to_id().map_err(|_| ExtrasError::InvalidInput)?;
    let path = format!("/track-credits-view/v0/experimental/{id}/credits");
    let body = tokio::time::timeout(
        TIMEOUT,
        session
            .spclient()
            .request_as_json(&Method::GET, &path, None, None),
    )
    .await
    .map_err(|_| ExtrasError::Timeout)?
    .map_err(|_| ExtrasError::Unavailable)?;
    let value = serde_json::from_slice(&body).map_err(|_| ExtrasError::Unavailable)?;
    track_credits_value(&value)
}

/// Pure parser: keeps the existing flat UI contract and all upstream role labels.
/// The returned roleCredits/sourceNames can be written as a whitelisted fixture.
pub fn track_credits_value(value: &Value) -> Result<Value> {
    if value.get("error").is_some_and(|v| !v.is_null())
        || value["errors"].as_array().is_some_and(|v| !v.is_empty())
    {
        return Err(ExtrasError::Unavailable);
    }
    let groups = value["roleCredits"]
        .as_array()
        .ok_or(ExtrasError::Unavailable)?;
    let mut contributors = Vec::new();
    let mut role_credits = Vec::new();
    for group in groups {
        let title = group["roleTitle"]
            .as_str()
            .ok_or(ExtrasError::Unavailable)?;
        let artists = group["artists"]
            .as_array()
            .ok_or(ExtrasError::Unavailable)?;
        let mut sanitized = Vec::new();
        for artist in artists {
            let name = artist["name"].as_str().ok_or(ExtrasError::Unavailable)?;
            if name.is_empty() {
                continue;
            }
            let uri = artist["uri"].as_str().unwrap_or_default();
            let uri = if uri.starts_with("spotify:artist:") {
                uri
            } else {
                ""
            };
            let subroles = match artist.get("subroles").filter(|v| !v.is_null()) {
                Some(v) => v
                    .as_array()
                    .ok_or(ExtrasError::Unavailable)?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .ok_or(ExtrasError::Unavailable)
                    })
                    .collect::<Result<Vec<_>>>()?,
                None => Vec::new(),
            };
            let role = credit_role(title);
            contributors.push(json!({"name":name, "role":role, "roleTitle":title, "uri":uri, "subroles":subroles}));
            sanitized.push(json!({"name":name, "uri":uri, "subroles":subroles}));
        }
        role_credits.push(json!({"roleTitle":title, "artists":sanitized}));
    }
    let sources = match value.get("sourceNames").filter(|v| !v.is_null()) {
        Some(v) => v
            .as_array()
            .ok_or(ExtrasError::Unavailable)?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or(ExtrasError::Unavailable)
            })
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    Ok(
        json!({"title":value["trackName"].as_str().or_else(|| value["title"].as_str()).unwrap_or_default(),
        "source":"track_credits", "partial":false, "contributors":contributors,
        "roleCredits":role_credits, "sourceNames":sources}),
    )
}

fn credit_role(title: &str) -> &str {
    match title.to_ascii_lowercase().as_str() {
        "performed by" | "artist" => "artist",
        "main artist" => "main_artist",
        "featured artist" => "featured_artist",
        "written by" | "songwriter" | "songwriters" => "songwriter",
        "composed by" | "composer" | "composers" => "composer",
        "produced by" | "producer" | "producers" => "producer",
        "remixed by" | "remixer" => "remixer",
        "engineered by" | "engineer" => "engineer",
        "mixed by" => "mixing_engineer",
        "mastered by" => "mastering_engineer",
        _ => title,
    }
}

fn role_key(role: ArtistRole) -> &'static str {
    match role {
        ArtistRole::ARTIST_ROLE_MAIN_ARTIST => "main_artist",
        ArtistRole::ARTIST_ROLE_FEATURED_ARTIST => "featured_artist",
        ArtistRole::ARTIST_ROLE_REMIXER => "remixer",
        ArtistRole::ARTIST_ROLE_ACTOR => "actor",
        ArtistRole::ARTIST_ROLE_COMPOSER => "composer",
        ArtistRole::ARTIST_ROLE_CONDUCTOR => "conductor",
        ArtistRole::ARTIST_ROLE_ORCHESTRA => "orchestra",
        _ => "unknown",
    }
}

fn credits_value(track: &Track) -> Value {
    let mut contributors: Vec<Value> = track
        .artists_with_role
        .iter()
        .filter(|a| !a.name.is_empty())
        .take(100)
        .map(|a| json!({"name": a.name, "role": role_key(a.role)}))
        .collect();
    if contributors.is_empty() {
        contributors = track
            .artists
            .iter()
            .filter(|a| !a.name.is_empty())
            .take(100)
            .map(|a| json!({"name": a.name, "role": "artist"}))
            .collect();
    }
    json!({"title": track.name, "source": "track_metadata", "partial": true,
           "contributors": contributors})
}

/// GET lógico por extended-metadata (POST de consulta): devuelve solo tamaño.
/// Sirve para verificar disponibilidad de schemas aún sin decodificador; no es
/// evidencia de que Mixear o los créditos completos estén implementados.
pub async fn extension_size(session: &Session, uri: &str, kind: ExtensionKind) -> Result<usize> {
    let id = parse_track_uri(uri).map_err(|_| ExtrasError::InvalidInput)?;
    tokio::time::timeout(TIMEOUT, session.spclient().get_metadata(kind, &id))
        .await
        .map_err(|_| ExtrasError::Timeout)?
        .map(|bytes| bytes.len())
        .map_err(|_| ExtrasError::Unavailable)
}

fn valid_device(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn dj_command() -> Value {
    json!({"command": {"endpoint": "play", "context": {"uri": DJ_LIVI_URI},
        "play_origin": {"feature_identifier": "velo"},
        "options": {"initially_paused": false}, "logging_params": {}}})
}

/// Connect commands share the authenticated session HTTP client. Never change SpClient's
/// retry strategy globally; mutations get one attempt (transfers at most two).
pub async fn connect_command(session: &Session, target: &str, command: Value) -> Result<()> {
    let path = command_path(session.device_id(), target)?;
    connect_request(session, &path, json!({"command": command})).await
}
pub fn command_path(source: &str, target: &str) -> Result<String> {
    if !valid_device(source) || !valid_device(target) {
        return Err(ExtrasError::InvalidInput);
    }
    Ok(format!(
        "/connect-state/v1/player/command/from/{source}/to/{target}"
    ))
}
pub fn transfer_request(source: &str, target: &str, play: bool) -> Result<(String, Value)> {
    if !valid_device(source) || !valid_device(target) || source == target {
        return Err(ExtrasError::InvalidInput);
    }
    Ok((
        format!("/connect-state/v1/connect/transfer/from/{source}/to/{target}"),
        json!({"transfer_options": {"restore_paused": if play { "resume" } else { "pause" }}}),
    ))
}
pub async fn transfer(session: &Session, source: &str, target: &str, play: bool) -> Result<()> {
    let (path, body) = transfer_request(source, target, play)?;
    let first = connect_request(session, &path, body.clone()).await;
    if first.is_ok() {
        return first;
    }
    tokio::time::sleep(Duration::from_millis(350)).await;
    connect_request(session, &path, body).await
}
async fn connect_request(session: &Session, path: &str, body: Value) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let base = session
            .spclient()
            .base_url()
            .await
            .map_err(|_| ExtrasError::Unavailable)?;
        let token = session
            .login5()
            .auth_token()
            .await
            .map_err(|_| ExtrasError::Unavailable)?;
        let authorization = format!("{} {}", token.token_type, token.access_token);
        let client_token = session.spclient().client_token().await.ok();
        post_connect_request(
            &format!("{base}{path}"),
            &authorization,
            client_token,
            body,
            |request| async {
                // request_body retries 429 internally. Use one HTTP attempt for mutations.
                let response = session
                    .http_client()
                    .request_fut(request)
                    .map_err(|_| ExtrasError::Unavailable)?
                    .await
                    .map_err(|_| ExtrasError::Unavailable)?;
                if !response.status().is_success() {
                    return Err(ExtrasError::Unavailable);
                }
                response
                    .into_body()
                    .collect()
                    .await
                    .map(|body| body.to_bytes())
                    .map_err(|_| ExtrasError::Unavailable)
            },
        )
        .await
    })
    .await
    .map_err(|_| ExtrasError::Timeout)?
}

/// DJ still validates a remote target; HTTP acceptance does not certify playback.
pub async fn start_dj(session: &Session, target_device: &str) -> Result<()> {
    if session.device_id() == target_device {
        return Err(ExtrasError::InvalidInput);
    }
    connect_command(session, target_device, dj_command()["command"].clone()).await
}

#[cfg(test)]
async fn post_dj_request<F, Fut>(
    url: &str,
    authorization: &str,
    client_token: Option<String>,
    send: F,
) -> Result<()>
where
    F: FnOnce(Request<Bytes>) -> Fut,
    Fut: std::future::Future<Output = Result<Bytes>>,
{
    post_connect_request(url, authorization, client_token, dj_command(), send).await
}

async fn post_connect_request<F, Fut>(
    url: &str,
    authorization: &str,
    client_token: Option<String>,
    body: Value,
    send: F,
) -> Result<()>
where
    F: FnOnce(Request<Bytes>) -> Fut,
    Fut: std::future::Future<Output = Result<Bytes>>,
{
    let body = body.to_string();
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(url)
        .header("authorization", authorization)
        .header("content-type", "application/json")
        .header("content-length", body.len());
    if let Some(client_token) = client_token {
        request = request.header("client-token", client_token);
    }
    let request = request
        .body(Bytes::from(body))
        .map_err(|_| ExtrasError::Unavailable)?;
    let response = send(request).await?;
    // Algunos servicios devuelven un error lógico dentro de una respuesta 2xx.
    if !response.is_empty() {
        let value: Value =
            serde_json::from_slice(&response).map_err(|_| ExtrasError::Unavailable)?;
        if value.get("error").is_some_and(|e| !e.is_null()) {
            return Err(ExtrasError::Unavailable);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_core::dealer::protocol::Command;

    #[test]
    fn transfer_and_player_command_paths_validate_ids_and_restore_transport() {
        let (path, body) = transfer_request("phone", "velo", true).unwrap();
        assert_eq!(
            path,
            "/connect-state/v1/connect/transfer/from/phone/to/velo"
        );
        assert_eq!(body["transfer_options"]["restore_paused"], "resume");
        assert_eq!(
            transfer_request("velo", "phone", false).unwrap().1["transfer_options"]["restore_paused"],
            "pause"
        );
        assert_eq!(
            command_path("velo", "phone").unwrap(),
            "/connect-state/v1/player/command/from/velo/to/phone"
        );
        for (a, b) in [("", "phone"), ("velo", "bad/id"), ("same", "same")] {
            assert!(transfer_request(a, b, true).is_err());
        }
    }

    #[tokio::test]
    async fn full_credits_do_not_poll_metadata_and_failures_keep_the_existing_fallback() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/spclient/credits.json")).unwrap();
        let complete = track_credits_value(&fixture).unwrap();
        let unexpected_metadata =
            async { panic!("metadata must remain unpolled on full credits success") };
        assert_eq!(
            prefer_complete_credits(
                std::future::ready(Ok(complete.clone())),
                unexpected_metadata
            )
            .await
            .unwrap(),
            complete
        );
        for error in [ExtrasError::Unavailable, ExtrasError::Timeout] {
            let fallback = json!({"source":"track_metadata","partial":true,"contributors":[{"name":"A","role":"composer"}]});
            assert_eq!(
                prefer_complete_credits(
                    std::future::ready(Err(error)),
                    std::future::ready(Ok(fallback.clone()))
                )
                .await
                .unwrap(),
                fallback
            );
        }
    }

    #[test]
    fn complete_credits_keep_every_role_and_sources_in_existing_ui_shape() {
        // Synthetic documented JSON shape; the leader replaces it with a real probe capture.
        let v: Value =
            serde_json::from_str(include_str!("../tests/fixtures/spclient/credits.json")).unwrap();
        let credits = track_credits_value(&v).unwrap();
        assert_eq!(credits["partial"], false);
        assert_eq!(credits["source"], "track_credits");
        assert_eq!(credits["title"], "Fixture Track");
        assert_eq!(credits["contributors"].as_array().unwrap().len(), 5);
        assert_eq!(credits["contributors"][0]["role"], "artist");
        assert_eq!(credits["contributors"][1]["role"], "songwriter");
        assert_eq!(credits["contributors"][2]["role"], "producer");
        assert_eq!(
            credits["contributors"][2]["subroles"],
            json!(["Producer", "Vocal Producer"])
        );
        assert_eq!(credits["contributors"][4]["role"], "Additional Instruments");
        assert_eq!(
            credits["sourceNames"],
            json!(["Fixture Label", "Fixture Publisher"])
        );
        assert_eq!(credits["roleCredits"], v["roleCredits"]);
    }

    #[test]
    fn unavailable_credits_are_distinct_from_a_valid_empty_list() {
        for v in [
            json!({"error":404}),
            json!({"error":{"status":500}, "roleCredits":[]}),
            json!({"errors":[{"message":"fixture failure"}], "roleCredits":[]}),
            json!({"roleCredits":null}),
            json!({"roleCredits":[{"roleTitle":"Produced By","artists":[{"name":"A","subroles":[42]}]}]}),
        ] {
            assert_eq!(track_credits_value(&v), Err(ExtrasError::Unavailable));
        }
        let empty = track_credits_value(&json!({"roleCredits":[]})).unwrap();
        assert_eq!(empty["contributors"], json!([]));
        assert_eq!(empty["partial"], false);
        let v = track_credits_value(&json!({"roleCredits":[{"roleTitle":"Composed By", "artists":[{"name":"A","uri":"https://example.invalid/token"}]}]})).unwrap();
        assert_eq!(v["contributors"][0]["role"], "composer");
        assert_eq!(v["contributors"][0]["uri"], "");
    }

    #[test]
    fn comando_dj_es_compatible_con_el_decoder_dealer_upstream() {
        let command: Command = serde_json::from_value(dj_command()["command"].clone()).unwrap();
        let Command::Play(play) = command else {
            panic!("se esperaba Play")
        };
        assert_eq!(play.context.uri(), DJ_LIVI_URI);
        assert_eq!(play.options.initially_paused, Some(false));
    }

    #[test]
    fn dispositivo_no_permite_inyectar_una_ruta() {
        for id in ["", "../other", "id?query=1", "a/b", "id%2f", "id\n"] {
            assert!(!valid_device(id));
        }
        assert!(valid_device("123abc-device"));
    }

    #[tokio::test]
    async fn uri_invalida_no_inicia_red() {
        let session = Session::new(Default::default(), None);
        assert_eq!(
            credits(&session, "spotify:album:bad").await,
            Err(ExtrasError::InvalidInput)
        );
        assert_eq!(
            extension_size(&session, "bad", ExtensionKind::TRACK_EXTENDED_CREDITS).await,
            Err(ExtrasError::InvalidInput)
        );
        assert_eq!(
            start_dj(&session, session.device_id()).await,
            Err(ExtrasError::InvalidInput)
        );
    }

    #[tokio::test]
    async fn comando_http_en_servidor_mock_no_reintenta_ni_acepta_error_logico() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, header, method, path},
        };
        let server = MockServer::start().await;
        let client = reqwest::Client::builder()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        for (endpoint, status, body, expected) in [
            ("/accepted", 204, None, Ok(())),
            (
                "/logical-error",
                200,
                Some(json!({"error": "rechazo simulado"})),
                Err(ExtrasError::Unavailable),
            ),
            ("/unavailable", 503, None, Err(ExtrasError::Unavailable)),
        ] {
            let mut response = ResponseTemplate::new(status);
            if let Some(body) = body {
                response = response.set_body_json(body);
            }
            Mock::given(method("POST"))
                .and(path(endpoint))
                .and(body_json(dj_command()))
                .and(header("authorization", "Bearer credencial-simulada"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                post_dj_request(
                    &format!("{}{endpoint}", server.uri()),
                    "Bearer credencial-simulada",
                    None,
                    |request| async {
                        let request = reqwest::Request::try_from(request)
                            .map_err(|_| ExtrasError::Unavailable)?;
                        let response = client
                            .execute(request)
                            .await
                            .map_err(|_| ExtrasError::Unavailable)?;
                        let response = response
                            .error_for_status()
                            .map_err(|_| ExtrasError::Unavailable)?;
                        response.bytes().await.map_err(|_| ExtrasError::Unavailable)
                    }
                )
                .await,
                expected
            );
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn transfer_posts_the_session_request_to_a_local_mock() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let server = MockServer::start().await;
        let (route, body) = transfer_request("phone", "velo", true).unwrap();
        Mock::given(method("POST"))
            .and(path(&route))
            .and(body_json(body.clone()))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        post_connect_request(
            &format!("{}{route}", server.uri()),
            "Bearer fixture",
            None,
            body,
            |request| async {
                let request =
                    reqwest::Request::try_from(request).map_err(|_| ExtrasError::Unavailable)?;
                let response = client
                    .execute(request)
                    .await
                    .map_err(|_| ExtrasError::Unavailable)?
                    .error_for_status()
                    .map_err(|_| ExtrasError::Unavailable)?;
                response.bytes().await.map_err(|_| ExtrasError::Unavailable)
            },
        )
        .await
        .unwrap();
        server.verify().await;
    }

    #[test]
    fn creditos_preservan_roles_reales_y_declaran_datos_parciales() {
        use librespot_protocol::metadata;
        let mut source = metadata::Track::new();
        source.set_gid(vec![0; 16]);
        source.set_name("Título de prueba".into());
        source.album.mut_or_insert_default().set_gid(vec![0; 16]);
        let mut artist = metadata::Artist::new();
        artist.set_gid(vec![0; 16]);
        artist.set_name("Artista".into());
        source.artist.push(artist);
        let mut role = metadata::ArtistWithRole::new();
        role.set_artist_gid(vec![0; 16]);
        role.set_artist_name("Compositor".into());
        role.set_role(ArtistRole::ARTIST_ROLE_COMPOSER);
        source.artist_with_role.push(role);
        let with_roles = credits_value(&Track::try_from(&source).unwrap());
        assert_eq!(with_roles["partial"], true);
        assert_eq!(with_roles["contributors"][0]["role"], "composer");
        assert_eq!(with_roles["contributors"][0]["name"], "Compositor");
        source.artist_with_role.clear();
        let fallback = credits_value(&Track::try_from(&source).unwrap());
        assert_eq!(fallback["contributors"][0]["role"], "artist");
    }
}
