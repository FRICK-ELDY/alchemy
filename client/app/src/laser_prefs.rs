//! Interaction laser prefs (x00ee L1).
//!
//! Guarantees: local JSON for [`InteractionLaserConfig`]; missing keys → Default;
//! coalesced background write (no XR/Desktop hitch on ± spam); spawn failure falls
//! back to sync drain (never leaves `WRITER_LIVE` stuck); [`flush_blocking`] for exit
//! (suppresses kick while draining); no secrets.
//! Non-guarantees: Vault sync; Room CRDT; Desktop beam overlay (→ L3).

use crate::spatial_lens::InteractionLaserConfig;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;

static PREFS_IO_LOCK: Mutex<()> = Mutex::new(());
/// Latest cfg waiting for disk; writer thread coalesces rapid `save` calls.
static PENDING_SAVE: Mutex<Option<InteractionLaserConfig>> = Mutex::new(None);
/// Only the active writer (or spawn-fail owner) clears this after claiming it.
static WRITER_LIVE: AtomicBool = AtomicBool::new(false);
static WRITER_JOIN: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
/// Exit / test flush: queue still accepted, but [`kick_writer`] is suppressed.
static FLUSHING: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
static TEST_PREFS_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
#[cfg(test)]
static FAIL_NEXT_SPAWN: AtomicBool = AtomicBool::new(false);

fn prefs_io_lock() -> MutexGuard<'static, ()> {
    PREFS_IO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn prefs_path() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(path) = TEST_PREFS_PATH
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            return path;
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("Alchemy").join("laser.json");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("alchemy")
            .join("laser.json");
    }
    PathBuf::from("alchemy_laser.json")
}

fn load_from_path(path: &Path) -> InteractionLaserConfig {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<InteractionLaserConfig>(&raw)
            .unwrap_or_default()
            .clamp(),
        Err(_) => InteractionLaserConfig::default(),
    }
}

fn save_to_path(path: &Path, cfg: &InteractionLaserConfig) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let clamped = cfg.clamp();
    match serde_json::to_string_pretty(&clamped) {
        Ok(raw) => {
            if let Err(err) = fs::write(path, raw) {
                log::warn!("laser prefs save failed: {err}");
            }
        }
        Err(err) => log::warn!("laser prefs encode failed: {err}"),
    }
}

fn load_unlocked() -> InteractionLaserConfig {
    load_from_path(&prefs_path())
}

fn save_unlocked(cfg: &InteractionLaserConfig) {
    save_to_path(&prefs_path(), cfg);
}

fn take_pending() -> Option<InteractionLaserConfig> {
    PENDING_SAVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

fn pending_is_some() -> bool {
    PENDING_SAVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
}

fn queue_pending(cfg: InteractionLaserConfig) {
    let mut pending = PENDING_SAVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *pending = Some(cfg);
}

fn join_slot_is_some() -> bool {
    WRITER_JOIN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
}

/// Sync-write every pending cfg (does **not** touch `WRITER_LIVE`).
fn drain_pending_writes_sync() {
    while let Some(cfg) = take_pending() {
        let _g = prefs_io_lock();
        save_unlocked(&cfg);
    }
}

/// Spawn-fail owner path: drain disk + release the LIVE claim we hold.
fn drain_pending_sync_release_live() {
    drain_pending_writes_sync();
    WRITER_LIVE.store(false, Ordering::Release);
}

fn writer_loop() {
    loop {
        match take_pending() {
            Some(cfg) => {
                let _g = prefs_io_lock();
                save_unlocked(&cfg);
            }
            None => {
                WRITER_LIVE.store(false, Ordering::Release);
                // Race: a save may have queued after take() and before clear.
                if FLUSHING.load(Ordering::Acquire) {
                    break;
                }
                if pending_is_some()
                    && WRITER_LIVE
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    continue;
                }
                break;
            }
        }
    }
}

fn kick_writer() {
    if FLUSHING.load(Ordering::Acquire) {
        return;
    }
    if WRITER_LIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    // Lost the race to flush after CAS: release and let flush drain.
    if FLUSHING.load(Ordering::Acquire) {
        drain_pending_sync_release_live();
        return;
    }

    #[cfg(test)]
    if FAIL_NEXT_SPAWN.swap(false, Ordering::AcqRel) {
        log::warn!("laser prefs writer spawn forced-fail (test); sync drain");
        drain_pending_sync_release_live();
        return;
    }

    match std::thread::Builder::new()
        .name("alchemy-laser-prefs".into())
        .spawn(writer_loop)
    {
        Ok(handle) => {
            let prev = {
                let mut slot = WRITER_JOIN
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let prev = slot.take();
                *slot = Some(handle);
                prev
            };
            // Join prior handle outside the mutex (flush may contend on the slot).
            if let Some(prev) = prev {
                let _ = prev.join();
            }
        }
        Err(err) => {
            log::warn!("laser prefs writer spawn failed ({err}); sync drain");
            drain_pending_sync_release_live();
        }
    }
}

pub fn load() -> InteractionLaserConfig {
    let _g = prefs_io_lock();
    load_unlocked()
}

/// Queue a coalesced background write (safe to call from XR session thread on ± spam).
pub fn save(cfg: &InteractionLaserConfig) {
    queue_pending(cfg.clamp());
    kick_writer();
}

/// Block until the writer finishes and any leftover pending is on disk (process exit).
pub fn flush_blocking() {
    FLUSHING.store(true, Ordering::Release);
    loop {
        let handle = WRITER_JOIN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        drain_pending_writes_sync();
        // Writer that CAS'd before FLUSHING: wait until it drops LIVE / publishes join.
        if WRITER_LIVE.load(Ordering::Acquire) || join_slot_is_some() || pending_is_some() {
            std::thread::yield_now();
            continue;
        }
        break;
    }
    // Last drain under FLUSHING: covers saves that queued after the quiesce check.
    drain_pending_writes_sync();
    WRITER_LIVE.store(false, Ordering::Release);
    FLUSHING.store(false, Ordering::Release);
    // Race after FLUSHING=false: sync-drain so exit flush does not rely on a new writer.
    // Steady-state later saves will kick_writer on their own.
    drain_pending_writes_sync();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that touch process-global writer / path override state.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_test_path(path: PathBuf, f: impl FnOnce()) {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        flush_blocking();
        *TEST_PREFS_PATH.lock().unwrap_or_else(|p| p.into_inner()) = Some(path);
        FAIL_NEXT_SPAWN.store(false, Ordering::Release);
        f();
        flush_blocking();
        *TEST_PREFS_PATH.lock().unwrap_or_else(|p| p.into_inner()) = None;
        FAIL_NEXT_SPAWN.store(false, Ordering::Release);
    }

    #[test]
    fn roundtrip_persists_show_in_desktop() {
        let dir = std::env::temp_dir().join(format!(
            "alchemy-laser-prefs-{}-{:?}-rt",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("laser.json");
        with_test_path(path.clone(), || {
            let mut cfg = InteractionLaserConfig::default();
            cfg.show_lasers_in_desktop_mode = true;
            cfg.laser_show_primary_min = 0.35;
            save(&cfg);
            flush_blocking();
            let loaded = load();
            assert!(loaded.show_lasers_in_desktop_mode);
            assert!((loaded.laser_show_primary_min - 0.35).abs() < 1e-5);
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = std::env::temp_dir().join(format!(
            "alchemy-laser-prefs-{}-{:?}-miss",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("missing-laser.json");
        with_test_path(path, || {
            assert_eq!(load(), InteractionLaserConfig::default());
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn coalesce_keeps_latest_after_flush() {
        let dir = std::env::temp_dir().join(format!(
            "alchemy-laser-prefs-{}-{:?}-coal",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("laser.json");
        with_test_path(path, || {
            let mut a = InteractionLaserConfig::default();
            a.laser_show_primary_min = 0.2;
            let mut b = InteractionLaserConfig::default();
            b.laser_show_primary_min = 0.8;
            b.show_lasers_in_desktop_mode = true;
            save(&a);
            save(&b);
            flush_blocking();
            let loaded = load();
            assert!((loaded.laser_show_primary_min - 0.8).abs() < 1e-5);
            assert!(loaded.show_lasers_in_desktop_mode);
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn spawn_fail_falls_back_to_sync_and_allows_later_saves() {
        let dir = std::env::temp_dir().join(format!(
            "alchemy-laser-prefs-{}-{:?}-spawnfail",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("laser.json");
        with_test_path(path, || {
            FAIL_NEXT_SPAWN.store(true, Ordering::Release);
            let mut cfg = InteractionLaserConfig::default();
            cfg.laser_stickiness = 0.9;
            save(&cfg);
            // spawn-fail path drains sync; WRITER_LIVE must not stick.
            assert!(!WRITER_LIVE.load(Ordering::Acquire));
            let loaded = load();
            assert!((loaded.laser_stickiness - 0.9).abs() < 1e-5);

            // Later saves must still work.
            cfg.laser_stickiness = 0.1;
            save(&cfg);
            flush_blocking();
            let loaded2 = load();
            assert!((loaded2.laser_stickiness - 0.1).abs() < 1e-5);
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_during_flush_still_lands_on_disk() {
        let dir = std::env::temp_dir().join(format!(
            "alchemy-laser-prefs-{}-{:?}-flushsave",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("laser.json");
        with_test_path(path, || {
            let mut cfg = InteractionLaserConfig::default();
            cfg.laser_smooth_speed = 20.0;
            save(&cfg);
            FLUSHING.store(true, Ordering::Release);
            cfg.laser_smooth_speed = 33.0;
            save(&cfg); // kick suppressed; pending kept
            assert!(pending_is_some());
            // Real exit path sets FLUSHING inside flush_blocking; here we only
            // prove queue-without-kick, then a normal flush drains it.
            FLUSHING.store(false, Ordering::Release);
            flush_blocking();
            let loaded = load();
            assert!((loaded.laser_smooth_speed - 33.0).abs() < 1e-5);
        });
        let _ = fs::remove_dir_all(&dir);
    }
}
