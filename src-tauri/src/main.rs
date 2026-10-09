//! Portal — a Windows take on the macOS "Blink" dev-server utility.
//!
//! Entry point: wires up the frameless widget window, the system tray, the
//! background poller (emits `snapshot` every 4 s) and the IPC commands.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod android;
mod commands;
mod detect;
mod ports;
mod processctl;
mod procs;
mod regsetup;
mod snapshot;
mod util;

use std::time::Duration;

use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use crate::snapshot::Snapshot;

/// Shared application state.
pub struct AppState {
    /// Latest snapshot emitted by the poller (served instantly to `get_snapshot`).
    pub latest: std::sync::Mutex<Option<Snapshot>>,
    /// Throttles `emulator -list-avds` to once every ~15 s.
    pub avd_cache: tokio::sync::Mutex<Option<(std::time::Instant, Vec<String>)>>,
}

/// Handles to the tray menu items (kept in state so menu events can read the
/// checkbox state).
pub struct TrayMenu {
    pub autostart: CheckMenuItem<tauri::Wry>,
}

fn main() {
    tauri::Builder::default()
        .manage(AppState {
            latest: std::sync::Mutex::new(None),
            avd_cache: tokio::sync::Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::open_url,
            commands::restart_server,
            commands::kill_server,
            commands::launch_avd,
            commands::kill_avd,
            commands::get_start_at_login,
            commands::set_start_at_login,
            commands::hide_window,
        ])
        .setup(|app| {
            build_tray(app.handle())?;

            // Background poller: first tick fires immediately, then every 4 s.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { poll_loop(handle).await });

            Ok(())
        })
        // Closing the window (Alt+F4, taskbar) hides to tray instead of quitting.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Portal");
}

/// Poll loop: rebuild the snapshot, push it to the frontend and refresh the
/// tray tooltip. Runs every 4 s on tokio's background runtime — far below 1%
/// CPU at idle.
async fn poll_loop(app: AppHandle) {
    let mut ticker = tokio::time::interval(Duration::from_secs(4));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;

        let state = app.state::<AppState>();
        let snap = {
            let mut cache = state.avd_cache.lock().await;
            snapshot::build_snapshot(&mut cache).await
        };

        let server_count = snap.servers.len();
        if let Ok(mut latest) = state.latest.lock() {
            *latest = Some(snap.clone());
        }
        let _ = app.emit("snapshot", &snap);

        if let Some(tray) = app.tray_by_id("portal-tray") {
            let tip = format!(
                "Portal — {server_count} listening port{}",
                if server_count == 1 { "" } else { "s" }
            );
            let _ = tray.set_tooltip(Some(tip));
        }
    }
}

/// Left-click on the tray icon toggles the main window's visibility.
fn toggle_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let visible = win.is_visible().unwrap_or(false);
        if visible {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

/// Builds the system tray icon and its menu:
/// Show/Hide · Start at Login (check) · Quit.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        "start-at-login",
        "Start at Login",
        true,
        regsetup::is_enabled(),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Portal", true, None::<&str>)?;
    let menu = MenuBuilder::new(app).items(&[&toggle, &autostart, &quit]).build()?;

    let mut builder = TrayIconBuilder::with_id("portal-tray")
        .menu(&menu)
        // Left click toggles the window (handled below); right click opens the menu.
        .show_menu_on_left_click(false)
        .tooltip("Portal — starting…");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_main_window(app),
            "quit" => app.exit(0),
            "start-at-login" => {
                // muda toggles the checkbox before the event fires, so
                // is_checked() already returns the NEW state — persist it.
                let items = app.state::<TrayMenu>();
                let enabled = items.autostart.is_checked().unwrap_or(false);
                if regsetup::set_enabled(enabled).is_err() {
                    // Registry write failed — revert the checkbox.
                    let _ = items.autostart.set_checked(!enabled);
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayMenu { autostart });
    Ok(())
}
