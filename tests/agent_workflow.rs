#![cfg(unix)]

use std::ffi::OsString;
use std::path::PathBuf;

use plasmate::agent_workflow::{
    execute_with_program, validate_bytes, StepStatus, WorkflowOptions, WorkflowStatus,
};
use serde_json::json;
use serial_test::serial;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_mcp.py")
}

fn run(
    mode: &str,
    steps: serde_json::Value,
    response_bytes: usize,
) -> (
    plasmate::agent_workflow::WorkflowReport,
    String,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("calls.log");
    let plan = json!({
        "schema":"plasmate.agent-workflow.v1",
        "name":"fixture",
        "limits": {
            "step_timeout_ms": 2000,
            "workflow_timeout_ms": 8000,
            "response_bytes": response_bytes,
            "stderr_bytes": 1024,
            "memory_mb": 0,
            "fail_fast": false
        },
        "steps": steps
    });
    let options = WorkflowOptions::default();
    let workflow = validate_bytes(&serde_json::to_vec(&plan).unwrap(), &options).unwrap();
    let report = execute_with_program(
        workflow,
        &options,
        fixture(),
        vec![OsString::from(mode), log.as_os_str().to_owned()],
    );
    let calls = std::fs::read_to_string(log).unwrap_or_default();
    (report, calls, temp)
}

fn three_steps() -> serde_json::Value {
    json!([
        {"id":"open","tool":"open_page","arguments":{"url":"https://example.test"}},
        {"id":"status","tool":"trace_status","arguments":{}},
        {"id":"export","tool":"trace_export","arguments":{}}
    ])
}

#[test]
fn happy_path_uses_stable_lifecycle_and_closes_session() {
    let (report, calls, _temp) = run("happy", three_steps(), 4096);
    assert_eq!(report.status, WorkflowStatus::Succeeded, "{report:?}");
    assert_eq!(report.summary.succeeded, 3);
    assert_eq!(calls, "open_page\ntrace_status\ntrace_export\nclose_page\n");
}

#[test]
fn timeout_is_terminal_even_when_fail_fast_is_false() {
    let (report, calls, _temp) = run("timeout", three_steps(), 4096);
    assert_eq!(
        report.steps[1].failure_class.as_deref(),
        Some("step_timeout")
    );
    assert_eq!(report.steps[2].status, StepStatus::Skipped);
    assert_eq!(calls, "open_page\ntrace_status\n");
}

#[test]
fn malformed_oversized_and_early_exit_are_terminal() {
    for (mode, class, response_bytes) in [
        ("malformed", "malformed_response", 4096),
        ("oversized", "oversized_response", 4096),
        ("early_exit", "early_exit", 4096),
    ] {
        let (report, calls, _temp) = run(mode, three_steps(), response_bytes);
        assert_eq!(report.steps[1].failure_class.as_deref(), Some(class));
        assert_eq!(report.steps[2].status, StepStatus::Skipped);
        assert_eq!(calls, "open_page\ntrace_status\n");
    }
}

#[test]
fn short_line_flood_is_backpressured_and_shutdown_does_not_deadlock() {
    let started = std::time::Instant::now();
    let (report, calls, _temp) = run("short_line_flood", three_steps(), 4096);
    assert_eq!(
        report.steps[1].failure_class.as_deref(),
        Some("protocol_mismatch")
    );
    assert_eq!(report.steps[2].status, StepStatus::Skipped);
    assert_eq!(calls, "open_page\ntrace_status\n");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "flood teardown must remain bounded"
    );
}

#[test]
fn child_that_stops_reading_stdin_cannot_escape_step_deadline() {
    let padding = "a".repeat(32 * 1024 - 64);
    let steps = json!([{
        "id":"open",
        "tool":"open_page",
        "arguments":{"url":format!("https://example.test/{padding}")}
    }]);
    let started = std::time::Instant::now();
    let (report, calls, temp) = run("stdin_backpressure", steps, 4096);
    assert_eq!(
        report.steps[0].failure_class.as_deref(),
        Some("step_timeout")
    );
    assert!(calls.is_empty());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "blocked stdin writes must be included in the step deadline"
    );
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(
        !temp.path().join("calls.descendant").exists(),
        "stdin timeout must terminate the full descendant process group"
    );
}

#[test]
fn all_arguments_are_schema_checked_before_first_tool_call() {
    let steps = json!([
        {"id":"open","tool":"open_page","arguments":{"url":"https://example.test"}},
        {"id":"invalid","tool":"trace_status","arguments":{"bogus":true}}
    ]);
    let (report, calls, _temp) = run("happy", steps, 4096);
    assert_eq!(report.status, WorkflowStatus::Failed);
    assert!(report
        .steps
        .iter()
        .all(|step| step.failure_class.as_deref() == Some("argument_schema_invalid")));
    assert!(
        calls.is_empty(),
        "no tools may run before full schema validation"
    );
}

#[test]
fn unsupported_advertised_assertion_fails_closed_before_calls() {
    let (report, calls, _temp) = run("schema_unsupported", three_steps(), 4096);
    assert_eq!(report.status, WorkflowStatus::Failed);
    assert!(report
        .steps
        .iter()
        .all(|step| step.failure_class.as_deref() == Some("tool_schema_drift")));
    assert!(calls.is_empty());
}

#[test]
#[serial]
fn resolved_secret_values_are_schema_checked_before_calls() {
    std::env::set_var(
        "PLASMATE_SCHEMA_SECRET",
        "https://example.test/a-secret-bearing-path-that-is-too-long",
    );
    let steps = json!([{
        "id":"open",
        "tool":"open_page",
        "arguments":{"url":{"$secret":"PLASMATE_SCHEMA_SECRET"}}
    }]);
    let (report, calls, _temp) = run("secret_max_length", steps, 4096);
    std::env::remove_var("PLASMATE_SCHEMA_SECRET");
    assert_eq!(report.status, WorkflowStatus::Failed);
    assert_eq!(
        report.steps[0].failure_class.as_deref(),
        Some("argument_schema_invalid")
    );
    assert!(calls.is_empty());
}

#[test]
#[serial]
fn child_does_not_inherit_unrelated_environment() {
    std::env::set_var("PLASMATE_UNRELATED_SENTINEL", "must-not-leak");
    let steps = json!([{
        "id":"open",
        "tool":"open_page",
        "arguments":{"url":"https://example.test"},
        "expect":{"json_pointer":"/sentinel_present","equals":false}
    }]);
    let (report, _, _temp) = run("happy", steps, 4096);
    std::env::remove_var("PLASMATE_UNRELATED_SENTINEL");
    assert_eq!(report.status, WorkflowStatus::Succeeded);
}

#[test]
fn timeout_kills_descendants_in_the_supervised_process_group() {
    let (report, calls, temp) = run("descendant_timeout", three_steps(), 4096);
    assert_eq!(
        report.steps[1].failure_class.as_deref(),
        Some("step_timeout")
    );
    assert_eq!(calls, "open_page\ntrace_status\n");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!temp.path().join("calls.descendant").exists());
}
