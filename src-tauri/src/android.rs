//! Android SDK / AVD controller.
//!
//! Talks to the SDK's CLI binaries (`adb.exe`, `emulator.exe`) with async
//! commands and hard timeouts, running them with `CREATE_NO_WINDOW` so no
//! rogue console windows appear. Running emulator instances (console ports
//! 5554–5584) are matched against AVD profiles via `adb devices` +
//! `adb -s <serial> emu avd name`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::process::Command;

/// Windows: run background CLI tools without a visible console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Console ports of running emulators are even numbers in 5554..=5584.
const EMULATOR_CONSOLE_MIN: u16 = 5554;
const EMULATOR_CONSOLE_MAX: u16 = 5584;

/// Re-list AVDs at most every ~15 s; reuse the cached list in between.
const AVD_CACHE_TTL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvdEntry {
    pub name: String,
    pub running: bool,
    /// adb serial (`emulator-5554`) when a running instance is attached.
    pub serial: Option<String>,
    /// "stopped" | "offline" | "booting" | "ready" | "unauthorized"
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorsState {
    pub sdk_path: Option<String>,
    pub sdk_error: Option<String>,
    pub avds: Vec<AvdEntry>,
}

/// Located Android SDK binaries.
#[derive(Debug, Clone)]
pub struct Sdk {
    pub adb: PathBuf,
    pub emulator: PathBuf,
}

impl Sdk {
    /// SDK root directory (`<root>\platform-tools\adb.exe` → `<root>`).
    pub fn root(&self) -> &Path {
        self.adb
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| self.adb.parent().unwrap_or(Path::new("")))
    }
}

/// Locates the Android SDK: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, then the
/// default per-user install path (`%LOCALAPPDATA%\Android\Sdk`).
pub fn discover_sdk() -> Result<Sdk, String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("ANDROID_HOME") {
        roots.push(PathBuf::from(home));
    }
    if let Ok(root) = std::env::var("ANDROID_SDK_ROOT") {
        roots.push(PathBuf::from(root));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Android").join("Sdk"));
    }

    for root in &roots {
        let adb = root.join("platform-tools").join("adb.exe");
        let emulator = root.join("emulator").join("emulator.exe");
        if adb.is_file() && emulator.is_file() {
            return Ok(Sdk { adb, emulator });
        }
    }

    Err(format!(
        "Android SDK not found — set ANDROID_HOME (checked: {})",
        roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Runs a CLI tool with `CREATE_NO_WINDOW`, a hard timeout and kill-on-drop
/// (stray tools never linger), returning trimmed stdout.
async fn run_tool(exe: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);

    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("'{}' timed out after {timeout:?}", exe.display()))?
        .map_err(|e| format!("failed to run '{}': {e}", exe.display()))?;

    if !out.status.success() && out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("'{}' exited with {}", exe.display(), out.status)
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// `emulator -list-avds` — one AVD name per line.
pub async fn list_avds(sdk: &Sdk) -> Result<Vec<String>, String> {
    let out = run_tool(&sdk.emulator, &["-list-avds"], Duration::from_secs(10)).await?;
    Ok(out
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// `adb devices` — `(serial, state)` pairs, header noise skipped.
pub async fn adb_devices(sdk: &Sdk) -> Result<Vec<(String, String)>, String> {
    let out = run_tool(&sdk.adb, &["devices"], Duration::from_secs(8)).await?;
    let mut devices = Vec::new();
    for line in out.lines().skip(1) {
        let t = line.trim();
        if t.is_empty() || t.starts_with("List of") || t.starts_with('*') {
            continue;
        }
        let mut parts = t.split_whitespace();
        if let (Some(serial), Some(state)) = (parts.next(), parts.next()) {
            devices.push((serial.to_string(), state.to_string()));
        }
    }
    Ok(devices)
}

/// `adb -s <serial> emu avd name` — first output line is the AVD profile name.
/// This is how a running emulator instance is matched to its AVD.
pub async fn avd_name_for_serial(sdk: &Sdk, serial: &str) -> Option<String> {
    let out = run_tool(
        &sdk.adb,
        &["-s", serial, "emu", "avd", "name"],
        Duration::from_secs(5),
    )
    .await
    .ok()?;
    out.lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `adb -s <serial> shell getprop sys.boot_completed` — "1" once booted.
async fn boot_completed(sdk: &Sdk, serial: &str) -> Option<bool> {
    let out = run_tool(
        &sdk.adb,
        &["-s", serial, "shell", "getprop", "sys.boot_completed"],
        Duration::from_secs(5),
    )
    .await
    .ok()?;
    Some(out.trim() == "1")
}

/// Launches an AVD detached in the background (`emulator -avd <name>`); with
/// `cold` it skips the saved snapshot (`-no-snapshot-load`).
pub fn launch_avd(sdk: &Sdk, name: &str, cold: bool) -> Result<(), String> {
    let mut args = vec!["-avd".to_string(), name.to_string()];
    if cold {
        args.push("-no-snapshot-load".to_string());
    }
    crate::processctl::spawn_hidden(&sdk.emulator.to_string_lossy(), &args, None)
}

/// Gracefully terminates a running emulator: `adb -s <serial> emu kill`.
pub async fn kill_avd(sdk: &Sdk, serial: &str) -> Result<(), String> {
    run_tool(
        &sdk.adb,
        &["-s", serial, "emu", "kill"],
        Duration::from_secs(5),
    )
    .await
    .map(|_| ())
}

/// Builds the emulators portion of the snapshot: SDK discovery, AVD listing
/// (cached ~15 s), running-instance matching and boot status.
pub async fn refresh_emulators(
    listen_ports: &[u16],
    cache: &mut Option<(Instant, Vec<String>)>,
) -> EmulatorsState {
    let sdk = match discover_sdk() {
        Ok(s) => s,
        Err(e) => {
            return EmulatorsState {
                sdk_error: Some(e),
                ..Default::default()
            }
        }
    };
    let sdk_path = sdk.root().to_string_lossy().to_string();

    // Console ports of any running emulators — used as a fallback when adb
    // itself is unusable.
    let console_ports: Vec<u16> = listen_ports
        .iter()
        .copied()
        .filter(|p| *p >= EMULATOR_CONSOLE_MIN && *p <= EMULATOR_CONSOLE_MAX && p % 2 == 0)
        .collect();

    // Re-list AVDs at most every AVD_CACHE_TTL.
    let avd_names: Vec<String> = match cache {
        Some((at, names)) if at.elapsed() < AVD_CACHE_TTL => names.clone(),
        _ => match list_avds(&sdk).await {
            Ok(names) => {
                *cache = Some((Instant::now(), names.clone()));
                names
            }
            Err(e) => {
                // Surface the error, but still try to report running devices.
                return finish_with_error(sdk, sdk_path, &console_ports, e).await;
            }
        },
    };

    // Running emulator devices: prefer adb's view, fall back to console ports.
    let mut serial_state: HashMap<String, String> = HashMap::new();
    match adb_devices(&sdk).await {
        Ok(devices) => {
            for (serial, state) in devices {
                if serial.starts_with("emulator-") {
                    serial_state.insert(serial, state);
                }
            }
        }
        Err(_) => {
            for p in &console_ports {
                serial_state.insert(format!("emulator-{p}"), "device".to_string());
            }
        }
    }

    // Resolve serial → AVD profile name once per running device.
    let mut serial_to_avd: HashMap<String, Option<String>> = HashMap::new();
    for serial in serial_state.keys() {
        let name = avd_name_for_serial(&sdk, serial).await;
        serial_to_avd.insert(serial.clone(), name);
    }

    let mut claimed: HashSet<String> = HashSet::new();
    let mut entries: Vec<AvdEntry> = Vec::new();

    // Known AVDs first, with their running instance (if any) attached.
    for name in &avd_names {
        let mut serial: Option<String> = None;
        for (s, avd) in &serial_to_avd {
            if avd.as_deref() == Some(name.as_str()) && !claimed.contains(s) {
                serial = Some(s.clone());
                break;
            }
        }
        if let Some(s) = &serial {
            claimed.insert(s.clone());
        }
        let status = match &serial {
            None => "stopped".to_string(),
            Some(s) => {
                status_for(
                    serial_state.get(s).map(String::as_str).unwrap_or("device"),
                    s,
                    &sdk,
                )
                .await
            }
        };
        entries.push(AvdEntry {
            name: name.clone(),
            running: serial.is_some(),
            serial,
            status,
        });
    }

    // Orphan running instances (deleted/renamed AVDs) keep their serial as label.
    for (s, avd) in &serial_to_avd {
        if claimed.contains(s) {
            continue;
        }
        let status = status_for(
            serial_state.get(s).map(String::as_str).unwrap_or("device"),
            s,
            &sdk,
        )
        .await;
        let label = avd.clone().unwrap_or_else(|| format!("{s} (unknown AVD)"));
        entries.push(AvdEntry {
            name: label,
            running: true,
            serial: Some(s.clone()),
            status,
        });
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    EmulatorsState {
        sdk_path: Some(sdk_path),
        sdk_error: None,
        avds: entries,
    }
}

/// Error variant that still reports any running emulator devices.
async fn finish_with_error(
    sdk: Sdk,
    sdk_path: String,
    console_ports: &[u16],
    error: String,
) -> EmulatorsState {
    let mut avds = Vec::new();
    match adb_devices(&sdk).await {
        Ok(devices) => {
            for (serial, state) in devices {
                if !serial.starts_with("emulator-") {
                    continue;
                }
                let status = match state.as_str() {
                    "offline" => "offline".to_string(),
                    "unauthorized" => "unauthorized".to_string(),
                    _ => match boot_completed(&sdk, &serial).await {
                        Some(true) => "ready".to_string(),
                        _ => "booting".to_string(),
                    },
                };
                avds.push(AvdEntry {
                    name: format!("{serial} (unknown AVD)"),
                    running: true,
                    serial: Some(serial),
                    status,
                });
            }
        }
        Err(_) => {
            // adb is unusable too — fall back to console ports only.
            for p in console_ports {
                avds.push(AvdEntry {
                    name: format!("emulator-{p} (unknown AVD)"),
                    running: true,
                    serial: Some(format!("emulator-{p}")),
                    status: "ready".to_string(),
                });
            }
        }
    }
    EmulatorsState {
        sdk_path: Some(sdk_path),
        sdk_error: Some(error),
        avds,
    }
}

/// Maps an adb device state to a UI status; "device" means still booting
/// until `sys.boot_completed` reports 1.
async fn status_for(state: &str, serial: &str, sdk: &Sdk) -> String {
    match state {
        "offline" => "offline".to_string(),
        "unauthorized" => "unauthorized".to_string(),
        _ => match boot_completed(sdk, serial).await {
            Some(true) => "ready".to_string(),
            _ => "booting".to_string(),
        },
    }
}
