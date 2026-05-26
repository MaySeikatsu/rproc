//! On-disk ring-buffer for the background sampler.
//!
//! Layout (little-endian, total size = 20 + 60 * 36 = 2180 bytes):
//!
//! ```text
//! Header (20 bytes)
//!   [0..4]   magic = b"RPRC"
//!   [4]      version = 1
//!   [5..8]   padding
//!   [8..12]  capacity (u32)            — sample count, fixed at 60
//!   [12..16] write_pos (u32)           — index of the next slot to write
//!   [16..20] count (u32, ≤ capacity)   — number of valid samples
//!
//! Samples (60 × 36 bytes), each:
//!   [0..8]   timestamp_secs (u64, unix epoch)
//!   [8..12]  cpu_total (f32, %)
//!   [12..16] ram_used_pct (f32, %)
//!   [16..20] net_rx_bps (f32, B/s)
//!   [20..24] net_tx_bps (f32, B/s)
//!   [24..28] disk_read_bps (f32, B/s)
//!   [28..32] disk_write_bps (f32, B/s)
//!   [32..36] gpu_util_pct (f32, % — NaN when no GPU is present)
//! ```
//!
//! The format is intentionally trivial: no allocations on the hot path,
//! one `write_all` per sample plus a header update, and any older or
//! corrupt file is reinitialised silently rather than aborting.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: [u8; 4] = *b"RPRC";
const VERSION: u8 = 1;
pub const CAPACITY: u32 = 60;
const HEADER_SIZE: u64 = 20;
const SAMPLE_SIZE: u64 = 36;
const FILE_SIZE: u64 = HEADER_SIZE + SAMPLE_SIZE * CAPACITY as u64;

#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub timestamp_secs: u64,
    pub cpu_total: f32,
    pub ram_used_pct: f32,
    pub net_rx_bps: f32,
    pub net_tx_bps: f32,
    pub disk_read_bps: f32,
    pub disk_write_bps: f32,
    pub gpu_util_pct: f32,
}

impl Sample {
    fn to_bytes(self) -> [u8; SAMPLE_SIZE as usize] {
        let mut b = [0u8; SAMPLE_SIZE as usize];
        b[0..8].copy_from_slice(&self.timestamp_secs.to_le_bytes());
        b[8..12].copy_from_slice(&self.cpu_total.to_le_bytes());
        b[12..16].copy_from_slice(&self.ram_used_pct.to_le_bytes());
        b[16..20].copy_from_slice(&self.net_rx_bps.to_le_bytes());
        b[20..24].copy_from_slice(&self.net_tx_bps.to_le_bytes());
        b[24..28].copy_from_slice(&self.disk_read_bps.to_le_bytes());
        b[28..32].copy_from_slice(&self.disk_write_bps.to_le_bytes());
        b[32..36].copy_from_slice(&self.gpu_util_pct.to_le_bytes());
        b
    }

    fn from_bytes(b: &[u8; SAMPLE_SIZE as usize]) -> Self {
        Self {
            timestamp_secs: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            cpu_total: f32::from_le_bytes(b[8..12].try_into().unwrap()),
            ram_used_pct: f32::from_le_bytes(b[12..16].try_into().unwrap()),
            net_rx_bps: f32::from_le_bytes(b[16..20].try_into().unwrap()),
            net_tx_bps: f32::from_le_bytes(b[20..24].try_into().unwrap()),
            disk_read_bps: f32::from_le_bytes(b[24..28].try_into().unwrap()),
            disk_write_bps: f32::from_le_bytes(b[28..32].try_into().unwrap()),
            gpu_util_pct: f32::from_le_bytes(b[32..36].try_into().unwrap()),
        }
    }
}

pub fn cache_dir() -> io::Result<PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".cache"))
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no HOME or XDG_CACHE_HOME"))?;
    let dir = base.join("rproc");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn history_path() -> io::Result<PathBuf> {
    Ok(cache_dir()?.join("history.bin"))
}

pub struct RingBuffer {
    file: File,
    write_pos: u32,
    count: u32,
}

impl RingBuffer {
    /// Open the ring-buffer for writing. Initialises a fresh file if
    /// missing, truncated, or written by an incompatible version.
    pub fn open_writer(path: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let valid = file.metadata()?.len() == FILE_SIZE && {
            let mut header = [0u8; HEADER_SIZE as usize];
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut header).is_ok()
                && header[0..4] == MAGIC
                && header[4] == VERSION
                && u32::from_le_bytes(header[8..12].try_into().unwrap()) == CAPACITY
        };

        if !valid {
            return Self::reinit(file);
        }

        let mut header = [0u8; HEADER_SIZE as usize];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header)?;
        let write_pos = u32::from_le_bytes(header[12..16].try_into().unwrap()) % CAPACITY;
        let count = u32::from_le_bytes(header[16..20].try_into().unwrap()).min(CAPACITY);
        Ok(Self {
            file,
            write_pos,
            count,
        })
    }

    fn reinit(mut file: File) -> io::Result<Self> {
        file.set_len(FILE_SIZE)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&MAGIC)?;
        file.write_all(&[VERSION, 0, 0, 0])?;
        file.write_all(&CAPACITY.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        // Zero the sample area so a freshly-(re)initialised file never
        // returns stale garbage to a reader inspecting raw slots.
        let zeros = [0u8; SAMPLE_SIZE as usize];
        for _ in 0..CAPACITY {
            file.write_all(&zeros)?;
        }
        file.flush()?;
        Ok(Self {
            file,
            write_pos: 0,
            count: 0,
        })
    }

    pub fn append(&mut self, s: &Sample) -> io::Result<()> {
        let offset = HEADER_SIZE + SAMPLE_SIZE * self.write_pos as u64;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(&s.to_bytes())?;

        self.write_pos = (self.write_pos + 1) % CAPACITY;
        if self.count < CAPACITY {
            self.count += 1;
        }
        // Header update is intentionally not atomic with the slot write —
        // a crash mid-append at worst leaves a slot whose contents the
        // header still claims is empty, which a reader will ignore.
        self.file.seek(SeekFrom::Start(12))?;
        self.file.write_all(&self.write_pos.to_le_bytes())?;
        self.file.write_all(&self.count.to_le_bytes())?;
        self.file.flush()?;
        Ok(())
    }

    /// Read all valid samples in chronological order (oldest → newest).
    /// Returns an empty vector if the file is missing, the wrong size,
    /// or has a header from an incompatible version — callers can treat
    /// "no history" and "corrupt history" identically.
    pub fn read_all(path: &Path) -> io::Result<Vec<Sample>> {
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        if file.metadata()?.len() != FILE_SIZE {
            return Ok(Vec::new());
        }
        let mut header = [0u8; HEADER_SIZE as usize];
        file.read_exact(&mut header)?;
        if header[0..4] != MAGIC
            || header[4] != VERSION
            || u32::from_le_bytes(header[8..12].try_into().unwrap()) != CAPACITY
        {
            return Ok(Vec::new());
        }
        let write_pos = u32::from_le_bytes(header[12..16].try_into().unwrap()) % CAPACITY;
        let count = u32::from_le_bytes(header[16..20].try_into().unwrap()).min(CAPACITY);
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut slots = vec![[0u8; SAMPLE_SIZE as usize]; CAPACITY as usize];
        for slot in slots.iter_mut() {
            file.read_exact(slot)?;
        }
        // When the buffer has wrapped, the oldest sample sits at write_pos
        // (the slot we're about to overwrite next); when it hasn't, the
        // oldest is at index 0.
        let start = if count == CAPACITY { write_pos } else { 0 };
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let idx = ((start + i) % CAPACITY) as usize;
            out.push(Sample::from_bytes(&slots[idx]));
        }
        Ok(out)
    }
}
