//! Framework / service detection and project-name resolution.
//!
//! Detection works in three passes over each *listening* process:
//!   1. binary identity — image file name (`com.docker.backend.exe`, `adb.exe`,
//!      `qemu-system-*`) identifies infrastructure services unambiguously;
//!   2. command-line evidence — word-boundary keyword matching (`next`,
//!      `vite`, `uvicorn`, `streamlit`, …) classifies dev servers;
//!   3. dependency evidence — `package.json` dependencies (`next`, `vue`,
//!      `express`, …) disambiguate servers started via `node server.js`.
//!
//! Each detected runtime gets a unique brand color; shades that would be
//! invisible on the dark UI (Next.js and Express brand black) are rendered as
//! light grays instead.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

/// A detected runtime's identity, display label and brand color.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Framework {
    pub id: &'static str,
    pub label: &'static str,
    pub color: &'static str,
}

// Brand palette — one unique color per detected runtime.
pub const NEXT: Framework = Framework { id: "next", label: "Next.js", color: "#e4e4e7" };
pub const REACT: Framework = Framework { id: "react", label: "React", color: "#61dafb" };
pub const VUE: Framework = Framework { id: "vue", label: "Vue.js", color: "#42b883" };
pub const VITE: Framework = Framework { id: "vite", label: "Vite", color: "#a78bfa" };
pub const EXPRESS: Framework = Framework { id: "express", label: "Express.js", color: "#cbd5e1" };
pub const FASTAPI: Framework = Framework { id: "fastapi", label: "FastAPI", color: "#009485" };
pub const STREAMLIT: Framework = Framework { id: "streamlit", label: "Streamlit", color: "#ff4b4b" };
pub const MLFLOW: Framework = Framework { id: "mlflow", label: "MLflow", color: "#0194e2" };
pub const DOCKER: Framework = Framework { id: "docker", label: "Docker / WSL", color: "#2496ed" };
pub const EMULATOR: Framework =
    Framework { id: "emulator", label: "Android Emulator", color: "#4ade80" };
pub const ADB: Framework = Framework { id: "adb", label: "ADB Server", color: "#3ddc84" };
pub const NODE: Framework = Framework { id: "node", label: "Node.js", color: "#539e43" };
pub const PYTHON: Framework = Framework { id: "python", label: "Python", color: "#ffd43b" };
pub const OTHER: Framework = Framework { id: "other", label: "Service", color: "#71717a" };

/// Detection result: which framework and what kind of row it is.
/// `kind` drives UI affordances (`web` rows get Open/Restart buttons).
#[derive(Debug, Clone, Copy)]
pub struct Detected {
    pub framework: Framework,
    pub kind: &'static str,
}

/// `package.json` metadata for a working directory.
#[derive(Debug, Clone, Default)]
pub struct PackageMeta {
    /// `name` field, npm scope stripped (`@vercel/pkg` → `pkg`).
    pub name: String,
    /// Union of dependencies / devDependencies / peerDependencies keys.
    pub deps: HashSet<String>,
}

/// Everything `detect` needs about one listening process.
pub struct DetectInput<'a> {
    /// Process name (`"node.exe"`) — secondary identity signal when the
    /// image path is unreadable.
    pub process_name: &'a str,
    /// Lowercased image file name.
    pub exe_file: &'a str,
    /// Lowercased command line, or None when unreadable.
    pub cmdline: Option<&'a str>,
    /// Parsed `package.json` for the working directory, when present.
    pub package: Option<&'a PackageMeta>,
}

/// Classifies one listening process.
pub fn detect(i: DetectInput) -> Detected {
    let cmd = i.cmdline.unwrap_or("");

    // ---- Pass 1: binary identity (process / image file name only) ----
    let image = format!("{} {}", i.process_name.to_lowercase(), i.exe_file);
    if image.contains("adb") {
        return Detected { framework: ADB, kind: "adb" };
    }
    if image.contains("com.docker")
        || image.contains("docker-proxy")
        || image.contains("vpnkit")
        || image.contains("wslrelay")
        || image.contains("wslhost")
    {
        return Detected { framework: DOCKER, kind: "docker" };
    }
    if image.contains("qemu-system") {
        return Detected { framework: EMULATOR, kind: "other" };
    }

    let is_node = i.exe_file == "node.exe" || i.exe_file == "deno.exe" || i.exe_file == "bun.exe";
    let is_python = i.exe_file.starts_with("python")
        || i.exe_file == "py.exe"
        || i.exe_file == "uvicorn.exe"
        || i.exe_file == "streamlit.exe"
        || i.exe_file == "mlflow.exe"
        || i.exe_file == "gunicorn.exe";

    // ---- Pass 2: command-line evidence ----
    if has_word(cmd, "next") {
        return Detected { framework: NEXT, kind: "web" };
    }
    if has_word(cmd, "react-scripts") {
        return Detected { framework: REACT, kind: "web" };
    }
    if has_word(cmd, "vite") {
        return Detected { framework: vite_flavor(i.package), kind: "web" };
    }
    if has_word(cmd, "nuxt") || has_word(cmd, "vue") {
        return Detected { framework: VUE, kind: "web" };
    }
    if has_word(cmd, "express") {
        return Detected { framework: EXPRESS, kind: "web" };
    }
    if has_word(cmd, "streamlit") {
        return Detected { framework: STREAMLIT, kind: "web" };
    }
    if has_word(cmd, "mlflow") {
        return Detected { framework: MLFLOW, kind: "web" };
    }
    if has_word(cmd, "uvicorn") || has_word(cmd, "fastapi") {
        return Detected { framework: FASTAPI, kind: "web" };
    }

    // ---- Pass 3: dependency evidence (node servers with a package.json) ----
    if is_node {
        if let Some(pkg) = i.package {
            let deps = &pkg.deps;
            if deps.contains("next") {
                return Detected { framework: NEXT, kind: "web" };
            }
            if deps.contains("react-scripts") {
                return Detected { framework: REACT, kind: "web" };
            }
            if deps.contains("nuxt") || deps.contains("vue") {
                return Detected { framework: VUE, kind: "web" };
            }
            if deps.contains("express") {
                return Detected { framework: EXPRESS, kind: "web" };
            }
            if deps.contains("vite") {
                return Detected { framework: vite_flavor(i.package), kind: "web" };
            }
            if deps.contains("fastapi") || deps.contains("uvicorn") {
                return Detected { framework: FASTAPI, kind: "web" };
            }
            if deps.contains("streamlit") {
                return Detected { framework: STREAMLIT, kind: "web" };
            }
            if deps.contains("mlflow") {
                return Detected { framework: MLFLOW, kind: "web" };
            }
        }
    }

    // ---- Pass 4: generic runtimes and system services ----
    if is_node {
        return Detected { framework: NODE, kind: "web" };
    }
    if is_python {
        return Detected { framework: PYTHON, kind: "web" };
    }
    if is_system_image(i.exe_file) {
        return Detected { framework: OTHER, kind: "system" };
    }

    Detected { framework: OTHER, kind: "other" }
}

/// `vite` is a bundler, not a framework — use package.json dependencies to
/// pick the framework flavor (Vue vs React vs plain Vite).
fn vite_flavor(pkg: Option<&PackageMeta>) -> Framework {
    if let Some(p) = pkg {
        if p.deps.contains("vue") || p.deps.contains("nuxt") {
            return VUE;
        }
        if p.deps.contains("react") {
            return REACT;
        }
    }
    VITE
}

/// Windows system processes that are never dev servers and never restartable.
pub fn is_system_image(exe_file: &str) -> bool {
    matches!(
        exe_file,
        "system" | "registry" | "memcompression" | "smss.exe" | "csrss.exe" | "wininit.exe"
            | "winlogon.exe" | "services.exe" | "lsass.exe" | "lsaiso.exe" | "svchost.exe"
            | "dwm.exe" | "explorer.exe" | "fontdrvhost.exe" | "sihost.exe" | "taskhostw.exe"
            | "runtimebroker.exe" | "searchhost.exe" | "startmenuexperiencehost.exe"
            | "textinputhost.exe" | "shellexperiencehost.exe" | "msmpeng.exe"
            | "securityhealthservice.exe" | "securityhealthsystray.exe" | "spoolsv.exe"
            | "smartscreen.exe" | "dllhost.exe" | "audiodg.exe" | "wudfhost.exe"
            | "conhost.exe" | "cmd.exe" | "sgrmbroker.exe" | "tabtip.exe" | "ctfmon.exe"
    )
}

/// Word-boundary-aware substring search: `"next"` matches `"next dev"` and
/// `"node_modules/next/dist/bin/next"` but not `"invite"` or `"nextjs-proxy"`.
fn has_word(hay: &str, word: &str) -> bool {
    let hay = hay.as_bytes();
    let w = word.as_bytes();
    if w.is_empty() || hay.len() < w.len() {
        return false;
    }
    let mut i = 0;
    while i + w.len() <= hay.len() {
        if &hay[i..i + w.len()] == w {
            let before_ok = i == 0 || !is_word_char(hay[i - 1]);
            let after_ok = i + w.len() == hay.len() || !is_word_char(hay[i + w.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ---------------------------------------------------------------------------
// Project name resolution
// ---------------------------------------------------------------------------

/// Resolves the human project name for a working directory:
/// `package.json` → `pyproject.toml` → `uv.lock` → directory name.
pub fn resolve_project_name(cwd: Option<&Path>, pkg: Option<&PackageMeta>) -> String {
    if let Some(p) = pkg {
        if !p.name.is_empty() {
            return p.name.clone();
        }
    }
    if let Some(dir) = cwd {
        if let Some(n) = pyproject_name(dir) {
            return n;
        }
        if let Some(n) = uv_lock_name(dir) {
            return n;
        }
        if let Some(f) = dir.file_name() {
            return f.to_string_lossy().to_string();
        }
    }
    "Unknown".to_string()
}

/// Parses `package.json` (name + dependency sets) with serde_json.
/// Returns None when the file is missing or not valid JSON.
pub fn parse_package_meta(dir: &Path) -> Option<PackageMeta> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;

    let name = value
        .get("name")
        .and_then(|n| n.as_str())
        .map(strip_npm_scope)
        .unwrap_or_default();

    let mut deps = HashSet::new();
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = value.get(key).and_then(|m| m.as_object()) {
            for k in map.keys() {
                deps.insert(k.clone());
            }
        }
    }
    Some(PackageMeta { name, deps })
}

/// `"@scope/pkg"` → `"pkg"`.
fn strip_npm_scope(name: &str) -> String {
    if let Some(rest) = name.strip_prefix('@') {
        if let Some(pos) = rest.rfind('/') {
            return rest[pos + 1..].to_string();
        }
    }
    name.to_string()
}

/// Naive line scan of `pyproject.toml` for `name = "…"` inside `[project]`
/// (kept dependency-free; full TOML parsing is unnecessary for one field).
fn pyproject_name(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("pyproject.toml")).ok()?;
    let mut in_project = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_project = t == "[project]";
            continue;
        }
        if in_project {
            if let Some(n) = toml_name_value(t) {
                return Some(n);
            }
        }
    }
    None
}

/// Naive line scan of `uv.lock`: the first `name = "…"` line belongs to the
/// project's own `[[package]]` entry.
fn uv_lock_name(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("uv.lock")).ok()?;
    for line in text.lines() {
        if let Some(n) = toml_name_value(line.trim()) {
            return Some(n);
        }
    }
    None
}

/// Extracts the value of a top-level `name = "…"` line.
fn toml_name_value(line: &str) -> Option<String> {
    let rest = line.strip_prefix("name")?.trim_start().strip_prefix('=')?;
    let val = rest.trim().trim_matches('"').trim_matches('\'');
    if val.is_empty() { None } else { Some(val.to_string()) }
}
