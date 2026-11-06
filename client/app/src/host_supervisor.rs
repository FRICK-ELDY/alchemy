//! On-demand local `host-server` child (replaces alchemy-launcher PM).
//!
//! Guarantees:
//! - Spawn only on host path ([`HostSupervisor::ensure_running`]); Join leaves it idle
//! - Ready file `ipc=127.0.0.1:port` (host binds ephemeral; no TOCTOU)
//! - Publish `ALCHEMY_HOST_IPC_TOKEN` / `ALCHEMY_HOST_IPC_ADDR` for shutdown helpers
//! - Windows: Job Object with `KILL_ON_JOB_CLOSE` (client crash → host dies)
//! - Linux: `PR_SET_PDEATHSIG` in child pre-exec (same)
//! - Graceful stop: localhost IPC `SHUTDOWN` then wait (exit 0)
//!
//! Non-guarantees:
//! - Exit Client only / host kept after client exit (abolished)
//! - host crash auto-restart / launcher discovery LOOKUP
//! - Handover success (see [`request_handover_stub`]; MVP fails → room ends)
//! - macOS hard-kill orphan prevention (Drop best-effort kill only)

use alchemy_protocol::host_ipc::{self, ENV_ADDR, ENV_TOKEN};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(180);

/// Best-effort host authority handoff before local host stop.
///
/// MVP: log only. Failure contract = room ends with host shutdown (no hang).
/// Full 0s migration lives with world-server verifier (P4).
pub fn request_handover_stub() {
    log::info!(
        "host handover stub: no world-server candidates yet — local host will stop (room ends)"
    );
}

/// Process-manager for a single local host-server child.
#[derive(Default)]
pub struct HostSupervisor {
    child: Option<Child>,
    ipc_token: Option<String>,
    ipc_addr: Option<String>,
    ready_path: Option<PathBuf>,
    #[cfg(windows)]
    job: Option<JobObject>,
}

impl HostSupervisor {
    pub fn is_running(&mut self) -> bool {
        let Some(child) = self.child.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                log::warn!("local host-server exited unexpectedly ({status})");
                self.clear_after_exit();
                false
            }
            Err(err) => {
                log::warn!("host try_wait: {err}");
                true
            }
        }
    }

    /// Spawn host-server if not already running; wait until ready file publishes IPC addr.
    pub fn ensure_running(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }
        self.clear_after_exit();

        let token = make_token();
        let ready_path =
            std::env::temp_dir().join(format!("alchemy-host-ready-{}.txt", std::process::id()));
        let _ = fs::remove_file(&ready_path);

        #[cfg(windows)]
        let job = JobObject::create()?;

        let mut child = spawn_host(&token, &ready_path).context("spawn host-server")?;
        log::info!("host-server spawned pid={}", child.id());

        #[cfg(windows)]
        {
            job.assign_pid(child.id())?;
        }

        let cancel = AtomicBool::new(false);
        let ipc_addr = match wait_ready(&ready_path, &mut child, &cancel) {
            Ok(addr) => addr,
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&ready_path);
                return Err(err);
            }
        };

        // Process-wide so `host_control` IPC path and child see the same secret/addr.
        // SAFETY: single-threaded wrt other ensure_running callers (Mutex on SharedState).
        std::env::set_var(ENV_TOKEN, &token);
        std::env::set_var(ENV_ADDR, &ipc_addr);
        // Discovery was launcher-only; clear stale LOOKUP targets from older sessions.
        std::env::remove_var(alchemy_protocol::host_ipc::ENV_DISCOVERY_ADDR);

        self.child = Some(child);
        self.ipc_token = Some(token);
        self.ipc_addr = Some(ipc_addr.clone());
        self.ready_path = Some(ready_path);
        #[cfg(windows)]
        {
            self.job = Some(job);
        }
        log::info!("host-server ready; ipc={ipc_addr}");
        Ok(())
    }

    /// IPC SHUTDOWN then wait. No-op when idle. Clears env token/addr when done.
    pub fn shutdown_graceful(&mut self) {
        if !self.is_running() && self.child.is_none() {
            return;
        }
        let token = self.ipc_token.clone();
        let addr = self.ipc_addr.clone();
        if let (Some(token), Some(addr)) = (token.as_deref(), addr.as_deref()) {
            if host_ipc::is_loopback_addr(addr) {
                match host_ipc::request_shutdown(addr, token) {
                    Ok(()) => log::info!("host supervisor: IPC shutdown → {addr}"),
                    Err(err) => log::warn!("host supervisor: IPC shutdown failed: {err}"),
                }
            }
        }
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        log::info!("host-server exited after shutdown ({status})");
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Ok(None) | Err(_) => {
                        log::warn!("host-server did not exit in time; killing");
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        self.clear_after_exit();
    }

    fn clear_after_exit(&mut self) {
        if let Some(path) = self.ready_path.take() {
            let _ = fs::remove_file(path);
        }
        self.child = None;
        self.ipc_token = None;
        self.ipc_addr = None;
        #[cfg(windows)]
        {
            self.job = None;
        }
        // Avoid leaving a stale token that could SHUTDOWN an unrelated later host.
        if std::env::var_os(ENV_TOKEN).is_some() {
            std::env::remove_var(ENV_TOKEN);
        }
        if std::env::var_os(ENV_ADDR).is_some() {
            std::env::remove_var(ENV_ADDR);
        }
    }
}

impl Drop for HostSupervisor {
    fn drop(&mut self) {
        if self.is_running() {
            log::warn!("HostSupervisor dropped with live host — killing");
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.clear_after_exit();
    }
}

fn make_token() -> String {
    let mix = format!(
        "alchemy-host-ipc-{}-{:?}",
        std::process::id(),
        Instant::now()
    );
    let hex = blake3::hash(mix.as_bytes()).to_hex();
    hex[..32].to_string()
}

fn spawn_host(ipc_token: &str, ready_path: &Path) -> Result<Child> {
    let mut cmd = host_command();
    cmd.env(ENV_TOKEN, ipc_token)
        .env("ALCHEMY_HOST_READY_FILE", ready_path)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    cmd.env_remove(ENV_ADDR);
    cmd.env_remove("ALCHEMY_HOST_CONTROL");
    cmd.env_remove(alchemy_protocol::host_ipc::ENV_DISCOVERY_ADDR);

    #[cfg(target_os = "linux")]
    {
        // SAFETY: runs in the forked child before exec; PR_SET_PDEATHSIG survives execve.
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                let rc = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                if rc != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    cmd.spawn().context("spawn host process")
}

fn host_command() -> Command {
    if let Ok(bin) = std::env::var("ALCHEMY_HOST_BIN") {
        return Command::new(bin);
    }
    // Packaged next to client.exe
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(if cfg!(windows) {
                "host-server.exe"
            } else {
                "host-server"
            });
            if sibling.is_file() {
                return Command::new(sibling);
            }
        }
    }
    let client_dir = resolve_client_dir();
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("-p")
        .arg("host-server")
        .arg("--manifest-path")
        .arg(client_dir.join("Cargo.toml"))
        .current_dir(&client_dir);
    cmd
}

fn resolve_client_dir() -> PathBuf {
    if let Ok(p) = std::env::var("ALCHEMY_CLIENT_DIR") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn wait_ready(ready_path: &Path, host: &mut Child, shutting_down: &AtomicBool) -> Result<String> {
    let start = Instant::now();
    while start.elapsed() < READY_TIMEOUT {
        if shutting_down.load(Ordering::SeqCst) {
            bail!("shutdown requested while waiting for host ready");
        }
        if ready_path.is_file() {
            let body = fs::read_to_string(ready_path).context("read ready file")?;
            let Some(addr) = host_ipc::parse_ready_ipc_addr(&body) else {
                std::thread::sleep(Duration::from_millis(50));
                if start.elapsed() > Duration::from_secs(5) {
                    bail!("ready file missing ipc= line (host IPC not bound?)");
                }
                continue;
            };
            if !host_ipc::is_loopback_addr(&addr) {
                bail!("ready ipc addr not loopback: {addr}");
            }
            return Ok(addr);
        }
        match host.try_wait().context("host try_wait")? {
            Some(status) => bail!("host exited before ready ({status})"),
            None => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    bail!(
        "timed out waiting for ALCHEMY_HOST_READY_FILE ({})",
        ready_path.display()
    )
}

/// Windows Job Object that kills members when the last handle closes (parent crash).
#[cfg(windows)]
struct JobObject {
    handle: winjob::Handle,
}

// SAFETY: exclusive ownership of the job HANDLE; CloseHandle/assign are thread-safe for our use.
#[cfg(windows)]
unsafe impl Send for JobObject {}

#[cfg(windows)]
impl JobObject {
    fn create() -> Result<Self> {
        Ok(Self {
            handle: winjob::create_kill_on_close_job()
                .context("create Windows Job Object for host-server")?,
        })
    }

    fn assign_pid(&self, pid: u32) -> Result<()> {
        winjob::assign_pid(self.handle, pid).context("AssignProcessToJobObject(host)")
    }
}

#[cfg(windows)]
impl Drop for JobObject {
    fn drop(&mut self) {
        // Closing the last handle with KILL_ON_JOB_CLOSE terminates members.
        winjob::close(self.handle);
        self.handle = std::ptr::null_mut();
    }
}

/// Minimal kernel32 Job Object bindings (layout-compatible; names kept Win32-shaped).
#[cfg(windows)]
#[allow(
    non_snake_case,
    non_camel_case_types,
    clippy::upper_case_acronyms,
    dead_code
)]
mod winjob {
    use anyhow::{bail, Result};
    use std::mem;

    pub type Handle = *mut core::ffi::c_void;
    type BOOL = i32;
    type DWORD = u32;

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x0000_2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: DWORD = 9;
    const PROCESS_TERMINATE: DWORD = 0x0001;
    const PROCESS_SET_QUOTA: DWORD = 0x0100;
    const SYNCHRONIZE: DWORD = 0x0010_0000;

    #[repr(C)]
    struct IO_COUNTERS {
        ReadOperationCount: u64,
        WriteOperationCount: u64,
        OtherOperationCount: u64,
        ReadTransferCount: u64,
        WriteTransferCount: u64,
        OtherTransferCount: u64,
    }

    #[repr(C)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        PerProcessUserTimeLimit: i64,
        PerJobUserTimeLimit: i64,
        LimitFlags: DWORD,
        MinimumWorkingSetSize: usize,
        MaximumWorkingSetSize: usize,
        ActiveProcessLimit: DWORD,
        Affinity: usize,
        PriorityClass: DWORD,
        SchedulingClass: DWORD,
    }

    #[repr(C)]
    struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION,
        IoInfo: IO_COUNTERS,
        ProcessMemoryLimit: usize,
        JobMemoryLimit: usize,
        PeakProcessMemoryUsed: usize,
        PeakJobMemoryUsed: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *mut core::ffi::c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            info_class: DWORD,
            info: *mut core::ffi::c_void,
            info_len: DWORD,
        ) -> BOOL;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> BOOL;
        fn OpenProcess(access: DWORD, inherit: BOOL, pid: DWORD) -> Handle;
        fn CloseHandle(handle: Handle) -> BOOL;
    }

    pub fn create_kill_on_close_job() -> Result<Handle> {
        // SAFETY: CreateJobObjectW with null name; failure → null handle.
        let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if handle.is_null() {
            bail!("CreateJobObjectW failed: {}", std::io::Error::last_os_error());
        }
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                PerProcessUserTimeLimit: 0,
                PerJobUserTimeLimit: 0,
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                MinimumWorkingSetSize: 0,
                MaximumWorkingSetSize: 0,
                ActiveProcessLimit: 0,
                Affinity: 0,
                PriorityClass: 0,
                SchedulingClass: 0,
            },
            IoInfo: IO_COUNTERS {
                ReadOperationCount: 0,
                WriteOperationCount: 0,
                OtherOperationCount: 0,
                ReadTransferCount: 0,
                WriteTransferCount: 0,
                OtherTransferCount: 0,
            },
            ProcessMemoryLimit: 0,
            JobMemoryLimit: 0,
            PeakProcessMemoryUsed: 0,
            PeakJobMemoryUsed: 0,
        };
        // SAFETY: valid job handle; info matches JobObjectExtendedLimitInformation layout.
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &mut info as *mut _ as *mut _,
                mem::size_of_val(&info) as u32,
            )
        };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            // SAFETY: handle from CreateJobObjectW on this failure path.
            unsafe { CloseHandle(handle) };
            bail!("SetInformationJobObject(KILL_ON_JOB_CLOSE): {err}");
        }
        Ok(handle)
    }

    pub fn assign_pid(job: Handle, pid: u32) -> Result<()> {
        // SAFETY: OpenProcess with SYNCHRONIZE|PROCESS_SET_QUOTA|PROCESS_TERMINATE for job assign.
        let proc =
            unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE | SYNCHRONIZE, 0, pid) };
        if proc.is_null() {
            bail!(
                "OpenProcess(host pid={pid}): {}",
                std::io::Error::last_os_error()
            );
        }
        // SAFETY: both handles valid; AssignProcessToJobObject.
        let ok = unsafe { AssignProcessToJobObject(job, proc) };
        let assign_err = if ok == 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };
        // SAFETY: process handle from OpenProcess above.
        unsafe { CloseHandle(proc) };
        if let Some(err) = assign_err {
            bail!("AssignProcessToJobObject: {err}");
        }
        Ok(())
    }

    pub fn close(handle: Handle) {
        if !handle.is_null() {
            // SAFETY: handle owned by caller.
            unsafe { CloseHandle(handle) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_token_is_hex32() {
        let t = make_token();
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn handover_stub_does_not_panic() {
        request_handover_stub();
    }

    #[test]
    fn idle_shutdown_is_noop() {
        let mut s = HostSupervisor::default();
        s.shutdown_graceful();
        assert!(!s.is_running());
    }
}
