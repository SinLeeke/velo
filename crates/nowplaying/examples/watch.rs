//! Run with Spotify open: cargo run -p velo-nowplaying --example watch

#[cfg(windows)]
mod console {
    use std::sync::{Mutex, OnceLock};
    use tokio::sync::oneshot;
    use windows::Win32::System::Console::{CTRL_C_EVENT, SetConsoleCtrlHandler};

    static STOP: OnceLock<Mutex<Option<oneshot::Sender<()>>>> = OnceLock::new();

    unsafe extern "system" fn handler(kind: u32) -> windows::core::BOOL {
        if kind != CTRL_C_EVENT {
            return false.into();
        }
        if let Some(sender) = STOP.get().and_then(|stop| stop.lock().ok()?.take()) {
            let _ = sender.send(());
        }
        true.into()
    }

    pub struct CtrlC;
    impl CtrlC {
        pub fn install() -> windows::core::Result<(Self, oneshot::Receiver<()>)> {
            let (tx, rx) = oneshot::channel();
            let _ = STOP.set(Mutex::new(Some(tx)));
            // The static handler and sender outlive the registration.
            unsafe {
                SetConsoleCtrlHandler(Some(handler), true)?;
            }
            Ok((Self, rx))
        }
    }
    impl Drop for CtrlC {
        fn drop(&mut self) {
            unsafe {
                let _ = SetConsoleCtrlHandler(Some(handler), false);
            }
        }
    }
}

#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use velo_domain::{NowPlayingEvent, NowPlayingSource};
    use velo_nowplaying::SmtcSource;
    fn print(event: NowPlayingEvent) {
        match event {
            NowPlayingEvent::TrackChanged {
                title,
                artist,
                album,
                duration_ms,
                thumbnail,
            } => {
                println!(
                    "TrackChanged {{ title: {title:?}, artist: {artist:?}, album: {album:?}, duration_ms: {duration_ms:?}, thumbnail_bytes: {:?} }}",
                    thumbnail.as_ref().map(Vec::len)
                );
            }
            event => println!("{event:?}"),
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter("velo_nowplaying=debug")
        .init();
    let (_handler, mut stop) = console::CtrlC::install()?;
    let source = SmtcSource::new()?;
    let mut events = source.subscribe();
    for event in source.current() {
        print(event);
    }
    println!("Watching Spotify SMTC; press Ctrl+C to stop.");
    loop {
        tokio::select! {
            _ = &mut stop => break,
            event = events.recv() => match event {
                Ok(event) => print(event),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    eprintln!("Skipped {count} events; restoring current state.");
                    for event in source.current() { print(event); }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    drop(source);
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("SMTC is available only on Windows.");
}
