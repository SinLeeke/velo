use super::*;
use librespot_protocol::playlist4_external::{Item, ListAttributes, ListItems, PictureSize};

fn me() -> UserProfile {
    UserProfile {
        id: "juan".into(),
        display_name: "Juan".into(),
        product: "premium".into(),
        images: vec![],
    }
}

fn meta(name: &str, owner: &str, length: i32, picture: &[u8]) -> MetaItem {
    let mut attributes = ListAttributes::new();
    attributes.set_name(name.into());
    attributes.set_picture(picture.to_vec());
    let mut size = PictureSize::new();
    size.set_target_name("default".into());
    size.set_url("https://pickasso.spotifycdn.com/image/x".into());
    attributes.picture_size.push(size);
    let mut m = MetaItem::new();
    m.attributes = protobuf::MessageField::some(attributes);
    m.set_owner_username(owner.into());
    m.set_length(length);
    m
}

fn item(uri: &str) -> Item {
    let mut i = Item::new();
    i.set_uri(uri.into());
    i
}

#[test]
fn rootlist_maps_playlists_skips_folders_and_names_own_ones() {
    let mut contents = ListItems::new();
    contents.items = vec![
        item("spotify:start-group:abc:Carpeta"),
        item("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
        item("spotify:playlist:4uLU6hMCjMI75M1A2tKUQC"),
        item("spotify:end-group:abc"),
    ];
    contents.meta_items = vec![
        MetaItem::new(),
        meta("Top", "spotify", 50, &[]),
        meta("Mía", "juan", 3, &[0xab; 20]),
        MetaItem::new(),
    ];
    let mut list = SelectedListContent::new();
    list.contents = protobuf::MessageField::some(contents);
    list.set_length(4);
    let page = rootlist_page(&list, 0, 50, Some(&me()));
    assert_eq!(page.total, 4);
    assert_eq!(page.items.len(), 2);
    let top = &page.items[0];
    assert_eq!(top.id, "37i9dQZF1DXcBWIGoYBM5M");
    assert_eq!(top.owner, "spotify");
    assert_eq!(top.track_count, 50);
    assert!(top.images[0].url.starts_with("https://pickasso."));
    let mine = &page.items[1];
    assert_eq!(
        (mine.owner.as_str(), mine.owner_id.as_str()),
        ("Juan", "juan")
    );
    assert!(mine.items_listable_by("juan"));
    assert_eq!(
        mine.images[0].url,
        format!("{IMAGE_HOST}{}", "ab".repeat(20))
    );
}

#[test]
fn profile_json_maps_name_image_and_falls_back_to_the_username() {
    let json = serde_json::json!({"name": "Juan", "image_url": "https://i.scdn.co/image/me"});
    let p = profile_from("juan", "premium", &json);
    assert_eq!((p.id.as_str(), p.display_name.as_str()), ("juan", "Juan"));
    assert_eq!(p.images[0].url, "https://i.scdn.co/image/me");
    let bare = profile_from(
        "juan",
        "free",
        &serde_json::json!({"image_url": "http://x"}),
    );
    assert_eq!(bare.display_name, "juan");
    assert!(bare.images.is_empty());
}

#[test]
fn saved_uris_go_to_their_set() {
    assert_eq!(
        set_of("spotify:track:4uLU6hMCjMI75M1A2tKUQC"),
        Some(LIKED_SET)
    );
    assert_eq!(
        set_of("spotify:album:4uLU6hMCjMI75M1A2tKUQC"),
        Some(LIKED_SET)
    );
    assert_eq!(
        set_of("spotify:artist:4uLU6hMCjMI75M1A2tKUQC"),
        Some(ARTIST_SET)
    );
    assert_eq!(set_of("spotify:playlist:4uLU6hMCjMI75M1A2tKUQC"), None);
}

/// Without a session every read falls back to the Web API (here a local mock).
struct NoSession;
#[async_trait]
impl SessionSource for NoSession {
    async fn session(&self) -> Option<Session> {
        None
    }
}

#[tokio::test]
async fn without_a_session_reads_fall_back_to_the_web_api_and_429_is_reported() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/me/playlists"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30"))
        .expect(1)
        .mount(&server)
        .await;
    let config = SpotifyConfig {
        client_id: String::new(),
        api_base: server.uri(),
        accounts_base: server.uri(),
        redirect_port: 0,
    };
    // No session: login5 has no token either, so the Web API is never reached, and a session
    // that is still connecting is a transient error, never "signed out".
    let api = SessionApi::with_web_config(Arc::new(NoSession), config.clone());
    assert!(matches!(api.me().await, Err(VeloError::Network(_))));
    // With a token, the rootlist falls back to the Web API, which answers 429 once; the next
    // call fails locally (no retry storm).
    struct Fixed;
    #[async_trait]
    impl TokenSource for Fixed {
        async fn token(&self, _: Option<&str>) -> Result<(String, Duration)> {
            Ok(("t".into(), Duration::from_secs(3600)))
        }
    }
    let web = Arc::new(SpotifyProvider::with_token_source(config, Arc::new(Fixed)));
    let api = SessionApi::with_web(Arc::new(NoSession), web);
    for _ in 0..2 {
        assert!(matches!(
            api.my_playlists(0, 20).await,
            Err(VeloError::RateLimited(_))
        ));
    }
}

struct ConnectFixture(std::sync::Mutex<Option<Vec<Device>>>);
#[async_trait]
impl SessionSource for ConnectFixture {
    async fn session(&self) -> Option<Session> {
        None
    }
    fn connect_devices(&self) -> Option<Vec<Device>> {
        self.0.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn v92_devices_use_dealer_on_empty_429_or_401_and_drop_disconnected_snapshot() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};
    struct Fixed(std::sync::atomic::AtomicUsize);
    #[async_trait]
    impl TokenSource for Fixed {
        async fn token(&self, _: Option<&str>) -> Result<(String, Duration)> {
            let sequence = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok((format!("fixture-{sequence}"), Duration::from_secs(3600)))
        }
    }
    for status in [200, 429, 401] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/player/devices"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("retry-after", "30")
                    .set_body_json(serde_json::json!({"devices":[]})),
            )
            .expect(if status == 401 {
                4
            } else if status == 200 {
                2
            } else {
                1
            })
            .mount(&server)
            .await;
        let phone = Device {
            id: Some("phone".into()),
            name: "Pixel".into(),
            kind: "Smartphone".into(),
            is_active: true,
            volume_percent: None,
            supports_volume: false,
        };
        let source = Arc::new(ConnectFixture(std::sync::Mutex::new(Some(vec![
            phone.clone(),
        ]))));
        let web = Arc::new(SpotifyProvider::with_token_source(
            SpotifyConfig {
                client_id: String::new(),
                api_base: server.uri(),
                accounts_base: server.uri(),
                redirect_port: 0,
            },
            Arc::new(Fixed(std::sync::atomic::AtomicUsize::new(0))),
        ));
        let api = SessionApi::with_web(source.clone(), web);
        assert_eq!(api.devices().await.unwrap(), vec![phone]);
        *source.0.lock().unwrap() = None;
        let next = api.devices().await;
        match status {
            200 => assert!(next.unwrap().is_empty()),
            429 => assert!(matches!(next, Err(VeloError::RateLimited(_)))),
            401 => assert!(matches!(next, Err(VeloError::NotAuthenticated))),
            _ => unreachable!(),
        }
    }
}
