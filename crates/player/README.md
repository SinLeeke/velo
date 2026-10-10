# velo-player

Motor de reproducción local de Velo con librespot 0.8, sin dependencia de Qt. Requiere Spotify Premium y se publica como dispositivo Spotify Connect. Las operaciones de red y decodificación se ejecutan fuera del hilo de interfaz; el estado se entrega mediante eventos y canales `watch`.

## Autorización y sesión

El login de audio utiliza el cliente de librespot, el scope `streaming` y el callback `http://127.0.0.1:8898/login`. El refresh token se guarda en el almacén de credenciales del sistema; los access tokens permanecen en memoria. El motor restaura la sesión guardada al arrancar.

`session_api` obtiene biblioteca y catálogo a través de la sesión. La Web API y su autorización opcional mediante una app personal se implementan en `velo-spotify`.

## Audio y almacenamiento

La salida mantiene reproducción gapless, normalización, volumen y ecualización. El tap PCM para visualización es optativo y toma las muestras antes del ecualizador, volumen y fades. No se exporta audio.

La caché persistente contiene únicamente datos cifrados de librespot. `EngineOptions` permite desactivarla o fijar su directorio por cuenta y límite de tamaño. Ajustes ofrece 512 MiB, 1 GiB o 4 GiB. Las credenciales se conservan por separado en el keyring.

Librespot también crea archivos temporales de streaming. Su limpieza normal depende del cierre de los streams y de los descriptores del sistema; una terminación forzada puede dejar temporales. El límite de caché persistente no limita esos archivos activos ni habilita descargas para escuchar offline.

Al pausar, el sink aplica un fade y libera WASAPI; al reanudar conserva el PCM pendiente. La sesión de Spotify mantiene sus comunicaciones de red incluso en pausa. Las duraciones de volumen, pausa, entrada y cambios manuales están definidas en `pcm.rs`.

## Cola y metadatos

Quitar, mover o vaciar la cola local recarga una lista explícita conservando la canción actual. Los duplicados se cuentan por ocurrencia; una resolución incompleta o ambigua se rechaza. Los dispositivos Connect remotos conservan las restricciones de la Web API.

El módulo también consulta letras de Spotify, metadatos de artistas, discografía, radio y DJ. Estas rutas dependen de servicios internos no documentados; las letras utilizan LRCLIB como respaldo en la aplicación. Consulta [los avisos de terceros](../../THIRD_PARTY_NOTICES.md).

## Comprobaciones manuales

Los ejemplos requieren conexión con Spotify y una cuenta Premium. Cierra Velo antes de ejecutar un ejemplo que utilice la misma sesión.

```powershell
cargo run -p velo-player --example play_probe -- --login spotify:track:4uLU6hMCjMI75M1A2tKUQC 20 --pause-seconds 10
cargo run -p velo-player --example volume_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC
cargo run -p velo-player --example lyrics_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC
```

Verifica reproducción, pausa, seek, mute, continuidad entre canciones, cambio de salida USB/HDMI y transferencia Connect. Los ejemplos de recursos miden el proceso y sus eventos; no sustituyen la escucha ni la medición del dispositivo real.

Los tests del crate usan fixtures y servidores locales, sin red real:

```powershell
cargo test -p velo-player
```
