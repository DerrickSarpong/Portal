//! Small shared helpers used across Portal's modules.

/// Encodes a `&str` as a NUL-terminated UTF-16 buffer for Win32 APIs.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Lowercased file-name component of an image path (`"C:\..\node.exe"` → `"node.exe"`).
pub fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}
