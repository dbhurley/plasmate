//! Reusable child-process supervision for workloads that may abort the process.
//!
//! V8 fatal errors cannot be recovered with Rust unwinding. Callers that execute
//! untrusted JavaScript should put that work in a child process and supervise it
//! with this module. Batch coverage workers and the line-oriented MCP workflow
//! runner share the same process-group containment primitives.

use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
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

/// Specification for a bounded, line-oriented supervised child. This is used
/// by protocol clients that cannot know all stdin up front (notably MCP).
#[derive(Debug, Clone)]
pub struct InteractiveProcessSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub max_line_bytes: usize,
    pub max_stderr_bytes: usize,
    pub memory_limit_bytes: u64,
}

#[derive(Debug, Error)]
pub enum InteractiveProcessError {
    #[error("failed to spawn supervised protocol child: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("supervised protocol child was missing its piped {0}")]
    MissingPipe(&'static str),
    #[error("failed to write to supervised protocol child: {0}")]
    Write(#[source] std::io::Error),
    #[error("failed to read from supervised protocol child: {0}")]
    Read(String),
    #[error("supervised protocol response exceeded the configured line limit")]
    Oversized,
    #[error("supervised protocol child closed stdout before responding")]
    EarlyExit,
    #[error("supervised protocol request timed out")]
    TimedOut,
}

enum LineEvent {
    Line(Vec<u8>),
    Oversized,
    Eof,
    Error(String),
}

const INTERACTIVE_LINE_QUEUE_CAPACITY: usize = 2;
const INTERACTIVE_WRITE_QUEUE_CAPACITY: usize = 1;

struct WriteRequest {
    bytes: Vec<u8>,
    acknowledged: std::sync::mpsc::SyncSender<Result<(), std::io::Error>>,
}

fn write_capped_requests(
    mut stdin: std::process::ChildStdin,
    requests: std::sync::mpsc::Receiver<WriteRequest>,
) {
    while let Ok(request) = requests.recv() {
        let result = stdin.write_all(&request.bytes).and_then(|()| stdin.flush());
        let failed = result.is_err();
        let _ = request.acknowledged.send(result);
        if failed {
            break;
        }
    }
}

fn read_capped_lines<R: Read>(reader: R, limit: usize, tx: std::sync::mpsc::SyncSender<LineEvent>) {
    let mut reader = std::io::BufReader::new(reader);
    let mut line = Vec::with_capacity(limit.min(16 * 1024));
    let mut oversized = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) => {
                let _ = tx.send(LineEvent::Error(error.to_string()));
                break;
            }
        };
        if available.is_empty() {
            if oversized {
                let _ = tx.send(LineEvent::Oversized);
            } else if !line.is_empty() {
                let _ = tx.send(LineEvent::Line(line));
            }
            let _ = tx.send(LineEvent::Eof);
            break;
        }
        let (segment, consumed, complete) = match available.iter().position(|byte| *byte == b'\n') {
            Some(position) => (&available[..position], position + 1, true),
            None => (available, available.len(), false),
        };
        let remaining = limit.saturating_sub(line.len());
        let keep = remaining.min(segment.len());
        line.extend_from_slice(&segment[..keep]);
        if keep < segment.len() {
            oversized = true;
        }
        reader.consume(consumed);
        if complete {
            let event = if oversized {
                LineEvent::Oversized
            } else {
                LineEvent::Line(std::mem::take(&mut line))
            };
            if tx.send(event).is_err() {
                break;
            }
            oversized = false;
            line.clear();
        }
    }
}

/// A process-group-isolated, line-oriented child. Dropping it kills the whole
/// group, so timeouts, malformed output, and caller cancellation cannot leave
/// browser descendants behind.
pub struct InteractiveProcess {
    child: std::process::Child,
    writes: Option<std::sync::mpsc::SyncSender<WriteRequest>>,
    writer: Option<std::thread::JoinHandle<()>>,
    lines: Option<std::sync::mpsc::Receiver<LineEvent>>,
    reader: Option<std::thread::JoinHandle<()>>,
    stderr: Option<std::thread::JoinHandle<std::io::Result<BoundedOutput>>>,
    process_tree: ProcessTreeGuard,
}

impl InteractiveProcess {
    pub fn spawn(spec: InteractiveProcessSpec) -> Result<Self, InteractiveProcessError> {
        let mut command = std::process::Command::new(&spec.program);
        command
            .env_clear()
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
        let mut child = command.spawn().map_err(InteractiveProcessError::Spawn)?;
        let pid = child.id();
        establish_process_group(pid);
        let process_tree = ProcessTreeGuard::new(pid);
        let stdin = child
            .stdin
            .take()
            .ok_or(InteractiveProcessError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(InteractiveProcessError::MissingPipe("stdout"))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or(InteractiveProcessError::MissingPipe("stderr"))?;
        let (write_tx, write_rx) = std::sync::mpsc::sync_channel(INTERACTIVE_WRITE_QUEUE_CAPACITY);
        let writer = std::thread::spawn(move || write_capped_requests(stdin, write_rx));
        // A protocol peer is allowed one response per request. Keep only one
        // response plus a terminal event in userspace; a child that floods
        // short lines is backpressured by the OS pipe instead of growing the
        // coordinator's heap without bound.
        let (tx, lines) = std::sync::mpsc::sync_channel(INTERACTIVE_LINE_QUEUE_CAPACITY);
        let line_limit = spec.max_line_bytes;
        let reader = std::thread::spawn(move || read_capped_lines(stdout, line_limit, tx));
        let stderr_limit = spec.max_stderr_bytes;
        let stderr = std::thread::spawn(move || drain_bounded(stderr_pipe, stderr_limit));
        Ok(Self {
            child,
            writes: Some(write_tx),
            writer: Some(writer),
            lines: Some(lines),
            reader: Some(reader),
            stderr: Some(stderr),
            process_tree,
        })
    }

    pub fn exchange(
        &mut self,
        line: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, InteractiveProcessError> {
        let remaining = self.write_line(line, timeout)?;
        let lines = self
            .lines
            .as_ref()
            .ok_or(InteractiveProcessError::EarlyExit)?;
        match lines.recv_timeout(remaining) {
            Ok(LineEvent::Line(line)) => Ok(line),
            Ok(LineEvent::Oversized) => Err(InteractiveProcessError::Oversized),
            Ok(LineEvent::Eof) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(InteractiveProcessError::EarlyExit)
            }
            Ok(LineEvent::Error(error)) => Err(InteractiveProcessError::Read(error)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(InteractiveProcessError::TimedOut)
            }
        }
    }

    pub fn notify(
        &mut self,
        line: &[u8],
        timeout: Duration,
    ) -> Result<(), InteractiveProcessError> {
        self.write_line(line, timeout).map(|_| ())
    }

    fn write_line(
        &mut self,
        line: &[u8],
        timeout: Duration,
    ) -> Result<Duration, InteractiveProcessError> {
        if timeout.is_zero() {
            return Err(InteractiveProcessError::TimedOut);
        }
        let started = Instant::now();
        let mut bytes = Vec::with_capacity(line.len().saturating_add(1));
        bytes.extend_from_slice(line);
        bytes.push(b'\n');
        let (acknowledged, completion) = std::sync::mpsc::sync_channel(1);
        let request = WriteRequest {
            bytes,
            acknowledged,
        };
        let writes = self
            .writes
            .as_ref()
            .ok_or(InteractiveProcessError::EarlyExit)?;
        match writes.try_send(request) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                return Err(InteractiveProcessError::TimedOut);
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                return Err(InteractiveProcessError::Write(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "supervised protocol writer stopped",
                )));
            }
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(InteractiveProcessError::TimedOut);
        }
        match completion.recv_timeout(remaining) {
            Ok(Ok(())) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    Err(InteractiveProcessError::TimedOut)
                } else {
                    Ok(remaining)
                }
            }
            Ok(Err(error)) => Err(InteractiveProcessError::Write(error)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(InteractiveProcessError::TimedOut)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(InteractiveProcessError::Write(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "supervised protocol writer stopped before acknowledgement",
                )))
            }
        }
    }

    pub fn shutdown(mut self, timeout: Duration) -> ProcessOutcome {
        self.writes.take();
        // Unblock a reader applying queue backpressure before joining it.
        self.lines.take();
        let started = Instant::now();
        let outcome = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break classify_status(status),
                Ok(None) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => {
                    self.process_tree.terminate();
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break ProcessOutcome::TimedOut;
                }
            }
        };
        self.process_tree.terminate();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
        outcome
    }
}

impl Drop for InteractiveProcess {
    fn drop(&mut self) {
        self.process_tree.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.writes.take();
        // A bounded sender may be waiting for queue capacity. Disconnecting
        // the receiver makes that send fail and lets the reader terminate.
        self.lines.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
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
pub(crate) fn configure_process_tree(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    // Run the child in a dedicated process group from the child side of spawn.
    // This closes the parent/exec race in which a fast child could create
    // descendants before the parent managed to call setpgid().
    command.process_group(0);
}

#[cfg(windows)]
pub(crate) fn configure_process_tree(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;

    // CREATE_NEW_PROCESS_GROUP. Cleanup uses taskkill /T below.
    command.creation_flags(0x0000_0200);
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn configure_process_tree(_command: &mut std::process::Command) {}

#[cfg(unix)]
fn configure_process(command: &mut std::process::Command, _memory_limit_bytes: u64) {
    configure_process_tree(command);
}

#[cfg(windows)]
fn configure_process(command: &mut std::process::Command, _memory_limit_bytes: u64) {
    configure_process_tree(command);
}

#[cfg(not(any(unix, windows)))]
fn configure_process(_command: &mut std::process::Command, _memory_limit_bytes: u64) {}

#[cfg(unix)]
pub(crate) fn terminate_process_tree(pid: u32) {
    // Negative pid targets the worker's dedicated process group.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(windows)]
pub(crate) fn terminate_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn terminate_process_tree(_pid: u32) {}

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

pub(crate) struct ProcessTreeGuard {
    pid: u32,
    armed: bool,
}

impl ProcessTreeGuard {
    pub(crate) fn new(pid: u32) -> Self {
        Self { pid, armed: true }
    }

    pub(crate) fn terminate(&mut self) {
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
    clear_env: bool,
) -> Result<ProcessOutput, SupervisorError> {
    let mut command = std::process::Command::new(&spec.program);
    if clear_env {
        command.env_clear();
    }
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

/// Synchronous counterpart to [`supervise`]. Long-lived protocol handlers that
/// are already synchronous (for example CDP Runtime.evaluate) use this rather
/// than duplicating process-group and bounded-pipe handling.
pub fn supervise_sync(spec: ProcessSpec) -> Result<ProcessOutput, SupervisorError> {
    let cancellation = Arc::new(CancellationState::default());
    supervise_blocking(spec, cancellation, false)
}

/// Synchronously supervise a child that starts from an empty environment.
/// Only the variables explicitly listed in [`ProcessSpec::env`] are inherited.
pub fn supervise_sync_clean_env(spec: ProcessSpec) -> Result<ProcessOutput, SupervisorError> {
    let cancellation = Arc::new(CancellationState::default());
    supervise_blocking(spec, cancellation, true)
}

/// Run a child process with a hard wall timeout and bounded output capture.
///
/// Output beyond each configured limit is drained and discarded so a noisy
/// worker cannot deadlock on a full pipe or exhaust the parent's memory.
pub async fn supervise(spec: ProcessSpec) -> Result<ProcessOutput, SupervisorError> {
    supervise_with_env_policy(spec, false).await
}

/// Supervise a child that starts from an empty environment. Only the variables
/// explicitly listed in [`ProcessSpec::env`] are inherited.
pub async fn supervise_clean_env(spec: ProcessSpec) -> Result<ProcessOutput, SupervisorError> {
    supervise_with_env_policy(spec, true).await
}

async fn supervise_with_env_policy(
    spec: ProcessSpec,
    clear_env: bool,
) -> Result<ProcessOutput, SupervisorError> {
    let wall_timeout = spec.timeout;
    let cancellation = Arc::new(CancellationState::default());
    let mut guard = CancellationGuard {
        state: cancellation.clone(),
        armed: true,
    };
    let task =
        tokio::task::spawn_blocking(move || supervise_blocking(spec, cancellation, clear_env));
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
