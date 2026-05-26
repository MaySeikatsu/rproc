use std::fs;
use std::path::{Path, PathBuf};

use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};

#[derive(Clone, Default, Debug)]
pub struct GpuInfo {
    pub vendor: String,
    pub name: String,
    pub util_pct: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub temp_c: f32,
    pub power_w: f32,
    pub clock_mhz: u32,
    pub mem_clock_mhz: u32,
    pub driver: String,
}

pub struct GpuCollector {
    nvml: Option<Nvml>,
    amd_cards: Vec<PathBuf>,
    intel_cards: Vec<PathBuf>,
}

impl GpuCollector {
    pub fn init() -> Self {
        let nvml = Nvml::init().ok();
        let (amd, intel) = scan_drm();
        Self {
            nvml,
            amd_cards: amd,
            intel_cards: intel,
        }
    }

    pub fn sample(&self) -> Vec<GpuInfo> {
        let mut out = Vec::new();
        if let Some(nvml) = &self.nvml
            && let Ok(count) = nvml.device_count()
        {
            let driver = nvml.sys_driver_version().unwrap_or_default();
            for i in 0..count {
                if let Ok(dev) = nvml.device_by_index(i) {
                    out.push(read_nvml(&dev, &driver));
                }
            }
        }
        for p in &self.amd_cards {
            out.push(read_amd(p));
        }
        for p in &self.intel_cards {
            out.push(read_intel(p));
        }
        out
    }
}

fn scan_drm() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut amd = Vec::new();
    let mut intel = Vec::new();
    let Ok(rd) = fs::read_dir("/sys/class/drm") else {
        return (amd, intel);
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let n = name.to_string_lossy();
        if !n.starts_with("card") || n.contains('-') {
            continue;
        }
        let device = entry.path().join("device");
        let vendor = fs::read_to_string(device.join("vendor")).unwrap_or_default();
        match vendor.trim() {
            "0x1002" => amd.push(device),
            "0x8086" => intel.push(device),
            _ => {}
        }
    }
    (amd, intel)
}

fn read_nvml(dev: &nvml_wrapper::Device, driver: &str) -> GpuInfo {
    let name = dev.name().unwrap_or_else(|_| "NVIDIA GPU".into());
    let util = dev.utilization_rates().map(|u| u.gpu as f32).unwrap_or(0.0);
    let mem = dev.memory_info().ok();
    let temp = dev.temperature(TemperatureSensor::Gpu).unwrap_or(0) as f32;
    let power = dev.power_usage().map(|p| p as f32 / 1000.0).unwrap_or(0.0);
    let clock = dev.clock_info(Clock::Graphics).unwrap_or(0);
    let mem_clock = dev.clock_info(Clock::Memory).unwrap_or(0);
    GpuInfo {
        vendor: "NVIDIA".into(),
        name,
        util_pct: util,
        mem_used: mem.as_ref().map(|m| m.used).unwrap_or(0),
        mem_total: mem.map(|m| m.total).unwrap_or(0),
        temp_c: temp,
        power_w: power,
        clock_mhz: clock,
        mem_clock_mhz: mem_clock,
        driver: driver.to_string(),
    }
}

fn read_file_u64(p: &PathBuf) -> Option<u64> {
    fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn read_file_f32(p: &PathBuf) -> Option<f32> {
    fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn read_amd(device: &Path) -> GpuInfo {
    let util = read_file_f32(&device.join("gpu_busy_percent")).unwrap_or(0.0);
    let mem_used = read_file_u64(&device.join("mem_info_vram_used")).unwrap_or(0);
    let mem_total = read_file_u64(&device.join("mem_info_vram_total")).unwrap_or(0);
    // hwmon temp (in millidegC) — look for any hwmon entry under device/hwmon/*/temp1_input
    let mut temp_c = 0.0;
    if let Ok(rd) = fs::read_dir(device.join("hwmon")) {
        for entry in rd.flatten() {
            if let Some(v) = read_file_f32(&entry.path().join("temp1_input")) {
                temp_c = v / 1000.0;
                break;
            }
        }
    }
    GpuInfo {
        vendor: "AMD".into(),
        name: pci_model(device).unwrap_or_else(|| "AMD GPU".into()),
        util_pct: util,
        mem_used,
        mem_total,
        temp_c,
        power_w: 0.0,
        clock_mhz: 0,
        mem_clock_mhz: 0,
        driver: "amdgpu".into(),
    }
}

fn read_intel(device: &Path) -> GpuInfo {
    // Intel sysfs exposes frequency but not utilization (without intel_gpu_top).
    let cur_freq = read_file_u64(&device.join("gt_act_freq_mhz"))
        .or_else(|| read_file_u64(&device.join("gt/gt0/rps_act_freq_mhz")))
        .unwrap_or(0) as u32;
    let mut temp_c = 0.0;
    if let Ok(rd) = fs::read_dir(device.join("hwmon")) {
        for entry in rd.flatten() {
            if let Some(v) = read_file_f32(&entry.path().join("temp1_input")) {
                temp_c = v / 1000.0;
                break;
            }
        }
    }
    GpuInfo {
        vendor: "Intel".into(),
        name: pci_model(device).unwrap_or_else(|| "Intel GPU".into()),
        util_pct: 0.0,
        mem_used: 0,
        mem_total: 0,
        temp_c,
        power_w: 0.0,
        clock_mhz: cur_freq,
        mem_clock_mhz: 0,
        driver: "i915/xe".into(),
    }
}

fn pci_model(device: &Path) -> Option<String> {
    // Try modalias / device / vendor id labels — fall back to drm card name.
    if let Ok(label) = fs::read_to_string(device.join("label")) {
        let t = label.trim();
        if !t.is_empty() {
            return Some(t.into());
        }
    }
    device
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
}
