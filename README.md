# Portal

Portal is a desktop utility for Windows. It monitors local development servers and Android emulators. It runs in the system tray and shows a small widget window.

The `Portal` file in this repository contains the architecture decisions, the setup notes, and the deploy steps. This README gives the user instructions.

## Features

- Portal scans all TCP ports that listen. It maps each port to a process ID (PID).
- It detects the framework behind each server: Next.js, React, Vue.js, Vite, Express.js, FastAPI, Streamlit, MLflow, Docker, WSL, and ADB.
- It shows the project name. It reads `package.json`, `pyproject.toml`, or `uv.lock`.
- It opens a server in your browser with one click.
- It restarts a server safely. It finds the root process, stops the process tree, and starts the same command again.
- It lists your Android Virtual Devices (AVDs). It starts, cold-starts, or stops each emulator.
- It stays in the system tray. You can hide the window and show it again.
- You can set it to start at login.
- Filter toggles hide system noise. A second toggle shows only dev servers.

## System requirements

| Item | Requirement |
|------|-------------|
| Operating system | Windows 10 or Windows 11, 64-bit |
| Rust | Stable toolchain with the MSVC target |
| Build tools | Visual Studio Build Tools with the Windows SDK |
| Node.js | Version 18 or later, with npm |
| Android SDK | Optional. You need it for the Android tab only. |

Note: Portal needs no administrator rights. It runs as a normal user.

## Install and run from source

1. Install Rust from `https://rustup.rs`. Select the MSVC toolchain.
2. Install Visual Studio Build Tools. Include the "Desktop development with C++" workload.
3. Install Node.js version 18 or later.
4. Open a terminal in the project folder.
5. Run `npm install`. This installs the frontend packages.
6. Run `npm run tauri dev`. This compiles the backend and opens the window.

Note: The first build takes five to ten minutes. Later builds are fast.

## Use the window

The window has no frame. Drag the header bar to move the window. The close button hides the window to the tray. Portal then stays in the background.

### Tray icon

- Left-click the tray icon. The window shows or hides.
- Right-click the tray icon. A menu opens with "Show / Hide", "Start at Login", and "Quit Portal".
- The tray tooltip shows the number of listening ports.

### Dev Servers tab

Each row shows a port, a project name, a framework badge, and a PID.

- Click the globe button. Portal opens `http://localhost:PORT` in your default browser.
- Click the restart button. Portal stops the process tree and starts the server again.
- Click the System toggle (eye icon). Portal hides or shows the system services. The hidden count shows on the toggle.
- Click the "Servers only" toggle. Portal shows only the dev-server rows.
- The header shows the total port count. The tab badge shows the visible count.

Warning: The restart action stops processes. Do not restart a server that runs important work.

### Android tab

Portal shows the SDK path and a list of AVDs. Each row shows a status badge.

| Status | Meaning |
|--------|---------|
| stopped | The AVD is off. |
| booting | The emulator starts. |
| ready | The emulator is fully on. |
| offline | The device does not answer. |
| unauthorized | You did not accept the debug prompt. |

- Click the play button to start an AVD.
- Click the bolt button to cold-start an AVD. Portal then ignores the saved snapshot.
- Click the stop button to stop a running emulator.

Note: If Portal does not find the SDK, set the `ANDROID_HOME` variable. The default path is `%LOCALAPPDATA%\Android\Sdk`.

## Start at login

1. Click the "Start at Login" box in the footer. Or use the tray menu.
2. Portal writes its path to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

To stop the auto-start, clear the box again.

## Build the installers

1. Run `npm run tauri build`. This builds the frontend and the backend.
2. Find the installers in `src-tauri\target\release\bundle\`.

| File | Type |
|------|------|
| `nsis\Portal_0.1.0_x64-setup.exe` | NSIS installer |
| `msi\Portal_0.1.0_x64_en-US.msi` | MSI package |

## Deploy to other computers

1. Copy the NSIS or MSI installer to the target computer.
2. Run the installer. The NSIS installer needs no administrator rights.
3. Start Portal. The tray icon appears.
4. Click "Start at Login". Portal then starts at each login.

Note: WebView2 is the only system dependency. Windows 10 and 11 include it.

## Project structure

| Folder or file | Contents |
|----------------|----------|
| `src-tauri\src\` | The Rust backend modules |
| `src\` | The React frontend |
| `Portal` | The architecture document |
| `src-tauri\tauri.conf.json` | The window and bundle settings |

## Architecture summary

- The backend is Rust. It uses the `sysinfo` crate and the Win32 API.
- The frontend is React, TypeScript, and Tailwind CSS in a Tauri v2 window.
- A background task scans the ports every four seconds.
- Portal reads the process working directory from the process PEB.
- See the `Portal` file for the full architecture decisions.

## Troubleshooting

| Problem | Solution |
|---------|----------|
| The Android tab shows "SDK not found". | Set `ANDROID_HOME`, or put the SDK in `%LOCALAPPDATA%\Android\Sdk`. |
| A server does not appear in the list. | Check that the port listens. Start the server again. |
| The restart action fails. | Portal could not read the process command line. Start the server manually. |
| The tray icon does not show the window. | Right-click the icon and use "Show / Hide". |
| The build fails with a linker error. | Install Visual Studio Build Tools with the Windows SDK. |
| The frontend build fails. | Run `npm install` again. Then run the build again. |

## Safety and data

- Portal needs no administrator rights.
- Portal reads only local system tables and local files.
- Portal sends no data to the network. The browser action opens localhost URLs only.
- Portal writes one registry value for the start-at-login option.
- Portal collects no telemetry.
