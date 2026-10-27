//! alchemy-launcher — minimal process manager (x0032 / P4-4).
//!
//! Guarantees:
//! - Spawn host-server then client after `ALCHEMY_HOST_READY_FILE` (includes bound IPC addr)
//! - Exit Client only: client dies, host kept
//! - Exit Client+Host / IPC SHUTDOWN: host exits 0 → launcher stops (no crash-restart)
//! - Host crash (non-zero): bounded backoff restart; Ctrl+C → IPC shutdown then exit
//!
//! Non-guarantees: alchemy:// deep links, tray UI, host handoff, Zenoh liveliness watch,
//! Compose integration, installer.

use alchemy_protocol::host_ipc::{self, ENV_ADDR, ENV_TOKEN};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MAX_HOST_RESTARTS: u32 = 3;
const READY_TIMEOUT: Duration = Duration::from_secs(180);

fn main() {
    if let Err(err) = run() {
        log::error!("{err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let client_dir = resolve_client_dir();
    log::info!("alchemy-launcher x0032; client_dir={}", client_dir.display());

    let shutting_down = Arc::new(AtomicBool::new(false));
    {
        let flag = Arc::clone(&shutting_down);
        if let Err(err) = ctrlc::set_handler(move || {
            flag.store(true, Ordering::SeqCst);
        }) {
            log::warn!("ctrlc handler unavailable: {err}");
        }
    }

    let ipc_token = make_token()?;
    let ready_path = std::env::temp_dir().join(format!(
        "alchemy-host-ready-{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&ready_path);

    // Host binds 127.0.0.1:0 itself; addr is published in the ready file (no TOCTOU).
    let mut host_restarts = 0u32;
    let mut host = spawn_host(&client_dir, &ipc_token, &ready_path).context("spawn host-server")?;
    log::info!("host-server pid={}", host.id());

    let mut ipc_addr = match wait_ready(&ready_path, &mut host, &shutting_down) {
        Ok(addr) => addr,
        Err(err) => {
            let _ = host.kill();
            return Err(err);
        }
    };
    log::info!("host IPC addr from ready file: {ipc_addr}");

    let mut client = Some(
        spawn_client(&client_dir, &ipc_addr, &ipc_token).context("spawn client")?,
    );
    if let Some(ref c) = client {
        log::info!("client pid={}", c.id());
    }

    while !shutting_down.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));

        if let Some(ref mut c) = client {
            match c.try_wait() {
                Ok(Some(status)) => {
                    log::info!("client exited ({status}); host kept running");
                    client = None;
                }
                Ok(None) => {}
                Err(err) => log::warn!("client try_wait: {err}"),
            }
        }

        match host.try_wait() {
            Ok(Some(status)) => {
                if shutting_down.load(Ordering::SeqCst) {
                    log::info!("host exited during shutdown ({status})");
                    break;
                }
                if is_intentional_host_stop(status) {
                    // Exit Client+Host / authenticated IPC: exit 0 — do not restart.
                    log::info!(
                        "host stopped intentionally ({status}); launcher exiting (no restart)"
                    );
                    if let Some(mut c) = client.take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                    let _ = fs::remove_file(&ready_path);
                    log::info!("launcher exit");
                    return Ok(());
                }
                host_restarts += 1;
                if host_restarts > MAX_HOST_RESTARTS {
                    log::error!(
                        "host crashed ({status}); restart budget exhausted ({MAX_HOST_RESTARTS})"
                    );
                    if let Some(mut c) = client.take() {
                        let _ = c.kill();
                    }
                    bail!("host restart budget exhausted");
                }
                let backoff = Duration::from_secs(1u64 << (host_restarts.min(3) - 1));
                log::warn!(
                    "host crashed ({status}); restart {host_restarts}/{MAX_HOST_RESTARTS} after {backoff:?}"
                );
                thread::sleep(backoff);
                let _ = fs::remove_file(&ready_path);
                host = spawn_host(&client_dir, &ipc_token, &ready_path)
                    .context("respawn host-server")?;
                log::info!("host-server restarted pid={}", host.id());
                match wait_ready(&ready_path, &mut host, &shutting_down) {
                    Ok(addr) => {
                        // Launcher Ctrl+C uses the fresh addr. Client process still has the
                        // old ALCHEMY_HOST_IPC_ADDR — Exit Client+Host after restart is
                        // non-guaranteed (see architecture/launcher.md).
                        log::info!("host IPC addr after restart: {addr}");
                        ipc_addr = addr;
                    }
                    Err(err) => {
                        log::error!("host restart ready wait failed: {err:#}");
                        shutting_down.store(true, Ordering::SeqCst);
                    }
                }
            }
            Ok(None) => {}
            Err(err) => log::warn!("host try_wait: {err}"),
        }
    }

    log::info!("launcher shutting down — IPC stop host");
    if let Err(err) = host_ipc::request_shutdown(&ipc_addr, &ipc_token) {
        log::warn!("IPC shutdown: {err}; killing host");
        let _ = host.kill();
    }
    let _ = host.wait();
    if let Some(mut c) = client.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = fs::remove_file(&ready_path);
    log::info!("launcher exit");
    Ok(())
}

/// Authenticated IPC / clean main-loop leave uses process exit code 0.
fn is_intentional_host_stop(status: ExitStatus) -> bool {
    status.success()
}

fn resolve_client_dir() -> PathBuf {
    if let Ok(p) = std::env::var("ALCHEMY_CLIENT_DIR") {
        return PathBuf::from(p);
    }
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    dir.push("client");
    dir
}

fn make_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).context("getrandom for IPC token")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn spawn_host(client_dir: &Path, ipc_token: &str, ready_path: &Path) -> Result<Child> {
    let mut cmd = host_command(client_dir);
    cmd.current_dir(client_dir)
        .env(ENV_TOKEN, ipc_token)
        .env("ALCHEMY_HOST_READY_FILE", ready_path)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // Host binds 127.0.0.1:0; do not pass a pre-picked ADDR (TOCTOU).
    cmd.env_remove(ENV_ADDR);
    cmd.env_remove("ALCHEMY_HOST_CONTROL");
    Ok(cmd.spawn().context("spawn host process")?)
}

fn spawn_client(client_dir: &Path, ipc_addr: &str, ipc_token: &str) -> Result<Child> {
    let mut cmd = client_command(client_dir);
    cmd.current_dir(client_dir)
        .env(ENV_ADDR, ipc_addr)
        .env(ENV_TOKEN, ipc_token)
        .env("ALCHEMY_FORCE_DESKTOP", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    cmd.env_remove("ALCHEMY_HOST_PID");
    cmd.env_remove("ALCHEMY_HOST_CONTROL");
    Ok(cmd.spawn().context("spawn client process")?)
}

fn host_command(client_dir: &Path) -> Command {
    if let Ok(bin) = std::env::var("ALCHEMY_HOST_BIN") {
        return Command::new(bin);
    }
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("-p")
        .arg("host-server")
        .arg("--manifest-path")
        .arg(client_dir.join("Cargo.toml"));
    cmd
}

fn client_command(client_dir: &Path) -> Command {
    if let Ok(bin) = std::env::var("ALCHEMY_CLIENT_BIN") {
        return Command::new(bin);
    }
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("-p")
        .arg("client")
        .arg("--manifest-path")
        .arg(client_dir.join("Cargo.toml"));
    cmd
}

/// Wait for ready file; return bound IPC addr from `ipc=` line.
fn wait_ready(
    ready_path: &Path,
    host: &mut Child,
    shutting_down: &AtomicBool,
) -> Result<String> {
    let start = Instant::now();
    while start.elapsed() < READY_TIMEOUT {
        if shutting_down.load(Ordering::SeqCst) {
            bail!("shutdown requested while waiting for host ready");
        }
        if ready_path.is_file() {
            let body = fs::read_to_string(ready_path).context("read ready file")?;
            let Some(addr) = host_ipc::parse_ready_ipc_addr(&body) else {
                // Host may write ready without IPC when token unset; launcher always sets token.
                thread::sleep(Duration::from_millis(50));
                if start.elapsed() > Duration::from_secs(5) {
                    bail!("ready file missing ipc= line (host IPC not bound?)");
                }
                continue;
            };
            if !host_ipc::is_loopback_addr(&addr) {
                bail!("ready ipc addr not loopback: {addr}");
            }
            log::info!("host ready file present: {}", ready_path.display());
            return Ok(addr);
        }
        match host.try_wait().context("host try_wait")? {
            Some(status) => bail!("host exited before ready ({status})"),
            None => thread::sleep(Duration::from_millis(200)),
        }
    }
    bail!(
        "timed out waiting for ALCHEMY_HOST_READY_FILE ({})",
        ready_path.display()
    )
}
