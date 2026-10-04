//! Process enumeration via the `sysinfo` crate, plus Win32 PEB reads for the
//! working directory and exact command line.
//!
//! `sysinfo` provides the process table, parent PIDs and image paths (used for
//! process-tree traversal and killing). It does not expose the working
//! directory on Windows, so Portal reads cwd — and the exact command line —
//! straight from the target process PEB through ntdll's
//! `NtQueryInformationProcess`. That works for processes running as the same
//! user and requires no elevation.

use std::collections::HashMap;

use sysinfo::{ProcessesToUpdate, System};

use crate::util;

/// Everything Portal knows about one process.
#[derive(Debug, Clone, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    /// Process name, e.g. `"node.exe"`.
    pub name: String,
    /// Full image path, when readable.
    pub exe: Option<String>,
    /// Full command line (PEB-exact when available, else sysinfo-joined).
    pub cmdline: Option<String>,
    /// Current working directory (PEB only; None when unreadable).
    pub cwd: Option<String>,
}

impl ProcessInfo {
    /// Lowercased image file name (`"node.exe"`), falling back to the process name.
    pub fn exe_name(&self) -> String {
        self.exe
            .as_deref()
            .map(util::file_name)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.name.to_lowercase())
    }
}

/// Snapshot of the full process table (pid → info).
///
/// Only the process list is refreshed — deliberately cheap (~10–30 ms) so the
/// 4-second poll loop stays far below 1% CPU.
pub fn snapshot_processes() -> HashMap<u32, ProcessInfo> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut out: HashMap<u32, ProcessInfo> = HashMap::with_capacity(sys.processes().len());
    for (pid, process) in sys.processes() {
        let joined = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        out.insert(
            pid.as_u32(),
            ProcessInfo {
                pid: pid.as_u32(),
                parent_pid: process.parent().map(|p| p.as_u32()),
                name: process.name().to_string_lossy().to_string(),
                exe: process.exe().map(|p| p.to_string_lossy().to_string()),
                cmdline: if joined.is_empty() { None } else { Some(joined) },
                cwd: None,
            },
        );
    }
    out
}

/// Fills in cwd / exact command line (from the PEB) for the given PIDs.
/// Quietly skips PIDs that cannot be opened (system processes, other users).
pub fn enrich_with_peb(procs: &mut HashMap<u32, ProcessInfo>, pids: &[u32]) {
    for pid in pids {
        let Some(info) = procs.get_mut(pid) else { continue };
        let need_cwd = info.cwd.is_none();
        let need_cmd = info.cmdline.is_none();
        if !need_cwd && !need_cmd {
            continue;
        }
        if let Some((cwd, cmd)) = read_peb(*pid) {
            if need_cwd {
                info.cwd = cwd;
            }
            if need_cmd {
                info.cmdline = cmd;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 PEB access (x64)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
mod peb {
    use windows::core::{PCSTR, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    use super::*;

    /// `ProcessBasicInformation` — the NtQueryInformationProcess class we need.
    const PROCESS_BASIC_INFORMATION: u32 = 0;

    // x64 offsets: PEB → ProcessParameters, and the fields inside
    // RTL_USER_PROCESS_PARAMETERS we care about.
    const PEB_OFF_PROCESS_PARAMETERS: usize = 0x20;
    const UPP_OFF_CURRENT_DIRECTORY: usize = 0x38; // CURDIR.DosPath (UNICODE_STRING)
    const UPP_OFF_COMMAND_LINE: usize = 0x70; // UNICODE_STRING CommandLine

    /// Layout-compatible subset of `PROCESS_BASIC_INFORMATION` (x64, 48 bytes).
    #[repr(C)]
    struct BasicInfo {
        exit_status: i32,
        _pad0: u32,
        peb_base: *mut core::ffi::c_void,
        affinity_mask: usize,
        base_priority: i32,
        _pad1: u32,
        unique_pid: usize,
        inherited_from_pid: usize,
    }

    /// Layout-compatible `UNICODE_STRING` (x64, 16 bytes).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UnicodeString {
        /// Length in *bytes*.
        length: u16,
        maximum_length: u16,
        _pad: u32,
        buffer: *mut u16,
    }

    type NtQueryInformationProcessFn = unsafe extern "system" fn(
        handle: HANDLE,
        process_information_class: u32,
        process_information: *mut core::ffi::c_void,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;

    /// Resolves `NtQueryInformationProcess` from the always-loaded ntdll.dll
    /// (dynamic loading avoids a hard dependency on the `ntapi` crate).
    fn ntdll_query_fn() -> Option<NtQueryInformationProcessFn> {
        let dll_name = util::wide("ntdll.dll");
        // GetProcAddress takes an ANSI (PCSTR) function name.
        let fn_name = b"NtQueryInformationProcess\0";
        unsafe {
            let ntdll = GetModuleHandleW(PCWSTR::from_raw(dll_name.as_ptr())).ok()?;
            let far = GetProcAddress(ntdll, PCSTR::from_raw(fn_name.as_ptr()))?;
            let f: Option<NtQueryInformationProcessFn> = std::mem::transmute(far);
            f
        }
    }

    unsafe fn read_mem(
        handle: HANDLE,
        src: *const core::ffi::c_void,
        dst: *mut core::ffi::c_void,
        size: usize,
    ) -> windows::core::Result<()> {
        ReadProcessMemory(handle, src, dst, size, None)
    }

    /// Reads a `UNICODE_STRING` target string out of the remote process.
    unsafe fn read_unicode_string(handle: HANDLE, us: &UnicodeString) -> Option<String> {
        if us.buffer.is_null() || us.length == 0 {
            return None;
        }
        let bytes = us.length as usize;
        let mut buf = vec![0u16; bytes / 2 + 1];
        read_mem(
            handle,
            us.buffer as *const core::ffi::c_void,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            bytes,
        )
        .ok()?;
        let s = String::from_utf16_lossy(&buf[..bytes / 2]);
        let trimmed = s.trim_end_matches('\0').trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    }

    /// Reads (cwd, cmdline) from a process PEB. Works for same-user processes
    /// without elevation; returns None otherwise (system processes, other users).
    pub fn read(pid: u32) -> Option<(Option<String>, Option<String>)> {
        unsafe {
            let handle =
                OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
            let out = read_inner(handle);
            let _ = CloseHandle(handle);
            out
        }
    }

    unsafe fn read_inner(handle: HANDLE) -> Option<(Option<String>, Option<String>)> {
        let query = ntdll_query_fn()?;

        // 1. PEB base address.
        let mut pbi = BasicInfo {
            exit_status: 0,
            _pad0: 0,
            peb_base: std::ptr::null_mut(),
            affinity_mask: 0,
            base_priority: 0,
            _pad1: 0,
            unique_pid: 0,
            inherited_from_pid: 0,
        };
        let status = query(
            handle,
            PROCESS_BASIC_INFORMATION,
            &mut pbi as *mut BasicInfo as *mut core::ffi::c_void,
            std::mem::size_of::<BasicInfo>() as u32,
            std::ptr::null_mut(),
        );
        if status != 0 || pbi.peb_base.is_null() {
            return None;
        }

        // 2. PEB → ProcessParameters.
        let mut params: usize = 0;
        read_mem(
            handle,
            (pbi.peb_base as usize + PEB_OFF_PROCESS_PARAMETERS) as *const core::ffi::c_void,
            &mut params as *mut usize as *mut core::ffi::c_void,
            std::mem::size_of::<usize>(),
        )
        .ok()?;
        if params == 0 {
            return None;
        }

        // 3. ProcessParameters → CurrentDirectory.DosPath and CommandLine.
        let mut dos_path =
            UnicodeString { length: 0, maximum_length: 0, _pad: 0, buffer: std::ptr::null_mut() };
        read_mem(
            handle,
            (params + UPP_OFF_CURRENT_DIRECTORY) as *const core::ffi::c_void,
            &mut dos_path as *mut UnicodeString as *mut core::ffi::c_void,
            std::mem::size_of::<UnicodeString>(),
        )
        .ok()?;
        let cwd = read_unicode_string(handle, &dos_path);

        let mut cmd =
            UnicodeString { length: 0, maximum_length: 0, _pad: 0, buffer: std::ptr::null_mut() };
        read_mem(
            handle,
            (params + UPP_OFF_COMMAND_LINE) as *const core::ffi::c_void,
            &mut cmd as *mut UnicodeString as *mut core::ffi::c_void,
            std::mem::size_of::<UnicodeString>(),
        )
        .ok()?;
        let cmdline = read_unicode_string(handle, &cmd);

        Some((cwd, cmdline))
    }
}

#[cfg(target_arch = "x86_64")]
pub use peb::read as read_peb;

/// Non-x64 fallback: PEB offsets differ, so report nothing rather than guess.
#[cfg(not(target_arch = "x86_64"))]
pub fn read_peb(_pid: u32) -> Option<(Option<String>, Option<String>)> {
    None
}
