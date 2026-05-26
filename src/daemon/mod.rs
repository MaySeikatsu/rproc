//! Headless background sampler that persists a 60-second rolling
//! window of system metrics so the GUI can show recent history the
//! moment it opens — even after a full restart.
//!
//! Lifecycle:
//! - `rproc --daemon` is the explicit entry point (systemd, manual launch).
//! - `spawn_if_absent()` is what the GUI calls on startup: it forks the
//!   current binary with `--daemon`, detaches it via `setsid(2)`, and
//!   lets the new process orphan-adopt onto PID 1. Closing the GUI then
//!   leaves the daemon untouched.

pub mod pidfile;
pub mod storage;

use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sysinfo::{Disks, MemoryRefreshKind, Networks, System};

use crate::monitor::{gpu, system as msystem};
use storage::{RingBuffer, Sample};

/// Fixed at 1 s so `CAPACITY = 60` samples means literally the last
/// 60 seconds, regardless of how the GUI is sampling.
const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

pub fn run() -> anyhow::Result<()> {
    let pid_path = pidfile::pid_path()?;
    let _lock = match pidfile::PidFile::acquire(&pid_path)? {
        Some(lock) => lock,
        // Already running — exit silently so duplicate spawns are a no-op.
        None => return Ok(()),
    };

    let hist_path = storage::history_path()?;
    let mut ring = RingBuffer::open_writer(&hist_path)?;

    let mut sys = System::new_all();
    let mut nets = Networks::new_with_refreshed_list();
    let mut disks = Disks::new_with_refreshed_list();
    let gpu_collector = gpu::GpuCollector::init();

    // sysinfo CPU usage needs two refreshes spaced apart to compute deltas.
    sys.refresh_cpu_usage();
    thread::sleep(Duration::from_millis(250));
    let mut last_refresh = Instant::now();

    loop {
        let now = Instant::now();
        let delta_secs = now.duration_since(last_refresh).as_secs_f64();
        last_refresh = now;

        sys.refresh_cpu_usage();
        sys.refresh_memory_specifics(MemoryRefreshKind::everything());
        nets.refresh(true);
        disks.refresh(true);

        let summary = msystem::SystemSummary::collect(&sys, &nets, &disks, delta_secs);
        let gpus = gpu_collector.sample();

        let disk_read: f64 = summary.disks.iter().map(|d| d.read_bps).sum();
        let disk_write: f64 = summary.disks.iter().map(|d| d.write_bps).sum();
        let gpu_util = gpus.first().map(|g| g.util_pct).unwrap_or(f32::NAN);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let sample = Sample {
            timestamp_secs: timestamp,
            cpu_total: summary.cpu_total,
            ram_used_pct: summary.ram_used_pct,
            net_rx_bps: summary.net_rx_bps as f32,
            net_tx_bps: summary.net_tx_bps as f32,
            disk_read_bps: disk_read as f32,
            disk_write_bps: disk_write as f32,
            gpu_util_pct: gpu_util,
        };

        if let Err(e) = ring.append(&sample) {
            eprintln!("rprocd: failed to append sample: {e}");
        }

        let elapsed = now.elapsed();
        if elapsed < SAMPLE_PERIOD {
            thread::sleep(SAMPLE_PERIOD - elapsed);
        }
    }
}

/// Spawn `rproc --daemon` as a detached background process if none is
/// running. Best-effort: any failure is logged to stderr but doesn't
/// block the GUI from starting.
pub fn spawn_if_absent() {
    let pid_path = match pidfile::pid_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("rproc: cache dir unavailable, skipping background sampler: {e}");
            return;
        }
    };
    if pidfile::PidFile::is_locked(&pid_path) {
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("rproc: cannot locate current_exe, skipping background sampler: {e}");
            return;
        }
    };

    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let spawn = unsafe {
        Command::new(exe)
            .arg("--daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Move the child into its own session so it survives the
            // GUI exiting and isn't reached by any SIGHUP propagated
            // from the launching terminal. setsid is async-signal-safe.
            .pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            })
            .spawn()
    };

    if let Err(e) = spawn {
        eprintln!("rproc: failed to spawn background sampler: {e}");
    }
}
