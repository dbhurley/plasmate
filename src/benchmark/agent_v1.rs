//! Reproducible task-success evidence for supervised agent workflows.
//!
//! The required suite uses only compiled, repository-owned scenarios and
//! ephemeral loopback fixtures. It never calls a model or a public network
//! service, and wall time is recorded only as observational evidence.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agent_workflow::{
    self, StepStatus, WorkflowOptions, WorkflowReport, WorkflowStatus, PLAN_SCHEMA, REPORT_SCHEMA,
};
use crate::som::compiler;
use crate::som::types::{Element, Som};

pub const SCHEMA_VERSION: &str = "plasmate.agent-task-benchmark.v1";
pub const SUITE_SCHEMA: &str = "plasmate.agent-task-benchmark-suite.v1";
pub const SUITE_NAME: &str = "deterministic-agent-workflows";
const REPORT_BYTES_LIMIT: usize = 2 * 1024 * 1024;
const DIGEST_CANONICALIZATION: &str =
    "RFC-8259 JSON with lexicographically ordered object keys; SHA-256 domain and u64be length framing";
const SUITE_MANIFEST: &str = include_str!("../../benchmarks/agent-workflow-v1/suite.json");
const START_HTML: &str = include_str!("../../benchmarks/agent-workflow-v1/start.html");
const DESTINATION_HTML: &str = include_str!("../../benchmarks/agent-workflow-v1/destination.html");
const FORM_HTML: &str = include_str!("../../benchmarks/agent-workflow-v1/form.html");
const FIXTURE_FILES: &[(&str, &str)] = &[
    ("benchmarks/agent-workflow-v1/start.html", START_HTML),
    (
        "benchmarks/agent-workflow-v1/destination.html",
        DESTINATION_HTML,
    ),
    ("benchmarks/agent-workflow-v1/form.html", FORM_HTML),
];

#[derive(Debug, Clone, Default)]
pub struct BenchmarkOptions {
    /// Override used by integration tests. The public CLI always executes its
    /// own exact binary and does not accept an alternate program.
    pub executable: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkReport {
    pub schema_version: String,
    pub generated_at_unix_seconds: u64,
    pub suite: SuiteEvidence,
    pub environment: Environment,
    pub execution: ExecutionPolicy,
    pub summary: Summary,
    pub tasks: Vec<TaskResult>,
    pub gate: GateEvaluation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteEvidence {
    pub schema: String,
    pub name: String,
    pub manifest_sha256: String,
    pub fixture_corpus_sha256: String,
    pub digest_canonicalization: String,
    pub fixture_files: Vec<String>,
    pub scenarios: Vec<ScenarioDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScenarioDescriptor {
    pub id: String,
    pub category: String,
    pub fixture: String,
    pub expected_workflow_outcome: WorkflowOutcome,
    pub contract: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuiteManifest {
    schema: String,
    suite: String,
    scenarios: Vec<ScenarioDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub plasmate_version: String,
    pub git_commit: Option<String>,
    pub git_dirty: Option<bool>,
    pub rustc_version: Option<String>,
    pub build_profile: String,
    pub operating_system: String,
    pub architecture: String,
    pub executable_sha256: Option<String>,
    pub runner: RunnerMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerMetadata {
    pub ci_provider: Option<String>,
    pub github_repository: Option<String>,
    pub github_run_id: Option<String>,
    pub github_run_attempt: Option<String>,
    pub github_job: Option<String>,
    pub github_sha: Option<String>,
    pub runner_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub fixture_transport: String,
    pub public_network_requests: bool,
    pub model_calls: bool,
    pub model_judgments: bool,
    pub child_process: String,
    pub latency_role: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    /// Complete scenario denominator; no observed outcome is excluded.
    pub tasks_total: usize,
    pub observed_succeeded: usize,
    pub observed_failed: usize,
    pub observed_crash: usize,
    pub observed_timeout: usize,
    pub task_contracts_passed: usize,
    pub task_contracts_failed: usize,
    pub workflow_steps_total: usize,
    pub workflow_steps_succeeded: usize,
    pub workflow_steps_failed: usize,
    pub workflow_steps_skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskResult {
    pub id: String,
    pub category: String,
    pub expected_workflow_outcome: WorkflowOutcome,
    pub observed_workflow_outcome: WorkflowOutcome,
    pub task_contract_passed: bool,
    /// Observational wall time. It is not a release threshold or microbenchmark.
    pub observational_wall_time_us: u64,
    pub workflow_report_schema: Option<String>,
    pub plan_schema: Option<String>,
    pub protocol_version: Option<String>,
    pub plan_fingerprint: Option<String>,
    pub process_outcome: Option<String>,
    pub workflow_summary: WorkflowSummary,
    pub steps: Vec<StepResult>,
    pub assertions: Vec<Assertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOutcome {
    Succeeded,
    Failed,
    Crash,
    Timeout,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSummary {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StepResult {
    pub id: String,
    pub tool: String,
    pub status: String,
    pub approved: bool,
    pub failure_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GateEvaluation {
    pub passed: bool,
    pub require_all_task_contracts: bool,
    pub require_no_crash_or_timeout: bool,
    pub violations: Vec<String>,
}

#[derive(Debug)]
struct ExpectedStep {
    id: &'static str,
    tool: &'static str,
    status: StepStatus,
    failure_class: Option<&'static str>,
}

struct BuiltScenario {
    plan: Value,
    options: WorkflowOptions,
    expected_steps: Vec<ExpectedStep>,
}

pub async fn run_suite(
    options: &BenchmarkOptions,
) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
    let manifest = compiled_manifest()?;
    let executable = match &options.executable {
        Some(path) => path.clone(),
        None => std::env::current_exe()?,
    };
    let (base_url, fixture_server) = start_fixture_server().await?;
    let mut tasks = Vec::with_capacity(manifest.scenarios.len());

    for descriptor in &manifest.scenarios {
        let started = Instant::now();
        let task = match build_scenario(descriptor, &base_url) {
            Ok(built) => match agent_workflow::validate_bytes(
                &serde_json::to_vec(&built.plan)?,
                &built.options,
            ) {
                Ok(workflow) => {
                    let report = agent_workflow::execute_for_local_benchmark(
                        workflow,
                        &built.options,
                        executable.clone(),
                    );
                    task_from_report(
                        descriptor,
                        report,
                        &built.options,
                        &built.expected_steps,
                        started.elapsed().as_micros() as u64,
                    )
                }
                Err(error) => internal_failure_task(
                    descriptor,
                    format!("compiled plan validation failed: {error}"),
                    started.elapsed().as_micros() as u64,
                ),
            },
            Err(error) => internal_failure_task(
                descriptor,
                format!("scenario construction failed: {error}"),
                started.elapsed().as_micros() as u64,
            ),
        };
        tasks.push(task);
    }

    fixture_server.abort();
    let _ = fixture_server.await;

    let summary = summarize(&tasks);
    let gate = evaluate_gate(&summary);
    Ok(BenchmarkReport {
        schema_version: SCHEMA_VERSION.to_string(),
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        suite: suite_evidence(&manifest)?,
        environment: environment(&executable),
        execution: expected_execution_policy(),
        summary,
        tasks,
        gate,
    })
}

pub fn validate_evidence(report: &BenchmarkReport) -> Result<(), String> {
    if report.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema {}; expected {SCHEMA_VERSION}",
            report.schema_version
        ));
    }
    let manifest = compiled_manifest().map_err(|error| error.to_string())?;
    let expected_suite = suite_evidence(&manifest).map_err(|error| error.to_string())?;
    if report.suite.schema != expected_suite.schema
        || report.suite.name != expected_suite.name
        || report.suite.manifest_sha256 != expected_suite.manifest_sha256
        || report.suite.fixture_corpus_sha256 != expected_suite.fixture_corpus_sha256
        || report.suite.digest_canonicalization != expected_suite.digest_canonicalization
        || report.suite.fixture_files != expected_suite.fixture_files
        || report.suite.scenarios != expected_suite.scenarios
    {
        return Err("suite provenance does not match the compiled fixture corpus".to_string());
    }
    if report.execution != expected_execution_policy() {
        return Err("execution policy does not match the deterministic suite".to_string());
    }
    validate_environment(&report.environment)?;
    if report.tasks.len() != manifest.scenarios.len() {
        return Err(format!(
            "task denominator mismatch: report={}, suite={}",
            report.tasks.len(),
            manifest.scenarios.len()
        ));
    }
    for (task, descriptor) in report.tasks.iter().zip(&manifest.scenarios) {
        if task.id != descriptor.id
            || task.category != descriptor.category
            || task.expected_workflow_outcome != descriptor.expected_workflow_outcome
        {
            return Err(format!(
                "task {} does not match its suite descriptor",
                task.id
            ));
        }
        let built = build_scenario(descriptor, "http://127.0.0.1:1")
            .map_err(|error| format!("compiled scenario {} is invalid: {error}", descriptor.id))?;
        if task.workflow_summary.total
            != task.workflow_summary.succeeded
                + task.workflow_summary.failed
                + task.workflow_summary.skipped
            || task.workflow_summary.total != task.steps.len()
        {
            return Err(format!("task {} step denominator mismatch", task.id));
        }
        let counted = count_steps(&task.steps);
        if task.workflow_summary != counted {
            return Err(format!(
                "task {} step aggregates do not match rows",
                task.id
            ));
        }
        if !task
            .plan_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| is_lower_hex(fingerprint, 64))
        {
            return Err(format!("task {} has invalid plan fingerprint", task.id));
        }
        if task.workflow_report_schema.as_deref() != Some(REPORT_SCHEMA)
            || task.plan_schema.as_deref() != Some(PLAN_SCHEMA)
            || task.protocol_version.as_deref() != Some(agent_workflow::MCP_PROTOCOL)
        {
            return Err(format!("task {} has invalid workflow identities", task.id));
        }
        if task.process_outcome.as_deref() != Some("exited:0") {
            return Err(format!("task {} child did not exit cleanly", task.id));
        }
        if task.observed_workflow_outcome != descriptor.expected_workflow_outcome {
            return Err(format!("task {} observed the wrong outcome", task.id));
        }
        if !steps_match_compiled_contract(&task.steps, &built.expected_steps, &built.options) {
            return Err(format!(
                "task {} steps do not match the compiled contract",
                task.id
            ));
        }
        if task.assertions != assertion_rows([true, true, true, true]) {
            return Err(format!("task {} assertion rows are not canonical", task.id));
        }
        if !task.task_contract_passed {
            return Err(format!("task {} contract is not passed", task.id));
        }
    }
    let summary = summarize(&report.tasks);
    if report.summary != summary {
        return Err("summary does not match the complete task rows".to_string());
    }
    let outcomes = summary.observed_succeeded
        + summary.observed_failed
        + summary.observed_crash
        + summary.observed_timeout;
    if outcomes != summary.tasks_total
        || summary.task_contracts_passed + summary.task_contracts_failed != summary.tasks_total
    {
        return Err("summary outcome or contract denominator is incomplete".to_string());
    }
    if report.gate != evaluate_gate(&summary) {
        return Err("gate evaluation does not match task evidence".to_string());
    }
    Ok(())
}

pub fn read_report(path: &Path) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > REPORT_BYTES_LIMIT as u64 {
        return Err("agent task benchmark report exceeds 2 MiB".into());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub fn write_report(
    path: &Path,
    report: &BenchmarkReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    if bytes.len() > REPORT_BYTES_LIMIT {
        return Err("agent task benchmark report exceeds 2 MiB".into());
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

fn compiled_manifest() -> Result<SuiteManifest, serde_json::Error> {
    serde_json::from_str(SUITE_MANIFEST)
}

fn suite_evidence(manifest: &SuiteManifest) -> Result<SuiteEvidence, Box<dyn std::error::Error>> {
    if manifest.schema != SUITE_SCHEMA || manifest.suite != SUITE_NAME {
        return Err("compiled agent benchmark suite identity is invalid".into());
    }
    let canonical_manifest = serde_json::to_vec(&serde_json::from_str::<Value>(SUITE_MANIFEST)?)?;
    Ok(SuiteEvidence {
        schema: manifest.schema.clone(),
        name: manifest.suite.clone(),
        manifest_sha256: framed_digest(
            b"plasmate.agent-task-benchmark-manifest.v1",
            &[(&b"suite.json"[..], canonical_manifest.as_slice())],
        ),
        fixture_corpus_sha256: fixture_corpus_digest(&canonical_manifest),
        digest_canonicalization: DIGEST_CANONICALIZATION.to_string(),
        fixture_files: FIXTURE_FILES
            .iter()
            .map(|(path, _)| (*path).to_string())
            .collect(),
        scenarios: manifest.scenarios.clone(),
    })
}

fn fixture_corpus_digest(canonical_manifest: &[u8]) -> String {
    let mut entries: Vec<(&[u8], &[u8])> = vec![(b"suite.json", canonical_manifest)];
    entries.extend(
        FIXTURE_FILES
            .iter()
            .map(|(path, body)| (path.as_bytes(), body.as_bytes())),
    );
    framed_digest(b"plasmate.agent-task-benchmark-fixtures.v1", &entries)
}

fn framed_digest(domain: &[u8], entries: &[(&[u8], &[u8])]) -> String {
    let mut digest = Sha256::new();
    append_framed(&mut digest, domain);
    for (name, contents) in entries {
        append_framed(&mut digest, name);
        append_framed(&mut digest, contents);
    }
    hex::encode(digest.finalize())
}

fn append_framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

async fn start_fixture_server() -> Result<(String, tokio::task::JoinHandle<()>), std::io::Error> {
    let router = Router::new()
        .route("/start", get(|| async { Html(START_HTML) }))
        .route("/destination", get(|| async { Html(DESTINATION_HTML) }))
        .route("/form", get(|| async { Html(FORM_HTML) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok((format!("http://{address}"), server))
}

fn build_scenario(
    descriptor: &ScenarioDescriptor,
    base_url: &str,
) -> Result<BuiltScenario, Box<dyn std::error::Error>> {
    let start_url = format!("{base_url}{}", descriptor.fixture);
    let destination_url = format!("{base_url}/destination");
    let email_id = email_element_id(&format!("{base_url}/form"))?;
    let limits = json!({
        "step_timeout_ms": 15000,
        "workflow_timeout_ms": 60000,
        "response_bytes": 524288,
        "stderr_bytes": 65536,
        "memory_mb": 0,
        "fail_fast": true
    });
    let (trace, steps, options, expected_steps) = match descriptor.id.as_str() {
        "navigate-between-pages" => (
            false,
            json!([
                {"id":"open","tool":"open_page","arguments":{"url":start_url},"expect":{"json_pointer":"/title","equals":"Workflow start"}},
                {"id":"navigate","tool":"navigate_to","arguments":{"url":destination_url},"expect":{"json_pointer":"/title","equals":"Workflow destination"}}
            ]),
            WorkflowOptions {
                confirm_steps: vec!["navigate".to_string()],
                ..Default::default()
            },
            vec![
                expected_step("open", "open_page", StepStatus::Succeeded, None),
                expected_step("navigate", "navigate_to", StepStatus::Succeeded, None),
            ],
        ),
        "type-and-observe-value" => (
            false,
            json!([
                {"id":"open","tool":"open_page","arguments":{"url":start_url},"expect":{"json_pointer":"/title","equals":"Workflow form"}},
                {"id":"type-email","tool":"type_text","arguments":{"element_id":email_id,"text":"agent@example.test"}},
                {"id":"verify-value","tool":"evaluate","arguments":{"expression":"document.getElementById('email').value"},"expect":{"json_pointer":"/result","equals":"agent@example.test"}}
            ]),
            WorkflowOptions {
                allow_evaluate: true,
                confirm_steps: vec!["type-email".to_string(), "verify-value".to_string()],
                ..Default::default()
            },
            vec![
                expected_step("open", "open_page", StepStatus::Succeeded, None),
                expected_step("type-email", "type_text", StepStatus::Succeeded, None),
                expected_step("verify-value", "evaluate", StepStatus::Succeeded, None),
            ],
        ),
        "trace-action-export" => (
            true,
            json!([
                {"id":"open","tool":"open_page","arguments":{"url":start_url}},
                {"id":"type-email","tool":"type_text","arguments":{"element_id":email_id,"text":"trace@example.test"}},
                {"id":"export-trace","tool":"trace_export","arguments":{},"expect":{"json_pointer":"/retained_events","equals":2}}
            ]),
            WorkflowOptions {
                confirm_steps: vec!["type-email".to_string()],
                ..Default::default()
            },
            vec![
                expected_step("open", "open_page", StepStatus::Succeeded, None),
                expected_step("type-email", "type_text", StepStatus::Succeeded, None),
                expected_step("export-trace", "trace_export", StepStatus::Succeeded, None),
            ],
        ),
        "replay-cross-session-refusal" => (
            true,
            json!([
                {"id":"open","tool":"open_page","arguments":{"url":start_url}},
                {"id":"validate-replay","tool":"replay_validate","arguments":{"trace_id":"not-the-live-trace","sequence":1,"confirmed":true},"expect":{"json_pointer":"/drift","equals":"cross_session"}}
            ]),
            WorkflowOptions::default(),
            vec![
                expected_step("open", "open_page", StepStatus::Succeeded, None),
                expected_step(
                    "validate-replay",
                    "replay_validate",
                    StepStatus::Succeeded,
                    None,
                ),
            ],
        ),
        "expected-tool-error-continues" => (
            false,
            json!([
                {"id":"open","tool":"open_page","arguments":{"url":start_url}},
                {"id":"missing-click","tool":"click","arguments":{"element_id":"missing-element"},"expect":{"is_error":true}},
                {"id":"trace-status","tool":"trace_status","arguments":{},"expect":{"json_pointer":"/enabled","equals":false}}
            ]),
            WorkflowOptions {
                confirm_steps: vec!["missing-click".to_string()],
                ..Default::default()
            },
            vec![
                expected_step("open", "open_page", StepStatus::Succeeded, None),
                expected_step("missing-click", "click", StepStatus::Succeeded, None),
                expected_step("trace-status", "trace_status", StepStatus::Succeeded, None),
            ],
        ),
        "unexpected-tool-error-is-contained" => (
            false,
            json!([
                {"id":"open","tool":"open_page","arguments":{"url":start_url}},
                {"id":"missing-click","tool":"click","arguments":{"element_id":"missing-element"}},
                {"id":"must-skip","tool":"trace_status","arguments":{}}
            ]),
            WorkflowOptions {
                confirm_steps: vec!["missing-click".to_string()],
                ..Default::default()
            },
            vec![
                expected_step("open", "open_page", StepStatus::Succeeded, None),
                expected_step(
                    "missing-click",
                    "click",
                    StepStatus::Failed,
                    Some("tool_error_expectation"),
                ),
                expected_step(
                    "must-skip",
                    "trace_status",
                    StepStatus::Skipped,
                    Some("fail_fast"),
                ),
            ],
        ),
        other => return Err(format!("unknown compiled scenario {other}").into()),
    };
    Ok(BuiltScenario {
        plan: json!({
            "schema": PLAN_SCHEMA,
            "name": descriptor.id,
            "trace": trace,
            "limits": limits,
            "steps": steps
        }),
        options,
        expected_steps,
    })
}

fn expected_step(
    id: &'static str,
    tool: &'static str,
    status: StepStatus,
    failure_class: Option<&'static str>,
) -> ExpectedStep {
    ExpectedStep {
        id,
        tool,
        status,
        failure_class,
    }
}

fn email_element_id(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let som = compiler::compile(FORM_HTML, url)?;
    find_element_by_html_id(&som, "email")
        .map(|element| element.id.clone())
        .ok_or_else(|| "email fixture did not compile to an interactive SOM element".into())
}

fn find_element_by_html_id<'a>(som: &'a Som, html_id: &str) -> Option<&'a Element> {
    fn visit<'a>(elements: &'a [Element], html_id: &str) -> Option<&'a Element> {
        for element in elements {
            if element.html_id.as_deref() == Some(html_id) {
                return Some(element);
            }
            if let Some(found) = element
                .children
                .as_deref()
                .and_then(|children| visit(children, html_id))
            {
                return Some(found);
            }
            if let Some(found) = element
                .shadow
                .as_ref()
                .and_then(|shadow| visit(&shadow.elements, html_id))
            {
                return Some(found);
            }
        }
        None
    }
    som.regions
        .iter()
        .find_map(|region| visit(&region.elements, html_id))
}

fn task_from_report(
    descriptor: &ScenarioDescriptor,
    report: WorkflowReport,
    options: &WorkflowOptions,
    expected_steps: &[ExpectedStep],
    observational_wall_time_us: u64,
) -> TaskResult {
    let observed = classify_outcome(&report);
    let steps: Vec<StepResult> = report
        .steps
        .iter()
        .map(|step| StepResult {
            id: step.id.clone(),
            tool: step.tool.clone(),
            status: step_status_name(&step.status).to_string(),
            approved: step.approved,
            failure_class: step.failure_class.clone(),
        })
        .collect();
    let exact_steps = steps_match_compiled_contract(&steps, expected_steps, options);
    let clean_child = report.process_outcome.as_deref() == Some("exited:0");
    let assertions = assertion_rows([
        observed == descriptor.expected_workflow_outcome,
        exact_steps,
        clean_child,
        report.schema == REPORT_SCHEMA
            && report.plan_schema == PLAN_SCHEMA
            && report.protocol_version == agent_workflow::MCP_PROTOCOL,
    ]);
    let task_contract_passed = assertions.iter().all(|assertion| assertion.passed);
    TaskResult {
        id: descriptor.id.clone(),
        category: descriptor.category.clone(),
        expected_workflow_outcome: descriptor.expected_workflow_outcome.clone(),
        observed_workflow_outcome: observed,
        task_contract_passed,
        observational_wall_time_us,
        workflow_report_schema: Some(report.schema.to_string()),
        plan_schema: Some(report.plan_schema.to_string()),
        protocol_version: Some(report.protocol_version.to_string()),
        plan_fingerprint: Some(report.plan_fingerprint),
        process_outcome: report.process_outcome,
        workflow_summary: WorkflowSummary {
            total: report.summary.total,
            succeeded: report.summary.succeeded,
            failed: report.summary.failed,
            skipped: report.summary.skipped,
        },
        steps,
        assertions,
    }
}

fn steps_match_compiled_contract(
    actual: &[StepResult],
    expected: &[ExpectedStep],
    options: &WorkflowOptions,
) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(actual, expected)| {
            actual.id == expected.id
                && actual.tool == expected.tool
                && actual.status == step_status_name(&expected.status)
                && actual.approved
                    == options
                        .confirm_steps
                        .iter()
                        .any(|approved| approved == expected.id)
                && actual.failure_class.as_deref() == expected.failure_class
        })
}

fn assertion_rows(passed: [bool; 4]) -> Vec<Assertion> {
    vec![
        Assertion {
            name: "workflow outcome matches scenario contract".to_string(),
            passed: passed[0],
            detail: "Observed outcomes remain distinct from task-contract pass/fail.".to_string(),
        },
        Assertion {
            name: "every declared step has its exact expected status".to_string(),
            passed: passed[1],
            detail: "Step IDs, tools, statuses, approvals, and stable failure classes are compared in order."
                .to_string(),
        },
        Assertion {
            name: "supervised MCP child exits cleanly".to_string(),
            passed: passed[2],
            detail: "Expected tool and expectation failures must not crash or strand the child."
                .to_string(),
        },
        Assertion {
            name: "workflow evidence uses the advertised contracts".to_string(),
            passed: passed[3],
            detail: "Report, plan, and MCP protocol identities are exact.".to_string(),
        },
    ]
}

fn internal_failure_task(
    descriptor: &ScenarioDescriptor,
    detail: String,
    observational_wall_time_us: u64,
) -> TaskResult {
    TaskResult {
        id: descriptor.id.clone(),
        category: descriptor.category.clone(),
        expected_workflow_outcome: descriptor.expected_workflow_outcome.clone(),
        observed_workflow_outcome: WorkflowOutcome::Failed,
        task_contract_passed: false,
        observational_wall_time_us,
        workflow_report_schema: None,
        plan_schema: None,
        protocol_version: None,
        plan_fingerprint: None,
        process_outcome: None,
        workflow_summary: WorkflowSummary::default(),
        steps: Vec::new(),
        assertions: vec![Assertion {
            name: "compiled scenario produced workflow evidence".to_string(),
            passed: false,
            detail,
        }],
    }
}

fn classify_outcome(report: &WorkflowReport) -> WorkflowOutcome {
    let timed_out = report.steps.iter().any(|step| {
        matches!(
            step.failure_class.as_deref(),
            Some("step_timeout" | "workflow_timeout")
        )
    }) || report.process_outcome.as_deref() == Some("timed_out");
    if timed_out {
        WorkflowOutcome::Timeout
    } else if report.status == WorkflowStatus::Succeeded {
        WorkflowOutcome::Succeeded
    } else if report
        .process_outcome
        .as_deref()
        .is_some_and(|outcome| outcome.starts_with("signaled:"))
    {
        WorkflowOutcome::Crash
    } else {
        WorkflowOutcome::Failed
    }
}

fn summarize(tasks: &[TaskResult]) -> Summary {
    let mut summary = Summary {
        tasks_total: tasks.len(),
        ..Default::default()
    };
    for task in tasks {
        match task.observed_workflow_outcome {
            WorkflowOutcome::Succeeded => summary.observed_succeeded += 1,
            WorkflowOutcome::Failed => summary.observed_failed += 1,
            WorkflowOutcome::Crash => summary.observed_crash += 1,
            WorkflowOutcome::Timeout => summary.observed_timeout += 1,
        }
        if task.task_contract_passed {
            summary.task_contracts_passed += 1;
        } else {
            summary.task_contracts_failed += 1;
        }
        summary.workflow_steps_total += task.workflow_summary.total;
        summary.workflow_steps_succeeded += task.workflow_summary.succeeded;
        summary.workflow_steps_failed += task.workflow_summary.failed;
        summary.workflow_steps_skipped += task.workflow_summary.skipped;
    }
    summary
}

fn count_steps(steps: &[StepResult]) -> WorkflowSummary {
    let mut summary = WorkflowSummary {
        total: steps.len(),
        ..Default::default()
    };
    for step in steps {
        match step.status.as_str() {
            "succeeded" => summary.succeeded += 1,
            "failed" => summary.failed += 1,
            "skipped" => summary.skipped += 1,
            _ => {}
        }
    }
    summary
}

fn evaluate_gate(summary: &Summary) -> GateEvaluation {
    let mut violations = Vec::new();
    if summary.task_contracts_failed > 0 {
        violations.push(format!(
            "{} deterministic task contract(s) failed",
            summary.task_contracts_failed
        ));
    }
    if summary.observed_crash > 0 || summary.observed_timeout > 0 {
        violations.push(format!(
            "observed {} crash(es) and {} timeout(s)",
            summary.observed_crash, summary.observed_timeout
        ));
    }
    GateEvaluation {
        passed: violations.is_empty(),
        require_all_task_contracts: true,
        require_no_crash_or_timeout: true,
        violations,
    }
}

fn expected_execution_policy() -> ExecutionPolicy {
    ExecutionPolicy {
        fixture_transport: "ephemeral-loopback-http; compiled destinations only".to_string(),
        public_network_requests: false,
        model_calls: false,
        model_judgments: false,
        child_process: "real supervised plasmate MCP stdio child per scenario".to_string(),
        latency_role: "observational only; no latency threshold or comparative claim".to_string(),
    }
}

fn validate_environment(environment: &Environment) -> Result<(), String> {
    if environment.plasmate_version != env!("CARGO_PKG_VERSION") {
        return Err("evidence Plasmate version does not match the validator".to_string());
    }
    let commit = environment
        .git_commit
        .as_deref()
        .ok_or("evidence is missing the Git commit")?;
    if !is_lower_hex(commit, 40) {
        return Err("evidence Git commit is not a full lowercase SHA".to_string());
    }
    if environment.git_dirty.is_none() {
        return Err("evidence is missing the Git dirty-state observation".to_string());
    }
    if !environment
        .rustc_version
        .as_deref()
        .is_some_and(|version| version.starts_with("rustc "))
    {
        return Err("evidence is missing a Rust compiler identity".to_string());
    }
    if !matches!(environment.build_profile.as_str(), "debug" | "release")
        || environment.operating_system.is_empty()
        || environment.architecture.is_empty()
    {
        return Err("evidence has incomplete build-platform provenance".to_string());
    }
    if !environment
        .executable_sha256
        .as_deref()
        .is_some_and(|digest| is_lower_hex(digest, 64))
    {
        return Err("evidence is missing a full executable SHA-256".to_string());
    }
    if environment.runner.ci_provider.as_deref() == Some("github-actions") {
        for (name, value) in [
            ("github_repository", &environment.runner.github_repository),
            ("github_run_id", &environment.runner.github_run_id),
            ("github_run_attempt", &environment.runner.github_run_attempt),
            ("github_job", &environment.runner.github_job),
            ("runner_name", &environment.runner.runner_name),
        ] {
            if value.as_deref().is_none_or(str::is_empty) {
                return Err(format!("GitHub Actions evidence is missing {name}"));
            }
        }
        let github_sha = environment
            .runner
            .github_sha
            .as_deref()
            .ok_or("GitHub Actions evidence is missing github_sha")?;
        if github_sha != commit {
            return Err("GitHub Actions SHA does not match the checked-out Git commit".to_string());
        }
    } else if environment.runner.ci_provider.is_some() {
        return Err("evidence names an unsupported CI provider".to_string());
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn environment(executable: &Path) -> Environment {
    Environment {
        plasmate_version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit: command_output("git", &["rev-parse", "HEAD"]),
        git_dirty: Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| !output.stdout.is_empty()),
        rustc_version: command_output("rustc", &["--version"]),
        build_profile: if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        },
        operating_system: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        executable_sha256: std::fs::read(executable)
            .ok()
            .map(|bytes| hex::encode(Sha256::digest(bytes))),
        runner: RunnerMetadata {
            ci_provider: std::env::var("GITHUB_ACTIONS")
                .ok()
                .filter(|value| value == "true")
                .map(|_| "github-actions".to_string()),
            github_repository: std::env::var("GITHUB_REPOSITORY").ok(),
            github_run_id: std::env::var("GITHUB_RUN_ID").ok(),
            github_run_attempt: std::env::var("GITHUB_RUN_ATTEMPT").ok(),
            github_job: std::env::var("GITHUB_JOB").ok(),
            github_sha: std::env::var("GITHUB_SHA").ok(),
            runner_name: std::env::var("RUNNER_NAME").ok(),
        },
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .filter(|output| !output.is_empty())
}

fn step_status_name(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Succeeded => "succeeded",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_valid_report() -> BenchmarkReport {
        let manifest = compiled_manifest().expect("manifest");
        let tasks: Vec<TaskResult> = manifest
            .scenarios
            .iter()
            .map(|descriptor| {
                let built =
                    build_scenario(descriptor, "http://127.0.0.1:1").expect("compiled scenario");
                let steps: Vec<StepResult> = built
                    .expected_steps
                    .iter()
                    .map(|step| StepResult {
                        id: step.id.to_string(),
                        tool: step.tool.to_string(),
                        status: step_status_name(&step.status).to_string(),
                        approved: built
                            .options
                            .confirm_steps
                            .iter()
                            .any(|approved| approved == step.id),
                        failure_class: step.failure_class.map(str::to_string),
                    })
                    .collect();
                TaskResult {
                    id: descriptor.id.clone(),
                    category: descriptor.category.clone(),
                    expected_workflow_outcome: descriptor.expected_workflow_outcome.clone(),
                    observed_workflow_outcome: descriptor.expected_workflow_outcome.clone(),
                    task_contract_passed: true,
                    observational_wall_time_us: 1,
                    workflow_report_schema: Some(REPORT_SCHEMA.to_string()),
                    plan_schema: Some(PLAN_SCHEMA.to_string()),
                    protocol_version: Some(agent_workflow::MCP_PROTOCOL.to_string()),
                    plan_fingerprint: Some("0".repeat(64)),
                    process_outcome: Some("exited:0".to_string()),
                    workflow_summary: count_steps(&steps),
                    steps,
                    assertions: assertion_rows([true, true, true, true]),
                }
            })
            .collect();
        let summary = summarize(&tasks);
        BenchmarkReport {
            schema_version: SCHEMA_VERSION.to_string(),
            generated_at_unix_seconds: 0,
            suite: suite_evidence(&manifest).expect("suite"),
            environment: environment(&std::env::current_exe().expect("test executable")),
            execution: expected_execution_policy(),
            summary: summary.clone(),
            tasks,
            gate: evaluate_gate(&summary),
        }
    }

    fn refresh_aggregates(report: &mut BenchmarkReport) {
        report.summary = summarize(&report.tasks);
        report.gate = evaluate_gate(&report.summary);
    }

    #[test]
    fn compiled_suite_has_exact_representative_denominator() {
        let manifest = compiled_manifest().expect("compiled manifest");
        assert_eq!(manifest.schema, SUITE_SCHEMA);
        assert_eq!(manifest.suite, SUITE_NAME);
        assert_eq!(manifest.scenarios.len(), 6);
        let categories: Vec<_> = manifest
            .scenarios
            .iter()
            .map(|scenario| scenario.category.as_str())
            .collect();
        assert_eq!(
            categories,
            [
                "navigation",
                "action",
                "trace",
                "replay",
                "safe_failure",
                "safe_failure"
            ]
        );
    }

    #[test]
    fn fixture_digest_changes_with_content_or_name() {
        let first = framed_digest(b"domain", &[(b"a", b"value")]);
        assert_ne!(first, framed_digest(b"domain", &[(b"b", b"value")]));
        assert_ne!(first, framed_digest(b"domain", &[(b"a", b"changed")]));
        assert_ne!(first, framed_digest(b"other", &[(b"a", b"value")]));
    }

    #[test]
    fn evidence_validator_rejects_denominator_drift() {
        let manifest = compiled_manifest().expect("manifest");
        let mut report = BenchmarkReport {
            schema_version: SCHEMA_VERSION.to_string(),
            generated_at_unix_seconds: 0,
            suite: suite_evidence(&manifest).expect("suite"),
            environment: environment(&std::env::current_exe().expect("test executable")),
            execution: expected_execution_policy(),
            summary: Summary::default(),
            tasks: Vec::new(),
            gate: evaluate_gate(&Summary::default()),
        };
        assert!(validate_evidence(&report)
            .expect_err("missing tasks must fail")
            .contains("task denominator"));
        report.schema_version = "plasmate.agent-task-benchmark.v2".to_string();
        assert!(validate_evidence(&report)
            .expect_err("unknown major must fail")
            .contains("unsupported schema"));
    }

    #[test]
    fn evidence_validator_accepts_only_the_compiled_task_contracts() {
        let report = synthetic_valid_report();
        validate_evidence(&report).expect("synthetic compiled contract must validate");

        let mut zero_steps = report.clone();
        zero_steps.tasks[0].steps.clear();
        zero_steps.tasks[0].workflow_summary = WorkflowSummary::default();
        refresh_aggregates(&mut zero_steps);
        assert!(validate_evidence(&zero_steps)
            .expect_err("zero-step forgery must fail")
            .contains("compiled contract"));

        let mut forged_assertions = report.clone();
        forged_assertions.tasks[0].assertions = vec![Assertion {
            name: "trust me".to_string(),
            passed: true,
            detail: "arbitrary all-passed assertion".to_string(),
        }];
        assert!(validate_evidence(&forged_assertions)
            .expect_err("forged assertions must fail")
            .contains("assertion rows"));

        let mut wrong_protocol = report.clone();
        wrong_protocol.tasks[0].protocol_version = Some("2099-01-01".to_string());
        assert!(validate_evidence(&wrong_protocol)
            .expect_err("wrong protocol must fail")
            .contains("workflow identities"));

        let mut wrong_process = report;
        wrong_process.tasks[0].process_outcome = Some("signaled:9".to_string());
        assert!(validate_evidence(&wrong_process)
            .expect_err("non-clean process must fail")
            .contains("did not exit cleanly"));
    }
}
