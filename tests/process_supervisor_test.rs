use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use plasmate::process_supervisor::{supervise, ProcessOutcome, ProcessSpec, SupervisorError};
use serial_test::serial;

fn fixture_path() -> PathBuf {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/process_supervisor_fixture.rs");
            let mut output =
                PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("process-supervisor-fixture");
            if cfg!(windows) {
                output.set_extension("exe");
            }
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
            let compilation = std::process::Command::new(rustc)
                .args([
                    "--edition=2021",
                    "--crate-name",
                    "process_supervisor_fixture",
                ])
                .arg(&source)
                .arg("-o")
                .arg(&output)
                .output()
                .expect("failed to launch rustc for process-supervisor fixture");
            assert!(
                compilation.status.success(),
                "failed to compile {}: stdout={:?}; stderr={:?}",
                source.display(),
                String::from_utf8_lossy(&compilation.stdout),
                String::from_utf8_lossy(&compilation.stderr)
            );
            output
        })
        .clone()
}

fn helper_spec(mode: &str) -> ProcessSpec {
    ProcessSpec {
        program: fixture_path(),
        args: vec![OsString::from("--mode"), OsString::from(mode)],
        env: Vec::new(),
        stdin: Vec::new(),
        timeout: Duration::from_secs(10),
        max_stdout_bytes: 4096,
        max_stderr_bytes: 4096,
        memory_limit_bytes: 0,
    }
}

#[tokio::test]
#[serial]
async fn classifies_clean_and_nonzero_exits() {
    let clean = supervise(helper_spec("ok")).await.unwrap();
    assert_eq!(clean.outcome, ProcessOutcome::Exited { code: 0 });
    assert_eq!(clean.stdout, b"ok\n");

    let mut nonzero_spec = helper_spec("exit");
    nonzero_spec
        .args
        .extend([OsString::from("--exit-code"), OsString::from("23")]);
    let nonzero = supervise(nonzero_spec).await.unwrap();
    assert_eq!(nonzero.outcome, ProcessOutcome::Exited { code: 23 });
}

#[tokio::test]
#[serial]
async fn classifies_abort_without_killing_coordinator() {
    let output = supervise(helper_spec("abort")).await.unwrap();
    #[cfg(unix)]
    assert!(matches!(output.outcome, ProcessOutcome::Signaled { .. }));
    #[cfg(not(unix))]
    assert!(matches!(output.outcome, ProcessOutcome::Exited { code } if code != 0));
}

#[tokio::test]
#[serial]
async fn enforces_wall_timeout() {
    let mut spec = helper_spec("hang");
    spec.timeout = Duration::from_millis(150);
    let start = Instant::now();
    let output = supervise(spec).await.unwrap();

    assert_eq!(output.outcome, ProcessOutcome::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
#[serial]
async fn drains_but_bounds_oversized_output() {
    let mut spec = helper_spec("output");
    spec.args.extend([
        OsString::from("--bytes"),
        OsString::from((1024 * 1024).to_string()),
    ]);
    let output = supervise(spec).await.unwrap();

    assert_eq!(output.outcome, ProcessOutcome::Exited { code: 0 });
    assert_eq!(output.stdout.len(), 4096);
    assert_eq!(output.stderr.len(), 4096);
    assert!(output.stdout_truncated);
    assert!(output.stderr_truncated);
}

#[tokio::test]
#[serial]
async fn classifies_launch_failure() {
    let mut spec = helper_spec("ok");
    spec.program = PathBuf::from("/definitely/not/a/plasmate-worker");
    let error = supervise(spec).await.unwrap_err();
    assert!(matches!(error, SupervisorError::Spawn(_)));
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn timeout_terminates_worker_descendants() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("descendant.pid");
    let mut spec = helper_spec("descendant-hang");
    spec.args
        .extend([OsString::from("--path"), pid_file.as_os_str().to_owned()]);
    spec.timeout = Duration::from_secs(2);
    let output = supervise(spec).await.unwrap();
    assert_eq!(output.outcome, ProcessOutcome::TimedOut);

    let pid_text = std::fs::read_to_string(&pid_file).unwrap_or_else(|error| {
        panic!(
            "descendant fixture did not publish {}: {error}; outcome={:?}; stdout={:?}; stderr={:?}",
            pid_file.display(),
            output.outcome,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let pid: libc::pid_t = pid_text.trim().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let alive = unsafe { libc::kill(pid, 0) == 0 };
        if !alive {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "descendant {pid} survived timeout"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn cancellation_terminates_worker_process_group() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("worker.pid");
    let mut spec = helper_spec("pid-file-hang");
    spec.args
        .extend([OsString::from("--path"), pid_file.as_os_str().to_owned()]);
    spec.timeout = Duration::from_secs(30);

    let task = tokio::spawn(supervise(spec));
    let startup_deadline = Instant::now() + Duration::from_secs(10);
    while !pid_file.exists() {
        assert!(
            Instant::now() < startup_deadline,
            "worker did not publish its pid"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let pid: libc::pid_t = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();

    task.abort();
    let _ = task.await;
    let cleanup_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let alive = unsafe { libc::kill(pid, 0) == 0 };
        if !alive {
            break;
        }
        assert!(
            Instant::now() < cleanup_deadline,
            "worker {pid} survived supervisor cancellation"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
