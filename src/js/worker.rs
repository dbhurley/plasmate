//! Process-isolated JavaScript execution protocol.
//!
//! V8 can terminate its host process on fatal errors and ordinary classic
//! scripts can run forever. Public CLI/server paths therefore send prepared
//! page work or one-shot stateful evaluations to a supervised Plasmate child.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::modules::ModuleGraph;
use super::runtime::{JsExecutionReport, JsRuntime, RuntimeConfig};
use crate::process_supervisor::{
    self, ProcessOutcome, ProcessOutput, ProcessSpec, SupervisorError,
};

pub const WORKER_PROTOCOL_VERSION: &str = "plasmate.js-worker.v1";
pub const DEFAULT_WORKER_TIMEOUT_MS: u64 = 15_000;
pub const DEFAULT_WORKER_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_WORKER_STDERR_BYTES: usize = 256 * 1024;
pub const MAX_WORKER_INPUT_BYTES: usize = 32 * 1024 * 1024;

fn protocol_version() -> String {
    WORKER_PROTOCOL_VERSION.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedPageRequest {
    #[serde(default = "protocol_version")]
    pub protocol_version: String,
    pub html: String,
    pub url: String,
    pub classic_scripts: Vec<(String, String)>,
    pub module_graph: ModuleGraph,
    pub runtime_config: RuntimeConfig,
    pub timer_drain_ms: u64,
    pub install_timer_shims: bool,
    pub inject_fetch_bridge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedPageResponse {
    pub effective_html: String,
    pub js_report: Option<JsExecutionReport>,
    pub runtime_capture: crate::webmcp::RuntimeCapture,
    pub execution_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationRequest {
    #[serde(default = "protocol_version")]
    pub protocol_version: String,
    pub html: String,
    pub url: String,
    /// The complete expression to evaluate. Callers remain responsible for
    /// any JSON-result wrapper required by their protocol.
    pub expression: String,
    pub return_effective_html: bool,
    pub runtime_config: RuntimeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResponse {
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_html: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum JsWorkerRequest {
    Page(PreparedPageRequest),
    Evaluate(EvaluationRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum JsWorkerResponse {
    Page { value: PreparedPageResponse },
    Evaluation { value: EvaluationResponse },
    Error { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct JsWorkerOptions {
    pub executable: Option<PathBuf>,
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub memory_limit_bytes: u64,
}

impl Default for JsWorkerOptions {
    fn default() -> Self {
        Self {
            executable: None,
            timeout: Duration::from_millis(DEFAULT_WORKER_TIMEOUT_MS),
            max_stdout_bytes: DEFAULT_WORKER_OUTPUT_BYTES,
            max_stderr_bytes: DEFAULT_WORKER_STDERR_BYTES,
            memory_limit_bytes: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JsContainmentFailureKind {
    Spawn,
    Timeout,
    Crash,
    Exit,
    OutputLimit,
    Protocol,
    WorkerError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsContainmentFailure {
    pub kind: JsContainmentFailureKind,
    pub code: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum JsWorkerError {
    #[error("could not resolve the Plasmate worker executable: {0}")]
    Executable(String),
    #[error("could not encode the JavaScript worker request: {0}")]
    Encode(String),
    #[error("JavaScript worker request exceeded {limit} bytes ({actual} bytes)")]
    InputLimit { limit: usize, actual: usize },
    #[error("JavaScript worker supervision failed: {0}")]
    Supervisor(#[from] SupervisorError),
    #[error("JavaScript worker exceeded its {timeout_ms}ms wall deadline")]
    Timeout { timeout_ms: u64 },
    #[error("JavaScript worker was terminated by signal {signal}{diagnostic}")]
    Crashed { signal: i32, diagnostic: String },
    #[error("JavaScript worker exited with code {code}{diagnostic}")]
    Exit { code: i32, diagnostic: String },
    #[error("JavaScript worker output exceeded its configured bound")]
    OutputLimit,
    #[error("JavaScript worker returned invalid protocol output: {0}")]
    Protocol(String),
    #[error("JavaScript worker rejected the request ({code}): {message}")]
    Worker { code: String, message: String },
}

impl JsWorkerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Executable(_) | Self::Supervisor(_) => "js_worker_spawn",
            Self::Encode(_) | Self::InputLimit { .. } => "js_worker_request",
            Self::Timeout { .. } => "js_worker_timeout",
            Self::Crashed { .. } => "js_worker_crash",
            Self::Exit { .. } => "js_worker_exit",
            Self::OutputLimit => "js_worker_output_limit",
            Self::Protocol(_) => "js_worker_protocol",
            Self::Worker { .. } => "js_worker_error",
        }
    }

    pub fn containment_failure(&self) -> JsContainmentFailure {
        let kind = match self {
            Self::Executable(_) | Self::Supervisor(_) => JsContainmentFailureKind::Spawn,
            Self::Timeout { .. } => JsContainmentFailureKind::Timeout,
            Self::Crashed { .. } => JsContainmentFailureKind::Crash,
            Self::Exit { .. } => JsContainmentFailureKind::Exit,
            Self::OutputLimit => JsContainmentFailureKind::OutputLimit,
            Self::Encode(_) | Self::InputLimit { .. } | Self::Protocol(_) => {
                JsContainmentFailureKind::Protocol
            }
            Self::Worker { .. } => JsContainmentFailureKind::WorkerError,
        };
        JsContainmentFailure {
            kind,
            code: self.code().to_string(),
            message: self.to_string(),
        }
    }
}

fn diagnostic(stderr: &[u8], truncated: bool) -> String {
    let mut text = String::from_utf8_lossy(stderr).trim().to_string();
    if text.len() > 2048 {
        let mut end = 2048;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    if truncated {
        text.push_str(" [stderr truncated]");
    }
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

fn resolve_executable(options: &JsWorkerOptions) -> Result<PathBuf, JsWorkerError> {
    if let Some(path) = &options.executable {
        return Ok(path.clone());
    }
    if let Some(path) = std::env::var_os("PLASMATE_JS_WORKER_EXECUTABLE") {
        return Ok(PathBuf::from(path));
    }
    let current =
        std::env::current_exe().map_err(|error| JsWorkerError::Executable(error.to_string()))?;
    let executable_name = if cfg!(windows) {
        "plasmate.exe"
    } else {
        "plasmate"
    };
    if current
        .file_name()
        .is_some_and(|name| name == executable_name)
    {
        return Ok(current);
    }
    if let Some(parent) = current.parent() {
        let sibling = parent.join(executable_name);
        if sibling.is_file() {
            return Ok(sibling);
        }
        // Cargo integration-test executables live in target/*/deps while the
        // Plasmate binary is one directory above.
        if parent.file_name().is_some_and(|name| name == "deps") {
            if let Some(profile) = parent.parent() {
                let sibling = profile.join(executable_name);
                if sibling.is_file() {
                    return Ok(sibling);
                }
            }
        }
    }
    Err(JsWorkerError::Executable(format!(
        "{} is not a Plasmate binary and no sibling worker was found; set PLASMATE_JS_WORKER_EXECUTABLE",
        current.display()
    )))
}

fn process_spec(
    request: &JsWorkerRequest,
    options: &JsWorkerOptions,
) -> Result<ProcessSpec, JsWorkerError> {
    let stdin =
        serde_json::to_vec(request).map_err(|error| JsWorkerError::Encode(error.to_string()))?;
    if stdin.len() > MAX_WORKER_INPUT_BYTES {
        return Err(JsWorkerError::InputLimit {
            limit: MAX_WORKER_INPUT_BYTES,
            actual: stdin.len(),
        });
    }
    Ok(ProcessSpec {
        program: resolve_executable(options)?,
        args: vec![OsString::from("__js-worker")],
        env: worker_environment(),
        stdin,
        timeout: options.timeout,
        max_stdout_bytes: options.max_stdout_bytes,
        max_stderr_bytes: options.max_stderr_bytes,
        memory_limit_bytes: options.memory_limit_bytes,
    })
}

fn worker_environment() -> Vec<(OsString, OsString)> {
    [
        "PLASMATE_ICU_DATA",
        crate::network::security::UNSAFE_PRIVATE_NETWORK_ENV,
    ]
    .into_iter()
    .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
    .collect()
}

fn classify_output(
    output: ProcessOutput,
    timeout: Duration,
) -> Result<JsWorkerResponse, JsWorkerError> {
    let detail = diagnostic(&output.stderr, output.stderr_truncated);
    match output.outcome {
        ProcessOutcome::TimedOut => {
            return Err(JsWorkerError::Timeout {
                timeout_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
            })
        }
        ProcessOutcome::Signaled { signal } => {
            return Err(JsWorkerError::Crashed {
                signal,
                diagnostic: detail,
            })
        }
        ProcessOutcome::Exited { code } if code != 0 => {
            return Err(JsWorkerError::Exit {
                code,
                diagnostic: detail,
            })
        }
        ProcessOutcome::Exited { .. } => {}
    }
    if output.stdout_truncated {
        return Err(JsWorkerError::OutputLimit);
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        JsWorkerError::Protocol(format!(
            "{error}{}",
            if detail.is_empty() {
                String::new()
            } else {
                detail
            }
        ))
    })
}

async fn exchange(
    request: JsWorkerRequest,
    options: JsWorkerOptions,
) -> Result<JsWorkerResponse, JsWorkerError> {
    let spec = process_spec(&request, &options)?;
    let output = process_supervisor::supervise_clean_env(spec).await?;
    classify_output(output, options.timeout)
}

fn exchange_sync(
    request: JsWorkerRequest,
    options: JsWorkerOptions,
) -> Result<JsWorkerResponse, JsWorkerError> {
    let spec = process_spec(&request, &options)?;
    let output = process_supervisor::supervise_sync_clean_env(spec)?;
    classify_output(output, options.timeout)
}

pub async fn execute_page(
    request: PreparedPageRequest,
    options: JsWorkerOptions,
) -> Result<PreparedPageResponse, JsWorkerError> {
    match exchange(JsWorkerRequest::Page(request), options).await? {
        JsWorkerResponse::Page { value } => Ok(value),
        JsWorkerResponse::Error { code, message } => Err(JsWorkerError::Worker { code, message }),
        JsWorkerResponse::Evaluation { .. } => Err(JsWorkerError::Protocol(
            "received evaluation response for page request".to_string(),
        )),
    }
}

pub fn execute_page_sync(
    request: PreparedPageRequest,
    options: JsWorkerOptions,
) -> Result<PreparedPageResponse, JsWorkerError> {
    match exchange_sync(JsWorkerRequest::Page(request), options)? {
        JsWorkerResponse::Page { value } => Ok(value),
        JsWorkerResponse::Error { code, message } => Err(JsWorkerError::Worker { code, message }),
        JsWorkerResponse::Evaluation { .. } => Err(JsWorkerError::Protocol(
            "received evaluation response for page request".to_string(),
        )),
    }
}

pub async fn evaluate(
    request: EvaluationRequest,
    options: JsWorkerOptions,
) -> Result<EvaluationResponse, JsWorkerError> {
    decode_evaluation(exchange(JsWorkerRequest::Evaluate(request), options).await?)
}

pub fn evaluate_sync(
    request: EvaluationRequest,
    options: JsWorkerOptions,
) -> Result<EvaluationResponse, JsWorkerError> {
    decode_evaluation(exchange_sync(JsWorkerRequest::Evaluate(request), options)?)
}

fn decode_evaluation(response: JsWorkerResponse) -> Result<EvaluationResponse, JsWorkerError> {
    match response {
        JsWorkerResponse::Evaluation { value } => Ok(value),
        JsWorkerResponse::Error { code, message } => Err(JsWorkerError::Worker { code, message }),
        JsWorkerResponse::Page { .. } => Err(JsWorkerError::Protocol(
            "received page response for evaluation request".to_string(),
        )),
    }
}

/// Run one already-isolated request. The binary entry point is responsible for
/// reading/writing the single JSON envelope and for never writing protocol data
/// to stderr.
pub fn run_worker_request(request: JsWorkerRequest) -> JsWorkerResponse {
    let request_version = match &request {
        JsWorkerRequest::Page(request) => &request.protocol_version,
        JsWorkerRequest::Evaluate(request) => &request.protocol_version,
    };
    if request_version != WORKER_PROTOCOL_VERSION {
        return JsWorkerResponse::Error {
            code: "unsupported_protocol".to_string(),
            message: format!(
                "unsupported JavaScript worker protocol {request_version:?}; expected {WORKER_PROTOCOL_VERSION}"
            ),
        };
    }
    match request {
        JsWorkerRequest::Page(request) => match super::pipeline::run_prepared_page(request) {
            Ok(value) => JsWorkerResponse::Page { value },
            Err(error) => JsWorkerResponse::Error {
                code: "page_execution".to_string(),
                message: error.to_string(),
            },
        },
        JsWorkerRequest::Evaluate(request) => run_evaluation(request),
    }
}

fn run_evaluation(request: EvaluationRequest) -> JsWorkerResponse {
    let mut runtime = JsRuntime::new(request.runtime_config);
    runtime.bootstrap_dom(&request.html, &request.url);
    let result = match runtime.eval(&request.expression) {
        Ok(result) => result,
        Err(error) => {
            return JsWorkerResponse::Error {
                code: "javascript_error".to_string(),
                message: error.to_string(),
            }
        }
    };
    let effective_html = if request.return_effective_html {
        match runtime.serialize_dom() {
            Ok(html) => Some(html),
            Err(error) => {
                return JsWorkerResponse::Error {
                    code: "dom_serialization".to_string(),
                    message: error.to_string(),
                }
            }
        }
    } else {
        None
    };
    JsWorkerResponse::Evaluation {
        value: EvaluationResponse {
            result,
            effective_html,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(outcome: ProcessOutcome, stdout: &[u8], truncated: bool) -> ProcessOutput {
        ProcessOutput {
            outcome,
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
            stdout_truncated: truncated,
            stderr_truncated: false,
        }
    }

    #[test]
    fn worker_outcomes_are_typed() {
        assert!(matches!(
            classify_output(
                output(ProcessOutcome::TimedOut, b"", false),
                Duration::from_millis(25)
            ),
            Err(JsWorkerError::Timeout { timeout_ms: 25 })
        ));
        assert!(matches!(
            classify_output(
                output(ProcessOutcome::Signaled { signal: 6 }, b"", false),
                Duration::from_secs(1)
            ),
            Err(JsWorkerError::Crashed { signal: 6, .. })
        ));
        assert!(matches!(
            classify_output(
                output(ProcessOutcome::Exited { code: 9 }, b"", false),
                Duration::from_secs(1)
            ),
            Err(JsWorkerError::Exit { code: 9, .. })
        ));
        assert!(matches!(
            classify_output(
                output(ProcessOutcome::Exited { code: 0 }, b"{}", true),
                Duration::from_secs(1)
            ),
            Err(JsWorkerError::OutputLimit)
        ));
    }

    #[test]
    fn normal_evaluation_preserves_dom_when_requested() {
        let response = run_evaluation(EvaluationRequest {
            protocol_version: WORKER_PROTOCOL_VERSION.to_string(),
            html: "<html><body><p id='x'>before</p></body></html>".to_string(),
            url: "https://example.com/".to_string(),
            expression: "document.getElementById('x').textContent = 'after'; 'ok'".to_string(),
            return_effective_html: true,
            runtime_config: RuntimeConfig::default(),
        });
        let JsWorkerResponse::Evaluation { value } = response else {
            panic!("expected successful evaluation")
        };
        assert_eq!(value.result, "ok");
        assert!(value.effective_html.unwrap().contains("after"));
    }

    #[test]
    fn unsupported_protocol_is_rejected_before_execution() {
        let response = run_worker_request(JsWorkerRequest::Evaluate(EvaluationRequest {
            protocol_version: "plasmate.js-worker.v0".to_string(),
            html: "<p>safe</p>".to_string(),
            url: "https://example.com/".to_string(),
            expression: "while (true) {}".to_string(),
            return_effective_html: false,
            runtime_config: RuntimeConfig::default(),
        }));
        assert!(matches!(
            response,
            JsWorkerResponse::Error { code, .. } if code == "unsupported_protocol"
        ));
    }
}
