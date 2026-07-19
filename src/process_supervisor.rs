//! Reusable child-process supervision for workloads that may abort the process.
//!
//! V8 fatal errors cannot be recovered with Rust unwinding. Callers that execute
//! untrusted JavaScript should put that work in a child process and supervise it
//! with this module. The coverage runner is the first consumer; MCP and long-lived
//! server sessions can adopt the same boundary without changing this API.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub stdin: Vec<u8>,
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    /// Address-space ceiling on Linux. Zero disables the OS-level limit.
    pub memory_limit_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    Exited { code: i32 },
    Signaled { signal: i32 },
    TimedOut,
}

#[derive(Debug)]
pub struct ProcessOutput {
    pub outcome: ProcessOutcome,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("failed to spawn supervised worker: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("failed while waiting for supervised worker: {0}")]
    Wait(#[source] std::io::Error),
    #[error("failed to collect supervised worker output: {0}")]
    Output(#[source] std::io::Error),
    #[error("supervised worker was missing its piped {0}")]
    MissingPipe(&'static str),
    #[error("supervisor task failed: {0}")]
    Task(String),
}

struct BoundedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn drain_bounded<R>(mut reader: R, limit: usize) -> std::io::Result<BoundedOutput>
where
    R: Read,
{
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut truncated = false;
    let mut chunk = [0_u8; 8192];

    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }

        let remaining = limit.saturating_sub(bytes.len());
        let keep = remaining.min(read);
        bytes.extend_from_slice(&chunk[..keep]);
        if keep < read {
            truncated = true;
        }
    }

    Ok(BoundedOutput { bytes, truncated })
}

fn classify_status(status: ExitStatus) -> ProcessOutcome {
    if let Some(code) = status.code() {
        return ProcessOutcome::Exited { code };
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ProcessOutcome::Signaled {
            signal: status.signal().unwrap_or_default(),
        }
    }

    #[cfg(not(unix))]
    {
        ProcessOutcome::Exited { code: -1 }
    }
}

#[cfg(unix)]
fn configure_process(_command: &mut std::process::Command, _memory_limit_bytes: u64) {}

#[cfg(windows)]
fn configure_process(command: &mut std::process::Command, _memory_limit_bytes: u64) {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_PROCESS_GROUP. Windows has no stable std API for Job Objects;
    // cleanup below uses taskkill /T to terminate the group descendants.
    command.creation_flags(0x0000_0200);
}

#[cfg(not(any(unix, windows)))]
fn configure_process(_command: &mut std::process::Command, _memory_limit_bytes: u64) {}

#[cfg(unix)]
fn terminate_process_tree(pid: u32) {
    // Negative pid targets the worker's dedicated process group.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(_pid: u32) {}

#[cfg(unix)]
fn establish_process_group(pid: u32) {
    // Best effort from the parent closes the interval before the worker calls
    // prepare_current_process(). EACCES is harmless if the child already exec'd.
    unsafe {
        libc::setpgid(pid as libc::pid_t, pid as libc::pid_t);
    }
}

#[cfg(not(unix))]
fn establish_process_group(_pid: u32) {}

/// Prepare a supervised Plasmate worker before it starts risky work.
///
/// The parent also attempts process-group creation immediately after spawn. The
/// child repeats it here to avoid a parent/exec race without using `pre_exec`,
/// which can deadlock a multithreaded process on macOS. Linux workers additionally
/// apply their configured address-space limit here.
pub fn prepare_current_process() -> std::io::Result<()> {
    if std::env::var_os("PLASMATE_SUPERVISED_WORKER").is_none() {
        return Ok(());
    }

    #[cfg(unix)]
    unsafe {
        if libc::setpgid(0, 0) != 0 {
            let error = std::io::Error::last_os_error();
            // The parent may already have created exactly the requested group.
            if error.raw_os_error() != Some(libc::EACCES) {
                return Err(error);
            }
        }
    }

    #[cfg(target_os = "linux")]
    if let Some(value) = std::env::var_os("PLASMATE_WORKER_MEMORY_LIMIT_BYTES") {
        let value = value.to_string_lossy();
        let bytes: u64 = value.parse().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid PLASMATE_WORKER_MEMORY_LIMIT_BYTES",
            )
        })?;
        if bytes > 0 {
            let limit = libc::rlimit {
                rlim_cur: bytes as libc::rlim_t,
                rlim_max: bytes as libc::rlim_t,
            };
            let result = unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) };
            if result != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }

    // Descendants must stay in this worker's group rather than creating nested
    // groups that the coordinator cannot tear down.
    std::env::remove_var("PLASMATE_SUPERVISED_WORKER");
    std::env::remove_var("PLASMATE_WORKER_MEMORY_LIMIT_BYTES");

    Ok(())
}

struct ProcessTreeGuard {
    pid: u32,
    armed: bool,
}

impl ProcessTreeGuard {
    fn new(pid: u32) -> Self {
        Self { pid, armed: true }
    }

    fn terminate(&mut self) {
        if self.armed {
            terminate_process_tree(self.pid);
            self.armed = false;
        }
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Default)]
struct CancellationState {
    pid: AtomicU32,
    cancelled: AtomicBool,
}

struct CancellationGuard {
    state: Arc<CancellationState>,
    armed: bool,
}

impl CancellationGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }

    fn cancel(&mut self) {
        if !self.armed {
            return;
        }
        self.state.cancelled.store(true, Ordering::Release);
        let pid = self.state.pid.load(Ordering::Acquire);
        if pid != 0 {
            terminate_process_tree(pid);
        }
        self.armed = false;
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.cancel();
    }
}

fn supervise_blocking(
    spec: ProcessSpec,
    cancellation: Arc<CancellationState>,
) -> Result<ProcessOutput, SupervisorError> {
    let mut command = std::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(spec.env)
        .env("PLASMATE_SUPERVISED_WORKER", "1")
        .env(
            "PLASMATE_WORKER_MEMORY_LIMIT_BYTES",
            spec.memory_limit_bytes.to_string(),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    configure_process(&mut command, spec.memory_limit_bytes);

    let mut child = command.spawn().map_err(SupervisorError::Spawn)?;
    let child_pid = child.id();
    establish_process_group(child_pid);
    cancellation.pid.store(child_pid, Ordering::Release);
    let mut process_tree = ProcessTreeGuard::new(child_pid);
    let mut stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or(SupervisorError::MissingPipe("stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or(SupervisorError::MissingPipe("stderr"))?;

    let stdin_bytes = spec.stdin;
    let stdin_thread = std::thread::spawn(move || {
        if let Some(ref mut pipe) = stdin {
            pipe.write_all(&stdin_bytes)?;
            pipe.flush()?;
        }
        Ok::<(), std::io::Error>(())
    });
    let stdout_limit = spec.max_stdout_bytes;
    let stderr_limit = spec.max_stderr_bytes;
    let stdout_thread = std::thread::spawn(move || drain_bounded(stdout, stdout_limit));
    let stderr_thread = std::thread::spawn(move || drain_bounded(stderr, stderr_limit));

    let started = Instant::now();
    let outcome = loop {
        if cancellation.cancelled.load(Ordering::Acquire) {
            process_tree.terminate();
            let _ = child.kill();
            let _ = child.wait();
            return Err(SupervisorError::Task("supervision cancelled".to_string()));
        }
        if let Some(status) = child.try_wait().map_err(SupervisorError::Wait)? {
            break classify_status(status);
        }
        if started.elapsed() >= spec.timeout {
            process_tree.terminate();
            let _ = child.kill();
            child.wait().map_err(SupervisorError::Wait)?;
            break ProcessOutcome::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    // Do not allow descendants to outlive a worker or retain its output pipes.
    process_tree.terminate();
    let _ = stdin_thread.join();
    let stdout = stdout_thread
        .join()
        .map_err(|_| SupervisorError::Task("stdout collector panicked".to_string()))?
        .map_err(SupervisorError::Output)?;
    let stderr = stderr_thread
        .join()
        .map_err(|_| SupervisorError::Task("stderr collector panicked".to_string()))?
        .map_err(SupervisorError::Output)?;

    Ok(ProcessOutput {
        outcome,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
    })
}

/// Run a child process with a hard wall timeout and bounded output capture.
///
/// Output beyond each configured limit is drained and discarded so a noisy
/// worker cannot deadlock on a full pipe or exhaust the parent's memory.
pub async fn supervise(spec: ProcessSpec) -> Result<ProcessOutput, SupervisorError> {
    let wall_timeout = spec.timeout;
    let cancellation = Arc::new(CancellationState::default());
    let mut guard = CancellationGuard {
        state: cancellation.clone(),
        armed: true,
    };
    let task = tokio::task::spawn_blocking(move || supervise_blocking(spec, cancellation));
    match tokio::time::timeout(wall_timeout, task).await {
        Ok(joined) => {
            let result = joined.map_err(|error| SupervisorError::Task(error.to_string()))?;
            guard.disarm();
            result
        }
        Err(_) => {
            guard.cancel();
            Ok(ProcessOutput {
                outcome: ProcessOutcome::TimedOut,
                stdout: Vec::new(),
                stderr: Vec::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            })
        }
    }
}
