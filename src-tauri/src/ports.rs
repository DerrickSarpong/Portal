//! TCP listener discovery via the Win32 `GetExtendedTcpTable` API.
//!
//! Both the IPv4 and IPv6 owner-PID listener tables are queried so dev
//! servers bound to `::` or `::1` (common for Node) are not missed. No
//! elevation is required: these tables are readable by any authenticated
//! user, which keeps Portal out of the Administrator-only path.

use std::collections::HashSet;

use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Networking::WinSock::AF_INET6;

/// `AF_INET` (2) — declared locally to avoid pulling extra WinSock constants.
const AF_INET: u32 = 2;

/// `TCP_STATE_LISTEN` — the value used in `MIB_TCPROW_OWNER_PID.dwState`.
const TCP_STATE_LISTEN: u32 = 2;

/// A TCP port in LISTEN state and the PID of the process that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortRow {
    pub port: u16,
    pub pid: u32,
}

/// Returns every TCP port currently in the LISTEN state (IPv4 + IPv6), mapped
/// to its owning PID, sorted by port. Ports present in both tables are
/// reported once (the IPv4 owner wins).
pub fn listening_ports() -> Result<Vec<PortRow>, String> {
    let mut out: Vec<PortRow> = Vec::new();
    let mut seen: HashSet<u16> = HashSet::new();

    for row in v4_rows().map_err(|e| format!("IPv4 TCP table: {e}"))? {
        if row.dwState != TCP_STATE_LISTEN {
            continue;
        }
        // dwLocalPort stores the port in network byte order in its low word.
        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
        if port == 0 || !seen.insert(port) {
            continue;
        }
        out.push(PortRow {
            port,
            pid: row.dwOwningPid,
        });
    }

    for row in v6_rows().map_err(|e| format!("IPv6 TCP table: {e}"))? {
        if row.dwState != TCP_STATE_LISTEN {
            continue;
        }
        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
        if port == 0 || !seen.insert(port) {
            continue;
        }
        out.push(PortRow {
            port,
            pid: row.dwOwningPid,
        });
    }

    out.sort_by_key(|r| r.port);
    Ok(out)
}

/// Queries the IPv4 owner-PID listener table.
fn v4_rows() -> Result<Vec<MIB_TCPROW_OWNER_PID>, String> {
    let mut size: u32 = 0;

    // First call with a null buffer: Windows reports the required table size.
    let err = unsafe {
        GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if err != ERROR_INSUFFICIENT_BUFFER.0 {
        return Err(format!("size probe failed (error {err})"));
    }

    let mut buf = vec![0u8; size as usize];
    let err = unsafe {
        GetExtendedTcpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            false,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if err != NO_ERROR.0 {
        return Err(format!("table read failed (error {err})"));
    }

    let table = unsafe { &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
    let count = table.dwNumEntries as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    // Copy rows out — `buf` is dropped when this function returns.
    Ok(unsafe { std::slice::from_raw_parts(table.table.as_ptr(), count) }.to_vec())
}

/// Queries the IPv6 owner-PID listener table (same two-call pattern).
fn v6_rows() -> Result<Vec<MIB_TCP6ROW_OWNER_PID>, String> {
    let mut size: u32 = 0;
    let err = unsafe {
        GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if err != ERROR_INSUFFICIENT_BUFFER.0 {
        return Err(format!("size probe failed (error {err})"));
    }

    let mut buf = vec![0u8; size as usize];
    let err = unsafe {
        GetExtendedTcpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if err != NO_ERROR.0 {
        return Err(format!("table read failed (error {err})"));
    }

    let table = unsafe { &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID) };
    let count = table.dwNumEntries as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    Ok(unsafe { std::slice::from_raw_parts(table.table.as_ptr(), count) }.to_vec())
}
