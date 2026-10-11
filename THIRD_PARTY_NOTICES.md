# Avisos de terceros

## spotdj

- Nombre: spotdj
- URL: https://github.com/spotlightishere/spotdj
- Licencia: MIT
- Referencia para DJ Livi (E5b): resolución lexicon y petición TTS protobuf con
  redirección al audio de Spotify. Implementación Rust en `crates/player/src/dj.rs`;
  la voz se decodifica exclusivamente en memoria.
- Copyright: no se encontró el aviso original en el código fuente disponible ni
  en los materiales disponibles. Pendiente verificar su línea de copyright en LICENSE
  de upstream al integrar; no se atribuye un nombre o año supuesto.

Texto de la licencia MIT (copyright original pendiente de verificar):

```text
MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Spotifast

- Nombre: Spotifast
- URL: https://github.com/crmne/spotifast
- Autor: © 2026 Carmine Paolino
- Licencia: MIT
- Adaptado:
  - Distribución e inicio de sesión (D2): Client ID público compartido, redirect Web API `8989/login`, app personal con comprobación de la misma cuenta y asistente. Referencias de solo lectura: src/auth.rs (DEFAULT_WEB_CLIENT_ID, WEB_REDIRECT_PORT, REDIRECT_PATH, personal_web_api), src/credentials.rs (separación de grants y descarte tras logout). Implementación en crates/spotify/src/oauth.rs, crates/app/src/engine/account.rs y qml/components/library/PersonalAppWizard.qml. Velo conserva un único grant Web API seleccionado y su grant de audio en el keyring; no incorpora el enrutado dual de Spotifast ni sus cachés de audio.
  - Inicio — Hecho para ti (términos, filtrado y deduplicación), Acceso rápido (Me gusta y playlists, ampliado con álbumes recientes de Velo) y Top tracks (short_term, 20 canciones y reproducción desde una fila). Referencias: src/ui/home.rs, src/model.rs, src/app.rs y src/backend.rs.
  - Cola (X2): vaciar la cola y guardarla como playlist. Referencias: src/ui/queue.rs y las acciones de cola en src/app.rs; en Velo, crates/player/src/queue.rs y crates/app/src/engine/player.rs.
  - Letras (SF2): letras oficiales de Spotify (color-lyrics) con respaldo LRCLIB. Referencias: src/player.rs:503, src/lyrics.rs:73, src/backend.rs:3995 (commit 12deee4).
  - Discografía (SF2): distinción de EP frente a sencillos. Referencia: src/player.rs:493.
  - Reconexión (SF2): presupuesto de reconexiones (6 caídas en 10 min) y conservación del volumen. Referencia: src/backend.rs:50,62,2572,2599. No se incorpora el fork de librespot ni sus parches.
  - Escritorio (X1): instancia única (loopback con token) y normalización de enlaces de Spotify. Referencias: src/single_instance.rs y src/link.rs; en Velo, crates/app/src/single_instance.rs y crates/app/src/engine/links.rs. La bandeja usa Win32 y Qt propios de Velo; no incorpora fastframe ni egui.

Texto completo de LICENSE:

```text
MIT License

Copyright (c) 2026 Carmine Paolino

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## librespot-connect (parcheado)

- Nombre: librespot-connect 0.8.0 (parte de librespot)
- URL: https://github.com/librespot-org/librespot
- Licencia: MIT
- Incluida en `third_party/librespot-connect/` y sustituida vía `[patch.crates-io]`.
  Parche de Velo: anuncia la sesión privada (`DeviceInfo.is_private_session`) a Spotify.
  Detalle en `third_party/librespot-connect/VELO-PATCH.md`.
