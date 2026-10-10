//! L1: ¿sirve el token login5 de la sesión de librespot para la Web API? (como spotify-player y
//! ncspot). Lecturas por defecto; smart-shuffle --apply requiere flags observados.
//! Imprime status HTTP, tipo/expiración/scopes del token y nunca el
//! token ni datos personales.
//!
//! `cargo run -p velo-player --example webapi_token_probe -- oauth` → abre el navegador (cliente
//!   de librespot, scopes de la Web API + `streaming`) y guarda el refresh token del probe en el
//!   keyring (entrada propia `velo-L1-probe-refresh`, nunca en archivos).
//! `cargo run -p velo-player --example webapi_token_probe` → reutiliza esa entrada (o, si no hay,
//!   el login de audio de Velo).
//! `cargo run -p velo-player --example webapi_token_probe -- forget` → borra la entrada del probe.
//! `cargo run -p velo-player --example webapi_token_probe -- profile <user_id>` → guarda
//! respuestas recortadas (metadatos públicos, nunca tokens) en tests/fixtures/spclient.
//! `-- credits <track_uri>` captura créditos JSON completos; `-- artist <artist_uri>`
//! resume estadísticas y biografía; `-- smart-shuffle` observa Connect durante 30 s;
//! `-- taste-exclude <playlist_uri>` investiga atributos/schema en modo solo lectura.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use librespot_core::{Session, SessionConfig, SpotifyId, authentication::Credentials};
use librespot_oauth::OAuthClientBuilder;
use protobuf::Message;
use velo_player::login::{self, LIBRESPOT_CLIENT_ID, REDIRECT_URI};
use velo_spotify::{KeyringTokenStore, TokenStore};

const API: &str = "https://api.spotify.com/v1";
const PROBE_ENTRY: &str = "velo-L1-probe-refresh";
const WEB_SCOPES: &str = "streaming user-read-playback-state user-modify-playback-state user-read-currently-playing user-read-private playlist-read-private playlist-read-collaborative playlist-modify-private playlist-modify-public user-library-read user-library-modify user-read-recently-played user-top-read user-follow-read";

const OWN_ENTRY: &str = "velo-L1-probe-own-refresh";

type Get = (u16, Option<String>, String);

/// Misma cuenta e IP con la app personal (PKCE de velo-spotify, redirect 127.0.0.1:8989): si
/// aquí hay 200 y con el cliente de librespot 429, el límite es por app y no por cuenta/IP.
async fn own_app(env_file: Option<String>) -> Result<()> {
    use velo_domain::{MusicProvider, TimeRange};
    use velo_spotify::{SpotifyConfig, SpotifyProvider};
    let path = env_file.unwrap_or_else(|| "../../../velo.local.env".into());
    let client_id = std::fs::read_to_string(&path)
        .with_context(|| format!("no se pudo leer {path}"))?
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("VELO_SPOTIFY_CLIENT_ID=")
                .map(|v| v.trim().to_string())
        })
        .context("falta VELO_SPOTIFY_CLIENT_ID")?;
    let provider = SpotifyProvider::new(
        SpotifyConfig {
            client_id,
            ..SpotifyConfig::default()
        },
        Arc::new(KeyringTokenStore::with_user(OWN_ENTRY)),
    );
    if provider.is_signed_in().await {
        println!("usando el refresh de la app personal guardado ({OWN_ENTRY})");
    } else {
        let r = provider.sign_in().await;
        println!("sign_in app personal: {}", kind(&r));
    }
    fn kind<T>(r: &velo_domain::Result<T>) -> String {
        match r {
            Ok(_) => "200 OK".into(),
            Err(e) => format!(
                "ERROR {}",
                format!("{e:?}").chars().take(60).collect::<String>()
            ),
        }
    }
    println!("{:<40} {}", "/me", kind(&provider.me().await));
    println!(
        "{:<40} {}",
        "/me/playlists",
        kind(&provider.my_playlists(0, 1).await)
    );
    println!(
        "{:<40} {}",
        "/me/tracks",
        kind(&provider.saved_tracks(0, 1).await)
    );
    println!(
        "{:<40} {}",
        "/search",
        kind(&provider.search("queen", 0, 1).await)
    );
    println!(
        "{:<40} {}",
        "/me/player/devices",
        kind(&provider.devices().await)
    );
    println!(
        "{:<40} {}",
        "/me/top/tracks",
        kind(&provider.top_tracks(TimeRange::ShortTerm, 0, 1).await)
    );
    println!(
        "{:<40} {}",
        "/me/following",
        kind(&provider.followed_artists(1).await)
    );
    Ok(())
}

async fn get(
    http: &reqwest::Client,
    url: &str,
    bearer: &str,
    client_token: Option<&str>,
) -> Result<Get> {
    let mut request = http.get(url).bearer_auth(bearer);
    if let Some(ct) = client_token {
        request = request.header("client-token", ct);
    }
    let response = request.send().await?;
    let status = response.status().as_u16();
    let retry = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = response.text().await.unwrap_or_default();
    Ok((status, retry, body))
}

fn line(label: &str, (status, retry, _): &Get) {
    println!("{label:<58} {status} {}", retry.as_deref().unwrap_or(""));
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    velo_player::install_crypto_provider();
    let mode = std::env::args().nth(1).unwrap_or_default();
    let probe_store: Arc<dyn TokenStore> = Arc::new(KeyringTokenStore::with_user(PROBE_ENTRY));
    if mode == "forget" {
        probe_store.clear().map_err(anyhow::Error::msg)?;
        KeyringTokenStore::with_user(OWN_ENTRY)
            .clear()
            .map_err(anyhow::Error::msg)?;
        println!("entradas del probe borradas");
        return Ok(());
    }
    if mode == "own" {
        return own_app(std::env::args().nth(2)).await;
    }

    let access = if mode == "oauth" {
        let scopes: Vec<&str> = WEB_SCOPES.split(' ').collect();
        let client = OAuthClientBuilder::new(LIBRESPOT_CLIENT_ID, REDIRECT_URI, scopes)
            .open_in_browser()
            .with_custom_message("<!doctype html><p>Velo (prueba L1): puedes volver.</p>")
            .build()?;
        let token = client.get_access_token_async().await?;
        println!(
            "OAuth librespot OK; scopes concedidos ({}): {:?}",
            token.scopes.len(),
            token.scopes
        );
        if !token.refresh_token.is_empty() {
            probe_store
                .save(&token.refresh_token)
                .map_err(anyhow::Error::msg)?;
            println!("refresh token del probe guardado en el keyring ({PROBE_ENTRY})");
        }
        token.access_token
    } else if probe_store.load().is_some() {
        println!("usando el login del probe guardado en el keyring");
        login::access_token_from_store(probe_store.clone())
            .await
            .context("refresh del probe")?
    } else {
        println!("usando el login de audio de Velo");
        login::access_token_from_store(login::keyring_store())
            .await
            .context("no hay login guardado; usa `-- oauth`")?
    };

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    // Sesión aparte de la de Velo (device id distinto); sin caché ni credenciales en disco.
    let session = Session::new(
        SessionConfig {
            device_id: velo_player::stable_device_id("velo-L1-webapi-probe"),
            ..SessionConfig::default()
        },
        None,
    );
    session
        .connect(Credentials::with_access_token(access.clone()), false)
        .await
        .context("sesión librespot")?;
    println!("sesión librespot OK");
    if mode == "profile" {
        let user_id = std::env::args().nth(2).context("profile <user_id>")?;
        return capture_profile(&session, &user_id).await;
    }
    if matches!(
        mode.as_str(),
        "credits" | "artist" | "smart-shuffle" | "taste-exclude"
    ) {
        let args: Vec<_> = std::env::args().skip(2).collect();
        let result = match mode.as_str() {
            "credits" => {
                capture_credits(&session, args.first().context("credits <track_uri>")?).await
            }
            "artist" => probe_artist(&session, args.first().context("artist <artist_uri>")?).await,
            "smart-shuffle" => probe_smart_shuffle(&session, &http, &args).await,
            _ => {
                probe_taste_exclude(
                    &session,
                    &http,
                    args.first().context("taste-exclude <playlist_uri>")?,
                )
                .await
            }
        };
        session.dealer().close().await;
        session.shutdown();
        return result;
    }
    let client_token = session.spclient().client_token().await.ok();
    println!("client-token disponible: {}", client_token.is_some());
    let ct = client_token.as_deref();

    let started = Instant::now();
    let token = session.login5().auth_token().await.context("login5")?;
    println!(
        "login5 OK en {} ms: tipo {}, expira en {} s, scopes ({}): {:?}",
        started.elapsed().as_millis(),
        token.token_type,
        token.expires_in.as_secs(),
        token.scopes.len(),
        token.scopes
    );
    let l5 = token.access_token.as_str();
    if mode == "sp" {
        return spclient_shapes(&session).await;
    }
    if mode == "api" {
        return session_api(session).await;
    }
    if mode == "pf" {
        // pf <operationName> <sha256> <variables JSON> <archivo de salida>
        let a: Vec<String> = std::env::args().skip(2).collect();
        let url = velo_player::session_api::pathfinder::query_url(
            &a[0],
            &serde_json::from_str(&a[2])?,
            &a[1],
        );
        let (status, _, body) = get(&http, &url, l5, ct).await?;
        std::fs::write(&a[3], &body)?;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let errors: Vec<_> = v["errors"]
            .as_array()
            .map(|e| {
                e.iter()
                    .map(|e| {
                        e["message"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(200)
                            .collect::<String>()
                    })
                    .collect()
            })
            .unwrap_or_default();
        println!(
            "{} {status} data={} bytes={} errors={errors:?}",
            a[0],
            !v["data"].is_null(),
            body.len()
        );
        session.shutdown();
        return Ok(());
    }

    println!("\n== api.spotify.com ==");
    let me = format!("{API}/me");
    line(
        "/me  token OAuth, sin client-token",
        &get(&http, &me, &access, None).await?,
    );
    line(
        "/me  token OAuth, con client-token",
        &get(&http, &me, &access, ct).await?,
    );
    line(
        "/me  login5, sin client-token",
        &get(&http, &me, l5, None).await?,
    );
    line(
        "/me  login5, con client-token",
        &get(&http, &me, l5, ct).await?,
    );
    for path in [
        "/me/playlists?limit=1",
        "/me/tracks?limit=1",
        "/search?q=queen&type=track&limit=1",
        "/me/player/devices",
        "/me/top/tracks?limit=1",
        "/me/following?type=artist&limit=1",
    ] {
        let url = format!("{API}{path}");
        line(
            &format!("{path}  login5+ct"),
            &get(&http, &url, l5, ct).await?,
        );
    }

    println!("\n== spclient (misma vía que audio/discover de Velo) ==");
    let username = session.username();
    match session
        .spclient()
        .get_user_profile(&username, Some(1), Some(1))
        .await
    {
        Ok(body) => println!(
            "{:<58} OK ({} bytes, json={})",
            "user-profile-view/v3/profile/<yo>",
            body.len(),
            serde_json::from_slice::<serde_json::Value>(&body).is_ok()
        ),
        Err(e) => println!(
            "{:<58} ERROR {:?}",
            "user-profile-view/v3/profile/<yo>", e.kind
        ),
    }
    let mut first_playlist = None;
    match session.spclient().get_rootlist(0, Some(10)).await {
        Ok(body) => {
            use librespot_protocol::playlist4_external::SelectedListContent;
            let list = SelectedListContent::parse_from_bytes(&body).ok();
            let n = list.as_ref().map(|l| l.contents.items.len()).unwrap_or(0);
            first_playlist = list.and_then(|l| {
                l.contents.items.iter().find_map(|i| {
                    i.uri()
                        .strip_prefix("spotify:playlist:")
                        .map(str::to_string)
                })
            });
            println!(
                "{:<58} OK ({n} elementos)",
                "playlist/v2/user/<yo>/rootlist"
            );
        }
        Err(e) => println!(
            "{:<58} ERROR {:?}",
            "playlist/v2/user/<yo>/rootlist", e.kind
        ),
    }
    if let Some(id) = first_playlist {
        let id = SpotifyId::from_base62(&id)?;
        match session.spclient().get_playlist(&id).await {
            Ok(body) => println!(
                "{:<58} OK ({} bytes)",
                "playlist/v2/playlist/<una>",
                body.len()
            ),
            Err(e) => println!("{:<58} ERROR {:?}", "playlist/v2/playlist/<una>", e.kind),
        }
        let url = format!("{API}/playlists/{}/items?limit=1", id.to_base62()?);
        line(
            "api /playlists/<una>/items  login5+ct",
            &get(&http, &url, l5, ct).await?,
        );
    }

    println!("\n== api-partner pathfinder ==");
    let pathfinder = "https://api-partner.spotify.com/pathfinder/v1/query?operationName=profileAttributes&variables=%7B%7D&extensions=%7B%22persistedQuery%22%3A%7B%22version%22%3A1%2C%22sha256Hash%22%3A%2253bcb064f6cd18c23f752bc324a791194d20df612d8e1239c735144ab0399ced%22%7D%7D";
    let pf = get(&http, pathfinder, l5, ct).await?;
    let looks_ok = serde_json::from_str::<serde_json::Value>(&pf.2)
        .map(|v| v.get("data").is_some())
        .unwrap_or(false);
    println!(
        "{:<58} {} data={looks_ok} {}",
        "pathfinder profileAttributes  login5+ct",
        pf.0,
        pf.1.as_deref().unwrap_or("")
    );

    // ¿Ventana corta? Respeta retry-after y repite /me con login5.
    for round in 1..=3 {
        let r = get(&http, &me, l5, ct).await?;
        let wait: u64 = r.1.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        println!("ronda {round}: /me login5+ct {} retry-after {wait}", r.0);
        if r.0 != 429 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(wait.min(120) + 2)).await;
    }
    session.shutdown();
    Ok(())
}

async fn capture_credits(session: &Session, uri: &str) -> Result<()> {
    let value = velo_player::extras::track_credits(session, uri).await?;
    let trimmed = serde_json::json!({"trackName": value["title"],
        "roleCredits": value["roleCredits"], "sourceNames": value["sourceNames"]});
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spclient");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("credits.json"),
        serde_json::to_vec_pretty(&trimmed)?,
    )?;
    println!(
        "credits.json: {} colaboradores, {} grupos, {} fuentes; captura recortada sin credenciales",
        value["contributors"].as_array().map_or(0, Vec::len),
        value["roleCredits"].as_array().map_or(0, Vec::len),
        value["sourceNames"].as_array().map_or(0, Vec::len)
    );
    Ok(())
}

async fn probe_artist(session: &Session, uri: &str) -> Result<()> {
    let a = velo_player::session_api::catalog::artist_overview(session, uri)
        .await
        .map_err(|_| anyhow::anyhow!("overview no disponible; error upstream omitido"))?;
    println!(
        "monthlyListeners={} followers={} verified={} biographyChars={} topCities={}",
        a.monthly_listeners,
        a.followers,
        a.verified,
        a.biography.chars().count(),
        serde_json::to_string(&a.top_cities)?
    );
    Ok(())
}

/// Direct session-authenticated HTTP; mutations have no retries and errors never
/// include an upstream body, URL, token or arbitrary response header.
async fn probe_request(
    session: &Session,
    http: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<(u16, bytes::Bytes)> {
    let token = session
        .login5()
        .auth_token()
        .await
        .map_err(|_| anyhow::anyhow!("login5 no disponible"))?;
    let mut request = http.request(method, url).bearer_auth(&token.access_token);
    if let Ok(ct) = session.spclient().client_token().await {
        request = request.header("client-token", ct);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("petición no disponible"))?;
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .await
        .map_err(|_| anyhow::anyhow!("respuesta no disponible"))?;
    Ok((status, body))
}

async fn probe_smart_shuffle(
    session: &Session,
    http: &reqwest::Client,
    args: &[String],
) -> Result<()> {
    use futures_util::StreamExt;
    use librespot_core::dealer::protocol::Message as DealerMessage;
    use librespot_protocol::connect::{Cluster, ClusterUpdate};
    use serde_json::json;
    let target = velo_player::EngineConfig::velo("Velo").device_id;
    let apply = args.iter().any(|s| s == "--apply");
    let arg = |key| {
        args.windows(2)
            .find(|pair| pair[0] == key)
            .map(|pair| pair[1].clone())
    };
    let mode_key = arg("--mode-key");
    let mode_value = arg("--mode-value");
    if apply && (mode_key.is_none() || mode_value.is_none()) {
        anyhow::bail!(
            "--apply requiere --mode-key CLAVE --mode-value VALOR observados en el cliente oficial; no se inventa una clave de smart shuffle"
        );
    }
    let mut updates = session
        .dealer()
        .listen_for::<ClusterUpdate>("hm://connect-state/v1/cluster", DealerMessage::from_raw)
        .map_err(|_| anyhow::anyhow!("suscripción connect-state no disponible"))?;
    tokio::time::timeout(Duration::from_secs(15), session.dealer().start())
        .await
        .map_err(|_| anyhow::anyhow!("Dealer tardó demasiado"))?
        .map_err(|_| anyhow::anyhow!("Dealer no disponible"))?;
    let base = session
        .spclient()
        .base_url()
        .await
        .map_err(|_| anyhow::anyhow!("spclient no disponible"))?;
    // The path is present in upstream for PUT/DELETE; GET is a read candidate.
    let url = format!("{base}/connect-state/v1/devices/{target}");
    let (status, body) = probe_request(session, http, reqwest::Method::GET, &url, None).await?;
    println!(
        "GET connect-state del dispositivo Velo: HTTP {status}; {} bytes (cuerpo omitido)",
        body.len()
    );
    let mut pending = if (200..300).contains(&status) {
        Cluster::parse_from_bytes(&body)
            .ok()
            .filter(|c| !c.device.is_empty())
    } else {
        None
    };
    println!(
        "Durante 30 s activa Velo y alterna smart shuffle en el cliente oficial; se observan options/modes, sin identidades."
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut sent = false;
    let mut observed = 0;
    loop {
        let cluster = if let Some(c) = pending.take() {
            c
        } else {
            match tokio::time::timeout_at(deadline, updates.next()).await {
                Ok(Some(Ok(update))) => match update.cluster.into_option() {
                    Some(c) => c,
                    None => continue,
                },
                Ok(Some(Err(_))) => {
                    println!("evento connect-state no decodificable");
                    continue;
                }
                _ => break,
            }
        };
        let is_velo = cluster.active_device_id == target;
        let options = &cluster.player_state.options;
        let modes: std::collections::BTreeMap<_, _> = options
            .modes
            .iter()
            .map(|(key, value)| (key, probe_flag_value(value)))
            .collect();
        println!(
            "connect-state: VeloPresent={} activeIsVelo={} shuffle={} modes={modes:?}",
            cluster.device.contains_key(&target),
            is_velo,
            options.shuffling_context
        );
        observed += 1;
        if apply && is_velo && !sent {
            let key = mode_key.as_ref().unwrap();
            let value = mode_value.as_ref().unwrap();
            let command = json!({"command":{"endpoint":"set_options", "shuffling_context":true,
                "modes":{key:value}, "logging_params":{}}});
            let url = format!(
                "{base}/connect-state/v1/player/command/from/{}/to/{target}",
                session.device_id()
            );
            let (status, body) =
                probe_request(session, http, reqwest::Method::POST, &url, Some(&command)).await?;
            println!(
                "set_options + modes candidato: HTTP {status}; {} bytes; confirmar cambio en Connect y recomendaciones (HTTP no prueba efecto)",
                body.len()
            );
            sent = true;
        }
    }
    println!(
        "{observed} estados observados; comando enviado={sent}. Clave modes/endpoint y efecto pendientes de comparar con el cliente oficial."
    );
    Ok(())
}

async fn probe_taste_exclude(session: &Session, http: &reqwest::Client, uri: &str) -> Result<()> {
    use librespot_protocol::playlist4_external::SelectedListContent;
    use serde_json::{Value, json};
    let id = uri
        .strip_prefix("spotify:playlist:")
        .context("taste-exclude <spotify:playlist:ID>")?;
    let id = SpotifyId::from_base62(id).map_err(|_| anyhow::anyhow!("playlist inválida"))?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        session.spclient().get_playlist(&id),
    )
    .await;
    if let Ok(Ok(body)) = result {
        if let Ok(list) = SelectedListContent::parse_from_bytes(&body) {
            let candidates: Vec<_> = list
                .attributes
                .format_attributes
                .iter()
                .filter(|a| {
                    let key = a.key().to_ascii_lowercase();
                    key.contains("taste") || key.contains("exclu")
                })
                .map(|a| json!({"key": a.key(), "flagValue": probe_flag_value(a.value())}))
                .collect();
            println!(
                "playlist/v2: atributos candidatos={candidates:?}; {} atributos totales; solo flags conocidos, cuerpo omitido",
                list.attributes.format_attributes.len()
            );
        } else {
            println!("playlist/v2: schema no reconocido, cuerpo omitido");
        }
    } else {
        println!("playlist/v2: no disponible, error upstream omitido");
    }
    // Read-only schema discovery, never an exclusion mutation. Unknown hashes are
    // not fabricated; a live schema may reveal the real operation and its arguments.
    let query = json!({"operationName":"VeloTasteExcludeSchema", "variables":{},
        "query":"query VeloTasteExcludeSchema { __type(name: \"Playlist\") { fields { name } } __schema { mutationType { fields { name args { name type { kind name ofType { kind name } } } } } } }"});
    let (status, body) = probe_request(
        session,
        http,
        reqwest::Method::POST,
        "https://api-partner.spotify.com/pathfinder/v1/query",
        Some(&query),
    )
    .await?;
    let value: Value = serde_json::from_slice(&body).unwrap_or_default();
    let interesting = |field: &&Value| {
        field["name"].as_str().is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            name.contains("taste") || name.contains("exclu")
        })
    };
    let fields: Vec<_> = value["data"]["__type"]["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(interesting)
        .map(|f| json!({"name":f["name"]}))
        .collect();
    let mutations: Vec<_> = value["data"]["__schema"]["mutationType"]["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(interesting)
        .map(|f| json!({"name":f["name"], "args":f["args"]}))
        .collect();
    println!(
        "pathfinder introspection (solo lectura): HTTP {status}; data={} errors={} campos={fields:?} operaciones={mutations:?}",
        !value["data"].is_null(),
        value["errors"].as_array().map_or(0, Vec::len)
    );
    println!(
        "Comparar atributos antes/después de excluir/reincluir desde el cliente oficial. Si la introspección entrega candidato, capturar la consulta de estado y método/variables/hash antes de habilitar --apply."
    );
    Ok(())
}

/// Only known flag scalars are printable; arbitrary upstream strings stay hidden.
fn probe_flag_value(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Some("true"),
        "false" => Some("false"),
        "0" => Some("0"),
        "1" => Some("1"),
        "on" => Some("on"),
        "off" => Some("off"),
        "enabled" => Some("enabled"),
        "disabled" => Some("disabled"),
        "normal" => Some("normal"),
        "smart" => Some("smart"),
        "enhanced" => Some("enhanced"),
        "default" => Some("default"),
        _ => None,
    }
}

async fn capture_profile(session: &Session, user_id: &str) -> Result<()> {
    use serde_json::{Value, json};
    use velo_player::session_api::catalog::encoded_user_id;
    let username = if user_id == "me" {
        session.username()
    } else {
        user_id.into()
    };
    let username =
        encoded_user_id(&username).map_err(|_| anyhow::anyhow!("id de usuario inválido"))?;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spclient");
    std::fs::create_dir_all(&dir)?;
    for (file, path) in [
        (
            "profile-real.json",
            format!("/user-profile-view/v3/profile/{username}?playlist_limit=3&artist_limit=0"),
        ),
        (
            "profile-playlists-real.json",
            format!("/user-profile-view/v3/profile/{username}/playlists?offset=0&limit=3"),
        ),
    ] {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            session
                .spclient()
                .request_as_json(&http::Method::GET, &path, None, None),
        )
        .await;
        let body = match result {
            Ok(Ok(body)) => body,
            _ => {
                println!("{file}: respuesta no disponible");
                continue;
            }
        };
        let value: Value = serde_json::from_slice(&body).context("JSON de perfil no válido")?;
        // Whitelist only parser-relevant public metadata. No raw upstream body on disk/logs.
        let mut trimmed = json!({});
        for key in [
            "name",
            "image_url",
            "followers_count",
            "total_public_playlists_count",
        ] {
            if let Some(v) = value.get(key) {
                trimmed[key] = v.clone();
            }
        }
        if let Some(rows) = value["public_playlists"].as_array() {
            trimmed["public_playlists"] = Value::Array(
                rows.iter()
                    .take(3)
                    .map(|row| {
                        let mut entry = json!({});
                        for key in [
                            "uri",
                            "name",
                            "image_url",
                            "owner_name",
                            "owner_uri",
                            "followers_count",
                            "tracks_count",
                            "public",
                        ] {
                            if let Some(v) = row.get(key) {
                                entry[key] = v.clone();
                            }
                        }
                        entry
                    })
                    .collect(),
            );
        }
        std::fs::write(dir.join(file), serde_json::to_vec_pretty(&trimmed)?)?;
        println!("{file}: metadatos públicos recortados guardados");
    }
    Ok(())
}

/// Formas (no valores) de lo que daría el proveedor interno: claves, conteos, tipos.
async fn spclient_shapes(session: &Session) -> Result<()> {
    use librespot_protocol::playlist4_external::SelectedListContent;
    use velo_player::collection;
    println!("atributo type = {:?}", session.get_user_attribute("type"));
    let username = session.username();
    match session
        .spclient()
        .get_user_profile(&username, Some(1), Some(1))
        .await
    {
        Ok(body) => {
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let keys: Vec<_> = v
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            println!(
                "perfil: claves {keys:?}; name? {} image_url? {}",
                v["name"].is_string(),
                v["image_url"].is_string()
            );
        }
        Err(e) => println!("perfil ERROR {:?}", e.kind),
    }
    match session.spclient().get_rootlist(0, Some(50)).await {
        Ok(body) => {
            let l = SelectedListContent::parse_from_bytes(&body)?;
            let c = &l.contents;
            let kinds: std::collections::BTreeMap<String, usize> =
                c.items.iter().fold(Default::default(), |mut m, i| {
                    *m.entry(i.uri().split(':').take(2).collect::<Vec<_>>().join(":"))
                        .or_default() += 1;
                    m
                });
            let named = c
                .meta_items
                .iter()
                .filter(|m| !m.attributes.name().is_empty())
                .count();
            let pic = c
                .meta_items
                .iter()
                .filter(|m| !m.attributes.picture().is_empty())
                .count();
            let sizes = c
                .meta_items
                .iter()
                .filter(|m| !m.attributes.picture_size.is_empty())
                .count();
            let owners = c
                .meta_items
                .iter()
                .filter(|m| !m.owner_username().is_empty())
                .count();
            let own = c
                .meta_items
                .iter()
                .filter(|m| m.owner_username() == username)
                .count();
            println!(
                "rootlist: length {} items {} meta {} kinds {kinds:?}; con nombre {named}, picture {pic}, picture_size {sizes}, owner {owners} (propias {own}), truncated {}",
                l.length(),
                c.items.len(),
                c.meta_items.len(),
                c.truncated()
            );
            if let Some(m) = c
                .meta_items
                .iter()
                .find(|m| !m.attributes.picture_size.is_empty())
            {
                let t: Vec<_> = m
                    .attributes
                    .picture_size
                    .iter()
                    .map(|p| {
                        (
                            p.target_name().to_string(),
                            p.url().split('/').nth(2).unwrap_or("").to_string(),
                        )
                    })
                    .collect();
                println!("  picture_size ejemplo (target, host): {t:?}");
            }
        }
        Err(e) => println!("rootlist ERROR {:?}", e.kind),
    }
    for set in [collection::LIKED_SET, collection::ARTIST_SET] {
        let started = Instant::now();
        match collection::read_set(session, set).await {
            Ok(rows) => {
                let kinds: std::collections::BTreeMap<String, usize> =
                    rows.iter().fold(Default::default(), |mut m, r| {
                        *m.entry(r.uri.split(':').nth(1).unwrap_or("?").to_string())
                            .or_default() += 1;
                        m
                    });
                let dated = rows.iter().filter(|r| r.added_at.is_some()).count();
                println!(
                    "colección {set}: {} filas en {} ms {kinds:?}, con fecha {dated}",
                    rows.len(),
                    started.elapsed().as_millis()
                );
            }
            Err(e) => println!("colección {set}: ERROR {e}"),
        }
    }
    session.shutdown();
    Ok(())
}

/// El proveedor interno de punta a punta (solo conteos y status, nada personal).
async fn session_api(session: Session) -> Result<()> {
    use velo_domain::{MusicProvider, TimeRange};
    use velo_player::session_api::{SessionApi, SessionSource};
    struct Fixed(Session);
    #[async_trait::async_trait]
    impl SessionSource for Fixed {
        async fn session(&self) -> Option<Session> {
            Some(self.0.clone())
        }
    }
    let api = SessionApi::new(Arc::new(Fixed(session.clone())));
    fn show<T>(what: &str, started: Instant, r: &velo_domain::Result<T>, n: impl Fn(&T) -> String) {
        match r {
            Ok(v) => println!(
                "{what:<28} OK  {:>5} ms  {}",
                started.elapsed().as_millis(),
                n(v)
            ),
            Err(e) => println!(
                "{what:<28} ERR {:>5} ms  {}",
                started.elapsed().as_millis(),
                format!("{e:?}").chars().take(80).collect::<String>()
            ),
        }
    }
    let t = Instant::now();
    let r = api.me().await;
    show("me", t, &r, |p| {
        format!("product={} imgs={}", p.product, p.images.len())
    });
    let t = Instant::now();
    let r = api.my_playlists(0, 50).await;
    show("my_playlists", t, &r, |p| {
        format!(
            "{} de {} (con imagen {})",
            p.items.len(),
            p.total,
            p.items.iter().filter(|x| !x.images.is_empty()).count()
        )
    });
    let first = r.ok().and_then(|p| p.items.into_iter().next());
    if let Some(p) = first {
        let t = Instant::now();
        let r = api.playlist(&p.id).await;
        show("playlist", t, &r, |p| format!("tracks={}", p.track_count));
        let t = Instant::now();
        let r = api.playlist_tracks(&p.id, 0, 20).await;
        show("playlist_tracks", t, &r, |p| {
            format!("{} de {}", p.items.len(), p.total)
        });
    }
    let t = Instant::now();
    let r = api.saved_tracks(0, 50).await;
    show("saved_tracks", t, &r, |p| {
        format!("{} de {}", p.items.len(), p.total)
    });
    let liked = r.ok().and_then(|p| p.items.into_iter().next());
    let t = Instant::now();
    let r = api.saved_albums(0, 20).await;
    show("saved_albums", t, &r, |p| {
        format!("{} de {}", p.items.len(), p.total)
    });
    let t = Instant::now();
    let r = api.followed_artists(20).await;
    show("followed_artists", t, &r, |v| format!("{}", v.len()));
    if let Some(track) = liked {
        let t = Instant::now();
        let r = api.is_saved(&track.uri).await;
        show("is_saved (me gusta)", t, &r, |v| format!("{v}"));
    }
    let t = Instant::now();
    let r = api.search("queen", 0, 10).await;
    show("search", t, &r, |s| {
        format!(
            "t{} al{} ar{} pl{}",
            s.tracks.items.len(),
            s.albums.items.len(),
            s.artists.items.len(),
            s.playlists.items.len()
        )
    });
    let t = Instant::now();
    let r = api.album("6i6folBtxKV28WX3msQ4FE").await;
    show("album", t, &r, |a| {
        format!("total_tracks={}", a.total_tracks)
    });
    let t = Instant::now();
    let r = api.album_tracks("6i6folBtxKV28WX3msQ4FE", 0, 50).await;
    show("album_tracks", t, &r, |p| {
        format!("{} de {}", p.items.len(), p.total)
    });
    let t = Instant::now();
    let r = api.artist("1dfeR4HaWDbWqFHLkxsg1d").await;
    show("artist", t, &r, |a| format!("imgs={}", a.images.len()));
    let t = Instant::now();
    let r = api.artist_albums("1dfeR4HaWDbWqFHLkxsg1d", 0, 10).await;
    show("artist_albums", t, &r, |p| {
        format!("{} de {}", p.items.len(), p.total)
    });
    let t = Instant::now();
    let r = api.top_artists(TimeRange::MediumTerm, 0, 20).await;
    show("top_artists (home)", t, &r, |p| {
        format!("{}", p.items.len())
    });
    {
        use velo_player::session_api::{catalog, home};
        let t = Instant::now();
        let r = home::top_artists(&session, TimeRange::MediumTerm, 0, 20).await;
        show("  interno home::top_artists", t, &r, |p| {
            format!("{}", p.items.len())
        });
        let rows = velo_player::collection::read_set(&session, velo_player::collection::LIKED_SET)
            .await
            .unwrap_or_default();
        let uris: Vec<String> = rows
            .iter()
            .filter(|r| r.uri.starts_with("spotify:album:"))
            .map(|r| r.uri.clone())
            .collect();
        for (i, uri) in uris.iter().enumerate() {
            let r = catalog::album(&session, uri).await;
            if let Err(e) = r {
                println!("  album #{i}: {e:?}");
            }
        }
    }
    let t = Instant::now();
    let r = api.recently_played(20).await;
    show("recently_played (web)", t, &r, |v| format!("{}", v.len()));
    let t = Instant::now();
    let r = api.user_profile("me").await;
    show("user_profile(me)", t, &r, |p| {
        format!("followers={:?}", p.followers.is_some())
    });
    session.shutdown();
    Ok(())
}
