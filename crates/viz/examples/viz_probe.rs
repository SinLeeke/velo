//! `cargo run -p velo-viz --example viz_probe -- [--pid 1234]`
//! For representative CPU numbers, add `--release` to the cargo command.
use std::time::{Duration, Instant};
use velo_domain::Visualizer;
use velo_viz::VizEngine;

#[cfg(windows)]
mod platform {
    use std::collections::HashSet;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    struct Snapshot(HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub fn spotify_pid() -> windows_core::Result<Option<u32>> {
        let snapshot = Snapshot(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? });
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut processes = Vec::new();
        unsafe {
            Process32FirstW(snapshot.0, &mut entry)?;
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                if String::from_utf16_lossy(&entry.szExeFile[..end])
                    .eq_ignore_ascii_case("Spotify.exe")
                {
                    processes.push((entry.th32ProcessID, entry.th32ParentProcessID));
                }
                if Process32NextW(snapshot.0, &mut entry).is_err() {
                    break;
                }
            }
        }
        let ids: HashSet<_> = processes.iter().map(|(pid, _)| *pid).collect();
        Ok(processes
            .into_iter()
            .find(|(_, parent)| !ids.contains(parent))
            .map(|(pid, _)| pid))
    }

    pub fn cpu_seconds() -> windows_core::Result<f64> {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )?;
        }
        let ticks =
            |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        Ok((ticks(kernel) + ticks(user)) as f64 / 10_000_000.0)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("velo_viz=warn")
        .try_init();
    let mut args = std::env::args().skip(1);
    let pid = match args.next().as_deref() {
        Some("--pid") => Some(args.next().ok_or("--pid requires a PID")?.parse::<u32>()?),
        None => {
            #[cfg(windows)]
            {
                platform::spotify_pid()?
            }
            #[cfg(not(windows))]
            {
                None
            }
        }
        _ => return Err("usage: viz_probe [--pid <Spotify PID>]".into()),
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    println!("Spotify PID: {pid:?}");
    if pid.is_none() {
        println!("Spotify no encontrado: modo SIMULADO (sin captura de audio).");
    }
    #[cfg(windows)]
    let cpu_start = platform::cpu_seconds()?;
    let start = Instant::now();
    let engine = VizEngine::new();
    let bands = engine.bands();
    engine.set_active(true, pid);
    const LEVELS: &[u8] = b" .:-=+*#%@";
    for tick in 1..=32 {
        std::thread::sleep(
            (start + Duration::from_millis(tick * 250)).saturating_duration_since(Instant::now()),
        );
        let line: String = bands
            .borrow()
            .iter()
            .map(|value| {
                LEVELS[(value.clamp(0.0, 1.0) * (LEVELS.len() - 1) as f32).round() as usize] as char
            })
            .collect();
        let mode = if engine.is_simulated() {
            "SIMULADO"
        } else {
            "REAL Spotify"
        };
        println!("{mode:12} |{line}|");
    }
    engine.set_active(false, None);
    drop(engine);
    let elapsed = start.elapsed().as_secs_f64();
    #[cfg(windows)]
    {
        let cpu = platform::cpu_seconds()? - cpu_start;
        println!(
            "CPU proceso: {cpu:.4} s / {elapsed:.3} s reales = {:.3}% de un nucleo",
            cpu / elapsed * 100.0
        );
    }
    #[cfg(not(windows))]
    println!("Tiempo real: {elapsed:.3} s; GetProcessTimes solo disponible en Windows.");
    Ok(())
}
