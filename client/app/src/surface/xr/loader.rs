//! Discover `openxr_loader` when Steam lives off the system drive (e.g. D:\).
//!
//! Guarantees: try default load, then env / Steam registry / common paths / libraryfolders.vdf.
//! Non-guarantees: every third-party OpenXR runtime layout.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

#[cfg(windows)]
const LOADER_NAME: &str = "openxr_loader.dll";
#[cfg(target_os = "macos")]
const LOADER_NAME: &str = "libopenxr_loader.dylib";
#[cfg(not(any(windows, target_os = "macos")))]
const LOADER_NAME: &str = "libopenxr_loader.so";

/// Load OpenXR entry: default search path, then SteamVR / env overrides.
///
/// # Safety
/// Same as `openxr::Entry::load` / `load_from`: the shared library must be a conforming loader.
pub unsafe fn load_entry() -> Result<openxr::Entry> {
    if let Ok(path) = std::env::var("ALCHEMY_OPENXR_LOADER") {
        let p = PathBuf::from(path);
        log::info!("OpenXR loader: ALCHEMY_OPENXR_LOADER={}", p.display());
        return unsafe { openxr::Entry::load_from(&p, &()) }
            .with_context(|| format!("load OpenXR loader from {}", p.display()));
    }

    match unsafe { openxr::Entry::load(&()) } {
        Ok(entry) => {
            log::info!("OpenXR loader: default search path ({LOADER_NAME})");
            return Ok(entry);
        }
        Err(err) => {
            log::info!("OpenXR loader: default load failed ({err}); searching Steam paths");
        }
    }

    let candidates = candidate_loader_paths();
    let mut last_err = None;
    for path in &candidates {
        if !path.is_file() {
            continue;
        }
        match unsafe { openxr::Entry::load_from(path, &()) } {
            Ok(entry) => {
                log::info!("OpenXR loader: loaded from {}", path.display());
                maybe_hint_runtime_json(path);
                return Ok(entry);
            }
            Err(err) => {
                log::debug!("OpenXR loader: skip {}: {err}", path.display());
                last_err = Some(err);
            }
        }
    }

    if let Some(err) = last_err {
        bail!(
            "load OpenXR loader: tried default + {} Steam candidates; last error: {err}",
            candidates.len()
        );
    }
    bail!(
        "load OpenXR loader: {LOADER_NAME} not found (default PATH / {} Steam candidates). \
         Install SteamVR or set ALCHEMY_OPENXR_LOADER to the full path of {LOADER_NAME}.",
        candidates.len()
    );
}

fn maybe_hint_runtime_json(loader_path: &Path) {
    if std::env::var_os("XR_RUNTIME_JSON").is_some() {
        return;
    }
    // .../SteamVR/bin/win64/openxr_loader.dll → .../SteamVR/steamxr_win64.json
    let steamvr_root = loader_path
        .parent() // win64
        .and_then(|p| p.parent()) // bin
        .and_then(|p| p.parent()); // SteamVR
    let Some(root) = steamvr_root else {
        return;
    };
    for name in ["steamxr_win64.json", "steamxr.json"] {
        let json = root.join(name);
        if json.is_file() {
            // Process-local only; helps when the active runtime registry entry is missing.
            std::env::set_var("XR_RUNTIME_JSON", &json);
            log::info!("OpenXR: set XR_RUNTIME_JSON={}", json.display());
            return;
        }
    }
}

fn candidate_loader_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in steam_roots() {
        push_steamvr_loader_under(&root, &mut out);
        if let Some(libs) = parse_library_folders(&root.join("steamapps/libraryfolders.vdf")) {
            for lib in libs {
                push_steamvr_loader_under(&lib, &mut out);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn push_steamvr_loader_under(steam_or_library_root: &Path, out: &mut Vec<PathBuf>) {
    let steamvr = steam_or_library_root
        .join("steamapps")
        .join("common")
        .join("SteamVR");
    out.push(steamvr.join("bin").join("win64").join(LOADER_NAME));
    out.push(steamvr.join("bin").join(LOADER_NAME));
    out.push(steamvr.join(LOADER_NAME));
}

fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(p) = std::env::var("STEAM_PATH") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            roots.push(p);
        }
    }

    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        for (hive, subkey, value) in [
            (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
            (
                HKEY_LOCAL_MACHINE,
                r"SOFTWARE\WOW6432Node\Valve\Steam",
                "InstallPath",
            ),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
        ] {
            if let Some(p) = read_reg_string(hive, subkey, value) {
                if p.is_dir() {
                    roots.push(p);
                }
            }
        }

        // Drive-letter scan: Steam on D: / E: is common.
        for drive in b'C'..=b'Z' {
            let letter = drive as char;
            for suffix in [
                r"Program Files (x86)\Steam",
                r"Program Files\Steam",
                r"Steam",
                r"Games\Steam",
                r"environment\Steam",
                r"SteamLibrary",
            ] {
                let p = PathBuf::from(format!("{letter}:\\{suffix}"));
                if p.is_dir() {
                    roots.push(p);
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            for rel in [".steam/steam", ".local/share/Steam"] {
                let p = PathBuf::from(&home).join(rel);
                if p.is_dir() {
                    roots.push(p);
                }
            }
        }
    }

    roots.sort();
    roots.dedup();
    roots
}

#[cfg(windows)]
fn read_reg_string(hive: winreg::HKEY, subkey: &str, value: &str) -> Option<PathBuf> {
    use winreg::RegKey;
    let root = RegKey::predef(hive);
    let key = root.open_subkey(subkey).ok()?;
    let s: String = key.get_value(value).ok()?;
    Some(PathBuf::from(s.replace('/', "\\")))
}

/// Minimal parse of Steam `libraryfolders.vdf` for `"path" "D:\\SteamLibrary"` lines.
fn parse_library_folders(vdf: &Path) -> Option<Vec<PathBuf>> {
    let text = fs::read_to_string(vdf).ok()?;
    let mut paths = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.contains("\"path\"") {
            continue;
        }
        let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
        let key = parts.next()?;
        if key != "path" {
            continue;
        }
        let path = parts.next()?;
        let p = PathBuf::from(path.replace("\\\\", "\\"));
        if p.is_dir() {
            paths.push(p);
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_library_folders_extracts_path() {
        let dir = std::env::temp_dir().join("alchemy_steam_vdf_test");
        let _ = fs::create_dir_all(&dir);
        let vdf = dir.join("libraryfolders.vdf");
        let escaped = dir.display().to_string().replace('\\', "\\\\");
        let body = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{escaped}\"\n\t}}\n}}\n"
        );
        fs::write(&vdf, body).unwrap();
        let libs = parse_library_folders(&vdf).expect("parse");
        assert!(libs.iter().any(|p| p == &dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_steamvr_loader_includes_win64_dll_name() {
        let mut out = Vec::new();
        push_steamvr_loader_under(Path::new(r"D:\environment\Steam"), &mut out);
        assert!(out.iter().any(|p| {
            let s = p.to_string_lossy();
            s.contains("SteamVR") && s.contains("win64") && s.ends_with(LOADER_NAME)
        }));
    }
}
