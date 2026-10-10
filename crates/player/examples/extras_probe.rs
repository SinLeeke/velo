//! E6: consultas sin mutación. No imprime cuerpos, tokens ni enlaces de invitación.
use anyhow::{Result, bail};
use futures_util::StreamExt;
use librespot_core::dealer::protocol::{FallbackWrapper, Message};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::social_connect_v2::SessionUpdate;
use std::time::Duration;
use velo_player::{
    Engine, EngineConfig,
    extras::{credits, extension_size, jam_summary},
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let Some(mode) = args.first().map(String::as_str) else {
        bail!("Uso: extras_probe -- credits|mix <spotify:track:ID> | jam | exclude");
    };
    match mode {
        "jam" if args.len() == 1 => return jam_probe().await,
        "exclude" => bail!(
            "Usa webapi_token_probe -- taste-exclude <playlist_uri> para investigar atributos y schema por la sesión en modo solo lectura. Este modo histórico no consulta ni muta la cuenta."
        ),
        "credits" | "mix" if args.len() == 2 => {}
        _ => bail!("Uso: extras_probe -- credits|mix <spotify:track:ID> | jam | exclude"),
    }
    velo_player::parse_track_uri(&args[1])?;
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let result = if mode == "credits" {
        let partial = credits(engine.session(), &args[1]).await;
        if let Ok(value) = &partial {
            println!("Créditos (partial={}): {}", value["partial"], value);
        }
        for kind in [
            ExtensionKind::TRACK_EXTENDED_CREDITS,
            ExtensionKind::CREDITS_TRAIT,
            ExtensionKind::CREDITS_V2_TRAIT,
        ] {
            println!(
                "{kind:?}: {:?}",
                extension_size(engine.session(), &args[1], kind).await
            );
        }
        partial.map(|_| ()).map_err(anyhow::Error::from)
    } else {
        let result = extension_size(engine.session(), &args[1], ExtensionKind::AUTOMIX_MODE).await;
        println!("AUTOMIX_MODE: {result:?}");
        println!(
            "CUEPOINTS: {:?}",
            extension_size(engine.session(), &args[1], ExtensionKind::CUEPOINTS).await
        );
        println!(
            "Datos Automix no equivalen al botón Mixear; faltan transiciones y soporte de render en el motor."
        );
        result.map(|_| ()).map_err(anyhow::Error::from)
    };
    engine.shutdown().await;
    result
}

async fn jam_probe() -> Result<()> {
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let result = async {
        let mut updates = engine.session().dealer().listen_for(
            "social-connect/v2/session_update", Message::try_from_json::<SessionUpdate>)
            .map_err(|_| anyhow::anyhow!("No se pudo suscribir a los eventos Jam"))?;
        println!("Durante 60 s inicia/administra/termina Jam en la app oficial. Velo solo observa; no crea ni une sesiones.");
        let deadline = tokio::time::sleep(Duration::from_secs(60));
        tokio::pin!(deadline);
        let mut events = 0;
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                update = updates.next() => match update {
                    Some(Ok(FallbackWrapper::Inner(update))) => {
                        println!("Jam: {}", jam_summary(&update));
                        events += 1;
                    },
                    Some(Ok(FallbackWrapper::Fallback(_))) => println!("Evento con schema desconocido; cuerpo omitido"),
                    Some(Err(_)) => println!("Evento no decodificable; error upstream omitido"),
                    None => break,
                }
            }
        }
        if events == 0 { bail!("No se observaron eventos Jam; esto no demuestra que Spotify rechace Jam."); }
        Ok(())
    }.await;
    engine.shutdown().await;
    result
}
