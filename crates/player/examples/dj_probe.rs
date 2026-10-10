//! Prueba manual E5b; usa el login de audio del keyring. Nunca guarda audio ni imprime tokens.
use anyhow::{Result, bail};
use std::{sync::Arc, time::Duration};
use velo_player::{
    Engine, EngineConfig, EngineState,
    dj::{fetch_block, fetch_voice},
    extras::{DJ_LIVI_URI, start_dj},
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("");
    if !matches!(
        mode,
        "resolve" | "lexicon" | "voice" | "play" | "start" | "change-wave"
    ) {
        bail!(
            "Uso: dj_probe -- resolve | lexicon | voice | play <segundos> | start <ID dispositivo> | change-wave"
        );
    }
    if mode == "change-wave" {
        bail!(
            "Usa play <segundos> para DJ local; Actions.djNextSegment() pide el siguiente bloque de la sesión."
        );
    }
    let seconds = if mode == "play" {
        if args.len() != 2 {
            bail!("play necesita una duración en segundos, por ejemplo: play 120");
        }
        let seconds: u64 = args[1].parse()?;
        if !(1..=86_400).contains(&seconds) {
            bail!("La duración debe estar entre 1 y 86400 segundos");
        }
        seconds
    } else {
        if (mode == "start" && args.len() != 2) || (mode != "start" && args.len() != 1) {
            bail!(
                "start requiere un ID; resolve, lexicon y voice no reciben archivos (todo queda en memoria)"
            );
        }
        0
    };
    let engine = Arc::new(Engine::from_keyring(EngineConfig::velo("Velo")).await?);
    let result = probe(&engine, mode, &args, seconds).await;
    engine.shutdown().await;
    result
}

async fn probe(engine: &Arc<Engine>, mode: &str, args: &[String], seconds: u64) -> Result<()> {
    match mode {
        "play" => play(engine, seconds).await,
        "start" => {
            start_dj(engine.session(), &args[1]).await?;
            println!("Comando Connect aceptado. Escucha el dispositivo seleccionado durante 30 s.");
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        }
        "resolve" => {
            let context = tokio::time::timeout(
                Duration::from_secs(15),
                engine.session().spclient().get_context(DJ_LIVI_URI),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Spotify tardó en resolver el contexto DJ"))?
            .map_err(|_| anyhow::anyhow!("Spotify no entregó el contexto DJ"))?;
            let tracks: usize = context.pages.iter().map(|p| p.tracks.len()).sum();
            println!(
                "Contexto DJ: {} páginas, {tracks} pistas; sesión lexicon: {}.",
                context.pages.len(),
                context.metadata.contains_key("lexicon_context_url")
            );
            Ok(())
        }
        "lexicon" => {
            let block = fetch_block(engine.session(), None).await?;
            println!(
                "Sesión DJ: {} canciones, {} intros, {} cierres; bloque siguiente: {}.",
                block.tracks.len(),
                block.tracks.iter().filter(|t| t.intro.is_some()).count(),
                block.tracks.iter().filter(|t| t.outro.is_some()).count(),
                block.next_page_url.is_some()
            );
            Ok(())
        }
        "voice" => {
            let block = fetch_block(engine.session(), None).await?;
            let narration = block
                .tracks
                .iter()
                .find_map(|t| t.intro.as_ref().or(t.outro.as_ref()))
                .ok_or_else(|| anyhow::anyhow!("La sesión DJ no trae narraciones"))?;
            let clip = fetch_voice(engine.session(), narration).await?;
            println!(
                "Voz de Livi decodificada en memoria: {} ms, {} canales; no se guardó audio.",
                clip.duration_ms(),
                clip.channels()
            );
            Ok(())
        }
        _ => unreachable!(),
    }
}

fn transition(state: &EngineState) -> (bool, bool, bool, String) {
    if let Some(speech) = &state.dj.speech {
        (state.dj.active, true, speech.playing, speech.title.clone())
    } else {
        (
            state.dj.active,
            false,
            state.playing,
            state
                .track
                .as_ref()
                .map(|t| t.uri.clone())
                .unwrap_or_default(),
        )
    }
}

async fn play(engine: &Arc<Engine>, seconds: u64) -> Result<()> {
    let mut state = engine.state();
    engine.start_dj().await?;
    println!(
        "DJ local iniciado; escucha durante {seconds} s. Al vencer el plazo se cierra el motor."
    );
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut previous = None;
    let mut errors = 0;
    loop {
        {
            let current = state.borrow_and_update();
            let key = transition(&current);
            if previous.as_ref() != Some(&key) {
                if let Some(speech) = &current.dj.speech {
                    println!(
                        "VOZ · DJ Livi · {:?} · {}",
                        speech.title,
                        if speech.playing { "hablando" } else { "pausa" }
                    );
                } else if current.dj.active {
                    if let Some(track) = &current.track {
                        println!(
                            "CANCIÓN · {:?} · {:?} · {}",
                            track.artist,
                            track.title,
                            if current.playing {
                                "sonando"
                            } else {
                                "pausa/cargando"
                            }
                        );
                    } else {
                        println!("DJ · preparando el bloque");
                    }
                } else {
                    println!("DJ · terminó");
                }
                previous = Some(key);
            }
            if current.dj_errors != errors {
                if let Some(error) = current.dj_error {
                    println!("DJ · {error}");
                }
                errors = current.dj_errors;
            }
        }
        tokio::select! {
            _ = &mut deadline => break,
            changed = state.changed() => if changed.is_err() { break },
        }
    }
    engine.stop_dj();
    Ok(())
}
