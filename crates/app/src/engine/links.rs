//! Enlaces externos, sin resolución de URLs ni red al analizarlos.
//! Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino).

use super::{Bus, core};
use crate::bridge::source::{PlayRoute, activate_device, retry_device_action};
use std::sync::OnceLock;
use velo_domain::{Result, VeloError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: String,
    pub id: String,
}

impl Link {
    fn uri(&self) -> String {
        format!("spotify:{}:{}", self.kind, self.id)
    }
}

pub fn parse(text: &str) -> Option<Link> {
    let text = text.trim();
    if text.chars().any(char::is_control) {
        return None;
    }
    let (scheme, rest) = text.split_once(':')?;
    let parts: Vec<String> = if scheme.eq_ignore_ascii_case("spotify") && !rest.starts_with("//") {
        rest.split(':').map(str::to_owned).collect()
    } else {
        let url = url::Url::parse(text).ok()?;
        if !matches!(url.scheme(), "https" | "http")
            || !url.host_str()?.eq_ignore_ascii_case("open.spotify.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return None;
        }
        // No aceptar normalización de traversal ni segmentos codificados.
        let raw_path = rest
            .split_once("//")?
            .1
            .split_once('/')?
            .1
            .split(['?', '#'])
            .next()?;
        if raw_path
            .split('/')
            .any(|p| p == "." || p == ".." || p.contains('%'))
        {
            return None;
        }
        url.path()
            .trim_end_matches('/')
            .trim_start_matches('/')
            .split('/')
            .map(str::to_owned)
            .collect()
    };
    let mut parts = parts.as_slice();
    if parts
        .first()
        .is_some_and(|p| p.to_ascii_lowercase().starts_with("intl-"))
    {
        let locale = &parts[0][5..];
        if locale.is_empty() || !locale.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-') {
            return None;
        }
        parts = &parts[1..];
    }
    if parts.len() == 4
        && parts[0].eq_ignore_ascii_case("user")
        && !parts[1].is_empty()
        && parts[2].eq_ignore_ascii_case("playlist")
    {
        parts = &parts[2..];
    }
    let [kind, id] = parts else { return None };
    let kind = kind.to_ascii_lowercase();
    if !matches!(kind.as_str(), "track" | "album" | "playlist" | "artist")
        || id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(Link {
        kind,
        id: id.clone(),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Play(String),
    Navigate(Link),
    Queue(Vec<String>),
}

fn plan(text: &str) -> Option<Plan> {
    if text.len() > 16 * 1024 {
        return None;
    }
    let links: Option<Vec<Link>> = text.split_whitespace().map(parse).collect();
    let mut links = links?;
    match links.len() {
        0 => None,
        1 => {
            let link = links.pop()?;
            Some(if link.kind == "track" {
                Plan::Play(link.uri())
            } else {
                Plan::Navigate(link)
            })
        }
        2..=100 if links.iter().all(|l| l.kind == "track") => {
            Some(Plan::Queue(links.iter().map(Link::uri).collect()))
        }
        _ => None,
    }
}

pub fn navigation() -> &'static Bus<Link> {
    static BUS: OnceLock<Bus<Link>> = OnceLock::new();
    BUS.get_or_init(Bus::new)
}

pub fn open(text: String) {
    let c = core();
    c.env.rt.spawn(async move {
        // Pasted text that is not a Spotify link is ignored, like the official app (no error).
        let Some(plan) = plan(&text) else {
            return;
        };
        let result = execute(plan).await;
        if let Err(e) = result {
            core().env.report(&e);
        }
    });
}

pub fn launch(args: Vec<String>) {
    if args.is_empty() {
        return;
    }
    // La entrega puede llegar antes de completar el login: esperar eventos, sin timer.
    core().env.rt.spawn(async move {
        let mut account = core().env.account.view.subscribe();
        while !account.borrow_and_update().signed_in {
            if account.changed().await.is_err() {
                return;
            }
        }
        open(args.join("\n"));
    });
}

async fn execute(plan: Plan) -> Result<()> {
    let c = core();
    if !c.env.account.snapshot().signed_in {
        return Err(VeloError::NotAuthenticated);
    }
    match plan {
        Plan::Navigate(link) => {
            navigation().send(link);
            Ok(())
        }
        Plan::Play(uri) => {
            let generation = c.player.begin_playback();
            let Some(route) = c.player.prepare_playback().await? else {
                return Ok(());
            };
            if !c.player.playback_current(generation) {
                return Ok(());
            }
            match route {
                PlayRoute::Velo => {
                    let engine = c.env.audio.engine().ok_or(VeloError::NoActiveDevice)?;
                    engine
                        .load(velo_player::LoadTarget::Tracks {
                            uris: vec![uri],
                            index: 0,
                            shuffle: false,
                        })
                        .map_err(|e| VeloError::Other(super::audio::audio_message(&e)))?;
                }
                PlayRoute::WebApi => {
                    let api = c.env.account.api().ok_or(VeloError::NotAuthenticated)?;
                    let call = || {
                        let api = api.clone();
                        let uri = uri.clone();
                        async move { api.play_uris(&[uri], None).await }
                    };
                    match call().await {
                        Err(VeloError::NoActiveDevice) => {
                            activate_device(&c.env, &*api, true).await?;
                            retry_device_action(call).await?;
                        }
                        other => other?,
                    }
                }
            }
            c.player.resync();
            Ok(())
        }
        Plan::Queue(uris) => {
            let api = c.env.account.api().ok_or(VeloError::NotAuthenticated)?;
            let result = append_ordered(&uris, |uri| {
                let api = api.clone();
                async move {
                    let call = || {
                        let api = api.clone();
                        let uri = uri.clone();
                        async move { api.add_to_queue(&uri).await }
                    };
                    match call().await {
                        // Un 404 de dispositivo es rechazo explícito, no entrega ambigua.
                        Err(VeloError::NoActiveDevice) => {
                            activate_device(&core().env, &*api, false).await?;
                            retry_device_action(call).await
                        }
                        other => other,
                    }
                }
            })
            .await;
            // Incluso tras un fallo se muestran las canciones que ya se aceptaron.
            c.player.refresh_queue();
            result
        }
    }
}

async fn append_ordered<F, Fut>(uris: &[String], mut append: F) -> Result<()>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    for uri in uris {
        append(uri.clone()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    #[test]
    fn accepts_shared_shapes_and_preserves_base62_case() {
        for link in [
            format!("SPOTIFY:Track:{ID}"),
            format!("https://OPEN.SPOTIFY.COM/intl-ES/track/{ID}?si=abc#x"),
            format!("spotify:track:{ID}"),
        ] {
            assert_eq!(parse(&link).unwrap().uri(), format!("spotify:track:{ID}"));
        }
        for link in [
            format!("https://open.spotify.com/user/juan/PLAYLIST/{ID}/?si=x"),
            format!("spotify:USER:juan:Playlist:{ID}"),
        ] {
            assert_eq!(parse(&link).unwrap().kind, "playlist");
        }
        for kind in ["album", "artist", "playlist"] {
            assert!(matches!(
                plan(&format!("spotify:{kind}:{ID}")),
                Some(Plan::Navigate(_))
            ));
        }
    }

    #[test]
    fn rejects_spoofed_domains_and_ambiguous_paths() {
        for link in [
            "https://open.spotify.com.evil.test/track/ABC",
            "https://evil.test@open.spotify.com/track/ABC",
            "https://open.spotify.com@evil.test/track/ABC",
            "https://open.spotify.com:444/track/ABC",
            "https://open.spotify.com/track/ABC/extra",
            "https://open.spotify.com/track/%41BC",
            "https://open.spotify.com/track/../track/ABC",
            "spotify:track:A_B",
            "spotify:track:ABC:extra",
            "spotify:local:a:b:c",
            "spotify:episode:ABC",
            "file:///track/ABC",
            "https://open.spotify.com/intl-/track/ABC",
        ] {
            assert!(parse(link).is_none(), "{link}");
        }
    }

    #[test]
    fn batch_preserves_order_duplicates_and_rejects_partial_input() {
        assert_eq!(
            plan("spotify:track:A\nspotify:track:B spotify:track:A"),
            Some(Plan::Queue(vec![
                "spotify:track:A".into(),
                "spotify:track:B".into(),
                "spotify:track:A".into()
            ]))
        );
        assert!(plan("spotify:track:A spotify:album:B").is_none());
        assert!(plan("spotify:track:A inválido").is_none());
        assert!(plan("").is_none());
    }

    #[tokio::test]
    async fn queue_awaits_each_append_and_stops_at_first_failure() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let uris = vec!["A".into(), "B".into(), "C".into()];
        let result = append_ordered(&uris, |uri| {
            let calls = calls.clone();
            async move {
                tokio::task::yield_now().await;
                calls.lock().unwrap().push(uri.clone());
                if uri == "B" {
                    Err(VeloError::NoActiveDevice)
                } else {
                    Ok(())
                }
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(*calls.lock().unwrap(), vec!["A", "B"]);
    }
}
