# Velo

Cliente de escritorio para tu cuenta de Spotify, escrito en Rust y Qt 6.10 (Qt Quick/QML). Reproduce música con librespot y ofrece una interfaz con biblioteca, letras y visualización de audio. Versión **0.9.1**, licencia MIT.

## Funciones

- Reproducción local con tu cuenta Premium y control de otros dispositivos Spotify Connect.
- Biblioteca, búsqueda con sugerencias, playlists, álbumes, artistas y Me gusta mediante la sesión de Spotify y, con una app personal, la Web API oficial.
- Letras sincronizadas, cola, historial local y selector de dispositivos.
- Visualizador del audio de Spotify, con barras y modo circular; nunca captura otras aplicaciones.
- Ecualizador de 10 bandas, zoom, listas compactas, proxy y temporizador de apagado.
- Temas claro y oscuro, controles multimedia de Windows y bandeja del sistema.
- Modo rendimiento con opción de mantener o apagar el visualizador. Los efectos visuales se detienen con la ventana minimizada o la música en pausa.

## Requisitos

- Windows 11 de 64 bits.
- Cuenta de Spotify Premium.

## Instalación

El release 0.9.1 trae dos opciones desde la página de Releases del repositorio:

- **Portable:** `Velo-win64.zip`. Descomprime el ZIP y ejecuta `velo.exe`; incluye las DLL de Qt.
- **Instalador:** `Velo-Setup-0.9.1.exe`. Ejecuta el instalador y sigue sus instrucciones.

Inicia sesión con tu cuenta de Spotify en la pantalla de inicio. Puedes cerrar sesión desde Ajustes.

## Actualizaciones

Al abrir Velo se busca una versión estable nueva en GitHub Releases. Si no hay novedades,
no aparece ningún aviso. «Actualizar» descarga y verifica el instalador, cierra Velo,
instala la versión y vuelve a abrirla. «Más tarde» oculta el aviso hasta el próximo inicio.
En la versión portable, «Actualizar» abre la publicación para descargarla manualmente.
En Ajustes › Acerca de puedes desactivar la búsqueda al iniciar o pulsar «Buscar ahora».

## Publicar una versión

1. Sube `version` en `[workspace.package]` de `Cargo.toml` a `X.Y.Z`.
2. Compila con `cargo build --release -p velo -j 4`.
3. Ejecuta `scripts\package.ps1 -Installer` (requiere Inno Setup 6).
4. Crea el release estable con tag `vX.Y.Z` y sube los tres archivos de `dist` con estos
   nombres exactos: `Velo-Setup-X.Y.Z.exe`, `Velo-win64.zip` y `SHA256SUMS.txt`.

El script toma la versión del workspace y genera sumas SHA-256 con líneas
`<sha256 hex>  <nombre de archivo>`. Sin la suma del instalador, Velo rechaza la instalación.

## Compilar desde el código

Necesitas Rust estable con la toolchain MSVC, las herramientas C++ de Visual Studio y Qt 6.10.3 `msvc2022_64`. `scripts\setup-windows.ps1`, ejecutado en PowerShell como administrador, prepara las herramientas en un PC nuevo.

```powershell
$env:QMAKE = 'C:\Qt\6.10.3\msvc2022_64\bin\qmake.exe'
cargo build --release -p velo
```

El ejecutable queda en `target\release\velo.exe`. Para crear el paquete portable, cierra Velo y ejecuta:

```powershell
powershell -File scripts\package.ps1
```

El script compila el release (o usa uno existente con `-ExecutablePath`), ejecuta `windeployqt`, comprueba que no haya configuración ni credenciales en el paquete y crea `dist\Velo-win64.zip`.

## Pruebas

```powershell
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

Los tests QML utilizan los mocks locales:

```powershell
& 'C:\Qt\6.10.3\msvc2022_64\bin\qmltestrunner.exe' -import qml/mock -import qml -input qml/preview/tests
```

Las comprobaciones manuales de reproducción requieren una cuenta Premium y conexión con Spotify.

## Estructura

| Ruta | Función |
|---|---|
| `crates/domain` | Tipos y contratos compartidos |
| `crates/spotify` | Autorización y Web API |
| `crates/player` | Audio con librespot y servicios de sesión |
| `crates/nowplaying` | Integración multimedia de Windows |
| `crates/services` | Ajustes, caché, letras e historial |
| `crates/viz` | Captura de audio y análisis FFT |
| `crates/app` | Aplicación, puente cxx-qt y código nativo |
| `qml/` | Interfaz Qt Quick, temas y recursos |
| `qml/mock`, `qml/preview/tests` | Datos de prueba y tests QML |
| `scripts/` | Preparación de herramientas y empaquetado |

`main` contiene las versiones publicadas; `develop` integra cambios de las ramas `feature/<módulo>`. Los commits usan prefijos convencionales y cada versión lleva un tag anotado.

## Privacidad

Los tokens de Spotify se guardan en el almacén de credenciales del sistema (keyring), nunca en archivos ni logs. El historial de escucha permanece local. La caché cifrada de audio de librespot es opcional, con límite configurable en Ajustes. Velo no extrae ni exporta audio a archivos.

## Compatibilidad con Spotify

Velo es un proyecto independiente, no afiliado, respaldado ni patrocinado por Spotify. Spotify es marca de sus propietarios.

Velo usa librespot y servicios internos de Spotify no documentados. Su disponibilidad puede cambiar y su uso puede estar sujeto a restricciones de Spotify. El inicio de sesión predeterminado utiliza la cuenta de Spotify a través de librespot. También puedes configurar una app personal de Spotify para la Web API, sujeta a sus permisos y cuotas.

## Licencia y créditos

Velo se distribuye bajo [MIT](LICENSE). Consulta [los avisos de terceros](THIRD_PARTY_NOTICES.md) para las referencias y licencias de los componentes utilizados.

## English

Velo 0.9.1 is an independent desktop Spotify client built with Rust and Qt Quick/QML. It plays music through librespot, controls Spotify Connect devices, and provides library browsing, search, lyrics, a queue, local history and audio visualization.

Requirements: 64-bit Windows 11 and Spotify Premium. The release includes `Velo-win64.zip` (portable; extract and run `velo.exe`) and `Velo-Setup-0.9.1.exe` (installer). Sign in with your Spotify account. A personal Spotify app can also be configured for the official Web API.

To build, install Rust stable (MSVC), Visual Studio C++ tools and Qt 6.10.3 `msvc2022_64`, set `QMAKE` to `qmake.exe`, then run `cargo build --release -p velo`. `scripts\package.ps1` creates the portable ZIP.

Tokens stay in the system keyring. Listening history stays local and the encrypted audio cache is optional. Velo is not affiliated with Spotify and depends on undocumented services whose availability may change. Licensed under MIT.
