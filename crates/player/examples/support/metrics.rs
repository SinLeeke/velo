//! Two native snapshots, no monitoring thread or periodic sampling.
use std::{io, time::Instant};

pub struct Snapshot {
    sampled_at: Instant,
    cpu_ticks: u64,
    working_set: usize,
    peak_working_set: usize,
}

impl Snapshot {
    pub fn take() -> io::Result<Self> {
        let (cpu_ticks, working_set, peak_working_set) = native::sample()?;
        Ok(Self {
            sampled_at: Instant::now(),
            cpu_ticks,
            working_set,
            peak_working_set,
        })
    }

    pub fn report_until(&self, end: &Self, label: &str) {
        let seconds = end.sampled_at.duration_since(self.sampled_at).as_secs_f64();
        let cpu_seconds = end.cpu_ticks.saturating_sub(self.cpu_ticks) as f64 / 10_000_000.0;
        let one_core_percent = if seconds > 0.0 {
            cpu_seconds / seconds * 100.0
        } else {
            0.0
        };
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        println!(
            "{label} ({seconds:.2}s): working set {:.2} MiB; peak {:.2} MiB; CPU media {:.3}% de un núcleo ({:.3}% / {cores} procesadores disponibles)",
            end.working_set as f64 / 1_048_576.0,
            end.peak_working_set as f64 / 1_048_576.0,
            one_core_percent,
            one_core_percent / cores as f64,
        );
    }
}

#[cfg(windows)]
mod native {
    use std::{ffi::c_void, io, mem::size_of};

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        fn ticks(&self) -> u64 {
            (u64::from(self.high) << 32) | u64::from(self.low)
        }
    }

    // PROCESS_MEMORY_COUNTERS uses DWORD for the first two fields and SIZE_T
    // for every counter. repr(C) includes the required x64 alignment/padding.
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    // These exports are in kernel32 on Windows 7+, including the Windows 11 target.
    // No additional dependency/features or lockfile change for native metrics.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    pub(super) fn sample() -> io::Result<(u64, usize, usize)> {
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        let size = size_of::<ProcessMemoryCounters>() as u32;
        let mut memory = ProcessMemoryCounters {
            cb: size,
            ..Default::default()
        };
        // SAFETY: the current-process pseudo-handle is valid for these read-only
        // APIs and requires no CloseHandle. Every output points to an initialized,
        // properly aligned, uniquely borrowed Win32-compatible structure.
        unsafe {
            let process = GetCurrentProcess();
            if GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) == 0
                || K32GetProcessMemoryInfo(process, &mut memory, size) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok((
            kernel.ticks().saturating_add(user.ticks()),
            memory.working_set_size,
            memory.peak_working_set_size,
        ))
    }
}

#[cfg(not(windows))]
mod native {
    use std::io;
    pub(super) fn sample() -> io::Result<(u64, usize, usize)> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "E1 process metrics target Windows",
        ))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn native_snapshot_reads_current_process_without_a_monitor() {
        let start = Snapshot::take().unwrap();
        let end = Snapshot::take().unwrap();
        assert!(end.working_set > 0);
        assert!(end.peak_working_set >= end.working_set);
        assert!(end.cpu_ticks >= start.cpu_ticks);
    }
}
