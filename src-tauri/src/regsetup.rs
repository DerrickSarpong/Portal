//! "Start at Login" persistence via the Windows Registry.
//!
//! Writes/reads/deletes the `Portal` value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` — the standard
//! per-user autostart location. HKCU needs no elevation.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    REG_VALUE_TYPE,
};

use crate::util;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Portal";

/// Opens (or creates) the Run key with the requested access.
fn open_key(write: bool) -> Result<HKEY, String> {
    let subkey = util::wide(RUN_KEY);
    let mut hk: HKEY = HKEY::default();
    let access = if write {
        KEY_SET_VALUE | KEY_QUERY_VALUE
    } else {
        KEY_QUERY_VALUE
    };

    let err = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(subkey.as_ptr()),
            0,
            access,
            &mut hk,
        )
    };
    if err == ERROR_SUCCESS {
        return Ok(hk);
    }
    if write {
        // The Run key normally exists; create it if it was deleted.
        let err = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR::from_raw(subkey.as_ptr()),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                None,
                &mut hk,
                None,
            )
        };
        if err == ERROR_SUCCESS {
            return Ok(hk);
        }
    }
    Err(format!("failed to open registry Run key (error {})", err.0))
}

/// Is the autostart entry present and sane?
pub fn is_enabled() -> bool {
    let Ok(hk) = open_key(false) else {
        return false;
    };
    let name = util::wide(VALUE_NAME);
    let mut buf = [0u16; 520]; // up to a 519-char path
    let mut size = (buf.len() * 2) as u32;
    let mut ty = REG_VALUE_TYPE::default();

    let err = unsafe {
        RegQueryValueExW(
            hk,
            PCWSTR::from_raw(name.as_ptr()),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    let _ = unsafe { RegCloseKey(hk) };
    err == ERROR_SUCCESS && ty == REG_SZ
}

/// Enables or disables "Start at Login" by writing/deleting the registry value.
/// The stored command is the quoted path of the running executable.
pub fn set_enabled(enable: bool) -> Result<(), String> {
    let hk = open_key(true)?;
    let name = util::wide(VALUE_NAME);
    let mut result = Ok(());

    unsafe {
        if enable {
            match std::env::current_exe() {
                Ok(exe) => {
                    let data = format!("\"{}\"", exe.display());
                    // REG_SZ is UTF-16LE; RegSetValueExW takes the data as a byte slice.
                    let w: Vec<u16> = data.encode_utf16().chain([0]).collect();
                    let bytes: Vec<u8> = w.iter().flat_map(|v| v.to_ne_bytes()).collect();
                    let err = RegSetValueExW(
                        hk,
                        PCWSTR::from_raw(name.as_ptr()),
                        0,
                        REG_SZ,
                        Some(bytes.as_slice()),
                    );
                    if err != ERROR_SUCCESS {
                        result = Err(format!("RegSetValueExW failed (error {})", err.0));
                    }
                }
                Err(e) => result = Err(format!("failed to resolve current exe: {e}")),
            }
        } else {
            let err = RegDeleteValueW(hk, PCWSTR::from_raw(name.as_ptr()));
            if err != ERROR_SUCCESS && err != ERROR_FILE_NOT_FOUND {
                result = Err(format!("RegDeleteValueW failed (error {})", err.0));
            }
        }

        let _ = RegCloseKey(hk);
    }
    result
}
