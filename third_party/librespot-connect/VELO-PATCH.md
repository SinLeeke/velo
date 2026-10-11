# Parche de Velo sobre librespot-connect 0.8.0

Origen: crates.io `librespot-connect` 0.8.0 (MIT), copiada tal cual del registro de cargo
(sin `Cargo.lock`). Activada con `[patch.crates-io]` en el `Cargo.toml` raíz.

## Qué cambia (todo marcado con `// VELO PATCH`)

- `src/state.rs`
  - `ConnectConfig.is_private_session: bool` (por defecto `false`); `ConnectState::new` lo copia a
    `DeviceInfo.is_private_session` (campo 11 de `connect.proto`).
  - `ConnectState::set_private_session(bool)`: cambia el flag dentro de `request.device.device_info`.
    Como `send_state` serializa siempre ese `request` completo, el flag persiste en todos los
    `PutStateRequest` siguientes (pausa, cambio de canción, volumen...).
  - `ConnectState::notify_device_info_changed`: reenvía el estado con `PutStateReason::NEW_DEVICE`.
  - Pruebas `velo_tests` (sin red).
- `src/spirc.rs`
  - Variante `SpircCommand::SetPrivateSession(bool)` y `Spirc::set_private_session(&self, bool)`.
  - Su manejo va antes de los brazos que ignoran comandos si el dispositivo no está activo, para
    que funcione también sin reproducir.

## Por qué

Spotify solo muestra el candado de sesión privada del dispositivo si el `PutStateRequest` lo
anuncia; librespot-connect 0.8 no lo expone y `ConnectState` es `pub(super)`.

## Regenerar al subir de versión

1. Copiar la nueva `librespot-connect-X.Y.Z` del registro sobre esta carpeta.
2. Reaplicar los cambios: `grep -rn "VELO PATCH"` en la versión anterior muestra cada sitio
   (3 en `state.rs` + tests, 3 en `spirc.rs`). Referencia: `git diff` entre el commit de vendor
   (original) y el commit del parche en el historial de Velo.
3. `cargo test -p librespot-connect` (pruebas `velo_tests`) y `cargo test -p velo-player`.
4. Si upstream añade su propia opción de sesión privada, quitar el parche y el `[patch]`.
