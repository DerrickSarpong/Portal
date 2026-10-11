//! Unified snapshot (web servers + Android emulators) pushed to the frontend.
//!
//! Built every poll cycle (~4 s): Win32 TCP tables → sysinfo process details →
//! PEB cwd/cmdline → framework detection → project name → Android state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::android::{self, EmulatorsState};
use crate::detect::{self, Framework};
use crate::ports;
use crate::procs;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub port: u16,
    pub pid: u32,
    pub process_name: String,
    /// Resolved project name (package.json / pyproject.toml / uv.lock / dir).
    pub project: String,
    pub framework: Framework,
    pub cwd: Option<String>,
    /// "web" | "adb" | "docker" | "system" | "other"
    pub kind: &'static str,
    /// Only web servers are restartable.
    pub restartable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub servers: Vec<ServerEntry>,
    pub emulators: EmulatorsState,
    /// Milliseconds since the epoch.
    pub updated_at: u64,
    pub error: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Builds one full snapshot. `avd_cache` throttles `emulator -list-avds` to
/// at most once every ~15 s across poll cycles.
pub async fn build_snapshot(avd_cache: &mut Option<(std::time::Instant, Vec<String>)>) -> Snapshot {
    let rows = match ports::listening_ports() {
        Ok(r) => r,
        Err(e) => {
            return Snapshot {
                servers: Vec::new(),
                emulators: EmulatorsState::default(),
                updated_at: now_ms(),
                error: Some(format!("TCP table scan failed: {e}")),
            };
        }
    };

    // Unique listening PIDs, enriched with PEB cwd / command line.
    let mut listen_pids: Vec<u32> = rows.iter().map(|r| r.pid).collect();
    listen_pids.sort_unstable();
    listen_pids.dedup();

    let mut procs = procs::snapshot_processes();
    procs::enrich_with_peb(&mut procs, &listen_pids);

    // Per-cycle cache of parsed package.json per working directory.
    let mut pkg_cache: HashMap<String, Option<detect::PackageMeta>> = HashMap::new();

    let mut servers = Vec::new();
    for row in &rows {
        let Some(info) = procs.get(&row.pid) else {
            continue;
        };

        // Project name from the process working directory.
        let (project, pkg) = match info.cwd.as_deref().map(PathBuf::from) {
            Some(cwd) => {
                let meta = pkg_cache
                    .entry(cwd.to_string_lossy().to_string())
                    .or_insert_with(|| detect::parse_package_meta(&cwd))
                    .clone();
                (
                    detect::resolve_project_name(Some(&cwd), meta.as_ref()),
                    meta,
                )
            }
            None => ("Unknown".to_string(), None),
        };

        let detected = detect::detect(detect::DetectInput {
            process_name: &info.name,
            exe_file: &info.exe_name(),
            cmdline: info.cmdline.as_deref(),
            package: pkg.as_ref(),
        });

        servers.push(ServerEntry {
            port: row.port,
            pid: row.pid,
            process_name: info.name.clone(),
            project,
            framework: detected.framework,
            cwd: info.cwd.clone(),
            kind: detected.kind,
            restartable: detected.kind == "web",
        });
    }

    // Android emulators (SDK discovery, AVD listing/status, instance matching).
    let ports_list: Vec<u16> = rows.iter().map(|r| r.port).collect();
    let emulators = android::refresh_emulators(&ports_list, avd_cache).await;

    Snapshot {
        servers,
        emulators,
        updated_at: now_ms(),
        error: None,
    }
}
