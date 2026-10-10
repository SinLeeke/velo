use super::{Button, Snapshot};
use tokio::sync::{mpsc, watch};
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
    SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::{
    ISystemMediaTransportControlsInterop, RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize,
};
use windows::core::{HSTRING, Result};

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

struct Session {
    controls: SystemMediaTransportControls,
    token: i64,
    last: Snapshot,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.controls.RemoveButtonPressed(self.token);
        let _ = self.controls.SetIsEnabled(false);
        if let Ok(updater) = self.controls.DisplayUpdater() {
            let _ = updater.ClearAll();
            let _ = updater.Update();
        }
    }
}

fn map_button(button: SystemMediaTransportControlsButton) -> Option<Button> {
    match button {
        SystemMediaTransportControlsButton::Play => Some(Button::Play),
        SystemMediaTransportControlsButton::Pause => Some(Button::Pause),
        SystemMediaTransportControlsButton::Next => Some(Button::Next),
        SystemMediaTransportControlsButton::Previous => Some(Button::Previous),
        _ => None,
    }
}

pub(super) fn run(
    hwnd: usize,
    buttons: mpsc::UnboundedSender<Button>,
    mut state: watch::Receiver<Option<Snapshot>>,
    runtime: tokio::runtime::Handle,
) -> Result<()> {
    // All interfaces and event tokens are created, used and removed on this apartment.
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let _apartment = Apartment;
    let interop = windows::core::factory::<
        SystemMediaTransportControls,
        ISystemMediaTransportControlsInterop,
    >()?;
    let controls: SystemMediaTransportControls =
        unsafe { interop.GetForWindow(HWND(hwnd as *mut _))? };
    controls.SetIsEnabled(false)?;
    controls.SetIsStopEnabled(false)?;
    controls.SetIsRecordEnabled(false)?;
    controls.SetIsFastForwardEnabled(false)?;
    controls.SetIsRewindEnabled(false)?;
    controls.SetIsChannelUpEnabled(false)?;
    controls.SetIsChannelDownEnabled(false)?;
    let token = controls.ButtonPressed(&TypedEventHandler::<
        SystemMediaTransportControls,
        SystemMediaTransportControlsButtonPressedEventArgs,
    >::new(move |_, args| {
        if let Some(button) = args
            .as_ref()
            .and_then(|a| a.Button().ok())
            .and_then(map_button)
        {
            let _ = buttons.send(button);
        }
        Ok(())
    }))?;
    let mut session = Session {
        controls,
        token,
        last: Snapshot::default(),
    };
    loop {
        let snapshot = state.borrow_and_update().clone();
        let Some(snapshot) = snapshot else { break };
        session.update(snapshot)?;
        if runtime.block_on(state.changed()).is_err() {
            break;
        }
    }
    Ok(())
}

impl Session {
    fn update(&mut self, snapshot: Snapshot) -> Result<()> {
        let controls = &self.controls;
        controls.SetIsPlayEnabled(snapshot.allows(Button::Play))?;
        controls.SetIsPauseEnabled(snapshot.allows(Button::Pause))?;
        controls.SetIsNextEnabled(snapshot.allows(Button::Next))?;
        controls.SetIsPreviousEnabled(snapshot.allows(Button::Previous))?;
        let updater = controls.DisplayUpdater()?;
        if !snapshot.enabled {
            controls.SetIsEnabled(false)?;
            controls.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
            updater.ClearAll()?;
            updater.Update()?;
        } else {
            if !self.last.enabled
                || (
                    &snapshot.title,
                    &snapshot.artist,
                    &snapshot.album,
                    &snapshot.cover_url,
                ) != (
                    &self.last.title,
                    &self.last.artist,
                    &self.last.album,
                    &self.last.cover_url,
                )
            {
                updater.ClearAll()?;
                updater.SetType(MediaPlaybackType::Music)?;
                let music = updater.MusicProperties()?;
                music.SetTitle(&HSTRING::from(&snapshot.title))?;
                music.SetArtist(&HSTRING::from(&snapshot.artist))?;
                music.SetAlbumTitle(&HSTRING::from(&snapshot.album))?;
                if !snapshot.cover_url.is_empty() {
                    // Windows loads this reference asynchronously. A bad cover must not disable controls.
                    if let Ok(uri) = Uri::CreateUri(&HSTRING::from(&snapshot.cover_url))
                        && let Ok(image) = RandomAccessStreamReference::CreateFromUri(&uri)
                    {
                        let _ = updater.SetThumbnail(&image);
                    }
                }
                updater.Update()?;
            }
            controls.SetPlaybackStatus(if snapshot.playing {
                MediaPlaybackStatus::Playing
            } else {
                MediaPlaybackStatus::Paused
            })?;
            controls.SetIsEnabled(true)?;
        }
        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        let (position, duration) = if snapshot.enabled {
            snapshot.timeline()
        } else {
            (0, 0)
        };
        timeline.SetStartTime(TimeSpan { Duration: 0 })?;
        timeline.SetMinSeekTime(TimeSpan { Duration: 0 })?;
        timeline.SetEndTime(TimeSpan { Duration: duration })?;
        timeline.SetMaxSeekTime(TimeSpan { Duration: duration })?;
        timeline.SetPosition(TimeSpan { Duration: position })?;
        controls.UpdateTimelineProperties(&timeline)?;
        self.last = snapshot;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_the_supported_transport_buttons() {
        for (native, button) in [
            (SystemMediaTransportControlsButton::Play, Button::Play),
            (SystemMediaTransportControlsButton::Pause, Button::Pause),
            (SystemMediaTransportControlsButton::Next, Button::Next),
            (
                SystemMediaTransportControlsButton::Previous,
                Button::Previous,
            ),
        ] {
            assert_eq!(map_button(native), Some(button));
        }
        assert_eq!(map_button(SystemMediaTransportControlsButton::Stop), None);
        assert_eq!(
            map_button(SystemMediaTransportControlsButton::FastForward),
            None
        );
    }
}
