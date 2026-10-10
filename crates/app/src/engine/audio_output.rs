//! Windows system playback endpoints. One sleeping COM worker, woken by notifications or
//! commands; Qt only receives rows. No polling, including while paused/minimized.
use std::sync::{Arc, mpsc};

use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};

use super::audio::AudioCtl;

pub const OUTPUT_ROLES: &str = "outputId,name,isDefault";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn rows(outputs: &[Output]) -> Vec<Value> {
    outputs
        .iter()
        .map(|o| json!({"outputId": o.id, "name": o.name, "isDefault": o.is_default}))
        .collect()
}

enum Command {
    Refresh,
    Set(String),
    Shutdown,
}

pub struct AudioOutputs {
    commands: mpsc::Sender<Command>,
    pub view: watch::Sender<Vec<Output>>,
}

impl AudioOutputs {
    pub fn start(audio: Arc<AudioCtl>, errors: broadcast::Sender<String>) -> Self {
        let (commands, receiver) = mpsc::channel();
        let view = watch::channel(Vec::new()).0;
        let updates = view.clone();
        let events = commands.clone();
        if std::thread::Builder::new()
            .name("velo-audio-outputs".into())
            .spawn(move || {
                platform::run(receiver, events, updates, audio, errors);
            })
            .is_err()
        {
            tracing::warn!("could not start audio output worker");
        }
        Self { commands, view }
    }
    pub fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }
    pub fn set(&self, id: String) {
        let _ = self.commands.send(Command::Set(id));
    }
    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

impl Drop for AudioOutputs {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn run(
        receiver: mpsc::Receiver<Command>,
        _: mpsc::Sender<Command>,
        view: watch::Sender<Vec<Output>>,
        _: Arc<AudioCtl>,
        errors: broadcast::Sender<String>,
    ) {
        view.send_replace(Vec::new());
        while let Ok(command) = receiver.recv() {
            match command {
                Command::Shutdown => break,
                Command::Refresh => {}
                Command::Set(id) => {
                    let _ = id;
                    let _ = errors.send(
                        "El selector de salida del sistema está disponible en Windows.".into(),
                    );
                }
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::{
        Win32::{
            Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
            Foundation::PROPERTYKEY,
            Media::Audio::{
                DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDeviceEnumerator,
                IMMEndpoint, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
                eCommunications, eConsole, eMultimedia, eRender,
            },
            System::Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize, STGM_READ,
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            },
        },
        core::{
            GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, PWSTR, Result, implement,
        },
    };

    // IPolicyConfig (Windows 7+), ABI used by EarTrumpet / SoundSwitch.
    // Ten methods between IUnknown and SetDefaultEndpoint, in order: GetMixFormat,
    // GetDeviceFormat, ResetDeviceFormat, SetDeviceFormat, Get/SetProcessingPeriod,
    // Get/SetShareMode, Get/SetPropertyValue. They are deliberately never called here.
    windows::core::define_interface!(
        IPolicyConfig,
        IPolicyConfigVtbl,
        0xf8679f50_850a_41cf_9c72_430f290290c8
    );
    #[repr(C)]
    pub struct IPolicyConfigVtbl {
        base: IUnknown_Vtbl,
        unused: [usize; 10],
        set_default: unsafe extern "system" fn(*mut std::ffi::c_void, PCWSTR, ERole) -> HRESULT,
        set_visibility: usize,
    }
    const POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    struct Registration {
        enumerator: IMMDeviceEnumerator,
        client: IMMNotificationClient,
    }
    impl Drop for Registration {
        fn drop(&mut self) {
            unsafe {
                let _ = self
                    .enumerator
                    .UnregisterEndpointNotificationCallback(&self.client);
            }
        }
    }

    #[implement(IMMNotificationClient)]
    struct Notifications(mpsc::Sender<Command>);
    impl Notifications {
        fn changed(&self) -> Result<()> {
            let _ = self.0.send(Command::Refresh);
            Ok(())
        }
    }
    #[allow(non_snake_case)]
    impl IMMNotificationClient_Impl for Notifications_Impl {
        fn OnDefaultDeviceChanged(&self, flow: EDataFlow, _: ERole, _: &PCWSTR) -> Result<()> {
            if flow == eRender {
                self.changed()?;
            }
            Ok(())
        }
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
            self.changed()
        }
        fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
            self.changed()
        }
        fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
            self.changed()
        }
        fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
            self.changed()
        }
    }

    // Both GetId and PropVariantToStringAlloc allocate with the COM task allocator.
    unsafe fn take_string(value: PWSTR) -> Result<String> {
        let result = unsafe { value.to_string() };
        unsafe { CoTaskMemFree(Some(value.0.cast())) };
        Ok(result?)
    }

    unsafe fn enumerate(enumerator: &IMMDeviceEnumerator) -> Result<(Vec<Output>, Option<String>)> {
        unsafe {
            let default = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .ok()
                .and_then(|d| d.GetId().ok())
                .and_then(|id| take_string(id).ok());
            let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let mut outputs = Vec::new();
            for index in 0..devices.GetCount()? {
                let device = devices.Item(index)?;
                let id = take_string(device.GetId()?)?;
                let properties = device.OpenPropertyStore(STGM_READ)?;
                let mut value = properties.GetValue(&PKEY_Device_FriendlyName)?;
                let name = PropVariantToStringAlloc(&value).and_then(|s| take_string(s));
                let _ = PropVariantClear(&mut value);
                outputs.push(Output {
                    is_default: default.as_ref() == Some(&id),
                    id,
                    name: name?,
                });
            }
            outputs.sort_by(|a, b| {
                b.is_default
                    .cmp(&a.is_default)
                    .then_with(|| a.name.cmp(&b.name))
            });
            Ok((outputs, default))
        }
    }

    unsafe fn set_default(enumerator: &IMMDeviceEnumerator, id: &str) -> Result<()> {
        // Reject stale/disconnected IDs before calling the undocumented interface.
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        let id = PCWSTR(wide.as_ptr());
        unsafe {
            let device = enumerator.GetDevice(id)?;
            if device.GetState()? != DEVICE_STATE_ACTIVE
                || device.cast::<IMMEndpoint>()?.GetDataFlow()? != eRender
            {
                return Err(windows::core::Error::from_hresult(HRESULT(
                    0x80070490u32 as i32,
                )));
            }
            let policy: IPolicyConfig =
                CoCreateInstance(&POLICY_CONFIG, None::<&IUnknown>, CLSCTX_ALL)?;
            for role in [eConsole, eMultimedia, eCommunications] {
                (policy.vtable().set_default)(policy.as_raw(), id, role).ok()?;
            }
        }
        Ok(())
    }

    pub fn run(
        receiver: mpsc::Receiver<Command>,
        events: mpsc::Sender<Command>,
        view: watch::Sender<Vec<Output>>,
        audio: Arc<AudioCtl>,
        errors: broadcast::Sender<String>,
    ) {
        // All COM creation, enumeration, changes and teardown stay on this worker.
        let result = unsafe { worker(receiver, events, view, audio, &errors) };
        if let Err(error) = result {
            tracing::warn!(%error, "audio endpoint worker failed");
            let _ = errors.send("No se pudieron consultar las salidas de audio de Windows.".into());
        }
    }
    unsafe fn worker(
        receiver: mpsc::Receiver<Command>,
        events: mpsc::Sender<Command>,
        view: watch::Sender<Vec<Output>>,
        audio: Arc<AudioCtl>,
        errors: &broadcast::Sender<String>,
    ) -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let _apartment = Apartment;
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None::<&IUnknown>, CLSCTX_ALL)?;
            let client: IMMNotificationClient = Notifications(events).into();
            enumerator.RegisterEndpointNotificationCallback(&client)?;
            let _registration = Registration {
                enumerator: enumerator.clone(),
                client,
            };
            let (outputs, mut default) = enumerate(&enumerator)?;
            view.send_replace(outputs);
            while let Ok(command) = receiver.recv() {
                match command {
                    Command::Shutdown => break,
                    Command::Set(id) => {
                        if id.contains('\0') || set_default(&enumerator, &id).is_err() {
                            let _ = errors.send("No se pudo cambiar la salida de audio de Windows. Vuelve a elegirla.".into());
                        }
                    }
                    Command::Refresh => {}
                }
                match enumerate(&enumerator) {
                    Ok((outputs, next)) => {
                        if next != default {
                            if let Some(engine) = audio.engine() {
                                engine.reopen_output();
                            }
                            default = next;
                        }
                        view.send_if_modified(|old| {
                            if *old == outputs {
                                false
                            } else {
                                *old = outputs;
                                true
                            }
                        });
                    }
                    Err(error) => tracing::warn!(%error, "audio endpoint refresh failed"),
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_keep_stable_endpoint_ids_and_default_marker() {
        let outputs = [Output {
            id: "{endpoint}".into(),
            name: "Auriculares".into(),
            is_default: true,
        }];
        assert_eq!(
            rows(&outputs),
            [json!({"outputId":"{endpoint}", "name":"Auriculares", "isDefault":true})]
        );
        assert!(rows(&[]).is_empty());
    }
}
