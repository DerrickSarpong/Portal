//! IPC surface exposed to the React frontend.
//!
//! All entry points validate their inputs (no arbitrary command injection) and
//! return explicit errors as `Result<T, String>`.

use tauri::State;

use crate::android;
use crate::processctl;
use crate::regsetup;
use crate::snapshot::{self, Snapshot};
use crate::AppState;

/// Returns the latest snapshot, building one on demand if the background
/// poller has not completed its first cycle yet.
#[tauri::command]
pub async fn get_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    {
        let guard = state.latest.lock().map_err(|_| "state poisoned")?;
        if let Some(snap) = guard.as_ref() {
            return Ok(snap.clone());
        }
    }
    let mut cache = state.avd_cache.lock().await;
    Ok(snapshot::build_snapshot(&mut cache).await)
}

/// Opens a localhost URL in the default browser.
#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://localhost:")
        || url.starts_with("http://127.0.0.1:")
        || url.starts_with("http://[::1]:"))
    {
        return Err("only localhost URLs can be opened".into());
    }
    processctl::open_in_browser(&url)
}

/// Kills the process tree behind `pid` and relaunches its root command.
#[tauri::command]
pub async fn restart_server(pid: u32) -> Result<String, String> {
    processctl::restart_server(pid).await
}

/// Boots an AVD in the background (`cold` = fresh boot, no saved snapshot).
#[tauri::command]
pub async fn launch_avd(name: String, cold: bool) -> Result<(), String> {
    validate_avd_name(&name)?;
    let sdk = android::discover_sdk()?;
    android::launch_avd(&sdk, &name, cold)
}

/// Gracefully stops a running emulator via `adb -s <serial> emu kill`.
#[tauri::command]
pub async fn kill_avd(serial: String) -> Result<(), String> {
    if !serial.starts_with("emulator-") {
        return Err("not an emulator serial".into());
    }
    let sdk = android::discover_sdk()?;
    android::kill_avd(&sdk, &serial).await
}

/// Reads the "Start at Login" registry entry.
#[tauri::command]
pub fn get_start_at_login() -> bool {
    regsetup::is_enabled()
}

/// Enables/disables "Start at Login"; returns the resulting state.
#[tauri::command]
pub fn set_start_at_login(enabled: bool) -> Result<bool, String> {
    regsetup::set_enabled(enabled)?;
    Ok(regsetup::is_enabled())
}

/// Hides the main window (the close button hides to tray instead of quitting).
#[tauri::command]
pub fn hide_window(window: tauri::Window) {
    let _ = window.hide();
}

/// AVD names become CLI arguments — restrict them to sane characters so a
/// crafted name can never smuggle extra flags into the emulator invocation.
fn validate_avd_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 100 {
        return Err("invalid AVD name".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ' | '(' | ')'))
    {
        return Err("invalid characters in AVD name".into());
    }
    Ok(())
}
