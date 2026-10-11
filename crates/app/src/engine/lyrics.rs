//! `Lyrics` engine: al cambiar de pista consulta primero las letras de Spotify (si la sesión
//! de audio ya está lista) y después LRCLIB con su caché en disco, y sigue la línea activa con
//! los eventos de posición. Sin timer propio: el reloj solo avanza mientras suena y es visible.
//! Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino),
//! src/backend.rs:3995 (`spotify_lyrics`, Spotify primero y LRCLIB de respaldo).

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::AbortHandle;
use velo_domain::{Lyrics, LyricsQuery, LyricsService};

use super::player::PlayerCtl;
use super::rows::{lyric_rows, lyrics_status};
use super::{Bus, Env};
use crate::model::ModelOp;

/// Waits for the duration to settle after a track change before asking the network.
const SETTLE: Duration = Duration::from_millis(200);

/// `Lyrics.source`: de dónde vienen las letras publicadas.
pub const SOURCE_SPOTIFY: &str = "spotify";
pub const SOURCE_LRCLIB: &str = "lrclib";

#[derive(Debug, Clone, PartialEq)]
pub struct LyricsView {
    /// "loading" | "synced" | "plain" | "none" | "instrumental"
    pub status: String,
    pub plain_text: String,
    pub active_index: i32,
    /// "spotify" | "lrclib" | "" (sin letras o cargando).
    pub source: String,
}

impl LyricsView {
    fn empty(status: &str) -> Self {
        Self {
            status: status.into(),
            plain_text: String::new(),
            active_index: -1,
            source: String::new(),
        }
    }
}

pub struct LyricsCtl {
    pub view: watch::Sender<LyricsView>,
    /// Changes for `Lyrics.model`.
    pub ui: Bus<ModelOp>,
    current: Mutex<CurrentLyrics>,
}

#[derive(Default)]
struct CurrentLyrics {
    generation: u64,
    lyrics: Option<Arc<Lyrics>>,
    source: &'static str,
}

/// Index of the line being sung at `position_ms`; -1 before the first line or without synced lines.
pub fn active_index(lyrics: &Lyrics, position_ms: u64) -> i32 {
    lyrics.line_at(position_ms).map_or(-1, |i| i as i32)
}

fn query_of(view: &super::player::PlayerView) -> Option<LyricsQuery> {
    (!view.dj_speaking && !view.title.trim().is_empty()).then(|| LyricsQuery {
        title: view.title.clone(),
        artist: view.artist.clone(),
        album: view.album.clone(),
        duration_s: ((view.duration_ms + 500) / 1000) as u32,
    })
}

/// Spotify primero; ausencia, error, timeout o formato desconocido → LRCLIB. La segunda
/// consulta solo empieza si la primera no sirvió.
async fn resolve<S, L>(
    spotify: Option<S>,
    lrclib: L,
    upgrade: bool,
) -> Option<(Lyrics, &'static str)>
where
    S: Future<Output = Option<Lyrics>>,
    L: Future<Output = Option<Lyrics>>,
{
    if let Some(spotify) = spotify
        && let Some(found) = spotify.await
    {
        return Some((found, SOURCE_SPOTIFY));
    }
    if upgrade {
        None
    } else {
        lrclib.await.map(|found| (found, SOURCE_LRCLIB))
    }
}

impl LyricsCtl {
    pub fn start(env: &Arc<Env>, player: &Arc<PlayerCtl>) -> Arc<Self> {
        let this = Arc::new(Self {
            view: watch::channel(LyricsView::empty("none")).0,
            ui: Bus::new(),
            current: Mutex::new(CurrentLyrics::default()),
        });

        // Track, the "show lyrics" setting or the audio session changed → fetch.
        let me = this.clone();
        let env2 = env.clone();
        let mut track = player.view.subscribe();
        let mut settings = env.prefs.watch_settings();
        let mut audio = env.audio.view.subscribe();
        env.rt.spawn(async move {
            // La URI distingue grabaciones con iguales metadatos. No se activa audio por
            // consultar letras: si la sesión queda lista después, se repite la consulta de la
            // misma pista sin vaciar lo que ya se muestra (sin parpadeo a «loading»).
            let mut last: Option<(LyricsQuery, String)> = None;
            let mut asked_spotify = false;
            let mut fetching: Option<AbortHandle> = None;
            loop {
                let show = settings.borrow_and_update().playback.show_lyrics;
                let ready = audio.borrow_and_update().ready;
                if !ready {
                    // Una caída de la sesión durante la consulta no debe impedir el reintento.
                    asked_spotify = false;
                }
                let wanted = if show {
                    let view = track.borrow_and_update();
                    query_of(&view).map(|query| (query, view.uri.clone()))
                } else {
                    None
                };
                let upgrade = wanted.is_some() && wanted == last && ready && !asked_spotify;
                if wanted != last || upgrade {
                    if let Some(old) = fetching.take() {
                        old.abort();
                    }
                    match &wanted {
                        Some((q, uri)) => {
                            let ticket = if upgrade {
                                me.ticket()
                            } else {
                                me.begin_loading()
                            };
                            asked_spotify = ready;
                            let (me2, env3) = (me.clone(), env2.clone());
                            let (q, uri) = (q.clone(), uri.clone());
                            fetching = Some(
                                env2.rt
                                    .spawn(async move {
                                        me2.fetch(&env3, q, uri, ticket, upgrade).await
                                    })
                                    .abort_handle(),
                            );
                        }
                        None => me.clear(),
                    }
                    last = wanted;
                }
                tokio::select! {
                    r = track.changed() => if r.is_err() { break },
                    r = settings.changed() => if r.is_err() { break },
                    r = audio.changed() => if r.is_err() { break },
                }
            }
        });

        // Position → active line.
        let me = this.clone();
        let mut position = env.position.subscribe();
        env.rt.spawn(async move {
            loop {
                let pos = *position.borrow_and_update();
                me.update_active(pos);
                if position.changed().await.is_err() {
                    break;
                }
            }
        });
        this
    }

    fn ticket(&self) -> u64 {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation
    }

    fn begin_loading(&self) -> u64 {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        current.generation = current.generation.wrapping_add(1);
        current.lyrics = None;
        current.source = "";
        self.ui.send(ModelOp::Replace(Vec::new()));
        self.view.send_replace(LyricsView::empty("loading"));
        current.generation
    }

    fn clear(&self) {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        current.generation = current.generation.wrapping_add(1);
        current.lyrics = None;
        current.source = "";
        self.ui.send(ModelOp::Replace(Vec::new()));
        self.view.send_replace(LyricsView::empty("none"));
    }

    async fn fetch(&self, env: &Env, query: LyricsQuery, uri: String, ticket: u64, upgrade: bool) {
        tokio::time::sleep(SETTLE).await;
        let spotify =
            env.audio
                .engine()
                .filter(|_| env.audio.is_ready())
                .map(|engine| async move {
                    match velo_player::lyrics::lyrics(&engine, &uri).await {
                        Ok(found) => found,
                        Err(e) => {
                            tracing::debug!(error = %e, "spotify lyrics unavailable");
                            None
                        }
                    }
                });
        let lrclib = async {
            match env.lyrics_client.lyrics(&query).await {
                Ok(found) => found,
                Err(e) => {
                    tracing::debug!(error = %e, "lyrics lookup failed");
                    None
                }
            }
        };
        let found = resolve(spotify, lrclib, upgrade).await;
        self.publish(ticket, found, *env.position.borrow());
    }

    fn publish(&self, ticket: u64, found: Option<(Lyrics, &'static str)>, position_ms: u64) {
        // Publicación y generación bajo el mismo lock: abortar no basta si la respuesta
        // anterior terminó justo antes de cambiar la pista o apagar «Mostrar letras».
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if current.generation != ticket {
            return;
        }
        let Some((lyrics, source)) = found else {
            // Una segunda consulta de la misma pista (sesión de audio recién lista) que no
            // encuentra nada no borra lo que ya se muestra.
            if current.lyrics.is_none() {
                self.ui.send(ModelOp::Replace(Vec::new()));
                self.view.send_replace(LyricsView::empty("none"));
            }
            return;
        };
        // La misma respuesta otra vez: no reiniciar el modelo (la lista perdería el scroll).
        if current.source == source && current.lyrics.as_deref() == Some(&lyrics) {
            return;
        }
        let status = lyrics_status(&lyrics);
        let rows = if status == "synced" {
            lyric_rows(&lyrics.synced)
        } else {
            Vec::new()
        };
        let plain = if status == "plain" {
            lyrics.plain.clone().unwrap_or_default()
        } else {
            String::new()
        };
        let active = if status == "synced" {
            active_index(&lyrics, position_ms)
        } else {
            -1
        };
        current.lyrics = Some(Arc::new(lyrics));
        current.source = source;
        self.ui.send(ModelOp::Replace(rows));
        self.view.send_replace(LyricsView {
            status: status.into(),
            plain_text: plain,
            active_index: active,
            source: source.into(),
        });
    }

    fn update_active(&self, position_ms: u64) {
        let current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let Some(lyrics) = &current.lyrics else {
            return;
        };
        let index = active_index(lyrics, position_ms);
        self.view.send_if_modified(|v| {
            if v.status == "synced" && v.active_index != index {
                v.active_index = index;
                true
            } else {
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::r4b_test_support::{ManualTime, Server, runtime};
    use super::*;
    use velo_domain::LyricLine;

    #[test]
    fn r4b_reconnect_failure_preserves_spotify_and_initial_lookup_still_uses_lrclib() {
        let rt = runtime();
        let (dir, env) = super::super::test_env(&rt);
        rt.block_on(async {
            let _time = ManualTime::new();
            let mut server = Server::new().await;
            let client = Arc::new(velo_services::LrcLibClient::with_base_url(dir.path().join("mock-lyrics"), &server.uri).unwrap());
            let query = LyricsQuery {title:"Pista".into(),artist:"Artista".into(),album:"Álbum".into(),duration_s:100};
            let ctl = Arc::new(controller());
            let mut ops = ctl.ui.take().unwrap();
            let ticket = ctl.begin_loading();
            ctl.publish(ticket, Some((synced(), SOURCE_SPOTIFY)), 1000);
            while ops.try_recv().is_ok() {}
            let visible = ctl.view.borrow().clone();
            env.audio.view.send_modify(|v| v.ready = false);
            env.audio.view.send_modify(|v| v.ready = true);
            // Misma pista/generación tras reconectar, Spotify falla al vencer su plazo.
            let reconnect = {
                let (ctl, client, query) = (ctl.clone(), client.clone(), query.clone());
                tokio::spawn(async move {
                    let found = resolve(Some(async {
                        tokio::time::timeout(Duration::from_secs(5), std::future::pending::<Option<Lyrics>>()).await.unwrap_or(None)
                    }), async { client.lyrics(&query).await.unwrap() }, true).await;
                    ctl.publish(ticket, found, 1000);
                })
            };
            super::super::r4b_test_support::settle().await;
            tokio::time::advance(Duration::from_secs(5)).await;
            // Una implementación regresiva sí consulta el mock y obtiene texto distinto.
            tokio::select! {
                result = reconnect => result.unwrap(),
                request = server.next() => {
                    request.respond(200, serde_json::json!({"plainLyrics":"LRCLIB distinto","duration":100}));
                    let mut view = ctl.view.subscribe();
                    view.wait_for(|v| v.source == SOURCE_LRCLIB).await.unwrap();
                }
            }
            assert_eq!(*ctl.view.borrow(), visible);
            assert!(ops.try_recv().is_err());
            assert!(server.requests.try_recv().is_err());

            // Primera consulta: la ausencia de Spotify sí usa el respaldo HTTP.
            let initial = {
                let (client, query) = (client.clone(), query.clone());
                tokio::spawn(async move { resolve(Some(async {None}), async {client.lyrics(&query).await.unwrap()}, false).await })
            };
            let request = server.next().await;
            assert!(request.path.starts_with("/api/get?"));
            request.respond(200, serde_json::json!({"plainLyrics":"LRCLIB distinto","duration":100}));
            let new_ticket = ctl.begin_loading();
            ctl.publish(new_ticket, initial.await.unwrap(), 0);
            assert_eq!(ctl.view.borrow().plain_text, "LRCLIB distinto");
            assert_eq!(ctl.view.borrow().source, SOURCE_LRCLIB);
            ctl.publish(ticket, Some((synced(), SOURCE_SPOTIFY)), 1000);
            assert_eq!(ctl.view.borrow().source, SOURCE_LRCLIB);
        });
    }

    fn synced() -> Lyrics {
        Lyrics {
            synced: vec![
                LyricLine {
                    time_ms: 1_000,
                    text: "a".into(),
                },
                LyricLine {
                    time_ms: 3_000,
                    text: "b".into(),
                },
            ],
            ..Lyrics::default()
        }
    }

    fn plain(text: &str) -> Lyrics {
        Lyrics {
            plain: Some(text.into()),
            ..Lyrics::default()
        }
    }

    #[test]
    fn active_index_is_minus_one_before_first_line() {
        let l = synced();
        assert_eq!(active_index(&l, 0), -1);
        assert_eq!(active_index(&l, 999), -1);
        assert_eq!(active_index(&l, 1_000), 0);
        assert_eq!(active_index(&l, 2_999), 0);
        assert_eq!(active_index(&l, 3_000), 1);
        assert_eq!(active_index(&Lyrics::default(), 5_000), -1);
    }

    #[test]
    fn query_needs_a_title_and_rounds_duration() {
        let mut v = super::super::player::PlayerView::default();
        assert!(query_of(&v).is_none());
        v.title = "T".into();
        v.artist = "A".into();
        v.duration_ms = 261_400;
        assert_eq!(query_of(&v).unwrap().duration_s, 261);
        v.duration_ms = 261_600;
        assert_eq!(query_of(&v).unwrap().duration_s, 262);
        v.dj_speaking = true;
        assert!(
            query_of(&v).is_none(),
            "la narración usa djLine, no una búsqueda de letras"
        );
    }

    #[test]
    fn active_index_handles_duplicate_timestamps_and_backward_seeks() {
        let mut l = synced();
        l.synced.push(LyricLine {
            time_ms: 3_000,
            text: "c".into(),
        });
        assert_eq!(active_index(&l, 30_000), 2);
        assert_eq!(active_index(&l, 3_000), 2);
        assert_eq!(active_index(&l, 1_500), 0);
        assert_eq!(active_index(&l, 500), -1);
    }

    fn controller() -> LyricsCtl {
        LyricsCtl {
            view: watch::channel(LyricsView::empty("none")).0,
            ui: Bus::new(),
            current: Mutex::new(CurrentLyrics::default()),
        }
    }

    #[tokio::test]
    async fn spotify_wins_and_lrclib_is_only_asked_when_spotify_has_nothing() {
        let lrclib_unused = async { panic!("LRCLIB no debe consultarse") };
        let found = resolve(Some(async { Some(synced()) }), lrclib_unused, false).await;
        assert_eq!(found, Some((synced(), SOURCE_SPOTIFY)));
        // Sin sesión de audio, o Spotify sin letras / con error: respaldo LRCLIB.
        let no_audio: Option<std::future::Ready<Option<Lyrics>>> = None;
        let found = resolve(no_audio, async { Some(plain("LRCLIB")) }, false).await;
        assert_eq!(found, Some((plain("LRCLIB"), SOURCE_LRCLIB)));
        let found = resolve(Some(async { None }), async { Some(plain("LRCLIB")) }, false).await;
        assert_eq!(found.map(|(_, s)| s), Some(SOURCE_LRCLIB));
        assert_eq!(
            resolve(Some(async { None }), async { None }, false).await,
            None
        );
    }

    #[test]
    fn late_answers_cannot_replace_a_new_track_or_reenable_hidden_lyrics() {
        let ctl = controller();
        let first = ctl.begin_loading();
        let second = ctl.begin_loading();
        ctl.publish(first, Some((synced(), SOURCE_SPOTIFY)), 3000);
        assert_eq!(ctl.view.borrow().status, "loading");
        ctl.publish(second, Some((synced(), SOURCE_LRCLIB)), 1000);
        assert_eq!(ctl.view.borrow().active_index, 0);
        assert_eq!(ctl.view.borrow().source, SOURCE_LRCLIB);
        // Un fallo obsoleto tampoco puede borrar el resultado nuevo.
        ctl.publish(first, None, 0);
        assert_eq!(ctl.view.borrow().status, "synced");
        ctl.clear();
        ctl.publish(second, Some((synced(), SOURCE_SPOTIFY)), 3000);
        ctl.update_active(3000);
        assert_eq!(ctl.view.borrow().status, "none");
        assert_eq!(ctl.view.borrow().active_index, -1);
        assert_eq!(ctl.view.borrow().source, "");
    }

    #[test]
    fn second_query_of_the_same_track_upgrades_without_blanking() {
        let ctl = controller();
        let mut ops = ctl.ui.take().unwrap();
        let ticket = ctl.begin_loading();
        ctl.publish(ticket, Some((synced(), SOURCE_LRCLIB)), 1000);
        // La sesión de audio quedó lista: misma generación, sin pasar por «loading».
        assert_eq!(ctl.ticket(), ticket);
        // Spotify sin letras y LRCLIB igual que antes: ni se borra ni se reinicia el modelo.
        ctl.publish(ticket, None, 1000);
        ctl.publish(ticket, Some((synced(), SOURCE_LRCLIB)), 1000);
        assert_eq!(ctl.view.borrow().status, "synced");
        assert_eq!(ctl.view.borrow().source, SOURCE_LRCLIB);
        let mut replaces = 0;
        while ops.try_recv().is_ok() {
            replaces += 1;
        }
        assert_eq!(replaces, 2, "loading + primera publicación, nada más");
        // Spotify sí tiene letras: reemplazan a las de LRCLIB.
        ctl.publish(ticket, Some((plain("Spotify"), SOURCE_SPOTIFY)), 1000);
        assert_eq!(ctl.view.borrow().status, "plain");
        assert_eq!(ctl.view.borrow().plain_text, "Spotify");
        assert_eq!(ctl.view.borrow().source, SOURCE_SPOTIFY);
    }

    #[test]
    fn fallback_plain_and_instrumental_keep_existing_contract() {
        let ctl = controller();
        let ticket = ctl.begin_loading();
        ctl.publish(ticket, Some((plain("Texto LRCLIB"), SOURCE_LRCLIB)), 1000);
        assert_eq!(ctl.view.borrow().status, "plain");
        assert_eq!(ctl.view.borrow().plain_text, "Texto LRCLIB");
        let ticket = ctl.begin_loading();
        assert_eq!(ctl.view.borrow().source, "");
        ctl.publish(
            ticket,
            Some((
                Lyrics {
                    instrumental: true,
                    ..Lyrics::default()
                },
                SOURCE_LRCLIB,
            )),
            1000,
        );
        assert_eq!(ctl.view.borrow().status, "instrumental");
        let ticket = ctl.begin_loading();
        ctl.publish(ticket, None, 0);
        assert_eq!(ctl.view.borrow().status, "none");
    }

    // T4: with another device playing, the position channel carries the remote's interpolated
    // clock; the active line follows it, a remote seek backwards moves it back, and a new remote
    // track starts from the position known when its lyrics arrive (not from the previous song's).
    #[test]
    fn remote_position_drives_the_active_line_across_seeks_and_track_changes() {
        let ctl = controller();
        let ticket = ctl.begin_loading();
        ctl.publish(ticket, Some((synced(), SOURCE_SPOTIFY)), 3_500);
        assert_eq!(ctl.view.borrow().active_index, 1);
        ctl.update_active(0);
        assert_eq!(ctl.view.borrow().active_index, -1);
        ctl.update_active(1_200);
        assert_eq!(ctl.view.borrow().active_index, 0);
        // The remote jumps forward, then back (seek on the other device).
        ctl.update_active(3_000);
        assert_eq!(ctl.view.borrow().active_index, 1);
        ctl.update_active(1_000);
        assert_eq!(ctl.view.borrow().active_index, 0);
        // The remote moves to another song: lyrics reload (loading → none of the old index) and
        // the new ones start at the remote's current position.
        let ticket = ctl.begin_loading();
        assert_eq!(ctl.view.borrow().active_index, -1);
        ctl.update_active(9_000); // while loading there are no lyrics to follow
        assert_eq!(ctl.view.borrow().active_index, -1);
        ctl.publish(ticket, Some((synced(), SOURCE_LRCLIB)), 3_200);
        assert_eq!(ctl.view.borrow().active_index, 1);
        assert_eq!(ctl.view.borrow().source, SOURCE_LRCLIB);
    }
}
