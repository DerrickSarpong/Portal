//! Process restart logic: root discovery along the Windows parent chain,
//! tree kill, and a detached, window-less relaunch.
//!
//! Frameworks spawn child processes (`npm` → `node` → server, shells between
//! them), so restarting the listening PID alone is not enough. Portal walks up
//! the parent chain to the *true root* of the launch, kills the whole tree
//! (deepest first), then relaunches the captured command line in the captured
//! working directory with `CREATE_NO_WINDOW`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    CreateProcessW, OpenProcess, TerminateProcess, CREATE_NO_WINDOW, PROCESS_INFORMATION,
    PROCESS_TERMINATE, STARTUPINFOW,
};

use crate::detect::is_system_image;
use crate::procs::ProcessInfo;

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

/// Launches a process fully detached and without a console window
/// (`CREATE_NO_WINDOW`). Returns immediately; every handle is dropped so the
/// child outlives Portal and keeps running in the background.
pub fn spawn_hidden(exe: &str, args: &[String], cwd: Option<&Path>) -> Result<(), String> {
    let mut cmdline = quote(exe);
    for a in args {
        cmdline.push(' ');
        cmdline.push_str(&quote(a));
    }
    spawn_hidden_cmdline(&cmdline, cwd)
}

/// Launches a process from a raw Windows command line (used by restart so the
/// original command line is reproduced byte-for-byte).
pub fn spawn_hidden_cmdline(cmdline: &str, cwd: Option<&Path>) -> Result<(), String> {
    let mut wide: Vec<u16> = cmdline.encode_utf16().chain([0]).collect();
    let cwd_wide: Option<Vec<u16>> = cwd.map(|c| {
        use std::os::windows::ffi::OsStrExt;
        c.as_os_str().encode_wide().chain([0]).collect()
    });
    let cwd_ptr = cwd_wide
        .as_ref()
        .map(|c| PCWSTR::from_raw(c.as_ptr()))
        .unwrap_or(PCWSTR::null());

    let mut si = STARTUPINFOW::default();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            PWSTR::from_raw(wide.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_NO_WINDOW,
            None,
            cwd_ptr,
            &si,
            &mut pi,
        )
        .map_err(|e| format!("CreateProcessW failed: {e}"))?;

        let _ = CloseHandle(pi.hProcess);
        let _ = CloseHandle(pi.hThread);
    }
    Ok(())
}

/// Opens a URL in the default browser without a console flash.
/// `cmd /c start "" <url>` — the empty title argument prevents `start` from
/// treating the quoted URL as a window title.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    spawn_hidden(
        "cmd.exe",
        &["/c".to_string(), "start".to_string(), "".to_string(), url.to_string()],
        None,
    )
}

/// Windows command-line quoting (MSVCRT rules: backslash runs before quotes
/// are doubled, arguments with spaces are wrapped in quotes).
fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && !arg.contains(' ')
        && !arg.contains('\t')
        && !arg.contains('"')
    {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(ch);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2)); // before the closing quote
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Killing
// ---------------------------------------------------------------------------

/// Kills `root_pid` and every descendant process (deepest first). Returns the
/// number of processes actually terminated.
pub fn kill_tree(root_pid: u32, procs: &HashMap<u32, ProcessInfo>) -> usize {
    let me = std::process::id();

    // Build parent → children edges from the process snapshot.
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs.values() {
        if let Some(pp) = p.parent_pid {
            children.entry(pp).or_default().push(p.pid);
        }
    }

    // BFS from the root, remembering depth so we can kill deepest-first.
    let mut order: Vec<(usize, u32)> = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut queue: VecDeque<(usize, u32)> = VecDeque::new();
    seen.insert(root_pid);
    queue.push_back((0, root_pid));
    while let Some((depth, pid)) = queue.pop_front() {
        order.push((depth, pid));
        if let Some(kids) = children.get(&pid) {
            for &kid in kids {
                if kid != me && seen.insert(kid) {
                    queue.push_back((depth + 1, kid));
                }
            }
        }
    }

    // Deepest first: parents cannot respawn children mid-flight.
    order.sort_by(|a, b| b.0.cmp(&a.0));
    order.iter().filter(|(_, pid)| terminate_pid(*pid)).count()
}

/// Terminates one PID. Refuses PIDs 0/4 (System) and Portal itself.
fn terminate_pid(pid: u32) -> bool {
    if pid <= 4 || pid == std::process::id() {
        return false;
    }
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) else {
            return false;
        };
        let ok = TerminateProcess(handle, 1).is_ok();
        let _ = CloseHandle(handle);
        ok
    }
}

// ---------------------------------------------------------------------------
// Root discovery + restart
// ---------------------------------------------------------------------------

/// Binaries Portal is allowed to walk across (and later relaunch).
fn is_runner(info: &ProcessInfo) -> bool {
    matches!(
        info.exe_name().as_str(),
        "cmd.exe" | "command.com" | "powershell.exe" | "pwsh.exe"
            | "node.exe" | "npm.exe" | "npx.exe" | "pnpm.exe" | "yarn.exe" | "yarnpkg.exe"
            | "bun.exe" | "deno.exe" | "tsx.exe"
            | "python.exe" | "pythonw.exe" | "py.exe" | "uv.exe" | "uvicorn.exe"
            | "gunicorn.exe" | "streamlit.exe" | "mlflow.exe" | "poetry.exe"
            | "dotnet.exe" | "java.exe" | "javaw.exe" | "ruby.exe"
            | "wt.exe" | "windowsterminal.exe" | "openconsole.exe" | "conhost.exe"
    )
}

/// Does the process carry a *real* command invocation? Interactive shells
/// (bare `pwsh.exe`, terminal hosts) do not, so the walk stops at them instead
/// of escaping into the user's terminal or editor.
fn has_invocation(info: &ProcessInfo) -> bool {
    let Some(cmd) = info.cmdline.as_deref() else { return false };
    let exe = info.exe_name();
    let shell = matches!(
        exe.as_str(),
        "cmd.exe" | "powershell.exe" | "pwsh.exe" | "conhost.exe" | "wt.exe"
            | "windowsterminal.exe" | "openconsole.exe" | "command.com"
    );
    let lower = cmd.to_lowercase();
    if shell {
        // Only shells explicitly running a command count (/c, /k, -c, -Command).
        lower.contains("/c")
            || lower.contains("/k")
            || lower.contains("-c ")
            || lower.contains("-command")
            || lower.contains("-encodedcommand")
    } else {
        // Any other runner must carry at least one argument beyond its image path.
        lower.len() > exe.len() + 1
    }
}

/// Case-insensitive Windows path comparison (trailing separators ignored).
fn paths_equal(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.trim_end_matches(&['\\', '/'][..]).to_lowercase();
    norm(a) == norm(b)
}

/// Walks up the parent chain from `leaf_pid` to the true root of the server
/// launch. The walk stops at:
///   - PID 0/4 or a missing parent,
///   - non-runner binaries (terminals, editors, anything unfamiliar),
///   - shells without a command invocation (interactive terminals),
///   - a project-directory change (never cross projects),
/// and returns the leaf itself when the chain is not walkable.
pub fn find_root(leaf_pid: u32, procs: &HashMap<u32, ProcessInfo>) -> u32 {
    let Some(mut cur) = procs.get(&leaf_pid) else {
        return leaf_pid;
    };
    let mut root_pid = leaf_pid;
    let mut hops = 0usize;
    loop {
        let Some(ppid) = cur.parent_pid else { break };
        if ppid == 0 || ppid == 4 {
            break;
        }
        let Some(parent) = procs.get(&ppid) else { break };
        if !is_runner(parent) || !has_invocation(parent) {
            break;
        }
        if let (Some(a), Some(b)) = (&cur.cwd, &parent.cwd) {
            if !paths_equal(a, b) {
                break;
            }
        }
        root_pid = ppid;
        cur = parent;
        hops += 1;
        if hops > 32 {
            break; // paranoia: malformed process cycles
        }
    }
    root_pid
}

/// Restarts the server behind `leaf_pid`: walks to the root of its launch
/// chain, kills the whole tree, waits for the OS to reap handles, then
/// relaunches the captured command line in the captured working directory.
pub async fn restart_server(leaf_pid: u32) -> Result<String, String> {
    let mut procs = crate::procs::snapshot_processes();

    // Collect the chain we might walk up (leaf + ancestors) and enrich it with
    // PEB cwd / exact command line data.
    let mut chain: Vec<u32> = Vec::new();
    let mut cursor = Some(leaf_pid);
    while let Some(pid) = cursor {
        chain.push(pid);
        cursor = procs.get(&pid).and_then(|p| p.parent_pid);
        if chain.len() > 64 {
            break; // paranoia: malformed process cycles
        }
    }
    crate::procs::enrich_with_peb(&mut procs, &chain);

    let root_pid = find_root(leaf_pid, &procs);
    let Some(root) = procs.get(&root_pid) else {
        return Err("root process not found (already exited?)".into());
    };
    // Safety rails: never restart Portal itself or a system process.
    if root_pid == std::process::id() {
        return Err("refusing to restart Portal itself".into());
    }
    if is_system_image(&root.exe_name()) {
        return Err(format!("refusing to restart system process '{}'", root.exe_name()));
    }
    let Some(relaunch_cmd) = root.cmdline.clone().or_else(|| root.exe.clone()) else {
        return Err("no command line captured for the root process".into());
    };
    let cwd = root.cwd.clone();
    let label = root.exe_name();

    // Kill, let Windows reap handles, relaunch detached.
    let killed = kill_tree(root_pid, &procs);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    spawn_hidden_cmdline(&relaunch_cmd, cwd.as_deref().map(Path::new))
        .map_err(|e| format!("killed {killed} process(es) but relaunch failed: {e}"))?;

    let where_ = cwd.unwrap_or_else(|| "its original directory".to_string());
    Ok(format!("Restarted {label}: killed {killed} process(es), relaunched in {where_}"))
}

/// Kills the server behind `leaf_pid`: walks to the root of its launch chain
/// and terminates the whole tree (deepest first), without relaunching.
pub async fn kill_server(leaf_pid: u32) -> Result<String, String> {
    let mut procs = crate::procs::snapshot_processes();

    // Same chain enrichment as restart so root discovery sees PEB cwd data.
    let mut chain: Vec<u32> = Vec::new();
    let mut cursor = Some(leaf_pid);
    while let Some(pid) = cursor {
        chain.push(pid);
        cursor = procs.get(&pid).and_then(|p| p.parent_pid);
        if chain.len() > 64 {
            break; // paranoia: malformed process cycles
        }
    }
    crate::procs::enrich_with_peb(&mut procs, &chain);

    let root_pid = find_root(leaf_pid, &procs);
    let Some(root) = procs.get(&root_pid) else {
        return Err("root process not found (already exited?)".into());
    };
    // Safety rails: never kill Portal itself or a system process.
    if root_pid == std::process::id() {
        return Err("refusing to kill Portal itself".into());
    }
    if is_system_image(&root.exe_name()) {
        return Err(format!("refusing to kill system process '{}'", root.exe_name()));
    }
    let label = root.exe_name();

    let killed = kill_tree(root_pid, &procs);
    if killed == 0 {
        return Err(format!("failed to terminate {label} (access denied?)"));
    }
    Ok(format!("Killed {label} and {} process(es)", killed - 1))
}
