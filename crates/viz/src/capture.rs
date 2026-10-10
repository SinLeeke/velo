//! WASAPI process loopback only. There is intentionally no endpoint enumerator.
use std::mem::{ManuallyDrop, size_of};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, E_ABORT, E_FAIL, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize, IAgileObject, IAgileObject_Impl,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, SetEvent, WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows_core::{HRESULT, Interface, PCWSTR, PWSTR, Ref, Result, implement};

use crate::{Shared, analysis::Ring};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: owned successful Win32 handle, closed exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(crate) struct Event(Handle);
// SAFETY: kernel event handles may be signaled/waited across threads; Arc<Shared>
// keeps the handle alive until both the controller and worker have released it.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Event {
    pub(crate) fn new() -> Result<Self> {
        unsafe { CreateEventW(None, false, false, PCWSTR::null()).map(|h| Self(Handle(h))) }
    }
    pub(crate) fn signal(&self) {
        unsafe {
            let _ = SetEvent(self.0.0);
        }
    }
    fn wait(&self) -> Result<()> {
        if unsafe { WaitForSingleObject(self.0.0, 100) } == WAIT_FAILED {
            Err(windows_core::Error::from_thread())
        } else {
            Ok(())
        }
    }
}

pub(crate) struct Com;
impl Com {
    pub(crate) fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

// RtlGetVersion is not included in the prepared windows features (Wdk). Its
// stable ntdll ABI uses the same OSVERSIONINFOW structure as the Win32 API.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}
fn supported() -> bool {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    unsafe { RtlGetVersion(&mut version) >= 0 && version.dwBuildNumber >= 20348 }
}

fn spotify_process(pid: u32) -> Result<Handle> {
    // Retain the process handle throughout capture, preventing PID reuse from
    // accidentally retargeting a different executable after Spotify exits.
    let process = Handle(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )?
    });
    let mut name = [0u16; 32_768];
    let mut len = name.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(name.as_mut_ptr()),
            &mut len,
        )?;
    }
    let path = String::from_utf16_lossy(&name[..len as usize]);
    if !path
        .rsplit(['\\', '/'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("Spotify.exe"))
    {
        return Err(E_FAIL.into());
    }
    Ok(process)
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Completion {
    done: mpsc::Sender<()>,
    // Keep the pointed-to activation blob alive even if cancellation wins.
    _params: Box<AUDIOCLIENT_ACTIVATION_PARAMS>,
}
impl IAgileObject_Impl for Completion_Impl {}
impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> Result<()> {
        let _ = self.done.send(());
        Ok(())
    }
}

fn activate(pid: u32, shared: &Shared, generation: u64) -> Result<IAudioClient> {
    let mut params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    });
    // Borrowed blob; do not PropVariantClear a pointer owned by the callback Box.
    let variant = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: (&mut *params as *mut AUDIOCLIENT_ACTIVATION_PARAMS).cast(),
                    },
                },
                ..Default::default()
            }),
        },
    };
    let (done, receiver) = mpsc::channel();
    let callback: IActivateAudioInterfaceCompletionHandler = Completion {
        done,
        _params: params,
    }
    .into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&variant),
            &callback,
        )?
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if !shared.current(generation) {
            return Err(E_ABORT.into());
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            _ => return Err(E_FAIL.into()),
        }
    }
    let mut status = HRESULT(0);
    let mut interface = None;
    // All audio interfaces are obtained/used/released on this same MTA worker.
    unsafe {
        operation.GetActivateResult(&mut status, &mut interface)?;
    }
    status.ok()?;
    interface
        .ok_or_else(|| windows_core::Error::from(E_FAIL))?
        .cast()
}

pub(crate) struct Capture {
    capture: IAudioCaptureClient,
    client: IAudioClient,
    process: Handle,
    float: bool,
}
impl Capture {
    pub(crate) fn open(pid: u32, shared: &Shared, generation: u64) -> Result<Self> {
        if !supported() {
            return Err(E_FAIL.into());
        }
        let event = shared
            .event
            .as_ref()
            .ok_or_else(|| windows_core::Error::from(E_FAIL))?;
        let process = spotify_process(pid)?;
        for float in [true, false] {
            if !shared.current(generation) {
                return Err(E_ABORT.into());
            }
            let client = activate(pid, shared, generation)?;
            let bits = if float { 32 } else { 16 };
            let format = WAVEFORMATEX {
                wFormatTag: if float { 3 } else { 1 }, // IEEE_FLOAT / PCM
                nChannels: 2,
                nSamplesPerSec: 48_000,
                nAvgBytesPerSec: 48_000 * 2 * u32::from(bits / 8),
                nBlockAlign: 2 * (bits / 8),
                wBitsPerSample: bits,
                cbSize: 0,
            };
            let initialized = unsafe {
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                    0,
                    0,
                    &format,
                    None,
                )
            };
            if let Err(error) = initialized {
                if float {
                    continue;
                }
                return Err(error);
            }
            unsafe {
                client.SetEventHandle(event.0.0)?;
                let capture = client.GetService()?;
                let stream = Self {
                    capture,
                    client,
                    process,
                    float,
                };
                if !shared.current(generation) {
                    return Err(E_ABORT.into());
                }
                stream.client.Start()?;
                return Ok(stream);
            }
        }
        Err(E_FAIL.into())
    }

    pub(crate) fn read(
        &mut self,
        ring: &mut Ring,
        shared: &Shared,
        generation: u64,
    ) -> Result<bool> {
        shared
            .event
            .as_ref()
            .ok_or_else(|| windows_core::Error::from(E_FAIL))?
            .wait()?;
        if unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_OBJECT_0 {
            return Err(E_FAIL.into());
        }
        let mut received = false;
        // Bound each drain as well as the event wait; a continuously full stream
        // must not delay cancellation or starve FFT/publication.
        for _ in 0..32 {
            if !shared.current(generation) {
                break;
            }
            if unsafe { self.capture.GetNextPacketSize()? } == 0 {
                break;
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0;
            let mut flags = 0;
            unsafe {
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            }
            if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                ring.clear();
            }
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null();
            for frame in 0..frames as usize {
                // SAFETY: WASAPI supplies frames * 2 samples of the initialized
                // format, valid until ReleaseBuffer. Silent packets need no data.
                let mono = unsafe {
                    if silent {
                        0.0
                    } else if self.float {
                        let p = data.cast::<f32>().add(frame * 2);
                        (p.read_unaligned() + p.add(1).read_unaligned()) * 0.5
                    } else {
                        let p = data.cast::<i16>().add(frame * 2);
                        (f32::from(p.read_unaligned()) + f32::from(p.add(1).read_unaligned()))
                            / 65536.0
                    }
                };
                ring.push(mono);
            }
            unsafe {
                self.capture.ReleaseBuffer(frames)?;
            }
            received |= frames != 0;
        }
        Ok(received)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_this_non_spotify_process() {
        assert!(spotify_process(std::process::id()).is_err());
    }
    #[test]
    fn completion_is_agile() {
        let (done, _) = mpsc::channel();
        let callback: IActivateAudioInterfaceCompletionHandler = Completion {
            done,
            _params: Box::default(),
        }
        .into();
        assert!(callback.cast::<IAgileObject>().is_ok());
    }
}
