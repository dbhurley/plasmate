//! Deterministic, bounded execution of stateful browser workflows over MCP.
//!
//! Plans receive complete static/policy validation before the supervised MCP
//! child is spawned, then every argument is checked against the child's
//! advertised schema before the first tool call. The CLI never accepts an
//! alternate child program, invokes a shell, or records tool payloads.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::process_supervisor::{
    InteractiveProcess, InteractiveProcessError, InteractiveProcessSpec, ProcessOutcome,
};

pub const PLAN_SCHEMA: &str = "plasmate.agent-workflow.v1";
pub const REPORT_SCHEMA: &str = "plasmate.agent-workflow-report.v1";
pub const MCP_PROTOCOL: &str = "2025-11-25";
const MAX_PLAN_BYTES: usize = 256 * 1024;
const MAX_PLAN_DEPTH: usize = 24;
const MAX_STEPS: usize = 64;
const MAX_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_STRING_BYTES: usize = 32 * 1024;
const MAX_NAME_BYTES: usize = 128;
const MAX_ID_BYTES: usize = 64;
const MAX_RESPONSE_BYTES_CEILING: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPlan {
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub trace: bool,
    #[serde(default)]
    pub limits: WorkflowLimits,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkflowLimits {
    pub step_timeout_ms: u64,
    pub workflow_timeout_ms: u64,
    pub response_bytes: usize,
    pub stderr_bytes: usize,
    pub memory_mb: u64,
    pub fail_fast: bool,
}

impl Default for WorkflowLimits {
    fn default() -> Self {
        Self {
            step_timeout_ms: 10_000,
            workflow_timeout_ms: 60_000,
            response_bytes: 512 * 1024,
            stderr_bytes: 64 * 1024,
            memory_mb: 0,
            fail_fast: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowStep {
    pub id: String,
    pub tool: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
    #[serde(default)]
    pub expect: StepExpectation,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StepExpectation {
    pub is_error: bool,
    pub json_pointer: Option<String>,
    pub equals: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowOptions {
    pub dry_run: bool,
    pub allow_evaluate: bool,
    pub allow_cookie_writes: bool,
    pub confirm_steps: Vec<String>,
}

#[derive(Debug)]
pub struct ValidatedWorkflow {
    plan: WorkflowPlan,
    plan_fingerprint: String,
    secret_refs: BTreeSet<String>,
}

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("failed to read workflow plan: {0}")]
    Read(#[source] std::io::Error),
    #[error("workflow plan exceeds {MAX_PLAN_BYTES} bytes")]
    PlanTooLarge,
    #[error("workflow plan is not valid JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("workflow plan validation failed: {0}")]
    Validation(String),
    #[error("failed to determine current executable: {0}")]
    CurrentExecutable(#[source] std::io::Error),
    #[error("failed to serialize workflow report: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write workflow report: {0}")]
    Write(#[source] std::io::Error),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Validated,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub id: String,
    pub tool: String,
    pub status: StepStatus,
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretSummary {
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub present: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowSummary {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowReport {
    pub schema: &'static str,
    pub plan_schema: &'static str,
    pub protocol_version: &'static str,
    pub workflow: String,
    pub plan_fingerprint: String,
    pub dry_run: bool,
    pub trace_requested: bool,
    pub status: WorkflowStatus,
    pub summary: WorkflowSummary,
    pub secrets: SecretSummary,
    pub steps: Vec<StepReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_outcome: Option<String>,
}

impl WorkflowReport {
    pub fn succeeded(&self) -> bool {
        matches!(
            self.status,
            WorkflowStatus::Succeeded | WorkflowStatus::Validated
        )
    }
}

pub fn load_and_validate(
    path: &Path,
    options: &WorkflowOptions,
) -> Result<ValidatedWorkflow, WorkflowError> {
    let metadata = std::fs::metadata(path).map_err(WorkflowError::Read)?;
    if metadata.len() > MAX_PLAN_BYTES as u64 {
        return Err(WorkflowError::PlanTooLarge);
    }
    let bytes = std::fs::read(path).map_err(WorkflowError::Read)?;
    validate_bytes(&bytes, options)
}

pub fn validate_bytes(
    bytes: &[u8],
    options: &WorkflowOptions,
) -> Result<ValidatedWorkflow, WorkflowError> {
    if bytes.len() > MAX_PLAN_BYTES {
        return Err(WorkflowError::PlanTooLarge);
    }
    let raw: Value = serde_json::from_slice(bytes).map_err(WorkflowError::Json)?;
    validate_value_shape(&raw, 0)?;
    let plan: WorkflowPlan = serde_json::from_value(raw).map_err(WorkflowError::Json)?;
    validate_plan(plan, options)
}

fn validate_value_shape(value: &Value, depth: usize) -> Result<(), WorkflowError> {
    if depth > MAX_PLAN_DEPTH {
        return Err(WorkflowError::Validation(format!(
            "JSON nesting exceeds {MAX_PLAN_DEPTH} levels"
        )));
    }
    match value {
        Value::String(text) if text.len() > MAX_STRING_BYTES => Err(WorkflowError::Validation(
            format!("a string exceeds {MAX_STRING_BYTES} bytes"),
        )),
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| validate_value_shape(value, depth + 1)),
        Value::Object(values) => values
            .values()
            .try_for_each(|value| validate_value_shape(value, depth + 1)),
        _ => Ok(()),
    }
}

fn validate_plan(
    plan: WorkflowPlan,
    options: &WorkflowOptions,
) -> Result<ValidatedWorkflow, WorkflowError> {
    if plan.schema != PLAN_SCHEMA {
        return Err(WorkflowError::Validation(format!(
            "schema must be {PLAN_SCHEMA}"
        )));
    }
    validate_label("workflow name", &plan.name, MAX_NAME_BYTES)?;
    if plan.steps.is_empty() || plan.steps.len() > MAX_STEPS {
        return Err(WorkflowError::Validation(format!(
            "steps must contain between 1 and {MAX_STEPS} entries"
        )));
    }
    validate_limits(&plan.limits)?;
    if plan.steps.first().map(|step| step.tool.as_str()) != Some("open_page") {
        return Err(WorkflowError::Validation(
            "the first step must use open_page".to_string(),
        ));
    }

    let mut ids = BTreeSet::new();
    let mut secret_refs = BTreeSet::new();
    let mut open_count = 0;
    for (index, step) in plan.steps.iter().enumerate() {
        validate_label("step id", &step.id, MAX_ID_BYTES)?;
        if !ids.insert(step.id.clone()) {
            return Err(WorkflowError::Validation(format!(
                "duplicate step id at index {index}"
            )));
        }
        if !allowed_tool(&step.tool) {
            return Err(WorkflowError::Validation(format!(
                "step {index} uses unsupported tool {}",
                step.tool
            )));
        }
        if step.tool == "open_page" {
            open_count += 1;
        }
        let arguments = step.arguments.as_object().ok_or_else(|| {
            WorkflowError::Validation(format!("step {index} arguments must be an object"))
        })?;
        if arguments.contains_key("session_id") {
            return Err(WorkflowError::Validation(format!(
                "step {index} must not provide session_id; the runner owns it"
            )));
        }
        if step.tool == "open_page" && arguments.contains_key("trace") {
            return Err(WorkflowError::Validation(format!(
                "step {index} must set top-level trace; open_page arguments must not provide it"
            )));
        }
        if serde_json::to_vec(&step.arguments)
            .map_err(WorkflowError::Serialize)?
            .len()
            > MAX_ARGUMENT_BYTES
        {
            return Err(WorkflowError::Validation(format!(
                "step {index} arguments exceed {MAX_ARGUMENT_BYTES} bytes"
            )));
        }
        collect_secret_refs(&step.arguments, &mut secret_refs, &format!("step {index}"))?;
        if step.tool == "evaluate" && !options.allow_evaluate {
            return Err(WorkflowError::Validation(
                "evaluate requires --allow-evaluate".to_string(),
            ));
        }
        if cookie_write(&step.tool) && !options.allow_cookie_writes {
            return Err(WorkflowError::Validation(
                "cookie writes require --allow-cookie-writes".to_string(),
            ));
        }
        validate_expectation(&step.expect, index)?;
    }
    if open_count != 1 {
        return Err(WorkflowError::Validation(
            "a workflow must contain exactly one open_page step".to_string(),
        ));
    }

    let mut approvals = BTreeSet::new();
    for id in &options.confirm_steps {
        if !approvals.insert(id.as_str()) {
            return Err(WorkflowError::Validation(format!(
                "duplicate --confirm-step approval for {id}"
            )));
        }
        if !ids.contains(id) {
            return Err(WorkflowError::Validation(format!(
                "--confirm-step names unknown step {id}"
            )));
        }
    }
    for step in &plan.steps {
        if mutates_page(&step.tool) && !approvals.contains(step.id.as_str()) {
            return Err(WorkflowError::Validation(format!(
                "mutating step {} ({}) requires --confirm-step {}",
                step.id, step.tool, step.id
            )));
        }
    }

    let canonical =
        serde_json::to_vec(&plan_for_fingerprint(&plan)).map_err(WorkflowError::Serialize)?;
    let plan_fingerprint = hex::encode(Sha256::digest(canonical));
    Ok(ValidatedWorkflow {
        plan,
        plan_fingerprint,
        secret_refs,
    })
}

fn plan_for_fingerprint(plan: &WorkflowPlan) -> Value {
    // serde_json's map implementation is deterministic for a fixed input. The
    // typed reconstruction also strips irrelevant source formatting.
    json!({
        "schema": plan.schema,
        "name": plan.name,
        "trace": plan.trace,
        "limits": {
            "step_timeout_ms": plan.limits.step_timeout_ms,
            "workflow_timeout_ms": plan.limits.workflow_timeout_ms,
            "response_bytes": plan.limits.response_bytes,
            "stderr_bytes": plan.limits.stderr_bytes,
            "memory_mb": plan.limits.memory_mb,
            "fail_fast": plan.limits.fail_fast,
        },
        "steps": plan.steps.iter().map(|step| json!({
            "id": step.id,
            "tool": step.tool,
            // Secret reference names are low-entropy identifiers. Including
            // them would make the otherwise useful plan fingerprint an
            // offline dictionary oracle, so every reference uses the same
            // constant marker in the fingerprint input.
            "arguments": normalize_secret_references(&step.arguments),
            "expect": {
                "is_error": step.expect.is_error,
                "json_pointer": step.expect.json_pointer,
                "equals": step.expect.equals,
            }
        })).collect::<Vec<_>>()
    })
}

fn normalize_secret_references(value: &Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.iter().map(normalize_secret_references).collect())
        }
        Value::Object(values) if values.len() == 1 && values.contains_key("$secret") => {
            json!({"$secret": "<secret-reference>"})
        }
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), normalize_secret_references(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn validate_limits(limits: &WorkflowLimits) -> Result<(), WorkflowError> {
    if !(10..=60_000).contains(&limits.step_timeout_ms) {
        return Err(WorkflowError::Validation(
            "step_timeout_ms must be between 10 and 60000".to_string(),
        ));
    }
    if !(limits.step_timeout_ms..=600_000).contains(&limits.workflow_timeout_ms) {
        return Err(WorkflowError::Validation(
            "workflow_timeout_ms must be at least step_timeout_ms and at most 600000".to_string(),
        ));
    }
    if !(1024..=MAX_RESPONSE_BYTES_CEILING).contains(&limits.response_bytes) {
        return Err(WorkflowError::Validation(format!(
            "response_bytes must be between 1024 and {MAX_RESPONSE_BYTES_CEILING}"
        )));
    }
    if limits.stderr_bytes > MAX_RESPONSE_BYTES_CEILING {
        return Err(WorkflowError::Validation(format!(
            "stderr_bytes must not exceed {MAX_RESPONSE_BYTES_CEILING}"
        )));
    }
    if limits.memory_mb > 16 * 1024 {
        return Err(WorkflowError::Validation(
            "memory_mb must not exceed 16384".to_string(),
        ));
    }
    Ok(())
}

fn validate_label(label: &str, value: &str, max: usize) -> Result<(), WorkflowError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b' '))
    {
        return Err(WorkflowError::Validation(format!(
            "{label} must be 1-{max} ASCII letters, digits, spaces, '.', '_' or '-'"
        )));
    }
    Ok(())
}

fn validate_expectation(expect: &StepExpectation, index: usize) -> Result<(), WorkflowError> {
    match (&expect.json_pointer, &expect.equals) {
        (None, Some(_)) => Err(WorkflowError::Validation(format!(
            "step {index} expect.equals requires expect.json_pointer"
        ))),
        (Some(pointer), _) if !pointer.is_empty() && !pointer.starts_with('/') => {
            Err(WorkflowError::Validation(format!(
                "step {index} expect.json_pointer must be empty or start with '/'"
            )))
        }
        _ => Ok(()),
    }
}

fn allowed_tool(tool: &str) -> bool {
    matches!(
        tool,
        "open_page"
            | "navigate_to"
            | "click"
            | "type_text"
            | "select_option"
            | "scroll"
            | "toggle"
            | "clear"
            | "evaluate"
            | "get_cookies"
            | "set_cookies"
            | "clear_cookies"
            | "trace_status"
            | "trace_export"
            | "trace_clear"
            | "replay_validate"
    )
}

fn mutates_page(tool: &str) -> bool {
    matches!(
        tool,
        "navigate_to"
            | "click"
            | "type_text"
            | "select_option"
            | "scroll"
            | "toggle"
            | "clear"
            | "evaluate"
            | "set_cookies"
            | "clear_cookies"
            | "trace_clear"
    )
}

fn cookie_write(tool: &str) -> bool {
    matches!(tool, "set_cookies" | "clear_cookies")
}

fn collect_secret_refs(
    value: &Value,
    output: &mut BTreeSet<String>,
    location: &str,
) -> Result<(), WorkflowError> {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_secret_refs(value, output, location)?;
            }
        }
        Value::Object(values) => {
            if let Some(secret) = values.get("$secret") {
                if values.len() != 1 {
                    return Err(WorkflowError::Validation(format!(
                        "{location} secret reference must contain only $secret"
                    )));
                }
                let name = secret.as_str().ok_or_else(|| {
                    WorkflowError::Validation(format!("{location} $secret must be a string"))
                })?;
                if name.is_empty()
                    || name.len() > 64
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
                    || name.as_bytes()[0].is_ascii_digit()
                {
                    return Err(WorkflowError::Validation(format!(
                        "{location} secret name must match [A-Z_][A-Z0-9_]{{0,63}}"
                    )));
                }
                output.insert(name.to_string());
            } else {
                for value in values.values() {
                    collect_secret_refs(value, output, location)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn secret_report(names: &BTreeSet<String>, inspect_environment: bool) -> SecretSummary {
    let present = inspect_environment.then(|| {
        names
            .iter()
            .filter(|name| std::env::var_os(name).is_some())
            .count()
    });
    SecretSummary {
        total: names.len(),
        present,
        missing: present.map(|present| names.len() - present),
    }
}

fn resolve_secrets(value: &Value) -> Result<Value, &'static str> {
    match value {
        Value::Array(values) => values
            .iter()
            .map(resolve_secrets)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) if values.contains_key("$secret") => {
            let name = values["$secret"]
                .as_str()
                .ok_or("invalid_secret_reference")?;
            let value = std::env::var(name).map_err(|_| "missing_secret")?;
            if value.len() > MAX_STRING_BYTES {
                return Err("secret_too_large");
            }
            Ok(Value::String(value))
        }
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), resolve_secrets(value)?)))
            .collect::<Result<Map<_, _>, &'static str>>()
            .map(Value::Object),
        _ => Ok(value.clone()),
    }
}

fn child_environment() -> Vec<(OsString, OsString)> {
    // Deliberately excludes HOME, cloud credentials, proxies, auth-profile
    // settings, and PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK. Secret references
    // are resolved by the parent and sent only in their individual request.
    const ALLOWED: &[&str] = &[
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "PLASMATE_ICU_DATA",
        "RUST_BACKTRACE",
        "SYSTEMROOT",
        "WINDIR",
    ];
    ALLOWED
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect()
}

pub fn execute(
    workflow: ValidatedWorkflow,
    options: &WorkflowOptions,
) -> Result<WorkflowReport, WorkflowError> {
    let program = std::env::current_exe().map_err(WorkflowError::CurrentExecutable)?;
    Ok(execute_with_program(
        workflow,
        options,
        program,
        vec![
            OsString::from("mcp"),
            OsString::from("--transport"),
            OsString::from("stdio"),
        ],
    ))
}

/// Test seam for deterministic fake MCP children. The public CLI never exposes
/// this program or argument override.
#[doc(hidden)]
pub fn execute_with_program(
    workflow: ValidatedWorkflow,
    options: &WorkflowOptions,
    program: PathBuf,
    args: Vec<OsString>,
) -> WorkflowReport {
    execute_with_child_environment(workflow, options, program, args, child_environment())
}

/// Execute one of the compiled deterministic agent-benchmark plans against the
/// real MCP child and its ephemeral loopback fixture. This exception is not
/// exposed by `agent-run`: benchmark plans contain no caller-controlled URL,
/// and the private-network opt-in exists only in the supervised child's empty
/// environment for the duration of that fixed plan.
pub(crate) fn execute_for_local_benchmark(
    workflow: ValidatedWorkflow,
    options: &WorkflowOptions,
    program: PathBuf,
) -> WorkflowReport {
    let mut environment = child_environment();
    environment.push((
        OsString::from(crate::network::security::UNSAFE_PRIVATE_NETWORK_ENV),
        OsString::from("1"),
    ));
    execute_with_child_environment(
        workflow,
        options,
        program,
        vec![
            OsString::from("mcp"),
            OsString::from("--transport"),
            OsString::from("stdio"),
        ],
        environment,
    )
}

fn execute_with_child_environment(
    workflow: ValidatedWorkflow,
    options: &WorkflowOptions,
    program: PathBuf,
    args: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
) -> WorkflowReport {
    let mut report = base_report(&workflow, options);
    if options.dry_run {
        report.status = WorkflowStatus::Validated;
        return report;
    }

    if workflow
        .secret_refs
        .iter()
        .any(|name| std::env::var_os(name).is_none())
    {
        mark_all_skipped(&mut report, "missing_secret");
        return report;
    }

    // Resolve and size-check every secret-bearing argument before process or
    // network side effects. The values remain parent-owned until their one
    // bounded request and are never copied into reports.
    let resolved_arguments: Vec<Value> = match workflow
        .plan
        .steps
        .iter()
        .map(|step| resolve_secrets(&step.arguments))
        .collect()
    {
        Ok(arguments) => arguments,
        Err(class) => {
            mark_all_skipped(&mut report, class);
            return report;
        }
    };
    if resolved_arguments.iter().any(|arguments| {
        serde_json::to_vec(arguments)
            .map(|encoded| encoded.len() > MAX_ARGUMENT_BYTES)
            .unwrap_or(true)
    }) {
        mark_all_skipped(&mut report, "resolved_arguments_too_large");
        return report;
    }

    let limits = workflow.plan.limits.clone();
    let spec = InteractiveProcessSpec {
        program,
        args,
        env: environment,
        max_line_bytes: limits.response_bytes,
        max_stderr_bytes: limits.stderr_bytes,
        memory_limit_bytes: limits.memory_mb.saturating_mul(1024 * 1024),
    };
    let mut child = match InteractiveProcess::spawn(spec) {
        Ok(child) => child,
        Err(_) => {
            mark_all_skipped(&mut report, "spawn_failed");
            return report;
        }
    };
    let started = Instant::now();
    let whole = Duration::from_millis(limits.workflow_timeout_ms);
    let step_timeout = Duration::from_millis(limits.step_timeout_ms);
    let mut request_id = 1_u64;

    let initialize = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "plasmate-agent-workflow", "version": env!("CARGO_PKG_VERSION")}
        }
    });
    if let Err(class) = rpc_request(
        &mut child,
        &initialize,
        request_id,
        bounded_timeout(started, whole, step_timeout),
    ) {
        mark_all_skipped(&mut report, class);
        report.process_outcome = Some(outcome_name(child.shutdown(Duration::from_millis(250))));
        return report;
    }
    let notification = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    let notification_timeout = bounded_timeout(started, whole, step_timeout);
    let notification_result = if notification_timeout.is_zero() {
        Err("workflow_timeout")
    } else {
        child
            .notify(
                &serde_json::to_vec(&notification).unwrap_or_default(),
                notification_timeout,
            )
            .map_err(classify_process_error)
    };
    if let Err(class) = notification_result {
        mark_all_skipped(&mut report, class);
        report.process_outcome = Some(outcome_name(child.shutdown(Duration::from_millis(250))));
        return report;
    }
    request_id += 1;
    let list = json!({"jsonrpc":"2.0","id":request_id,"method":"tools/list","params":{}});
    let listed = rpc_request(
        &mut child,
        &list,
        request_id,
        bounded_timeout(started, whole, step_timeout),
    )
    .ok()
    .and_then(|response| response.get("result").cloned());
    let schemas = listed
        .as_ref()
        .ok_or("tool_schema_drift")
        .and_then(advertised_schemas)
        .and_then(|schemas| {
            validate_advertised_arguments(&workflow.plan, &resolved_arguments, &schemas)?;
            Ok(schemas)
        });
    if let Err(class) = schemas {
        mark_all_skipped(&mut report, class);
        report.process_outcome = Some(outcome_name(child.shutdown(Duration::from_millis(250))));
        return report;
    }

    let mut session_id: Option<String> = None;
    let mut stop = false;
    let mut transport_failed = false;
    for (index, step) in workflow.plan.steps.iter().enumerate() {
        if stop {
            report.steps[index].status = StepStatus::Skipped;
            report.steps[index].failure_class = Some("fail_fast".to_string());
            continue;
        }
        if started.elapsed() >= whole {
            report.steps[index].status = StepStatus::Failed;
            report.steps[index].failure_class = Some("workflow_timeout".to_string());
            stop = true;
            continue;
        }
        let mut arguments = resolved_arguments[index].clone();
        if step.tool == "open_page" {
            arguments
                .as_object_mut()
                .expect("validated arguments object")
                .insert("trace".to_string(), Value::Bool(workflow.plan.trace));
        } else if let Some(session_id) = &session_id {
            arguments
                .as_object_mut()
                .expect("validated arguments object")
                .insert("session_id".to_string(), Value::String(session_id.clone()));
        } else {
            report.steps[index].status = StepStatus::Failed;
            report.steps[index].failure_class = Some("missing_session".to_string());
            stop = limits.fail_fast;
            continue;
        }
        request_id += 1;
        let request = json!({
            "jsonrpc":"2.0",
            "id":request_id,
            "method":"tools/call",
            "params":{"name":step.tool,"arguments":arguments}
        });
        let response = rpc_request(
            &mut child,
            &request,
            request_id,
            bounded_timeout(started, whole, step_timeout),
        );
        match response {
            Err(class) => {
                report.steps[index].status = StepStatus::Failed;
                report.steps[index].failure_class = Some(class.to_string());
                stop = true;
                transport_failed = true;
            }
            Ok(response) => match evaluate_response(&response, &step.expect) {
                Ok(payload) => {
                    if step.tool == "open_page" {
                        session_id = payload
                            .as_ref()
                            .and_then(|payload| payload.get("session_id"))
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty() && id.len() <= 64)
                            .map(str::to_string);
                        if session_id.is_none() {
                            report.steps[index].status = StepStatus::Failed;
                            report.steps[index].failure_class = Some("invalid_session".to_string());
                            stop = limits.fail_fast;
                            continue;
                        }
                    }
                    report.steps[index].status = StepStatus::Succeeded;
                }
                Err(class) => {
                    report.steps[index].status = StepStatus::Failed;
                    report.steps[index].failure_class = Some(class.to_string());
                    stop = limits.fail_fast;
                }
            },
        }
    }

    // Cleanup is runner-owned and never depends on a user confirmation. It is
    // best effort and its payload is never exposed in the report.
    if !transport_failed {
        if let Some(session_id) = session_id {
            request_id += 1;
            let close = json!({
                "jsonrpc":"2.0","id":request_id,"method":"tools/call",
                "params":{"name":"close_page","arguments":{"session_id":session_id}}
            });
            let _ = rpc_request(
                &mut child,
                &close,
                request_id,
                bounded_timeout(started, whole, Duration::from_millis(1_000)),
            );
        }
    }
    let process_outcome = child.shutdown(if transport_failed {
        Duration::ZERO
    } else {
        Duration::from_millis(500)
    });
    let clean_exit = matches!(&process_outcome, ProcessOutcome::Exited { code: 0 });
    report.process_outcome = Some(outcome_name(process_outcome));
    finish_summary(&mut report);
    if !clean_exit {
        report.status = WorkflowStatus::Failed;
    }
    report
}

fn bounded_timeout(started: Instant, whole: Duration, step: Duration) -> Duration {
    whole.saturating_sub(started.elapsed()).min(step)
}

fn rpc_request(
    child: &mut InteractiveProcess,
    request: &Value,
    expected_id: u64,
    timeout: Duration,
) -> Result<Value, &'static str> {
    if timeout.is_zero() {
        return Err("workflow_timeout");
    }
    let encoded = serde_json::to_vec(request).map_err(|_| "request_serialization")?;
    let response = child
        .exchange(&encoded, timeout)
        .map_err(classify_process_error)?;
    let response: Value = serde_json::from_slice(&response).map_err(|_| "malformed_response")?;
    if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || response.get("id").and_then(Value::as_u64) != Some(expected_id)
    {
        return Err("protocol_mismatch");
    }
    if response.get("error").is_some() {
        return Err("json_rpc_error");
    }
    if response.get("result").is_none() {
        return Err("missing_result");
    }
    Ok(response)
}

fn classify_process_error(error: InteractiveProcessError) -> &'static str {
    match error {
        InteractiveProcessError::TimedOut => "step_timeout",
        InteractiveProcessError::Oversized => "oversized_response",
        InteractiveProcessError::EarlyExit => "early_exit",
        InteractiveProcessError::Spawn(_) => "spawn_failed",
        InteractiveProcessError::MissingPipe(_) => "missing_pipe",
        InteractiveProcessError::Write(_) => "write_failed",
        InteractiveProcessError::Read(_) => "read_failed",
    }
}

fn advertised_schemas(result: &Value) -> Result<BTreeMap<String, Value>, &'static str> {
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or("tool_schema_drift")?;
    let mut schemas = BTreeMap::new();
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or("tool_schema_drift")?;
        let schema = tool.get("inputSchema").ok_or("tool_schema_drift")?;
        if schemas.insert(name.to_string(), schema.clone()).is_some() {
            return Err("tool_schema_drift");
        }
    }
    Ok(schemas)
}

fn validate_advertised_arguments(
    plan: &WorkflowPlan,
    resolved_arguments: &[Value],
    schemas: &BTreeMap<String, Value>,
) -> Result<(), &'static str> {
    if !schemas.contains_key("close_page") {
        return Err("tool_schema_drift");
    }
    if resolved_arguments.len() != plan.steps.len() {
        return Err("tool_schema_drift");
    }
    for (step, resolved) in plan.steps.iter().zip(resolved_arguments) {
        let schema = schemas.get(&step.tool).ok_or("tool_schema_drift")?;
        let mut arguments = resolved.clone();
        let object = arguments.as_object_mut().ok_or("argument_schema_invalid")?;
        if step.tool == "open_page" {
            object.insert("trace".to_string(), Value::Bool(plan.trace));
        } else {
            object.insert(
                "session_id".to_string(),
                Value::String("validated-session-placeholder".to_string()),
            );
        }
        validate_schema_value(&arguments, schema)?;
    }
    Ok(())
}

fn validate_schema_value(value: &Value, schema: &Value) -> Result<(), &'static str> {
    const SUPPORTED: &[&str] = &[
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "const",
        "minLength",
        "maxLength",
        "pattern",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minItems",
        "maxItems",
        "uniqueItems",
        "minProperties",
        "maxProperties",
        // Explicit annotations. `format` is intentionally not an assertion.
        "title",
        "description",
        "default",
        "examples",
        "format",
        "deprecated",
        "readOnly",
        "writeOnly",
    ];
    let schema = schema.as_object().ok_or("tool_schema_drift")?;
    if schema.keys().any(|key| !SUPPORTED.contains(&key.as_str())) {
        return Err("tool_schema_drift");
    }
    validate_annotation_shapes(schema)?;

    if let Some(allowed_value) = schema.get("const") {
        if value != allowed_value {
            return Err("argument_schema_invalid");
        }
    }
    if let Some(enum_value) = schema.get("enum") {
        let allowed = enum_value.as_array().ok_or("tool_schema_drift")?;
        if !allowed.contains(value) {
            return Err("argument_schema_invalid");
        }
    }
    let schema_type = schema
        .get("type")
        .and_then(Value::as_str)
        .ok_or("tool_schema_drift")?;
    for keyword in schema.keys() {
        let generic = matches!(
            keyword.as_str(),
            "type"
                | "enum"
                | "const"
                | "title"
                | "description"
                | "default"
                | "examples"
                | "format"
                | "deprecated"
                | "readOnly"
                | "writeOnly"
        );
        let applicable = match schema_type {
            "object" => matches!(
                keyword.as_str(),
                "properties"
                    | "required"
                    | "additionalProperties"
                    | "minProperties"
                    | "maxProperties"
            ),
            "array" => matches!(
                keyword.as_str(),
                "items" | "minItems" | "maxItems" | "uniqueItems"
            ),
            "string" => matches!(keyword.as_str(), "minLength" | "maxLength" | "pattern"),
            "integer" | "number" => matches!(
                keyword.as_str(),
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf"
            ),
            "boolean" | "null" => false,
            _ => return Err("tool_schema_drift"),
        };
        if !generic && !applicable {
            return Err("tool_schema_drift");
        }
    }
    match schema_type {
        "object" => {
            let object = value.as_object().ok_or("argument_schema_invalid")?;
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .ok_or("tool_schema_drift")?;
            // Stateful handlers are not uniformly strict about unknown fields.
            // The workflow contract is deliberately closed to catch plan drift.
            if object.keys().any(|key| !properties.contains_key(key)) {
                return Err("argument_schema_invalid");
            }
            if let Some(required) = schema.get("required") {
                let required = required.as_array().ok_or("tool_schema_drift")?;
                for key in required {
                    let key = key.as_str().ok_or("tool_schema_drift")?;
                    if !object.contains_key(key) {
                        return Err("argument_schema_invalid");
                    }
                }
            }
            for (key, nested) in object {
                validate_schema_value(nested, &properties[key])?;
            }
            validate_count_constraint(schema, "minProperties", object.len(), false)?;
            validate_count_constraint(schema, "maxProperties", object.len(), true)?;
            if let Some(additional) = schema.get("additionalProperties") {
                if !additional.is_boolean() {
                    return Err("tool_schema_drift");
                }
            }
        }
        "array" => {
            let array = value.as_array().ok_or("argument_schema_invalid")?;
            let items = schema.get("items").ok_or("tool_schema_drift")?;
            for nested in array {
                validate_schema_value(nested, items)?;
            }
            validate_count_constraint(schema, "minItems", array.len(), false)?;
            validate_count_constraint(schema, "maxItems", array.len(), true)?;
            if let Some(unique) = schema.get("uniqueItems") {
                let unique = unique.as_bool().ok_or("tool_schema_drift")?;
                if unique
                    && array
                        .iter()
                        .enumerate()
                        .any(|(index, item)| array[index + 1..].contains(item))
                {
                    return Err("argument_schema_invalid");
                }
            }
        }
        "string" => {
            let text = value.as_str().ok_or("argument_schema_invalid")?;
            validate_count_constraint(schema, "minLength", text.chars().count(), false)?;
            validate_count_constraint(schema, "maxLength", text.chars().count(), true)?;
            if let Some(pattern) = schema.get("pattern") {
                let pattern = pattern.as_str().ok_or("tool_schema_drift")?;
                let expression = regex::Regex::new(pattern).map_err(|_| "tool_schema_drift")?;
                if !expression.is_match(text) {
                    return Err("argument_schema_invalid");
                }
            }
        }
        "integer" => {
            if !(value.as_i64().is_some() || value.as_u64().is_some()) {
                return Err("argument_schema_invalid");
            }
            validate_numeric_constraints(schema, value)?;
        }
        "number" => {
            if !value.is_number() {
                return Err("argument_schema_invalid");
            }
            validate_numeric_constraints(schema, value)?;
        }
        "boolean" if !value.is_boolean() => return Err("argument_schema_invalid"),
        "null" if !value.is_null() => return Err("argument_schema_invalid"),
        "boolean" | "null" => {}
        _ => return Err("tool_schema_drift"),
    }
    Ok(())
}

fn validate_annotation_shapes(schema: &Map<String, Value>) -> Result<(), &'static str> {
    for key in ["title", "description", "format"] {
        if schema.get(key).is_some_and(|value| !value.is_string()) {
            return Err("tool_schema_drift");
        }
    }
    if schema
        .get("examples")
        .is_some_and(|value| !value.is_array())
    {
        return Err("tool_schema_drift");
    }
    for key in ["deprecated", "readOnly", "writeOnly"] {
        if schema.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err("tool_schema_drift");
        }
    }
    Ok(())
}

fn validate_count_constraint(
    schema: &Map<String, Value>,
    keyword: &str,
    actual: usize,
    is_maximum: bool,
) -> Result<(), &'static str> {
    let Some(limit) = schema.get(keyword) else {
        return Ok(());
    };
    let limit = limit.as_u64().ok_or("tool_schema_drift")?;
    if (is_maximum && actual as u64 > limit) || (!is_maximum && (actual as u64) < limit) {
        return Err("argument_schema_invalid");
    }
    Ok(())
}

fn validate_numeric_constraints(
    schema: &Map<String, Value>,
    value: &Value,
) -> Result<(), &'static str> {
    let actual = value.as_f64().ok_or("argument_schema_invalid")?;
    for (keyword, comparison) in [
        ("minimum", 0_u8),
        ("maximum", 1),
        ("exclusiveMinimum", 2),
        ("exclusiveMaximum", 3),
    ] {
        let Some(limit) = schema.get(keyword) else {
            continue;
        };
        let limit = limit.as_f64().ok_or("tool_schema_drift")?;
        let violates = match comparison {
            0 => actual < limit,
            1 => actual > limit,
            2 => actual <= limit,
            _ => actual >= limit,
        };
        if violates {
            return Err("argument_schema_invalid");
        }
    }
    if let Some(divisor) = schema.get("multipleOf") {
        let divisor = divisor.as_f64().ok_or("tool_schema_drift")?;
        if divisor <= 0.0 {
            return Err("tool_schema_drift");
        }
        let quotient = actual / divisor;
        if (quotient - quotient.round()).abs() > f64::EPSILON * quotient.abs().max(1.0) * 8.0 {
            return Err("argument_schema_invalid");
        }
    }
    Ok(())
}

fn evaluate_response(
    response: &Value,
    expectation: &StepExpectation,
) -> Result<Option<Value>, &'static str> {
    let result = response.get("result").ok_or("missing_result")?;
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error != expectation.is_error {
        return Err("tool_error_expectation");
    }
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str);
    let payload = text.and_then(|text| serde_json::from_str::<Value>(text).ok());
    if let Some(pointer) = expectation.json_pointer.as_deref() {
        let actual = payload
            .as_ref()
            .and_then(|payload| payload.pointer(pointer))
            .ok_or("expectation_path_missing")?;
        if let Some(expected) = &expectation.equals {
            if actual != expected {
                return Err("expectation_mismatch");
            }
        }
    }
    Ok(payload)
}

fn base_report(workflow: &ValidatedWorkflow, options: &WorkflowOptions) -> WorkflowReport {
    let approved: BTreeSet<_> = options.confirm_steps.iter().map(String::as_str).collect();
    WorkflowReport {
        schema: REPORT_SCHEMA,
        plan_schema: PLAN_SCHEMA,
        protocol_version: MCP_PROTOCOL,
        workflow: workflow.plan.name.clone(),
        plan_fingerprint: workflow.plan_fingerprint.clone(),
        dry_run: options.dry_run,
        trace_requested: workflow.plan.trace,
        status: WorkflowStatus::Failed,
        summary: WorkflowSummary {
            total: workflow.plan.steps.len(),
            succeeded: 0,
            failed: 0,
            skipped: workflow.plan.steps.len(),
        },
        secrets: secret_report(&workflow.secret_refs, !options.dry_run),
        steps: workflow
            .plan
            .steps
            .iter()
            .map(|step| StepReport {
                id: step.id.clone(),
                tool: step.tool.clone(),
                status: StepStatus::Skipped,
                approved: approved.contains(step.id.as_str()),
                failure_class: None,
            })
            .collect(),
        process_outcome: None,
    }
}

fn mark_all_skipped(report: &mut WorkflowReport, class: &str) {
    for step in &mut report.steps {
        step.failure_class = Some(class.to_string());
    }
    finish_summary(report);
}

fn finish_summary(report: &mut WorkflowReport) {
    report.summary.succeeded = report
        .steps
        .iter()
        .filter(|step| step.status == StepStatus::Succeeded)
        .count();
    report.summary.failed = report
        .steps
        .iter()
        .filter(|step| step.status == StepStatus::Failed)
        .count();
    report.summary.skipped =
        report.summary.total - report.summary.succeeded - report.summary.failed;
    report.status = if report.summary.failed == 0 && report.summary.skipped == 0 {
        WorkflowStatus::Succeeded
    } else {
        WorkflowStatus::Failed
    };
}

fn outcome_name(outcome: ProcessOutcome) -> String {
    match outcome {
        ProcessOutcome::Exited { code } => format!("exited:{code}"),
        ProcessOutcome::Signaled { signal } => format!("signaled:{signal}"),
        ProcessOutcome::TimedOut => "timed_out".to_string(),
    }
}

pub fn write_report(path: &Path, report: &WorkflowReport) -> Result<(), WorkflowError> {
    let mut bytes = serde_json::to_vec_pretty(report).map_err(WorkflowError::Serialize)?;
    bytes.push(b'\n');
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(WorkflowError::Write)?;
    use std::io::Write;
    temporary.write_all(&bytes).map_err(WorkflowError::Write)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(WorkflowError::Write)?;
    temporary
        .persist(path)
        .map_err(|error| WorkflowError::Write(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_plan(extra_step: &str) -> Vec<u8> {
        format!(
            r#"{{
              "schema":"plasmate.agent-workflow.v1",
              "name":"test plan",
              "steps":[
                {{"id":"open","tool":"open_page","arguments":{{"url":"https://example.test"}}}}
                {extra_step}
              ]
            }}"#
        )
        .into_bytes()
    }

    #[test]
    fn dry_run_is_complete_and_has_no_process_outcome() {
        let options = WorkflowOptions {
            dry_run: true,
            ..Default::default()
        };
        let workflow = validate_bytes(&valid_plan(""), &options).unwrap();
        let report = execute_with_program(
            workflow,
            &options,
            PathBuf::from("definitely-not-executed"),
            Vec::new(),
        );
        assert_eq!(report.status, WorkflowStatus::Validated);
        assert_eq!(report.summary.total, 1);
        assert!(report.process_outcome.is_none());
    }

    #[test]
    fn replay_validation_is_a_read_only_allowed_workflow_step() {
        let options = WorkflowOptions {
            dry_run: true,
            ..Default::default()
        };
        let plan = valid_plan(
            r#",{"id":"replay","tool":"replay_validate","arguments":{"trace_id":"another-trace","sequence":1}}"#,
        );
        let workflow = validate_bytes(&plan, &options)
            .expect("read-only replay validation must not require mutation approval");
        let report = execute_with_program(
            workflow,
            &options,
            PathBuf::from("not-executed"),
            Vec::new(),
        );
        assert_eq!(report.status, WorkflowStatus::Validated);
        assert_eq!(report.summary.total, 2);
    }

    #[test]
    fn rejects_unconfirmed_mutation_before_execution() {
        let options = WorkflowOptions {
            confirm_steps: vec![],
            ..Default::default()
        };
        let error = validate_bytes(
            &valid_plan(r#",{"id":"click","tool":"click","arguments":{"element_id":"e1"}}"#),
            &options,
        )
        .unwrap_err();
        assert!(error.to_string().contains("--confirm-step click"));

        let self_attested = valid_plan(
            r#",{"id":"click","tool":"click","confirmed":true,"arguments":{"element_id":"e1"}}"#,
        );
        assert!(validate_bytes(&self_attested, &options).is_err());
    }

    #[test]
    fn sensitive_tools_require_specific_opt_ins() {
        let evaluate =
            valid_plan(r#",{"id":"eval","tool":"evaluate","arguments":{"expression":"1+1"}}"#);
        let evaluate_options = WorkflowOptions {
            confirm_steps: vec!["eval".to_string()],
            ..Default::default()
        };
        assert!(validate_bytes(&evaluate, &evaluate_options)
            .unwrap_err()
            .to_string()
            .contains("--allow-evaluate"));
        let cookies = valid_plan(r#",{"id":"cookies","tool":"clear_cookies","arguments":{}}"#);
        let cookie_options = WorkflowOptions {
            confirm_steps: vec!["cookies".to_string()],
            ..Default::default()
        };
        assert!(validate_bytes(&cookies, &cookie_options)
            .unwrap_err()
            .to_string()
            .contains("--allow-cookie-writes"));
    }

    #[test]
    fn rejects_session_injection_and_multiple_open_steps() {
        let injected = br#"{"schema":"plasmate.agent-workflow.v1","name":"x","steps":[{"id":"open","tool":"open_page","arguments":{"url":"x","session_id":"stolen"}}]}"#;
        assert!(validate_bytes(injected, &WorkflowOptions::default()).is_err());
        let duplicate = valid_plan(
            r#",{"id":"open2","tool":"open_page","arguments":{"url":"https://example.test"}}"#,
        );
        assert!(validate_bytes(&duplicate, &WorkflowOptions::default()).is_err());
        let trace_injection = br#"{"schema":"plasmate.agent-workflow.v1","name":"x","steps":[{"id":"open","tool":"open_page","arguments":{"url":"x","trace":true}}]}"#;
        assert!(validate_bytes(trace_injection, &WorkflowOptions::default()).is_err());
    }

    #[test]
    fn one_approval_cannot_authorize_another_step() {
        let plan = valid_plan(
            r#",{"id":"first","tool":"click","arguments":{"element_id":"e1"}},{"id":"second","tool":"clear","arguments":{"element_id":"e2"}}"#,
        );
        let options = WorkflowOptions {
            confirm_steps: vec!["first".to_string()],
            ..Default::default()
        };
        let error = validate_bytes(&plan, &options).unwrap_err();
        assert!(error.to_string().contains("--confirm-step second"));
    }

    #[test]
    fn rejects_unknown_and_duplicate_approvals() {
        for confirmations in [
            vec!["unknown".to_string()],
            vec!["open".to_string(), "open".to_string()],
        ] {
            let options = WorkflowOptions {
                confirm_steps: confirmations,
                ..Default::default()
            };
            assert!(validate_bytes(&valid_plan(""), &options).is_err());
        }
    }

    #[test]
    fn rejects_unknown_fields_deep_nesting_and_oversized_strings() {
        let unknown =
            br#"{"schema":"plasmate.agent-workflow.v1","name":"x","unknown":true,"steps":[]}"#;
        assert!(validate_bytes(unknown, &WorkflowOptions::default()).is_err());
        let mut nested = json!(null);
        for _ in 0..=MAX_PLAN_DEPTH {
            nested = json!([nested]);
        }
        assert!(validate_bytes(
            &serde_json::to_vec(&nested).unwrap(),
            &WorkflowOptions::default()
        )
        .is_err());
        let huge = "x".repeat(MAX_STRING_BYTES + 1);
        let bytes = serde_json::to_vec(&json!({"schema":PLAN_SCHEMA,"name":"x","steps":[{"id":"open","tool":"open_page","arguments":{"url":huge}}]})).unwrap();
        assert!(validate_bytes(&bytes, &WorkflowOptions::default()).is_err());
    }

    #[test]
    fn secret_references_are_narrow_and_reports_do_not_name_them() {
        let bytes = serde_json::to_vec(&json!({
            "schema": PLAN_SCHEMA,
            "name": "secret",
            "steps": [{"id":"open","tool":"open_page","arguments":{"url":{"$secret":"PLASMATE_TEST_URL"}}}]
        })).unwrap();
        let workflow = validate_bytes(&bytes, &WorkflowOptions::default()).unwrap();
        let report = base_report(&workflow, &WorkflowOptions::default());
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("PLASMATE_TEST_URL"));
        assert!(!encoded.contains("$secret"));
    }

    #[test]
    fn secret_names_cannot_be_inferred_from_report_fingerprints() {
        use hmac::{Hmac, Mac};

        fn plan(name: &str) -> Vec<u8> {
            serde_json::to_vec(&json!({
                "schema": PLAN_SCHEMA,
                "name": "secret-identity",
                "steps": [{
                    "id":"open",
                    "tool":"open_page",
                    "arguments":{"url":{"$secret":name}}
                }]
            }))
            .unwrap()
        }

        let options = WorkflowOptions {
            dry_run: true,
            ..Default::default()
        };
        let first_name = "PLASMATE_PRIMARY_TOKEN";
        let second_name = "PLASMATE_BACKUP_TOKEN";
        let first = validate_bytes(&plan(first_name), &options).unwrap();
        let second = validate_bytes(&plan(second_name), &options).unwrap();
        assert_eq!(first.plan_fingerprint, second.plan_fingerprint);

        let first_report = base_report(&first, &options);
        let second_report = base_report(&second, &options);
        assert_eq!(
            serde_json::to_value(&first_report).unwrap(),
            serde_json::to_value(&second_report).unwrap(),
            "changing only a secret reference name must not change report identity"
        );

        let encoded = serde_json::to_string(&first_report).unwrap();
        for name in [first_name, second_name] {
            let ordinary_hash = hex::encode(Sha256::digest(name.as_bytes()));
            let mut mac =
                Hmac::<Sha256>::new_from_slice(b"plasmate-workflow-secret-reference-v1").unwrap();
            mac.update(name.as_bytes());
            let legacy_hmac = hex::encode(&mac.finalize().into_bytes()[..12]);
            assert!(!encoded.contains(name));
            assert!(!encoded.contains(&ordinary_hash));
            assert!(!encoded.contains(&legacy_hmac));
        }
        assert_eq!(first_report.secrets.total, 1);
        assert_eq!(first_report.secrets.present, None);
        assert_eq!(first_report.secrets.missing, None);
    }

    #[test]
    fn dry_run_does_not_inspect_secret_environment() {
        let bytes = serde_json::to_vec(&json!({
            "schema": PLAN_SCHEMA,
            "name": "secret-dry-run",
            "steps": [{"id":"open","tool":"open_page","arguments":{"url":{"$secret":"PLASMATE_DRY_RUN_SECRET"}}}]
        })).unwrap();
        std::env::set_var("PLASMATE_DRY_RUN_SECRET", "not-observed");
        let options = WorkflowOptions {
            dry_run: true,
            ..Default::default()
        };
        let workflow = validate_bytes(&bytes, &options).unwrap();
        let report = execute_with_program(workflow, &options, PathBuf::from("not-run"), Vec::new());
        std::env::remove_var("PLASMATE_DRY_RUN_SECRET");
        assert_eq!(report.secrets.present, None);
    }

    #[test]
    fn expectation_checks_parsed_text_without_returning_payload() {
        let response = json!({"result":{"content":[{"type":"text","text":"{\"ok\":true}"}]}});
        let expectation = StepExpectation {
            json_pointer: Some("/ok".to_string()),
            equals: Some(Value::Bool(true)),
            ..Default::default()
        };
        assert!(evaluate_response(&response, &expectation).is_ok());
        let mismatch = StepExpectation {
            equals: Some(Value::Bool(false)),
            ..expectation
        };
        assert_eq!(
            evaluate_response(&response, &mismatch),
            Err("expectation_mismatch")
        );
    }

    #[test]
    fn schema_validator_enforces_numeric_string_and_array_constraints() {
        let numeric = json!({"type":"integer","minimum":2,"maximum":4});
        assert_eq!(
            validate_schema_value(&json!(1), &numeric),
            Err("argument_schema_invalid")
        );
        assert!(validate_schema_value(&json!(3), &numeric).is_ok());

        let string = json!({"type":"string","minLength":3,"maxLength":5,"pattern":"^[a-z]+$"});
        assert!(validate_schema_value(&json!("abcd"), &string).is_ok());
        assert_eq!(
            validate_schema_value(&json!("AB"), &string),
            Err("argument_schema_invalid")
        );

        let array = json!({"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":3,"uniqueItems":true});
        assert!(validate_schema_value(&json!([1, 2]), &array).is_ok());
        assert_eq!(
            validate_schema_value(&json!([1, 1]), &array),
            Err("argument_schema_invalid")
        );
        assert_eq!(
            validate_schema_value(&json!([1, 2, 3, 4]), &array),
            Err("argument_schema_invalid")
        );
    }

    #[test]
    fn schema_validator_fails_closed_on_unsupported_or_malformed_keywords() {
        assert_eq!(
            validate_schema_value(&json!("x"), &json!({"type":"string","oneOf":[]})),
            Err("tool_schema_drift")
        );
        assert_eq!(
            validate_schema_value(&json!("x"), &json!({"type":"string","maxLength":"4"})),
            Err("tool_schema_drift")
        );
        assert_eq!(
            validate_schema_value(&json!(3), &json!({"type":"integer","minimum":"2"})),
            Err("tool_schema_drift")
        );
    }
}
