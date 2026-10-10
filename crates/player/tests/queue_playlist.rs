//! X2: integración con Web API simulada; nunca accede a Spotify ni al keyring.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::{Value, json};
use velo_player::queue::save_as_playlist;
use velo_spotify::{MemoryTokenStore, SpotifyConfig, SpotifyProvider, TokenStore};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{body_json, method, path},
};

const CURRENT: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

async fn setup() -> (MockServer, SpotifyProvider) {
    let server = MockServer::start().await;
    let store = Arc::new(MemoryTokenStore::new());
    store.save("refresh-de-prueba").unwrap();
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-de-prueba", "expires_in": 3600
        })))
        .mount(&server)
        .await;
    let api = SpotifyProvider::new(
        SpotifyConfig {
            client_id: "cliente-de-prueba".into(),
            api_base: server.uri(),
            accounts_base: server.uri(),
            redirect_port: 0,
        },
        store,
    );
    (server, api)
}

async fn creation(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/me/playlists"))
        .and(body_json(
            json!({"name": "Mi cola", "description": "Cola guardada desde Velo", "public": false}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "lista", "uri": "spotify:playlist:lista", "name": "Mi cola",
            "owner": {"id": "yo", "display_name": "Yo"}, "items": {"total": 0}
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn batches(server: &MockServer) -> Vec<Vec<String>> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/playlists/lista/items")
        .map(|r| {
            serde_json::from_slice::<Value>(&r.body).unwrap()["uris"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn saves_current_then_visible_in_batches_of_100_preserving_occurrences() {
    let (server, api) = setup().await;
    creation(&server).await;
    Mock::given(method("POST"))
        .and(path("/playlists/lista/items"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"snapshot_id": "s"})))
        .expect(3)
        .mount(&server)
        .await;
    let mut visible: Vec<String> = (0..204).map(|i| format!("spotify:track:{i:022}")).collect();
    visible[1] = CURRENT.into();
    visible[100] = CURRENT.into();
    visible[203] = "spotify:episode:0123456789012345678901".into();
    let saved = save_as_playlist(&api, "  Mi cola  ", CURRENT, &visible)
        .await
        .unwrap();
    assert!(saved.items_result.is_ok());
    assert_eq!(saved.playlist.id, "lista");
    let actual = batches(&server).await;
    assert_eq!(
        actual.iter().map(Vec::len).collect::<Vec<_>>(),
        [100, 100, 5]
    );
    let expected: Vec<_> = std::iter::once(CURRENT.to_string())
        .chain(visible)
        .collect();
    assert_eq!(actual.into_iter().flatten().collect::<Vec<_>>(), expected);
}

#[tokio::test]
async fn empty_upcoming_still_saves_the_current_song() {
    let (server, api) = setup().await;
    creation(&server).await;
    Mock::given(method("POST"))
        .and(path("/playlists/lista/items"))
        .and(body_json(json!({"uris": [CURRENT]})))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        save_as_playlist(&api, "Mi cola", CURRENT, &[])
            .await
            .unwrap()
            .items_result
            .is_ok()
    );
}

#[tokio::test]
async fn invalid_name_or_uri_never_creates_a_partial_playlist() {
    let (server, api) = setup().await;
    for (name, current, visible) in [
        (" \t ", CURRENT, vec![]),
        ("Mi cola", "", vec![CURRENT.into()]),
        (
            "Mi cola",
            CURRENT,
            vec!["spotify:local:artista:album:cancion".into()],
        ),
        ("Mi cola", CURRENT, vec!["spotify:track:corto".into()]),
    ] {
        assert!(
            save_as_playlist(&api, name, current, &visible)
                .await
                .is_err()
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn create_failure_never_attempts_to_add_items() {
    let (server, api) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/playlists"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        save_as_playlist(&api, "Mi cola", CURRENT, &[])
            .await
            .is_err()
    );
    assert!(batches(&server).await.is_empty());
}

#[tokio::test]
async fn second_batch_failure_keeps_created_playlist_and_stops_without_repeating_items() {
    let (server, api) = setup().await;
    creation(&server).await;
    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/playlists/lista/items"))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                201
            } else {
                403
            })
        })
        .expect(2)
        .mount(&server)
        .await;
    let visible: Vec<String> = (0..204).map(|i| format!("spotify:track:{i:022}")).collect();
    let saved = save_as_playlist(&api, "Mi cola", CURRENT, &visible)
        .await
        .unwrap();
    assert_eq!(
        saved.playlist.id, "lista",
        "permite refrescar también la playlist parcial"
    );
    assert!(saved.items_result.is_err());
    let actual = batches(&server).await;
    assert_eq!(actual.iter().map(Vec::len).collect::<Vec<_>>(), [100, 100]);
    assert_eq!(actual[0][0], CURRENT);
    assert_eq!(actual[1][0], visible[99]);
}
