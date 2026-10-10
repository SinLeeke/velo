use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::*};

async fn setup() -> (MockServer, SpotifyProvider, Arc<MemoryTokenStore>) {
    let server = MockServer::start().await;
    let store = Arc::new(MemoryTokenStore::new());
    store.save("refresh-original").unwrap();
    let provider = SpotifyProvider::new(
        SpotifyConfig {
            client_id: "client".into(),
            api_base: server.uri(),
            accounts_base: server.uri(),
            redirect_port: 0,
        },
        store.clone(),
    );
    provider.auth.lock().await.access = Some(AccessToken {
        value: "access-initial".into(),
        expires_at: Instant::now() + Duration::from_secs(3600),
    });
    (server, provider, store)
}

fn song() -> Value {
    json!({"type":"track","id":"song","uri":"spotify:track:song","name":"Song","duration_ms":180000,"artists":[{"id":"artist","name":"Artist"}],"album":{"id":"album","name":"Album","images":[{"url":"https://i.scdn.co/image/cover","width":300,"height":300}]}})
}

fn playlist() -> Value {
    json!({"id":"playlist","name":"Mix","uri":"spotify:playlist:playlist","owner":{"id":"owner","display_name":"Owner"},"collaborative":false,"items":{"total":3},"images":[]})
}

fn page(items: Value) -> Value {
    json!({"items":items,"offset":5,"limit":50,"total":100})
}

fn form_matches(request: &Request, expected: &[(&str, &str)]) -> bool {
    let form: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(&request.body).collect();
    form.len() == expected.len()
        && expected
            .iter()
            .all(|(k, v)| form.get(*k).is_some_and(|actual| actual == v))
}

async fn get_mock(server: &MockServer, endpoint: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(endpoint))
        .and(header("authorization", "Bearer access-initial"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn code_exchange_sends_pkce_form_and_persists_only_refresh_token() {
    let (server, provider, store) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(|r: &Request| {
            form_matches(
                r,
                &[
                    ("grant_type", "authorization_code"),
                    ("code", "code+value"),
                    ("redirect_uri", "http://127.0.0.1:54321/callback"),
                    ("client_id", "client"),
                    ("code_verifier", "verifier"),
                ],
            )
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    provider
        .exchange_code(
            "code+value",
            "http://127.0.0.1:54321/callback",
            "verifier",
            0,
        )
        .await
        .unwrap();
    assert_eq!(store.load().as_deref(), Some("new-refresh"));
    assert_eq!(provider.access_token(None).await.unwrap(), "new-access");
}

#[tokio::test]
async fn unauthorized_refreshes_once_rotates_refresh_token_and_retries() {
    let (server, provider, store) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer access-initial"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .and(|r: &Request| {
            form_matches(
                r,
                &[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", "refresh-original"),
                    ("client_id", "client"),
                ],
            )
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"replacement","refresh_token":"rotated","expires_in":3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer replacement"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"owner","display_name":"Owner","product":"premium"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(provider.me().await.unwrap().id, "owner");
    assert_eq!(store.load().as_deref(), Some("rotated"));
}

#[tokio::test]
async fn refreshes_sixty_seconds_early_and_preserves_unrotated_refresh_token() {
    let (server, provider, store) = setup().await;
    provider
        .auth
        .lock()
        .await
        .access
        .as_mut()
        .unwrap()
        .expires_at = Instant::now() + Duration::from_secs(60);
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"fresh","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(provider.access_token(None).await.unwrap(), "fresh");
    assert_eq!(store.load().as_deref(), Some("refresh-original"));
    assert_eq!(provider.access_token(None).await.unwrap(), "fresh");
}

#[tokio::test]
async fn concurrent_rejections_share_one_refresh() {
    let (server, provider, _) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"fresh","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (a, b) = tokio::join!(
        provider.access_token(Some("access-initial")),
        provider.access_token(Some("access-initial"))
    );
    assert_eq!(a.unwrap(), "fresh");
    assert_eq!(b.unwrap(), "fresh");
}

#[tokio::test]
async fn persistent_unauthorized_is_bounded() {
    let (server, provider, _) = setup().await;
    Mock::given(path("/me"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(path("/api/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"fresh","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        provider.me().await,
        Err(VeloError::NotAuthenticated)
    ));
    assert!(provider.auth.lock().await.access.is_none());
}

#[tokio::test]
async fn invalid_grant_clears_session_and_sign_out_cancels_old_login() {
    let (server, provider, store) = setup().await;
    Mock::given(path("/api/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"invalid_grant"})))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        provider.access_token(Some("access-initial")).await,
        Err(VeloError::NotAuthenticated)
    ));
    assert!(store.load().is_none());
    provider.sign_out().await.unwrap();
    assert!(!provider.is_signed_in().await);
    assert!(matches!(
        provider
            .exchange_code("code", "redirect", "verifier", 0)
            .await,
        Err(VeloError::NotAuthenticated)
    ));
}

#[tokio::test]
async fn playback_204_means_no_active_playback() {
    let (server, provider, _) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/player"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    assert!(provider.playback().await.unwrap().is_none());
}

#[tokio::test]
async fn playback_maps_snapshot_and_queue_uses_queue_field() {
    let (server, provider, _) = setup().await;
    get_mock(&server,"/me/player",json!({"item":song(),"is_playing":true,"progress_ms":1234,"shuffle_state":true,"repeat_state":"context","context":{"uri":"spotify:album:album"},"device":{"id":"pc","name":"PC","type":"Computer","is_active":true,"volume_percent":50,"supports_volume":true}})).await;
    let p = provider.playback().await.unwrap().unwrap();
    assert_eq!(p.track.unwrap().title, "Song");
    assert_eq!(p.progress_ms, 1234);
    assert!(p.shuffle);
    assert_eq!(p.repeat, RepeatMode::Context);
    assert_eq!(p.context_uri.as_deref(), Some("spotify:album:album"));
    get_mock(
        &server,
        "/me/player/queue",
        json!({"currently_playing":song(),"queue":[song(),null,{"type":"episode"}]}),
    )
    .await;
    assert_eq!(provider.queue().await.unwrap().len(), 1);
}

#[tokio::test]
async fn d3_queue_snapshot_keeps_current_and_upcoming_from_one_response() {
    let (server, provider, _) = setup().await;
    get_mock(
        &server,
        "/me/player/queue",
        json!({"currently_playing":song(),"queue":[song(),null,{"type":"episode"}]}),
    )
    .await;
    let snapshot = provider.queue_snapshot().await.unwrap();
    assert_eq!(snapshot.current.as_deref(), Some("spotify:track:song"));
    assert_eq!(snapshot.tracks.len(), 1);
}

#[tokio::test]
async fn premium_required_and_no_active_device_and_not_found_are_distinct() {
    let (server, provider, _) = setup().await;
    Mock::given(path("/me/player/play"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error":{"status":403,"reason":"PREMIUM_REQUIRED"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me/player/pause"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"error":{"status":404,"reason":"NO_ACTIVE_DEVICE"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/artists/missing"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"error":{"status":404,"message":"Not found"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        provider.play().await,
        Err(VeloError::PremiumRequired)
    ));
    assert!(matches!(
        provider.pause().await,
        Err(VeloError::NoActiveDevice)
    ));
    assert!(matches!(
        provider.artist("missing").await,
        Err(VeloError::NotFound)
    ));
}

#[tokio::test]
async fn rate_limit_retries_once_after_short_delay() {
    let (server, provider, _) = setup().await;
    let calls = AtomicUsize::new(0);
    Mock::given(method("PUT"))
        .and(path("/me/player/play"))
        .respond_with(move |_: &Request| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(429).insert_header("Retry-After", "1")
            } else {
                ResponseTemplate::new(204)
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let start = Instant::now();
    provider.play().await.unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
}

#[tokio::test]
async fn long_and_repeated_rate_limits_return_retry_after() {
    let (server, provider, _) = setup().await;
    let mut unavailable = provider.app_unavailable.subscribe();
    Mock::given(path("/me/player/play"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "4"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me/player/next"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    assert!(
        matches!(provider.play().await,Err(VeloError::RateLimited(d)) if d == Duration::from_secs(4))
    );
    // L1: within Retry-After every request fails locally, without hitting Spotify again.
    assert!(
        matches!(provider.next().await, Err(VeloError::RateLimited(d)) if d > Duration::from_secs(3))
    );
    assert_eq!(unavailable.try_recv().unwrap(), "quota");
    assert!(unavailable.try_recv().is_err());

    let (server, provider, _) = setup().await;
    let mut unavailable = provider.app_unavailable.subscribe();
    Mock::given(path("/me/player/pause"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .expect(2)
        .mount(&server)
        .await;
    assert!(matches!(provider.pause().await,Err(VeloError::RateLimited(d)) if d == Duration::ZERO));
    assert_eq!(unavailable.try_recv().unwrap(), "quota");
}

#[tokio::test]
async fn d2_invalid_client_offers_fallback_without_erasing_the_session() {
    let (server, provider, store) = setup().await;
    let mut unavailable = provider.app_unavailable.subscribe();
    Mock::given(path("/api/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":"invalid_client"})))
        .expect(1)
        .mount(&server)
        .await;
    provider.auth.lock().await.access = None;
    assert!(matches!(provider.me().await, Err(VeloError::Other(m)) if m == "invalid_client"));
    assert_eq!(unavailable.try_recv().unwrap(), "invalid_client");
    assert_eq!(store.load().as_deref(), Some("refresh-original"));
}

#[tokio::test]
async fn d2_candidate_grant_stays_staged_until_accepted_and_logout_clears_both_tokens() {
    let (_, candidate, staged) = setup().await;
    let durable = Arc::new(MemoryTokenStore::new());
    let target = SpotifyProvider::new(SpotifyConfig::default(), durable.clone());
    assert!(durable.load().is_none());
    target.adopt_session_from(&candidate).await.unwrap();
    assert_eq!(durable.load(), staged.load());
    assert_eq!(target.access_token(None).await.unwrap(), "access-initial");
    target.sign_out().await.unwrap();
    assert!(durable.load().is_none());
    assert!(target.auth.lock().await.access.is_none());
}

#[test]
fn d2_default_application_and_personal_id_validation() {
    let config = SpotifyConfig::default();
    assert_eq!(config.client_id, oauth::DEFAULT_WEB_CLIENT_ID);
    assert_eq!(config.redirect_port, 8989);
    assert_eq!(oauth::WEB_REDIRECT_URI, "http://127.0.0.1:8989/login");
    assert!(oauth::valid_client_id(oauth::DEFAULT_WEB_CLIENT_ID));
    assert!(oauth::valid_client_id(" 0123456789ABCDEF0123456789abcdef "));
    for invalid in [
        "",
        "client",
        "0123456789abcdef0123456789abcdeg",
        "0123456789abcdef0123456789abcdef0",
    ] {
        assert!(!oauth::valid_client_id(invalid));
    }
}

#[tokio::test]
async fn foreign_playlist_is_forbidden_and_items_use_item() {
    let (server, provider, _) = setup().await;
    Mock::given(path("/playlists/foreign/items"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        provider.playlist_tracks("foreign", 0, 50).await,
        Err(VeloError::Forbidden(_))
    ));
    Mock::given(method("GET"))
        .and(path("/playlists/own/items"))
        .and(query_param("offset", "5"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            json!([{"item":song()},{"item":null},{"item":{"type":"episode"}}]),
        )))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider.playlist_tracks("own", 5, 100).await.unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].id.as_deref(), Some("song"));
    assert_eq!((p.offset, p.limit, p.total), (5, 50, 100));
}

#[tokio::test]
async fn search_clamps_limit_to_ten_and_encodes_query() {
    let (server, provider, _) = setup().await;
    Mock::given(method("GET")).and(path("/search")).and(query_param("limit","10"))
        .and(query_param("offset","7")).and(query_param("q","Björk & jazz"))
        .and(query_param("type","track,album,artist,playlist"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tracks":page(json!([song()])),"playlists":page(json!([playlist(),null])),"albums":page(json!([{"id":"album","name":"Album"}])),"artists":page(json!([{"id":"artist","name":"Artist"}]))})))
        .expect(1).mount(&server).await;
    let result = provider.search("Björk & jazz", 7, 50).await.unwrap();
    assert_eq!(result.tracks.items.len(), 1);
    assert_eq!(result.playlists.items.len(), 1);
    assert_eq!(result.albums.items.len(), 1);
    assert_eq!(result.artists.items.len(), 1);
}

#[tokio::test]
async fn saved_membership_and_mutations_use_generic_library_uri() {
    let (server, provider, _) = setup().await;
    let uri = "spotify:track:song";
    Mock::given(method("GET"))
        .and(path("/me/library/contains"))
        .and(query_param("uris", uri))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([true])))
        .expect(1)
        .mount(&server)
        .await;
    for verb in ["PUT", "DELETE"] {
        Mock::given(method(verb))
            .and(path("/me/library"))
            .and(query_param("uris", uri))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    assert!(provider.is_saved(uri).await.unwrap());
    provider.set_saved(uri, true).await.unwrap();
    provider.set_saved(uri, false).await.unwrap();
}

#[tokio::test]
async fn album_tracks_inherit_album_art_and_metadata() {
    let (server, provider, _) = setup().await;
    get_mock(&server,"/albums/album",json!({"id":"album","name":"Album","images":[{"url":"https://i.scdn.co/image/art","width":640,"height":640}]})).await;
    get_mock(&server,"/albums/album/tracks",page(json!([{"id":"song","type":"track","uri":"spotify:track:song","name":"Song","artists":[]}]))).await;
    let p = provider.album_tracks("album", 5, 50).await.unwrap();
    assert_eq!(p.items[0].images[0].url, "https://i.scdn.co/image/art");
    assert_eq!(p.items[0].album, "Album");
    assert_eq!(p.items[0].album_id.as_deref(), Some("album"));
}

#[tokio::test]
async fn playlist_editing_uses_current_routes_and_items_body() {
    let (server, provider, _) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/playlists"))
        .and(body_json(
            json!({"name":"Mix","description":"Description","public":false}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(playlist()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/playlists/playlist/items"))
        .and(body_json(json!({"uris":["spotify:track:song"]})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"snapshot_id":"a"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/playlists/playlist/items"))
        .and(body_json(json!({"items":[{"uri":"spotify:track:song"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"snapshot_id":"b"})))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        provider
            .create_playlist("Mix", "Description", false)
            .await
            .unwrap()
            .owner_id,
        "owner"
    );
    provider
        .add_to_playlist("playlist", &["spotify:track:song".into()])
        .await
        .unwrap();
    provider
        .remove_from_playlist("playlist", &["spotify:track:song".into()])
        .await
        .unwrap();
}

#[tokio::test]
async fn playlist_details_use_put_with_name_and_description() {
    let (server, provider, _) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/playlists/playlist"))
        .and(header("authorization", "Bearer access-initial"))
        .and(body_json(
            json!({"name":"Nuevo nombre","description":"Para la tarde"}),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    provider
        .update_playlist_details("playlist", "Nuevo nombre", "Para la tarde")
        .await
        .unwrap();
    // An id that would change the route never reaches the network.
    assert!(
        provider
            .update_playlist_details("../me", "x", "")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn playlist_details_report_spotify_errors() {
    let (server, provider, _) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/playlists/foreign"))
        .respond_with(ResponseTemplate::new(403).set_body_json(
            json!({"error":{"status":403,"message":"You cannot change details of this playlist"}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        provider
            .update_playlist_details("foreign", "x", "")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn playback_commands_have_correct_verbs_query_and_optional_body() {
    let (server, provider, _) = setup().await;
    for (verb, endpoint) in [
        ("PUT", "/me/player/play"),
        ("PUT", "/me/player/pause"),
        ("POST", "/me/player/next"),
        ("POST", "/me/player/previous"),
    ] {
        Mock::given(method(verb))
            .and(path(endpoint))
            .and(body_string(""))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    for (endpoint, key, val) in [
        ("seek", "position_ms", "12345"),
        ("volume", "volume_percent", "100"),
        ("shuffle", "state", "true"),
        ("repeat", "state", "track"),
    ] {
        Mock::given(method("PUT"))
            .and(path(format!("/me/player/{endpoint}")))
            .and(query_param(key, val))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("PUT"))
        .and(path("/me/player"))
        .and(body_json(json!({"device_ids":["pc"],"play":true})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    for body in [
        json!({"context_uri":"spotify:album:album","offset":{"position":2}}),
        json!({"uris":["spotify:track:song"],"offset":{"position":0}}),
        json!({"context_uri":"spotify:playlist:playlist"}),
    ] {
        Mock::given(method("PUT"))
            .and(path("/me/player/play"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/me/player/queue"))
        .and(query_param("uri", "spotify:track:song"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    provider.play().await.unwrap();
    provider.pause().await.unwrap();
    provider.next().await.unwrap();
    provider.previous().await.unwrap();
    provider.seek(12345).await.unwrap();
    provider.set_volume(255).await.unwrap();
    provider.set_shuffle(true).await.unwrap();
    provider.set_repeat(RepeatMode::Track).await.unwrap();
    provider.transfer("pc", true).await.unwrap();
    provider
        .play_context("spotify:album:album", Some(2))
        .await
        .unwrap();
    provider
        .play_uris(&["spotify:track:song".into()], Some(0))
        .await
        .unwrap();
    provider
        .play_context("spotify:playlist:playlist", None)
        .await
        .unwrap();
    provider.add_to_queue("spotify:track:song").await.unwrap();
}

#[tokio::test]
async fn library_and_catalog_read_routes_map_wrappers() {
    let (server, provider, _) = setup().await;
    get_mock(&server, "/me/playlists", page(json!([playlist()]))).await;
    get_mock(&server, "/playlists/playlist", playlist()).await;
    get_mock(&server, "/me/tracks", page(json!([{"track":song()}]))).await;
    get_mock(
        &server,
        "/me/albums",
        page(json!([{"album":{"id":"album","name":"Album"}}])),
    )
    .await;
    get_mock(
        &server,
        "/artists/artist",
        json!({"id":"artist","name":"Artist","genres":[]}),
    )
    .await;
    get_mock(
        &server,
        "/me/player/devices",
        json!({"devices":[{"id":"pc","name":"PC","type":"Computer","is_active":true}]}),
    )
    .await;
    Mock::given(path("/artists/artist/albums"))
        .and(query_param("include_groups", "album,single"))
        .and(query_param("offset", "5"))
        .and(query_param("limit", "10"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(page(json!([{"id":"album","name":"Album"}]))),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me/following")).and(query_param("type","artist")).and(query_param("limit","50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"artists":{"items":[{"id":"artist","name":"Artist"}],"cursors":{"after":"artist"}}}))).expect(1).mount(&server).await;
    Mock::given(path("/me/player/recently-played"))
        .and(query_param("limit", "50"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"items":[{"track":song(),"played_at":"2026-03-01T00:00:00Z"}]}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(provider.my_playlists(5, 50).await.unwrap().items.len(), 1);
    assert_eq!(provider.playlist("playlist").await.unwrap().track_count, 3);
    assert_eq!(provider.saved_tracks(5, 50).await.unwrap().items.len(), 1);
    assert_eq!(provider.saved_albums(5, 50).await.unwrap().items.len(), 1);
    assert_eq!(provider.artist("artist").await.unwrap().name, "Artist");
    assert_eq!(
        provider
            .artist_albums("artist", 5, 100)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(provider.devices().await.unwrap()[0].kind, "Computer");
    assert_eq!(provider.followed_artists(50).await.unwrap().len(), 1);
    assert_eq!(provider.recently_played(100).await.unwrap().len(), 1);
}

#[tokio::test]
async fn followed_artists_uses_cursors_to_return_requested_count() {
    let (server, provider, _) = setup().await;
    let first: Vec<_> = (0..50)
        .map(|n| json!({"id":format!("artist{n}"),"name":format!("Artist {n}")}))
        .collect();
    Mock::given(method("GET")).and(path("/me/following"))
        .and(query_param("type", "artist")).and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"artists": {
            "items":first,"cursors":{"after":"artist49"},"next":"https://api.spotify.com/v1/me/following?after=artist49"
        }}))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/me/following"))
        .and(query_param("type", "artist")).and(query_param("limit", "1"))
        .and(query_param("after", "artist49"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"artists": {
            "items":[{"id":"artist50","name":"Artist 50"}],"cursors":{"after":"artist50"},"next":null
        }}))).expect(1).mount(&server).await;
    let artists = provider.followed_artists(51).await.unwrap();
    assert_eq!(artists.len(), 51);
    assert_eq!(artists[50].id, "artist50");
    assert!(provider.followed_artists(0).await.unwrap().is_empty());
}

#[tokio::test]
async fn top_items_encode_every_time_range_and_pagination() {
    let (server, provider, _) = setup().await;
    for (range, param) in [
        (TimeRange::ShortTerm, "short_term"),
        (TimeRange::MediumTerm, "medium_term"),
        (TimeRange::LongTerm, "long_term"),
    ] {
        for kind in ["tracks", "artists"] {
            Mock::given(method("GET"))
                .and(path(format!("/me/top/{kind}")))
                .and(query_param("time_range", param))
                .and(query_param("offset", "5"))
                .and(query_param("limit", "50"))
                .respond_with(ResponseTemplate::new(200).set_body_json(page(json!([]))))
                .expect(1)
                .mount(&server)
                .await;
        }
        assert!(
            provider
                .top_tracks(range, 5, 100)
                .await
                .unwrap()
                .items
                .is_empty()
        );
        assert!(
            provider
                .top_artists(range, 5, 100)
                .await
                .unwrap()
                .items
                .is_empty()
        );
    }
}

#[tokio::test]
async fn connection_errors_are_network_and_do_not_expose_tokens() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (_, mut provider, _) = setup().await;
    provider.config.api_base = format!("http://127.0.0.1:{port}");
    let error = provider.me().await.unwrap_err();
    assert!(matches!(error, VeloError::Network(_)));
    assert!(!error.to_string().contains("access-initial"));
}

#[tokio::test]
async fn session_presence_is_local_and_refresh_is_deferred_until_use() {
    let (server, provider, store) = setup().await;
    assert!(provider.is_signed_in().await);
    store.clear().unwrap();
    // Even an expired access token represents a session until its first use.
    provider
        .auth
        .lock()
        .await
        .access
        .as_mut()
        .unwrap()
        .expires_at = Instant::now();
    assert!(provider.is_signed_in().await);
    provider.auth.lock().await.access = None;
    assert!(!provider.is_signed_in().await);
    store.save("refresh-original").unwrap();
    assert!(provider.is_signed_in().await);
    assert!(server.received_requests().await.unwrap().is_empty());
    Mock::given(path("/api/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"fresh","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(provider.access_token(None).await.unwrap(), "fresh");
}

#[tokio::test]
async fn refresh_network_failure_preserves_session_and_allows_retry() {
    let (server, mut provider, store) = setup().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    provider.config.accounts_base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    provider.auth.lock().await.access = None;
    assert!(provider.is_signed_in().await);
    assert!(matches!(provider.me().await, Err(VeloError::Network(_))));
    assert_eq!(store.load().as_deref(), Some("refresh-original"));
    assert!(provider.is_signed_in().await);
    provider.config.accounts_base = server.uri();
    Mock::given(path("/api/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"retry-access","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(provider.access_token(None).await.unwrap(), "retry-access");
}

#[tokio::test]
async fn token_errors_other_than_invalid_grant_preserve_refresh_token() {
    for (status, error) in [
        (401, "invalid_client"),
        (400, "invalid_request"),
        (503, "server_error"),
    ] {
        let (server, provider, store) = setup().await;
        provider.auth.lock().await.access = None;
        Mock::given(path("/api/token"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({"error":error})))
            .expect(1)
            .mount(&server)
            .await;
        assert!(matches!(
            provider.access_token(None).await,
            Err(VeloError::Other(_))
        ));
        assert_eq!(store.load().as_deref(), Some("refresh-original"));
    }
}

struct FailingTokenStore {
    inner: Arc<MemoryTokenStore>,
    saves: AtomicUsize,
    clears: AtomicUsize,
}

impl TokenStore for FailingTokenStore {
    fn load(&self) -> Option<String> {
        self.inner.load()
    }
    fn save(&self, _: &str) -> std::result::Result<(), String> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        Err("storage unavailable".into())
    }
    fn clear(&self) -> std::result::Result<(), String> {
        self.clears.fetch_add(1, Ordering::SeqCst);
        Err("storage unavailable".into())
    }
}

#[tokio::test]
async fn failed_login_persistence_reports_spanish_error_and_preserves_old_session() {
    let (server, mut provider, store) = setup().await;
    let failing = Arc::new(FailingTokenStore {
        inner: store.clone(),
        saves: AtomicUsize::new(0),
        clears: AtomicUsize::new(0),
    });
    provider.token_store = failing.clone();
    Mock::given(path("/api/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let error = provider
        .exchange_code("code", "redirect", "verifier", 0)
        .await
        .unwrap_err();
    assert!(matches!(&error, VeloError::Other(message)
        if message == "No se pudo guardar la sesión en el Administrador de credenciales"));
    assert_eq!(failing.saves.load(Ordering::SeqCst), 1);
    assert_eq!(failing.clears.load(Ordering::SeqCst), 0);
    assert_eq!(store.load().as_deref(), Some("refresh-original"));
    assert_eq!(provider.access_token(None).await.unwrap(), "access-initial");
}

#[tokio::test]
async fn clear_failure_is_propagated_for_sign_out_and_invalid_grant() {
    let (server, mut provider, store) = setup().await;
    let failing = Arc::new(FailingTokenStore {
        inner: store.clone(),
        saves: AtomicUsize::new(0),
        clears: AtomicUsize::new(0),
    });
    provider.token_store = failing.clone();
    assert!(matches!(
        provider.sign_out().await,
        Err(VeloError::Other(_))
    ));
    Mock::given(path("/api/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"invalid_grant"})))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        provider.access_token(None).await,
        Err(VeloError::Other(_))
    ));
    assert_eq!(failing.clears.load(Ordering::SeqCst), 2);
    assert_eq!(store.load().as_deref(), Some("refresh-original"));
}

#[tokio::test]
async fn cancelled_exchange_preserves_old_session() {
    let (server, provider, store) = setup().await;
    Mock::given(path("/api/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}),
        ).set_delay(Duration::from_secs(1)))
        .mount(&server).await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            provider.exchange_code("code", "redirect", "verifier", 0)
        )
        .await
        .is_err()
    );
    assert_eq!(store.load().as_deref(), Some("refresh-original"));
    assert_eq!(provider.access_token(None).await.unwrap(), "access-initial");
}

#[tokio::test]
async fn exchange_without_new_refresh_token_preserves_old_session() {
    for refresh in [Value::Null, json!("")] {
        let (server, provider, store) = setup().await;
        Mock::given(path("/api/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"access_token":"new-access","refresh_token":refresh,"expires_in":3600}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        assert!(matches!(
            provider
                .exchange_code("code", "redirect", "verifier", 0)
                .await,
            Err(VeloError::Other(_))
        ));
        assert_eq!(store.load().as_deref(), Some("refresh-original"));
        assert_eq!(provider.access_token(None).await.unwrap(), "access-initial");
    }
}

#[tokio::test]
async fn search_beyond_window_is_empty_without_network_or_authentication() {
    let (server, provider, store) = setup().await;
    provider.auth.lock().await.access = None;
    store.clear().unwrap();
    for (offset, limit) in [(991, 10), (1000, 1), (u32::MAX, u32::MAX)] {
        let result = provider.search("query", offset, limit).await.unwrap();
        assert!(
            result.tracks.items.is_empty()
                && result.playlists.items.is_empty()
                && result.albums.items.is_empty()
                && result.artists.items.is_empty()
        );
        assert_eq!(result.tracks.offset, offset);
        assert_eq!(result.tracks.limit, limit.clamp(1, 10));
        assert!(!result.tracks.has_more());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn search_at_window_boundary_uses_clamped_limit() {
    let (server, provider, _) = setup().await;
    Mock::given(path("/search"))
        .and(query_param("offset", "990"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    provider.search("query", 990, 50).await.unwrap();
}

#[tokio::test]
async fn followed_artists_stops_on_repeated_cursor() {
    let (server, provider, _) = setup().await;
    Mock::given(path("/me/following"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"artists": {
            "items":[{"id":"artist","name":"Artist"}],
            "cursors":{"after":"artist"},"next":"https://unused.example"
        }})))
        .expect(2)
        .mount(&server)
        .await;
    assert_eq!(provider.followed_artists(100).await.unwrap().len(), 2);
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0].url.query_pairs().any(|(key, _)| key == "after"));
    assert!(
        requests[1]
            .url
            .query_pairs()
            .any(|(key, value)| key == "after" && value == "artist")
    );
}

#[tokio::test]
async fn bodiless_commands_declare_zero_content_length() {
    // Spotify rejects bodiless PUT/POST without Content-Length with 411 Length Required.
    let (server, provider, _) = setup().await;
    for (verb, endpoint) in [("PUT", "/me/player/pause"), ("POST", "/me/player/next")] {
        Mock::given(method(verb))
            .and(path(endpoint))
            .and(header("content-length", "0"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    provider.pause().await.unwrap();
    provider.next().await.unwrap();
}

#[tokio::test]
async fn other_errors_carry_spotify_message() {
    let (server, provider, _) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/me/player/play"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"status":400,"message":"Invalid context uri"}})),
        )
        .mount(&server)
        .await;
    let err = provider.play_context("spotify:x", None).await.unwrap_err();
    assert!(err.to_string().contains("Invalid context uri"));
}

#[tokio::test]
async fn artist_albums_never_asks_more_than_ten() {
    let (server, provider, _) = setup().await;
    Mock::given(method("GET"))
        .and(path("/artists/a1/albums"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(json!([]))))
        .expect(1)
        .mount(&server)
        .await;
    provider.artist_albums("a1", 0, 50).await.unwrap();
}

#[tokio::test]
async fn profiles_read_me_or_a_user_with_followers() {
    let (server, provider, _) = setup().await;
    get_mock(
        &server,
        "/me",
        json!({"id":"juan","display_name":"Juan","images":[{"url":"https://i.scdn.co/image/me","width":300,"height":300}],"followers":{"total":12}}),
    )
    .await;
    get_mock(
        &server,
        "/users/iria.sol_2",
        json!({"id":"iria.sol_2","display_name":null,"images":[]}),
    )
    .await;
    let me = provider.user_profile("me").await.unwrap();
    assert_eq!(
        (me.id.as_str(), me.display_name.as_str(), me.followers),
        ("juan", "Juan", Some(12))
    );
    assert_eq!(me.images.len(), 1);
    let other = provider.user_profile("iria.sol_2").await.unwrap();
    // No display name: the id stands in; no followers object is unknown, not zero.
    assert_eq!(other.display_name, "iria.sol_2");
    assert_eq!(other.followers, None);
}

#[tokio::test]
async fn profile_rejects_user_ids_that_would_change_the_path() {
    let (server, provider, _) = setup().await;
    for id in ["", "a/b", "x?y=1", "..%2F", "a b"] {
        assert!(provider.user_profile(id).await.is_err(), "{id}");
        assert!(
            provider.user_public_playlists(id, 0, 50).await.is_err(),
            "{id}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn public_playlists_keep_only_public_ones_the_user_owns() {
    let (server, provider, _) = setup().await;
    let owned = |id: &str, public: Value| {
        json!({"id":id,"name":id,"uri":format!("spotify:playlist:{id}"),"public":public,
               "owner":{"id":"iria","display_name":"Iria"},"items":{"total":4},"images":[]})
    };
    Mock::given(method("GET"))
        .and(path("/users/iria/playlists"))
        .and(query_param("offset", "0"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items":[owned("pub", json!(true)), owned("priv", json!(false)), owned("unknown", Value::Null), playlist()],
            "offset":0,"limit":50,"total":120
        })))
        .expect(1)
        .mount(&server)
        .await;
    let page = provider.user_public_playlists("iria", 0, 50).await.unwrap();
    let ids: Vec<_> = page.items.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["pub", "unknown"]);
    assert_eq!(page.items[0].owner, "Iria");
    assert_eq!(page.items[0].track_count, 4);
    // Paging still describes Spotify's unfiltered list.
    assert_eq!((page.offset, page.limit, page.total), (0, 50, 120));
}

/// L1: tokens handed out by an external source (the librespot session's login5).
struct Source {
    calls: AtomicUsize,
    rejected: std::sync::Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl TokenSource for Source {
    async fn token(&self, rejected: Option<&str>) -> Result<(String, Duration)> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.rejected
            .lock()
            .unwrap()
            .push(rejected.map(str::to_string));
        Ok((format!("session-{n}"), Duration::from_secs(3600)))
    }
}

fn sourced(server: &MockServer) -> (SpotifyProvider, Arc<Source>) {
    let source = Arc::new(Source {
        calls: AtomicUsize::new(0),
        rejected: std::sync::Mutex::new(Vec::new()),
    });
    let provider = SpotifyProvider::with_token_source(
        SpotifyConfig {
            client_id: String::new(),
            api_base: server.uri(),
            accounts_base: server.uri(),
            redirect_port: 0,
        },
        source.clone(),
    );
    (provider, source)
}

#[tokio::test]
async fn external_token_source_is_used_cached_and_replaced_once_after_401() {
    let server = MockServer::start().await;
    let (provider, source) = sourced(&server);
    assert!(provider.is_signed_in().await);
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer session-0"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer session-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"me","display_name":"Me","product":"premium"})),
        )
        .expect(2)
        .mount(&server)
        .await;
    assert_eq!(provider.me().await.unwrap().id, "me");
    // Cached: the second call reuses session-1 without asking the source again.
    assert_eq!(provider.me().await.unwrap().id, "me");
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        *source.rejected.lock().unwrap(),
        [None, Some("session-0".to_string())]
    );
    // The browser login belongs to the source, never to this provider.
    assert!(provider.sign_in().await.is_err());
}

#[tokio::test]
async fn long_429_pauses_requests_locally_without_a_quota_warning() {
    let server = MockServer::start().await;
    let (provider, _source) = sourced(&server);
    let mut warnings = provider.app_unavailable.subscribe();
    Mock::given(method("GET"))
        .and(path("/me/player/devices"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "50"))
        .expect(1)
        .mount(&server)
        .await;
    for _ in 0..3 {
        match provider.devices().await {
            Err(VeloError::RateLimited(left)) => {
                assert!(left > Duration::from_secs(40) && left <= Duration::from_secs(50))
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }
    assert!(warnings.try_recv().is_err());
}
