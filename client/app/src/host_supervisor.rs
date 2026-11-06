//! On-demand local `host-server` child (replaces alchemy-launcher PM).
//!
//! Guarantees:
//! - Spawn only on host path ([`HostSupervisor::ensure_running`]); Join leaves it idle
//! - Ready file `ipc=127.0.0.1:port` after host IPC bind (not after first frame)
//! - Windows: Job Object `KILL_ON_JOB_CLOSE` when child is the real host binary
//! - Linux: `PR_SET_PDEATHSIG` on the spawned process image (same caveat for `cargo run`)
//! - Graceful stop: localhost IPC `SHUTDOWN` then wait (exit 0)
//! - `ensure_running` does **not** hold the state mutex across ready-wait; Exit can cancel
//! - [`Self::latch_exit_and_shutdown`] sticks an exit latch that `ensure_running` never clears
//!   (so Exit during Start's HTTP phase cannot be undone by a later ensure clearing cancel)
//!
//! Non-guarantees:
//! - Exit Client only / host kept after client exit (abolished)
//! - host crash auto-restart / launcher discovery LOOKUP
//! - Handover success (see [`request_handover_stub`]; MVP fails → room ends)
//! - macOS hard-kill orphan prevention (Drop best-effort kill only)
//! - Crash 道連れ when spawning via `cargo run -p host-server` (PDEATHSIG/Job attach to
//!   cargo, not the eventual host binary — use `ALCHEMY_HOST_BIN` or adjacent exe)
//! - Leave/Logout do **not** stop the supervised host (only Exit / failed Start / Drop)

use alchemy_protocol::host_ipc::{self, ENV_TOKEN};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
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
///
/// Interior mutex is held only for short state transitions. Ready-wait runs unlocked so
/// [`Self::shutdown_graceful`] can set [`Self::cancel_spawn`] and reclaim the child.
#[derive(Default)]
pub struct HostSupervisor {
    /// Aborts an in-flight Reserved/Starting start (and reclaim). Cleared on a fresh Idle
    /// claim so a failed Start can retry. Exit stickiness lives in [`Self::exit_latched`]
    /// (this flag alone is not sticky — Exit always sets both).
    cancel_spawn: AtomicBool,
    /// Set by [`Self::latch_exit_and_shutdown`] only. Never cleared — blocks later ensure.
    exit_latched: AtomicBool,
    state: Mutex<SupervisorState>,
}

#[derive(Default)]
struct SupervisorState {
    phase: Phase,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    /// Claim taken under lock; OS spawn in progress (no child handle yet).
    Reserved,
    /// Child spawned; ready wait may be in progress on another thread.
    Starting {
        child: Child,
        ipc_token: String,
        ready_path: PathBuf,
        #[cfg(windows)]
        job: JobObject,
    },
    Running {
        child: Child,
        ipc_token: String,
        ipc_addr: String,
        ready_path: PathBuf,
        #[cfg(windows)]
        job: JobObject,
    },
}

impl HostSupervisor {
    pub fn is_running(&self) -> bool {
        let Ok(mut g) = self.state.lock() else {
            return false;
        };
        g.refresh_exited();
        matches!(g.phase, Phase::Running { .. })
    }

    /// True when this supervisor owns Reserved / Starting / Running (not Idle).
    pub fn owns_local_host(&self) -> bool {
        let Ok(mut g) = self.state.lock() else {
            return false;
        };
        g.refresh_exited();
        !matches!(g.phase, Phase::Idle)
    }

    /// True after [`Self::latch_exit_and_shutdown`] (client Exit). Never cleared.
    pub fn is_exit_latched(&self) -> bool {
        self.exit_latched.load(Ordering::SeqCst)
    }

    /// Spawn host-server if not already running; wait until ready file publishes IPC addr.
    ///
    /// Reservation is taken under the state mutex. Ready-wait runs unlocked so
    /// [`Self::shutdown_graceful`] can cancel via [`Self::cancel_spawn`].
    pub fn ensure_running(&self) -> Result<()> {
        if self.exit_latched.load(Ordering::SeqCst) {
            bail!("client exit latched — not starting host");
        }

        // Claim under lock so a second ensure_running cannot orphan a first child.
        {
            let mut g = self.state.lock().expect("host_supervisor state");
            g.refresh_exited();
            match &g.phase {
                Phase::Running { .. } => return Ok(()),
                Phase::Starting { .. } | Phase::Reserved => {
                    bail!("local host-server start already in progress")
                }
                Phase::Idle => {
                    // Re-check latch under lock (Exit may have raced).
                    if self.exit_latched.load(Ordering::SeqCst) {
                        bail!("client exit latched — not starting host");
                    }
                    // Clear in-flight cancel from a prior *failed* Start shutdown only.
                    // exit_latched is never cleared here.
                    self.cancel_spawn.store(false, Ordering::SeqCst);
                    g.phase = Phase::Reserved;
                }
            }
        }

        if self.exit_latched.load(Ordering::SeqCst) || self.cancel_spawn.load(Ordering::SeqCst) {
            let mut g = self.state.lock().expect("host_supervisor state");
            if matches!(g.phase, Phase::Reserved) {
                g.phase = Phase::Idle;
            }
            bail!("local host-server start cancelled");
        }

        let token = make_token();
        let ready_path =
            std::env::temp_dir().join(format!("alchemy-host-ready-{}.txt", std::process::id()));
        let _ = fs::remove_file(&ready_path);

        let finish_fail = |err: anyhow::Error| -> Result<()> {
            let mut g = self.state.lock().expect("host_supervisor state");
            if matches!(g.phase, Phase::Reserved) {
                g.phase = Phase::Idle;
            }
            Err(err)
        };

        #[cfg(windows)]
        let job = match JobObject::create() {
            Ok(j) => j,
            Err(err) => return finish_fail(err),
        };

        let mut child = match spawn_host(&token, &ready_path) {
            Ok(c) => c,
            Err(err) => {
                let _ = fs::remove_file(&ready_path);
                return finish_fail(err.context("spawn host-server"));
            }
        };
        log::info!("host-server spawned pid={}", child.id());

        #[cfg(windows)]
        if let Err(err) = job.assign_pid(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&ready_path);
            return finish_fail(err);
        }

        {
            let mut g = self.state.lock().expect("host_supervisor state");
            if self.exit_latched.load(Ordering::SeqCst)
                || self.cancel_spawn.load(Ordering::SeqCst)
                || !matches!(g.phase, Phase::Reserved)
            {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&ready_path);
                if matches!(g.phase, Phase::Reserved) {
                    g.phase = Phase::Idle;
                }
                bail!("local host-server start cancelled");
            }
            g.phase = Phase::Starting {
                child,
                ipc_token: token.clone(),
                ready_path: ready_path.clone(),
                #[cfg(windows)]
                job,
            };
        }

        // Unlocked wait — shutdown may cancel and/or take the child.
        let wait_result = wait_ready_polling(&ready_path, &self.cancel_spawn, || {
            let mut g = self.state.lock().expect("host_supervisor state");
            match &mut g.phase {
                Phase::Starting { child, .. } => child.try_wait().context("host try_wait"),
                Phase::Idle | Phase::Reserved => bail!("host start cancelled (state cleared)"),
                Phase::Running { .. } => bail!("host became running during wait"),
            }
        });

        let mut g = self.state.lock().expect("host_supervisor state");
        if self.exit_latched.load(Ordering::SeqCst) || self.cancel_spawn.load(Ordering::SeqCst) {
            g.force_kill_clear();
            bail!("local host-server start cancelled");
        }

        let Phase::Starting {
            child,
            ipc_token,
            ready_path,
            #[cfg(windows)]
            job,
        } = std::mem::replace(&mut g.phase, Phase::Idle)
        else {
            return wait_result.and(Err(anyhow::anyhow!(
                "local host-server start cancelled after ready"
            )));
        };

        let ipc_addr = match wait_result {
            Ok(addr) => addr,
            Err(err) => {
                kill_child(child);
                let _ = fs::remove_file(&ready_path);
                #[cfg(windows)]
                drop(job);
                return Err(err);
            }
        };

        if !host_ipc::is_loopback_addr(&ipc_addr) {
            kill_child(child);
            let _ = fs::remove_file(&ready_path);
            #[cfg(windows)]
            drop(job);
            bail!("ready ipc addr not loopback: {ipc_addr}");
        }

        g.phase = Phase::Running {
            child,
            ipc_token,
            ipc_addr: ipc_addr.clone(),
            ready_path,
            #[cfg(windows)]
            job,
        };
        log::info!("host-server ready; ipc={ipc_addr}");
        Ok(())
    }

    /// Client Exit: sticky latch + cancel + stop. Later [`Self::ensure_running`] will refuse.
    pub fn latch_exit_and_shutdown(&self) {
        self.exit_latched.store(true, Ordering::SeqCst);
        self.shutdown_graceful();
    }

    /// Always sets cancel first (so Reserved/Starting abort), then stops any owned child.
    /// Does **not** set [`Self::exit_latched`] — failed Start may retry ensure afterward.
    /// IPC wait runs **outside** the state mutex.
    pub fn shutdown_graceful(&self) {
        self.cancel_spawn.store(true, Ordering::SeqCst);
        let taken = {
            let mut g = self.state.lock().expect("host_supervisor state");
            std::mem::replace(&mut g.phase, Phase::Idle)
        };
        match taken {
            Phase::Idle | Phase::Reserved => {}
            Phase::Starting {
                mut child,
                ready_path,
                #[cfg(windows)]
                job,
                ..
            } => {
                log::info!("host supervisor: cancelling in-flight start");
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(ready_path);
                #[cfg(windows)]
                drop(job);
            }
            Phase::Running {
                mut child,
                ipc_token,
                ipc_addr,
                ready_path,
                #[cfg(windows)]
                job,
            } => {
                if host_ipc::is_loopback_addr(&ipc_addr) {
                    match host_ipc::request_shutdown(&ipc_addr, &ipc_token) {
                        Ok(()) => log::info!("host supervisor: IPC shutdown → {ipc_addr}"),
                        Err(err) => log::warn!("host supervisor: IPC shutdown failed: {err}"),
                    }
                }
                wait_child_exit(&mut child, Duration::from_secs(10));
                let _ = fs::remove_file(ready_path);
                #[cfg(windows)]
                drop(job);
            }
        }
    }
}

impl Drop for HostSupervisor {
    fn drop(&mut self) {
        self.exit_latched.store(true, Ordering::SeqCst);
        self.cancel_spawn.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.state.lock() {
            g.force_kill_clear();
        }
    }
}

impl SupervisorState {
    fn refresh_exited(&mut self) {
        let exited = match &mut self.phase {
            Phase::Starting { child, .. } | Phase::Running { child, .. } => {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        log::warn!("local host-server exited unexpectedly ({status})");
                        true
                    }
                    Ok(None) => false,
                    Err(err) => {
                        log::warn!("host try_wait: {err}");
                        false
                    }
                }
            }
            Phase::Idle | Phase::Reserved => false,
        };
        if exited {
            self.force_kill_clear();
        }
    }

    fn force_kill_clear(&mut self) {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle | Phase::Reserved => {}
            Phase::Starting {
                mut child,
                ready_path,
                #[cfg(windows)]
                job,
                ..
            } => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(ready_path);
                #[cfg(windows)]
                drop(job);
            }
            Phase::Running {
                mut child,
                ready_path,
                #[cfg(windows)]
                job,
                ..
            } => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(ready_path);
                #[cfg(windows)]
                drop(job);
            }
        }
    }
}

fn kill_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_child_exit(child: &mut Child, budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                log::info!("host-server exited after shutdown ({status})");
                return;
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) | Err(_) => {
                log::warn!("host-server did not exit in time; killing");
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

fn make_token() -> String {
    // Local IPC secret only. blake3 mixes pid / time / thread / stack address.
    // Cryptographic `getrandom` can replace this once a direct dependency is approved.
    let anchor = 0u8;
    let mix = format!(
        "alchemy-host-ipc-{}-{:?}-{:?}-{:p}",
        std::process::id(),
        Instant::now(),
        std::thread::current().id(),
        &anchor as *const u8
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
    cmd.env_remove(alchemy_protocol::host_ipc::ENV_ADDR);
    cmd.env_remove("ALCHEMY_HOST_CONTROL");
    cmd.env_remove(alchemy_protocol::host_ipc::ENV_DISCOVERY_ADDR);

    #[cfg(target_os = "linux")]
    {
        // SAFETY: runs in the forked child before exec; PR_SET_PDEATHSIG survives execve
        // of the *immediate* image (cargo when using `cargo run`, not the eventual host).
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
    log::warn!(
        "spawning host via `cargo run` — crash 道連れ (Job/PDEATHSIG) attaches to cargo, not host-server; set ALCHEMY_HOST_BIN for full guarantee"
    );
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

fn wait_ready_polling(
    ready_path: &Path,
    cancel: &AtomicBool,
    mut try_wait_host: impl FnMut() -> Result<Option<std::process::ExitStatus>>,
) -> Result<String> {
    let start = Instant::now();
    while start.elapsed() < READY_TIMEOUT {
        if cancel.load(Ordering::SeqCst) {
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
        match try_wait_host()? {
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
        winjob::close(self.handle);
        self.handle = std::ptr::null_mut();
    }
}

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
            bail!(
                "CreateJobObjectW failed: {}",
                std::io::Error::last_os_error()
            );
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
        let s = HostSupervisor::default();
        s.shutdown_graceful();
        assert!(!s.is_running());
        assert!(!s.owns_local_host());
        assert!(!s.is_exit_latched());
    }

    #[test]
    fn exit_latch_blocks_ensure_even_after_cancel_would_clear() {
        let s = HostSupervisor::default();
        // Simulate Exit during Start HTTP (before ensure): latch then a later ensure.
        s.latch_exit_and_shutdown();
        assert!(s.is_exit_latched());
        let err = s
            .ensure_running()
            .expect_err("latched exit must block ensure");
        assert!(
            err.to_string().contains("exit latched"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn failed_start_shutdown_does_not_latch() {
        let s = HostSupervisor::default();
        s.shutdown_graceful();
        assert!(
            !s.is_exit_latched(),
            "failed-Start shutdown must allow a later ensure (cancel may clear; latch must not)"
        );
    }
}
