# velo-player — motor de audio propio de Velo (E2)

Velo reproduce el audio él mismo con librespot 0.8 (streaming con la cuenta Premium del
usuario, sin guardar canciones) y se publica como el dispositivo Spotify Connect **"Velo"**.
No depende de Qt. Todo corre en tokio; el estado llega por eventos de librespot (sin polling).

- **Login de audio** (`login`): Client ID de librespot (`65b708073fc0480ea92a077233ca87bd`),
  redirect `http://127.0.0.1:8898/login`, scope `streaming`. Se hace una vez en el navegador;
  solo el refresh token queda en el Administrador de credenciales (`velo` /
  `spotify-librespot-refresh-token`). La Web API sigue con el Client ID de Velo.
- **Motor** (`Engine`): Session + Player (gapless, normalización, 320 kbps) + Spirc (dispositivo
  tipo Computer). Comandos por Spirc; estado en un `watch`. Enganche PCM apagado por defecto (E3).
- **En la app** (`crates/app`): `engine/audio.rs` arranca el motor al abrir si hay login guardado,
  reconecta tras cortes (2 s, 10 s, 30 s) y lo cierra al salir; `bridge/source_velo.rs` es la
  tercera `PlaybackSource`. QML: `Account.audioReady`, `audioBusy`, `enableAudio()`,
  `audioFailed(reason)` (ver docs/CONTRATO.md). UI: Ajustes → Cuenta → "Reproducción en Velo"
  y aviso clicable en la barra de reproducción cuando no hay nada sonando.

- **Cola y reintentos** (`queue`, E2b+Q1): Spirc 0.8 no tiene «set queue» público; quitar/mover
  recarga la reproducción como lista explícita (canción actual + posición + filas editadas + resto
  del contexto). La misma recarga reintenta (1 s, 3 s) una canción que librespot saltó por
  «Audio key response timeout». `EngineState.failures/last_failure` distinguen fallo transitorio
  (carga lenta o fin prematuro) de rechazo inmediato; `unavailable` cuenta precargas fallidas.

## Prueba en vivo (líder con el usuario)

### S4c: caché y temporales del audio (revisión offline de librespot 0.8.0)

Velo sigue usando `Session::new(..., None)`: **0 bytes de caché persistente de
audio**, sin archivos de credenciales/volumen. Aunque `librespot-core/src/cache.rs`
ofrece `Cache::new(credentials_path, volume_path, audio_path, size_limit)`, no se
activa para conservar canciones entre sesiones. El ajuste `audioCacheLimitMb = 200`
queda reservado; no promete un límite sobre los temporales del stream.

En `librespot-audio/src/fetch/mod.rs`, `AudioFileStreaming::open` crea un
`NamedTempFile::new_in(session.config().tmp_dir)` y preasigna el tamaño de la
canción cifrada. Reabre un descriptor para lectura. Al completar la recepción,
el temporal pasa por `complete_tx`; solo se copia a una caché si
`session.cache()` existe. Con `None`, el `NamedTempFile` se descarta sin persistir.
`tempfile` 3.27.0 borra la ruta en `TempPath::drop`; los streams en vuelo también
liberan el temporal al cancelar/cerrar. En Windows el borrado está sujeto a
descriptores abiertos y errores del sistema: es limpieza RAII, no una garantía
ante finalización forzada, corte de energía o permisos. No se barre `%TEMP%`
desde Ajustes porque contiene archivos de otras aplicaciones.

No se exporta PCM ni se guardan canciones para escuchar offline. No hay un límite
público de 200 MiB para los temporales activos en esta versión; un stream largo
y su precarga pueden superar ese tamaño transitoriamente. Tampoco se certifica
que toda la instalación ocupe ≤1 GB: eso se mide al empaquetar (S4).

Prueba del líder, sin la app oficial y con el target/Qt habituales:
`cargo run -p velo-player --example play_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC 20 --pause-seconds 10`.
Comprobar con Process Monitor los temporales creados/borrados por el PID de Velo
al cambiar canciones y cerrar normalmente; comparar con terminación forzada.
En la app: llenar carátulas, reiniciar, abrir Ajustes, bajar el límite y borrar;
comprobar tamaño, regeneración de imágenes y que la sesión sigue iniciada.

F6: perfil ARTIST_V4 y ruta candidata de estadísticas (sin audio ni tokens en salida):
`cargo run -p velo-player --example artist_probe -- spotify:artist:4LLpKhyESsyAXpc4laK94U`.
Alcance, fuentes locales y limitaciones en `docs/F6-artistas.md`; todavía requiere prueba en vivo.

SF2: ALBUM_V4 conserva el tipo EP (el filtro «Sencillos y EP» incluye ambos).
Letras Spotify, sin imprimir versos ni tokens y sin reproducir audio:
`cargo run -p velo-player --example lyrics_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC`.
La app recurre a LRCLIB si no están disponibles. El límite de seis ciclos de
reconexión por diez minutos vive en `AudioCtl` de la app; no lo prueba este crate
standalone. Alcance, pruebas manuales y licencia: `docs/SF2-backend.md` y
`THIRD_PARTY_NOTICES.md`.

```powershell
# 1. Motor solo, sin app (abre el navegador la primera vez):
cargo run -p velo-player --example play_probe -- --login spotify:track:4uLU6hMCjMI75M1A2tKUQC 20 --pause-seconds 10
# 2. Sin URI: espera; desde el celular, "Conectar a un dispositivo" → Velo.
cargo run -p velo-player --example play_probe
# 3. App completa: Ajustes → Cuenta → Activar; cerrar la app oficial de Spotify y reproducir.
cargo run -p velo
```

Checklist: activar audio, reproducir sin la app oficial, siguiente/anterior, biblioteca,
búsqueda, artista, letras, transferir desde el celular y de vuelta, RAM/CPU reproduciendo y en
pausa (el probe las imprime; para comparar usar `--release`).

---

# Historia: E1, prueba de factibilidad de librespot 0.8

Se continúa el commit `2b3334c` ("E1 WIP: primeros módulos"). Este crate no
depende de Qt y no implementa todavía el contrato del motor E2 ni publica un
dispositivo Connect. Las llamadas load/play/pause/seek encolan comandos; los
resultados llegan por eventos. Construcción, OAuth y cierre deben ejecutarse
desde un runtime Tokio de trabajo, fuera del hilo de UI. Preferir `shutdown().await`:
el Drop de librespot espera al decoder y el adaptador delega ese cierre a un hilo.

## Prueba real del líder (Windows, red y cuenta Premium)

Usar el mismo `VELO_SPOTIFY_CLIENT_ID` del ejemplo `login`; ambos ejemplos también
leen esa clave de `velo.local.env`. No ejecutar simultáneamente Velo/login y el
probe: pueden competir al rotar el refresh token del mismo keyring.

```powershell
$env:CARGO_NET_OFFLINE = 'true' # solo Cargo; el ejecutable sí necesita red
cargo run -p velo-spotify --example login -- --sign-out
cargo run -p velo-spotify --example login
cargo run -p velo-player --example play_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC 20
cargo run -p velo-player --example play_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC 20 --pause-seconds 130 --pcm
```

El login nuevo solicita `streaming`. Un refresh token anterior no gana ese scope
por refrescarse: el probe exige que la respuesta OAuth lo confirme y distingue
scope ausente de scope explícitamente insuficiente. Los errores no contienen
tokens ni cuerpos OAuth. Solo se persiste el refresh token, mediante
`velo_spotify::KeyringTokenStore`; el access token queda en memoria. El método
privado `SpotifyProvider::access_token` no expone tokens ni scopes, por lo que este
adaptador reproduce sus reglas (expiración válida, rotación, invalid_grant y un
reintento 429 corto). E2 necesita una API compartida y una sola exclusión de refresh.

El plazo comienza al recibir `Playing`, con timeout de carga de 45 s. Los eventos
`AudioError`/`Unavailable`, cierre inesperado y ausencia de PCM con `--pcm` hacen
fallar el ejemplo. Imprime working set actual/máximo y CPU media (kernel + usuario)
durante reproducción, pausa opcional y proceso completo antes del cierre. Son dos
lecturas Win32 por intervalo, sin hilo monitor: `GetProcessTimes` y
`K32GetProcessMemoryInfo`. Se muestra CPU equivalente a un núcleo (puede superar
100%) y dividida por los procesadores disponibles. Una build debug y la impresión
de eventos influyen en el consumo: repetir con `--release` para comparar recursos.

PCM f64 estéreo, 44.1 kHz, después del volumen por software: opt-in, dos paquetes
en memoria como máximo, entrega lossy con `try_reserve` sin bloquear el decoder.
Desactivar al minimizar/perder foco y vaciar paquetes antiguos antes de reactivar.
La posición viene del decoder (4 Hz nominales durante reproducción, sin timer) y
puede adelantar al altavoz por la cola Rodio, aproximadamente medio segundo. FFT,
regulación a la tasa de refresco de la pantalla y publicación de posición solo con ventana visible son E2.

## Resultados de la revisión de pausa y límites de E1

Revisado en las fuentes descargadas de librespot 0.8.0:

- `librespot-playback/src/player.rs`: `handle_pause` deja de decodificar,
  detiene el sink y después emite `Paused`; `poll` devuelve `Pending` esperando
  comandos cuando no reproduce. Las posiciones se calculan al decodificar.
- El backend Rodio original conserva su `OutputStream` aun pausado. `TappedSink`
  lo abre bajo demanda y lo destruye en `stop`, después de drenar la cola una vez;
  así libera CPAL/WASAPI y sus callbacks. El test con sink simulado comprueba
  un único stop/drop, ausencia de PCM estando cerrado y posibilidad de reabrir.
- **La sesión sigue con trabajo de red:** `librespot-core/src/session.rs`,
  `DispatchTask`, mantiene timeouts y Ping/Pong (ciclo de unos 120 s) aun en pausa;
  el prefetch de `librespot-audio/src/fetch/receive.rs` puede continuar recibiendo
  bytes comprimidos (la transición a pausa no desactiva su modo streaming). Por eso NO
  se afirma cumplimiento estricto de cero timers/hilos periódicos en pausa, ni
  CPU medida cerca de cero. La opción `--pause-seconds 130` permite al líder medir
  ese límite. Suspender/recrear sesión para una pausa totalmente inactiva es E2.
- **Sin caché no significa sin temporales:**
  `librespot-audio/src/fetch/mod.rs` crea `NamedTempFile` para bytes comprimidos
  recibidos por streaming. No hay opción pública en 0.8 para usar solo RAM.
  `Session::new(..., None)` y `connect(..., false)` desactivan caché persistente y
  archivos de credenciales; Velo no guarda canciones ni exporta PCM. Si la regla
  exige también cero temporales comprimidos, esta dependencia falla ese criterio;
  requeriría un cambio upstream/fork, fuera del alcance autorizado de E1.
- El constructor Rodio upstream usa `unwrap` al abrir el dispositivo. Se contiene
  con `catch_unwind` en debug, pero `panic=abort` en release puede terminar el
  proceso sin evento; además el propio Player upstream tiene rutas `exit(1)` al
  fallar un sink. Probar dispositivo real, pausa, seek, volumen y desconexiones
  antes de integrar. Velo no agrega `unwrap` en código productivo del adaptador.

No se verificaron reproducción, permisos reales del Client ID, compatibilidad de
login5 ni condiciones de uso de Spotify en este sandbox sin red. La aceptación de
los términos y la prueba de Premium/Windows corresponden al líder. Ningún dato
de consumo real se inventa. No se añadieron paquetes que faltaran en caché.

## Verificación offline y alcance

El `Cargo.lock` raíz heredado no lista las dependencias directas reqwest/serde/
serde_json/url añadidas en el WIP (ni wiremock añadido aquí para tests). Todas ya
existen en ese lockfile y en la caché. Cargo sincroniza automáticamente esa entrada
al ejecutar los comandos anteriores sin `--locked`. No se editó el lockfile raíz:
las compilaciones/tests/clippy se ejecutaron con una copia temporal del workspace
(domain, spotify y player idénticos), su propio lockfile y el target del worktree.
El líder deberá sincronizar el lockfile al integrar.

Resultados offline: build de ejemplos correcto; 14 tests de player, 41 de spotify
y 3 del ejemplo pasan (0 fallos). Clippy de player con `--tests --examples -- -D
warnings` está limpio. El test nativo solo verifica lectura de métricas del propio
proceso; no sustituye la medición de reproducción/pausa real.

`cargo fmt --all -- --check` también detecta formato previo en
`crates/domain/src/lib.rs` y `crates/spotify/examples/login.rs`, fuera del alcance
de E1. Los archivos Rust de player y el cambio autorizado en oauth.rs se formatean
y comprueban por separado, sin modificar esos archivos ajenos.

## R3b: volumen, continuidad y fades

La salida lee la ganancia atómica original del SoftMixer por frame, aunque el decoder espere.
Las duraciones están documentadas como constantes en `pcm.rs`: volumen ~5 ms, pausa ~120 ms,
entrada ~150 ms y micro-fades de seek/carga manual ~8 ms por lado, con curva coseno. Pausa y
play pueden invertir el gradiente desde su ganancia vigente. Un worker suelta WASAPI al llegar
a cero y luego espera eventos sin timer. El avance audible consume PCM; el resto y el canal
pendiente sobreviven a la reapertura. Los fines naturales mantienen el gapless. El tap toma
PCM anterior al volumen/fades y fecha el primer frame de cada generación de activación.

Prueba auditiva (cerrar Velo, requiere red y login de audio guardado):

```powershell
cargo run -p velo-player --example volume_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC
cargo run -p velo-player --example play_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC 20 --pause-seconds 130 --pcm
```

El primer probe prueba mute/arrastre, pausas y reanudaciones rápidas a 20/80 %, inversión de
fades y seek. En la app: repetir a 20/40/50/100 %, cambiar pista manualmente y escuchar un
álbum continuo (fin natural sin fade). Cambiar USB/HDMI en reproducción y pausa, y medir CPU
tras el fade. La latencia audible y la continuidad real del dispositivo quedan pendientes de
escucha; el probe solo mide confirmación de eventos, no el altavoz. Librespot conserva su
keepalive de sesión, límite ya explicado arriba. La descripción histórica de E1 de PCM
posterior al volumen no describe este sink.

Cola: si falla una página, timeout o límite de resolución, la edición/reintento se rechaza y
la UI vuelve a su lista anterior. Duplicados se cuentan por ocurrencia: listas explícitas
con índice conocido y multiconjunto en aleatorio. Para una pista actual duplicada de un
contexto externo cuyo índice no expone Spirc 0.8, se rechaza la ambigüedad en vez de adivinar.
El temporizador local «al terminar esta canción» se arma en el sink con la URI/ocurrencia
actual. EOF fija el límite de su PCM encolado; el fade termina en ese límite, y el PCM de la
siguiente canción queda intacto. El decoder recibe pausa antes de seguir escribiendo después
del límite; Spirc refleja el evento Paused en Connect. Pausa/reanudación y seek previos al EOF
mantienen el armado, una carga manual lo cancela y una nueva activación usa otro canal watch.
El probe de volumen también hace seek a los últimos 3 s y verifica la finalización local.
