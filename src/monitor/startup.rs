use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Default, PartialEq, Debug)]
pub enum StartupSource {
    #[default]
    UserAutostart,
    SystemAutostart,
    SystemdSystem,
    SystemdUser,
}

#[derive(Clone, Default)]
pub struct StartupEntry {
    pub source: StartupSource,
    pub path: PathBuf,
    pub name: String,
    pub exec: String,
    pub comment: String,
    pub icon: String,
    pub enabled: bool,
    pub boot_time_ms: Option<u64>,
    pub critical: bool,
}

pub fn collect() -> Vec<StartupEntry> {
    let mut out = Vec::new();

    if let Ok(home) = std::env::var("HOME") {
        out.extend(scan_dir(
            PathBuf::from(format!("{home}/.config/autostart")),
            StartupSource::UserAutostart,
        ));
    }
    out.extend(scan_dir(
        PathBuf::from("/etc/xdg/autostart"),
        StartupSource::SystemAutostart,
    ));

    let protected_system = protected_units(false);
    let protected_user = protected_units(true);

    out.extend(collect_systemd(false, &protected_system));
    out.extend(collect_systemd(true, &protected_user));

    // Sort: critical first (so user sees what's protected), then by descending boot time,
    // then entries without a measured time at the end, alphabetically.
    out.sort_by(|a, b| {
        b.critical
            .cmp(&a.critical)
            .then_with(|| match (a.boot_time_ms, b.boot_time_ms) {
                (Some(x), Some(y)) => y.cmp(&x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            })
    });
    out
}

fn scan_dir(dir: PathBuf, source: StartupSource) -> Vec<StartupEntry> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(&dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        if let Some(e) = parse_desktop(&path, source.clone()) {
            out.push(e);
        }
    }
    out
}

fn parse_desktop(path: &PathBuf, source: StartupSource) -> Option<StartupEntry> {
    let content = fs::read_to_string(path).ok()?;
    let mut kv: HashMap<String, String> = HashMap::new();
    let mut in_main = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.insert(k.trim().into(), v.trim().into());
        }
    }
    let hidden = kv.get("Hidden").map(|s| s == "true").unwrap_or(false);
    let xdg_autostart_enabled = kv
        .get("X-GNOME-Autostart-enabled")
        .map(|s| s == "true")
        .unwrap_or(true);
    Some(StartupEntry {
        path: path.clone(),
        source,
        name: kv.get("Name").cloned().unwrap_or_default(),
        exec: kv.get("Exec").cloned().unwrap_or_default(),
        comment: kv.get("Comment").cloned().unwrap_or_default(),
        icon: kv.get("Icon").cloned().unwrap_or_default(),
        enabled: !hidden && xdg_autostart_enabled,
        boot_time_ms: None,
        critical: false,
    })
}

/// Parse `systemd-analyze [--user] blame` and return one entry per service.
fn collect_systemd(user: bool, protected: &HashSet<String>) -> Vec<StartupEntry> {
    let mut cmd = Command::new("systemd-analyze");
    if user {
        cmd.arg("--user");
    }
    cmd.args(["blame", "--no-pager"]);
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let source = if user {
        StartupSource::SystemdUser
    } else {
        StartupSource::SystemdSystem
    };
    let mut entries = Vec::new();
    let mut units: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Format: "<duration> <unit>"
        let Some((dur_str, unit)) = line.rsplit_once(' ') else {
            continue;
        };
        let unit = unit.trim().to_string();
        // Only list services — targets/sockets/mounts are infrastructure that
        // users shouldn't toggle from a "startup apps" screen.
        if !unit.ends_with(".service") {
            continue;
        }
        let Some(ms) = parse_duration_to_ms(dur_str.trim()) else {
            continue;
        };
        let is_protected = protected.contains(&unit);
        units.push(unit.clone());
        entries.push(StartupEntry {
            source: source.clone(),
            path: PathBuf::from(&unit),
            name: unit.trim_end_matches(".service").to_string(),
            exec: unit.clone(),
            comment: String::new(),
            icon: String::new(),
            enabled: true,
            boot_time_ms: Some(ms),
            critical: is_protected,
        });
    }
    let descriptions = fetch_descriptions(user, &units);
    for e in &mut entries {
        if let Some(d) = descriptions.get(&e.exec) {
            e.comment = d.clone();
        }
    }
    entries
}

/// Batch-fetch unit descriptions via `systemctl show --property=Id,Description`.
/// Returns a map keyed by the unit name (`Id`).
fn fetch_descriptions(user: bool, units: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if units.is_empty() {
        return map;
    }
    let mut cmd = Command::new("systemctl");
    if user {
        cmd.arg("--user");
    }
    cmd.args(["show", "--property=Id", "--property=Description", "--"]);
    cmd.args(units);
    let Ok(out) = cmd.output() else { return map };
    if !out.status.success() {
        return map;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // `systemctl show` separates units with a blank line; properties are KEY=VALUE.
    let mut id = String::new();
    let mut desc = String::new();
    for line in text.lines() {
        if line.is_empty() {
            if !id.is_empty() {
                map.insert(std::mem::take(&mut id), std::mem::take(&mut desc));
            } else {
                desc.clear();
            }
            continue;
        }
        if let Some(v) = line.strip_prefix("Id=") {
            id = v.to_string();
        } else if let Some(v) = line.strip_prefix("Description=") {
            desc = v.to_string();
        }
    }
    if !id.is_empty() {
        map.insert(id, desc);
    }
    map
}

/// Set of service units that the user cannot meaningfully disable: those whose
/// unit-file state is `static`, `generated`, or `alias`. These are pulled in by
/// dependency, not by enable symlinks, so `systemctl disable` is a no-op or
/// refused. This is the right proxy for "do not disable" — unlike
/// `systemd-analyze critical-chain`, which reports the slowest path to the
/// default target (a performance view, not a safety view).
fn protected_units(user: bool) -> HashSet<String> {
    let mut cmd = Command::new("systemctl");
    if user {
        cmd.arg("--user");
    }
    cmd.args([
        "list-unit-files",
        "--type=service",
        "--no-legend",
        "--no-pager",
        "--plain",
    ]);
    let Ok(out) = cmd.output() else {
        return HashSet::new();
    };
    if !out.status.success() {
        return HashSet::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut set = HashSet::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(unit) = parts.next() else { continue };
        let Some(state) = parts.next() else { continue };
        if matches!(state, "static" | "generated" | "alias") {
            set.insert(unit.to_string());
        }
    }
    set
}

/// Parse systemd-analyze duration strings: "12ms", "1.234s", "1min 2.345s", "2h 3min 4s".
fn parse_duration_to_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total: f64 = 0.0;
    let mut matched_any = false;
    let mut rest = s;
    while !rest.is_empty() {
        rest = rest.trim_start();
        let num_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if num_end == 0 {
            return None;
        }
        let num: f64 = rest[..num_end].parse().ok()?;
        let after = &rest[num_end..];
        let unit_end = after
            .find(|c: char| c.is_ascii_digit() || c.is_whitespace())
            .unwrap_or(after.len());
        let unit = &after[..unit_end];
        let factor_ms = match unit {
            "ms" => 1.0,
            "s" => 1_000.0,
            "min" => 60_000.0,
            "h" => 3_600_000.0,
            "d" => 86_400_000.0,
            _ => {
                return if matched_any {
                    Some(total as u64)
                } else {
                    None
                };
            }
        };
        total += num * factor_ms;
        matched_any = true;
        rest = &after[unit_end..];
    }
    Some(total as u64)
}

/// Toggle an autostart entry. Returns Err for critical systemd units (cannot be disabled).
pub fn set_enabled(entry: &StartupEntry, enabled: bool) -> Result<(), String> {
    if entry.critical {
        return Err(
            "This unit is managed by systemd dependencies (static/generated) and cannot be disabled directly."
                .into(),
        );
    }
    match entry.source {
        StartupSource::UserAutostart | StartupSource::SystemAutostart => {
            set_enabled_desktop(entry, enabled).map_err(|e| e.to_string())
        }
        StartupSource::SystemdSystem | StartupSource::SystemdUser => {
            set_enabled_systemd(entry, enabled)
        }
    }
}

fn set_enabled_desktop(entry: &StartupEntry, enabled: bool) -> std::io::Result<()> {
    let content = fs::read_to_string(&entry.path)?;

    let mut new_lines: Vec<String> = Vec::new();
    let mut in_main = false;
    let mut wrote_hidden = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_main && !wrote_hidden {
                new_lines.push(format!("Hidden={}", !enabled));
                wrote_hidden = true;
            }
            in_main = trimmed == "[Desktop Entry]";
            new_lines.push(line.to_string());
            continue;
        }
        if in_main && trimmed.starts_with("Hidden=") {
            new_lines.push(format!("Hidden={}", !enabled));
            wrote_hidden = true;
        } else if in_main && trimmed.starts_with("X-GNOME-Autostart-enabled=") {
            new_lines.push(format!("X-GNOME-Autostart-enabled={enabled}"));
        } else {
            new_lines.push(line.to_string());
        }
    }
    if in_main && !wrote_hidden {
        new_lines.push(format!("Hidden={}", !enabled));
    }

    let target = if entry.source == StartupSource::SystemAutostart {
        let home = std::env::var("HOME")
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "no HOME"))?;
        let user_dir = PathBuf::from(format!("{home}/.config/autostart"));
        fs::create_dir_all(&user_dir)?;
        user_dir.join(
            entry
                .path
                .file_name()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no name"))?,
        )
    } else {
        entry.path.clone()
    };
    fs::write(&target, new_lines.join("\n"))
}

fn set_enabled_systemd(entry: &StartupEntry, enabled: bool) -> Result<(), String> {
    let mut cmd = Command::new("systemctl");
    if entry.source == StartupSource::SystemdUser {
        cmd.arg("--user");
    }
    cmd.args([if enabled { "enable" } else { "disable" }, &entry.exec]);
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}
