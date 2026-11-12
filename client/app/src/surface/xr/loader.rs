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
        // Safety: `p` is an absolute path to a loader DLL/so; openxr binds symbols for this process.
        return unsafe { openxr::Entry::load_from(&p, &()) }
            .with_context(|| format!("load OpenXR loader from {}", p.display()));
    }

    // Safety: default loader search path; Entry owns the loaded library until dropped.
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
        // Safety: `path` was checked `is_file`; Entry owns the loaded library until dropped.
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
///
/// Reads the file, parses path strings (pure), then keeps only existing directories.
fn parse_library_folders(vdf: &Path) -> Option<Vec<PathBuf>> {
    let text = fs::read_to_string(vdf).ok()?;
    let paths = filter_existing_dirs(parse_library_folder_paths(&text));
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

/// Extract library roots from VDF text. Does not touch the filesystem.
fn parse_library_folder_paths(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.contains("\"path\"") {
            continue;
        }
        let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
        let Some(key) = parts.next() else {
            continue;
        };
        if key != "path" {
            continue;
        }
        let Some(path) = parts.next() else {
            continue;
        };
        paths.push(PathBuf::from(path.replace("\\\\", "\\")));
    }
    paths
}

fn filter_existing_dirs(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    paths.into_iter().filter(|p| p.is_dir()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique under `temp_dir` without fixed names (avoids parallel / leftover collisions).
    fn unique_temp_subdir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "alchemy_xr_loader_{label}_{}_{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn parse_library_folder_paths_extracts_escaped_path() {
        let body =
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n";
        let libs = parse_library_folder_paths(body);
        assert_eq!(libs, vec![PathBuf::from(r"D:\SteamLibrary")]);
    }

    #[test]
    fn parse_library_folder_paths_skips_non_path_keys() {
        let body = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"label\"\t\t\"Games\"\n\t\t\"path\"\t\t\"E:\\\\Lib\"\n\t}\n}\n";
        let libs = parse_library_folder_paths(body);
        assert_eq!(libs, vec![PathBuf::from(r"E:\Lib")]);
    }

    #[test]
    fn parse_library_folder_paths_skips_malformed_path_line() {
        // `"path"` present but no value token → skip that line, keep later valid paths.
        let body = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\n\t\t\"path\"\t\t\"F:\\\\Ok\"\n\t}\n}\n";
        let libs = parse_library_folder_paths(body);
        assert_eq!(libs, vec![PathBuf::from(r"F:\Ok")]);
    }

    #[test]
    fn filter_existing_dirs_keeps_only_directories() {
        let existing = std::env::temp_dir();
        assert!(
            existing.is_dir(),
            "std::env::temp_dir must exist for this filter test"
        );
        let missing = unique_temp_subdir("missing");
        assert!(!missing.exists(), "unique missing path must not exist yet");
        let filtered = filter_existing_dirs([existing.clone(), missing]);
        assert_eq!(filtered, vec![existing]);
    }

    #[test]
    fn parse_library_folders_read_parse_filter() {
        let dir = unique_temp_subdir("vdf");
        fs::create_dir_all(&dir).expect("create unique temp dir");
        let vdf = dir.join("libraryfolders.vdf");
        let escaped = dir.display().to_string().replace('\\', "\\\\");
        let missing = unique_temp_subdir("absent_lib");
        let missing_escaped = missing.display().to_string().replace('\\', "\\\\");
        let body = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{escaped}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{missing_escaped}\"\n\t}}\n}}\n"
        );
        fs::write(&vdf, body).expect("write vdf");
        let libs = parse_library_folders(&vdf).expect("parse_library_folders");
        assert_eq!(libs, vec![dir.clone()]);
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
