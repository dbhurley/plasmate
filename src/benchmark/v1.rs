//! Reproducible benchmark schema and deterministic fixture suite.
//!
//! This suite deliberately separates product-task success from byte compression.
//! Public-web coverage belongs in a separate, optional report because availability,
//! blocking, and network conditions are not deterministic release gates.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use axum::response::{Html, Redirect};
use axum::routing::get;
use axum::Router;
use reqwest::cookie::Jar;
use serde::{Deserialize, Serialize};

use crate::cache::store::{CacheConfig, CacheLookup, SomCache};
use crate::js::dom_bridge::{ClickResult, NodeRegistry};
use crate::js::pipeline::{self, PipelineConfig};
use crate::network::fetch::{self, FetchError};
use crate::process_supervisor::{self, ProcessOutcome, ProcessOutput, ProcessSpec};
use crate::som::compiler;
use crate::som::types::{Element, ElementRole, Som};

pub const SCHEMA_VERSION: &str = "plasmate.benchmark.v1";
const NAVIGATION_HTML: &str = r#"<!doctype html><title>Navigation</title><nav aria-label="Primary"><a href="/destination">Open destination</a></nav>"#;
const FORM_HTML: &str = r#"<!doctype html><title>Form</title><main><form action="/submit" method="post"><label for="email">Email address</label><input id="email" name="email" type="email"><button type="submit">Continue</button></form></main>"#;

#[derive(Debug, Clone)]
pub struct BenchmarkOptions {
    pub js_enabled: bool,
    pub max_cold_ms: u64,
    pub max_warm_ms: u64,
    pub worker_timeout_ms: u64,
    pub worker_memory_bytes: u64,
    pub worker_output_bytes: usize,
    pub worker_executable: Option<PathBuf>,
}

impl Default for BenchmarkOptions {
    fn default() -> Self {
        Self {
            js_enabled: false,
            max_cold_ms: 2_000,
            max_warm_ms: 2_000,
            worker_timeout_ms: 15_000,
            worker_memory_bytes: 0,
            worker_output_bytes: 2 * 1024 * 1024,
            worker_executable: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub schema_version: String,
    pub generated_at_unix_seconds: u64,
    pub suite: String,
    pub environment: Environment,
    pub config: BenchmarkConfig,
    pub summary: Summary,
    pub tasks: Vec<TaskResult>,
    pub thresholds: Thresholds,
    pub threshold_evaluation: ThresholdEvaluation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub plasmate_version: String,
    pub git_commit: Option<String>,
    pub git_dirty: Option<bool>,
    pub rustc_version: Option<String>,
    pub build_profile: String,
    pub operating_system: String,
    pub architecture: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkConfig {
    pub js_enabled: bool,
    pub external_scripts_enabled: bool,
    pub repetitions_per_task: usize,
    pub fixture_transport: String,
    pub execution_isolation: String,
    pub worker_timeout_ms: Option<u64>,
    pub worker_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Summary {
    /// Complete task input denominator. No outcome is excluded from this count.
    pub inputs_total: usize,
    pub success: usize,
    pub blocked: usize,
    pub failed: usize,
    pub crash: usize,
    pub timeout: usize,
    pub tasks_passed: usize,
    pub tasks_failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub id: String,
    pub category: String,
    pub requested_url: String,
    pub expected_outcome: Outcome,
    pub task_passed: bool,
    pub assertion_wall_time_us: u64,
    pub assertions: Vec<Assertion>,
    pub cold: Sample,
    pub warm: Sample,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Blocked,
    Failed,
    Crash,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Cold,
    Warm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    Miss,
    Hit,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub phase: Phase,
    pub outcome: Outcome,
    pub cache_state: CacheState,
    pub wall_time_us: u64,
    /// Cumulative process RSS high-water mark observed when this sample ended.
    pub process_peak_rss_bytes_at_sample_end: Option<u64>,
    pub final_url: Option<String>,
    pub http_status: Option<u16>,
    pub html_bytes: usize,
    pub som_bytes: usize,
    pub compression_ratio: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    pub require_all_task_contracts: bool,
    pub require_cold_cache_miss: bool,
    pub require_warm_cache_hit: bool,
    pub max_cold_ms: u64,
    pub max_warm_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdEvaluation {
    pub passed: bool,
    pub violations: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum FixtureKind {
    Navigation,
    FormInput,
    Extraction,
    Redirect,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkWorkerRequest {
    fixture_index: usize,
}

#[derive(Debug, Clone)]
struct FixtureTask {
    id: &'static str,
    category: &'static str,
    path: &'static str,
    kind: FixtureKind,
    expected_outcome: Outcome,
}

fn fixtures() -> Vec<FixtureTask> {
    vec![
        FixtureTask {
            id: "navigation-link-contract",
            category: "navigation",
            path: "/navigation",
            kind: FixtureKind::Navigation,
            expected_outcome: Outcome::Success,
        },
        FixtureTask {
            id: "form-input-contract",
            category: "form_input",
            path: "/form",
            kind: FixtureKind::FormInput,
            expected_outcome: Outcome::Success,
        },
        FixtureTask {
            id: "main-text-extraction-contract",
            category: "extraction",
            path: "/extract",
            kind: FixtureKind::Extraction,
            expected_outcome: Outcome::Success,
        },
        FixtureTask {
            id: "redirect-contract",
            category: "redirect",
            path: "/redirect",
            kind: FixtureKind::Redirect,
            expected_outcome: Outcome::Success,
        },
        FixtureTask {
            id: "http-error-contract",
            category: "error",
            path: "/error",
            kind: FixtureKind::Error,
            expected_outcome: Outcome::Failed,
        },
    ]
}

/// Run only local deterministic inputs. This command never contacts a public site.
pub async fn run_deterministic_suite(
    options: &BenchmarkOptions,
) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
    let local_fixture_server = if options.js_enabled {
        None
    } else {
        Some(start_fixture_server().await?)
    };
    let mut tasks = Vec::new();

    for (fixture_index, fixture) in fixtures().into_iter().enumerate() {
        let task = if options.js_enabled {
            run_fixture_supervised(fixture_index, &fixture, options).await
        } else {
            let base_url = &local_fixture_server
                .as_ref()
                .expect("non-JS benchmark always starts its local fixture server")
                .0;
            match execute_fixture(&fixture, base_url, false).await {
                Ok(task) => task,
                Err(error) => worker_failure_task(
                    &fixture,
                    Outcome::Failed,
                    format!("deterministic fixture failed: {error}"),
                    0,
                ),
            }
        };
        tasks.push(task);
    }

    if let Some((_, server)) = local_fixture_server {
        server.abort();
        let _ = server.await;
    }
    let summary = summarize(&tasks);
    let thresholds = Thresholds {
        require_all_task_contracts: true,
        require_cold_cache_miss: true,
        require_warm_cache_hit: true,
        max_cold_ms: options.max_cold_ms,
        max_warm_ms: options.max_warm_ms,
    };
    let threshold_evaluation = evaluate_thresholds(&tasks, &summary, &thresholds);

    Ok(BenchmarkReport {
        schema_version: SCHEMA_VERSION.to_string(),
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        suite: "deterministic-product-contracts".to_string(),
        environment: environment(),
        config: BenchmarkConfig {
            js_enabled: options.js_enabled,
            external_scripts_enabled: false,
            repetitions_per_task: 2,
            fixture_transport: "ephemeral-loopback-http".to_string(),
            execution_isolation: if options.js_enabled {
                "supervised-process-per-task".to_string()
            } else {
                "in-process-no-javascript".to_string()
            },
            worker_timeout_ms: options.js_enabled.then_some(options.worker_timeout_ms),
            worker_memory_bytes: options.js_enabled.then_some(options.worker_memory_bytes),
        },
        summary,
        tasks,
        thresholds,
        threshold_evaluation,
    })
}

pub async fn run_worker(request: BenchmarkWorkerRequest) -> Result<TaskResult, String> {
    let fixture = fixtures()
        .into_iter()
        .nth(request.fixture_index)
        .ok_or_else(|| format!("unknown benchmark fixture index {}", request.fixture_index))?;
    // A worker always creates its own fixed fixture server. No caller-provided URL
    // can reach the narrowly scoped local-fixture network policy.
    let (base_url, server) = start_fixture_server()
        .await
        .map_err(|error| format!("cannot start benchmark fixture server: {error}"))?;
    let result = execute_fixture(&fixture, &base_url, true)
        .await
        .map_err(|error| error.to_string());
    server.abort();
    let _ = server.await;
    result
}

async fn execute_fixture(
    fixture: &FixtureTask,
    base_url: &str,
    js_enabled: bool,
) -> Result<TaskResult, Box<dyn std::error::Error>> {
    let jar = Arc::new(Jar::default());
    let client = fetch::build_client_for_local_fixture(jar)?;
    let cache = SomCache::new(CacheConfig {
        prefetch_enabled: false,
        ..Default::default()
    });
    let requested_url = format!("{}{}", base_url, fixture.path);
    let (mut cold, cold_som) =
        run_sample(&client, &cache, &requested_url, Phase::Cold, js_enabled).await;
    let (mut warm, warm_som) =
        run_sample(&client, &cache, &requested_url, Phase::Warm, js_enabled).await;
    normalize_fixture_url(&mut cold, base_url);
    normalize_fixture_url(&mut warm, base_url);
    let assertion_started = Instant::now();
    let assertions = assertions_for(fixture, &cold, cold_som.as_ref(), js_enabled);
    let assertion_wall_time_us = assertion_started.elapsed().as_micros() as u64;
    let task_passed = cold.outcome == fixture.expected_outcome
        && warm.outcome == fixture.expected_outcome
        && assertions.iter().all(|assertion| assertion.passed)
        && (fixture.expected_outcome != Outcome::Success || warm_som.is_some());

    Ok(TaskResult {
        id: fixture.id.to_string(),
        category: fixture.category.to_string(),
        requested_url: format!("fixture://local{}", fixture.path),
        expected_outcome: fixture.expected_outcome.clone(),
        task_passed,
        assertion_wall_time_us,
        assertions,
        cold,
        warm,
    })
}

async fn run_fixture_supervised(
    fixture_index: usize,
    fixture: &FixtureTask,
    options: &BenchmarkOptions,
) -> TaskResult {
    let executable = match &options.worker_executable {
        Some(path) => path.clone(),
        None => match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                return worker_failure_task(
                    fixture,
                    Outcome::Failed,
                    format!("cannot resolve benchmark worker executable: {error}"),
                    0,
                );
            }
        },
    };
    let request = BenchmarkWorkerRequest { fixture_index };
    let stdin = match serde_json::to_vec(&request) {
        Ok(stdin) => stdin,
        Err(error) => {
            return worker_failure_task(
                fixture,
                Outcome::Failed,
                format!("cannot encode benchmark worker request: {error}"),
                0,
            );
        }
    };
    let started = Instant::now();
    let output = process_supervisor::supervise(ProcessSpec {
        program: executable,
        args: vec![OsString::from("__benchmark-worker")],
        env: Vec::new(),
        stdin,
        timeout: Duration::from_millis(options.worker_timeout_ms),
        max_stdout_bytes: options.worker_output_bytes,
        max_stderr_bytes: options.worker_output_bytes,
        memory_limit_bytes: options.worker_memory_bytes,
    })
    .await;
    let elapsed_us = started.elapsed().as_micros() as u64;
    match output {
        Ok(output) => classify_worker_output(fixture, output, elapsed_us),
        Err(error) => worker_failure_task(
            fixture,
            Outcome::Failed,
            format!("benchmark worker supervision failed: {error}"),
            elapsed_us,
        ),
    }
}

fn classify_worker_output(
    fixture: &FixtureTask,
    output: ProcessOutput,
    elapsed_us: u64,
) -> TaskResult {
    let diagnostic = bounded_worker_diagnostic(&output.stderr, output.stderr_truncated);
    match output.outcome {
        ProcessOutcome::TimedOut => worker_failure_task(
            fixture,
            Outcome::Timeout,
            append_diagnostic("supervised benchmark worker timed out", &diagnostic),
            elapsed_us,
        ),
        ProcessOutcome::Signaled { signal } => worker_failure_task(
            fixture,
            Outcome::Crash,
            append_diagnostic(
                &format!("supervised benchmark worker terminated by signal {signal}"),
                &diagnostic,
            ),
            elapsed_us,
        ),
        ProcessOutcome::Exited { code } if code != 0 => worker_failure_task(
            fixture,
            Outcome::Failed,
            append_diagnostic(
                &format!("supervised benchmark worker exited with code {code}"),
                &diagnostic,
            ),
            elapsed_us,
        ),
        ProcessOutcome::Exited { .. } if output.stdout_truncated => worker_failure_task(
            fixture,
            Outcome::Failed,
            "supervised benchmark worker response exceeded the output limit".to_string(),
            elapsed_us,
        ),
        ProcessOutcome::Exited { .. } => match serde_json::from_slice(&output.stdout) {
            Ok(task) => task,
            Err(error) => worker_failure_task(
                fixture,
                Outcome::Failed,
                append_diagnostic(
                    &format!("invalid benchmark worker response: {error}"),
                    &diagnostic,
                ),
                elapsed_us,
            ),
        },
    }
}

fn worker_failure_task(
    fixture: &FixtureTask,
    outcome: Outcome,
    error: String,
    wall_time_us: u64,
) -> TaskResult {
    let sample = |phase| Sample {
        phase,
        outcome: outcome.clone(),
        cache_state: CacheState::NotApplicable,
        wall_time_us,
        process_peak_rss_bytes_at_sample_end: None,
        final_url: None,
        http_status: None,
        html_bytes: 0,
        som_bytes: 0,
        compression_ratio: None,
        error: Some(error.clone()),
    };
    TaskResult {
        id: fixture.id.to_string(),
        category: fixture.category.to_string(),
        requested_url: format!("fixture://local{}", fixture.path),
        expected_outcome: fixture.expected_outcome.clone(),
        task_passed: false,
        assertion_wall_time_us: 0,
        assertions: vec![Assertion {
            name: "supervised worker completed with valid output".to_string(),
            passed: false,
            detail: error.clone(),
        }],
        cold: sample(Phase::Cold),
        warm: sample(Phase::Warm),
    }
}

fn bounded_worker_diagnostic(stderr: &[u8], truncated: bool) -> String {
    let mut diagnostic = String::from_utf8_lossy(stderr).trim().to_string();
    if diagnostic.len() > 2_048 {
        diagnostic.truncate(2_048);
        diagnostic.push('…');
    }
    if truncated {
        diagnostic.push_str(" [worker stderr truncated]");
    }
    diagnostic
}

fn append_diagnostic(message: &str, diagnostic: &str) -> String {
    if diagnostic.is_empty() {
        message.to_string()
    } else {
        format!("{message}: {diagnostic}")
    }
}

fn normalize_fixture_url(sample: &mut Sample, base_url: &str) {
    if let Some(final_url) = &mut sample.final_url {
        if let Some(path) = final_url.strip_prefix(base_url) {
            *final_url = format!("fixture://local{path}");
        }
    }
}

fn fixture_router() -> Router {
    Router::new()
        .route(
            "/navigation",
            get(|| async { Html(NAVIGATION_HTML) }),
        )
        .route(
            "/form",
            get(|| async { Html(FORM_HTML) }),
        )
        .route(
            "/extract",
            get(|| async {
                Html(r#"<!doctype html><title>Extraction</title><main><h1>Release evidence</h1><p>The deterministic extraction fixture is observable.</p><script>document.title = 'Extraction JS';</script></main>"#)
            }),
        )
        .route(
            "/redirect",
            get(|| async { Redirect::temporary("/redirect-target") }),
        )
        .route(
            "/redirect-target",
            get(|| async {
                Html(r#"<!doctype html><title>Redirect target</title><main><p>Redirect completed.</p></main>"#)
            }),
        )
        .route(
            "/error",
            get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "expected fixture failure") }),
        )
}

async fn start_fixture_server() -> Result<(String, tokio::task::JoinHandle<()>), std::io::Error> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, fixture_router()).await;
    });
    Ok((format!("http://{address}"), server))
}

async fn run_sample(
    client: &reqwest::Client,
    cache: &SomCache,
    requested_url: &str,
    phase: Phase,
    js_enabled: bool,
) -> (Sample, Option<Som>) {
    let started = Instant::now();
    let fetched = match fetch::fetch_url_for_local_fixture(client, requested_url, 2_000).await {
        Ok(result) => result,
        Err(error) => {
            let outcome = match error {
                FetchError::Timeout(_) => Outcome::Timeout,
                FetchError::UrlBlocked(_) => Outcome::Blocked,
                FetchError::HttpError { .. }
                | FetchError::NavigationFailed(_)
                | FetchError::TooManyRedirects(_)
                | FetchError::BodyTooLarge { .. } => Outcome::Failed,
            };
            return (
                Sample {
                    phase,
                    outcome,
                    cache_state: CacheState::NotApplicable,
                    wall_time_us: started.elapsed().as_micros() as u64,
                    process_peak_rss_bytes_at_sample_end: process_peak_rss_bytes(),
                    final_url: None,
                    http_status: http_status(&error),
                    html_bytes: 0,
                    som_bytes: 0,
                    compression_ratio: None,
                    error: Some(error.to_string()),
                },
                None,
            );
        }
    };

    let content_hash = SomCache::content_hash(fetched.html.as_bytes());
    let lookup = cache.lookup(requested_url, content_hash);
    let (cache_state, som_result) = match lookup {
        CacheLookup::Hit(entry) => (
            CacheState::Hit,
            serde_json::from_slice::<Som>(&entry.som_json).map_err(|error| error.to_string()),
        ),
        CacheLookup::Miss | CacheLookup::Stale { .. } => {
            let config = PipelineConfig {
                execute_js: js_enabled,
                fetch_external_scripts: false,
                // JavaScript benchmark tasks already run in the supervised
                // `__benchmark-worker` process.
                isolate_js: false,
                ..Default::default()
            };
            let compiled = if js_enabled {
                pipeline::process_page(&fetched.html, &fetched.url, &config)
                    .map(|page| page.som)
                    .map_err(|error| error.to_string())
            } else {
                compiler::compile(&fetched.html, &fetched.url).map_err(|error| error.to_string())
            };
            let state = CacheState::Miss;
            if let Ok(ref som) = compiled {
                if let Ok(json) = serde_json::to_vec(som) {
                    cache.store(requested_url, content_hash, json, fetched.html_bytes);
                }
            }
            (state, compiled)
        }
    };

    match som_result {
        Ok(som) => {
            let som_bytes = serde_json::to_vec(&som).map(|json| json.len()).unwrap_or(0);
            let ratio = (som_bytes > 0).then_some(fetched.html_bytes as f64 / som_bytes as f64);
            (
                Sample {
                    phase,
                    outcome: Outcome::Success,
                    cache_state,
                    wall_time_us: started.elapsed().as_micros() as u64,
                    process_peak_rss_bytes_at_sample_end: process_peak_rss_bytes(),
                    final_url: Some(fetched.url),
                    http_status: Some(fetched.status),
                    html_bytes: fetched.html_bytes,
                    som_bytes,
                    compression_ratio: ratio,
                    error: None,
                },
                Some(som),
            )
        }
        Err(error) => (
            Sample {
                phase,
                outcome: Outcome::Failed,
                cache_state,
                wall_time_us: started.elapsed().as_micros() as u64,
                process_peak_rss_bytes_at_sample_end: process_peak_rss_bytes(),
                final_url: Some(fetched.url),
                http_status: Some(fetched.status),
                html_bytes: fetched.html_bytes,
                som_bytes: 0,
                compression_ratio: None,
                error: Some(error),
            },
            None,
        ),
    }
}

fn http_status(error: &FetchError) -> Option<u16> {
    match error {
        FetchError::HttpError { status, .. } => Some(*status),
        _ => None,
    }
}

fn assertions_for(
    fixture: &FixtureTask,
    cold: &Sample,
    som: Option<&Som>,
    js_enabled: bool,
) -> Vec<Assertion> {
    let elements = som.map(all_elements).unwrap_or_default();
    let assertion = match fixture.kind {
        FixtureKind::Navigation => {
            let found = elements.iter().any(|element| {
                element.role == ElementRole::Link
                    && element.text.as_deref() == Some("Open destination")
                    && element
                        .attrs
                        .as_ref()
                        .and_then(|attrs| attrs.get("href"))
                        .and_then(|href| href.as_str())
                        == Some("/destination")
            });
            (
                "link target is preserved and executes navigation",
                found && fixture_navigation_action_succeeds(),
            )
        }
        FixtureKind::FormInput => {
            let semantic_action = elements.iter().any(|element| {
                element.role == ElementRole::TextInput
                    && element.label.as_deref() == Some("Email address")
                    && element
                        .actions
                        .as_ref()
                        .is_some_and(|actions| actions.iter().any(|action| action == "type"))
            });
            (
                "labeled text input exposes and executes type action",
                semantic_action && fixture_form_input_action_succeeds(),
            )
        }
        FixtureKind::Extraction => {
            let text_found = elements.iter().any(|element| {
                element
                    .text
                    .as_deref()
                    .is_some_and(|text| text.contains("deterministic extraction fixture"))
            });
            let expected_title = if js_enabled {
                "Extraction JS"
            } else {
                "Extraction"
            };
            let title_found = som.is_some_and(|som| som.title == expected_title);
            (
                "main text and JS-dependent document title are observable",
                text_found && title_found,
            )
        }
        FixtureKind::Redirect => {
            let found = cold
                .final_url
                .as_deref()
                .is_some_and(|url| url.ends_with("/redirect-target"))
                && som.is_some_and(|som| som.title == "Redirect target");
            ("redirect final URL and document are preserved", found)
        }
        FixtureKind::Error => (
            "HTTP 500 is reported as a failed input",
            cold.outcome == Outcome::Failed && cold.http_status == Some(500),
        ),
    };

    vec![Assertion {
        name: assertion.0.to_string(),
        passed: assertion.1,
        detail: format!("fixture={}, js_enabled={js_enabled}", fixture.id),
    }]
}

fn fixture_navigation_action_succeeds() -> bool {
    let Some((registry, link_id)) = fixture_registry_and_selector(NAVIGATION_HTML, "a") else {
        return false;
    };
    matches!(
        registry.click(link_id),
        Ok(ClickResult::Navigate(destination)) if destination == "/destination"
    )
}

fn fixture_form_input_action_succeeds() -> bool {
    let Some((registry, input_id)) = fixture_registry_and_selector(FORM_HTML, "#email") else {
        return false;
    };
    registry.type_text(input_id, "agent@example.test").is_ok()
        && registry.get_attribute(input_id, "value").as_deref() == Some("agent@example.test")
}

fn fixture_registry_and_selector(html: &str, selector: &str) -> Option<(NodeRegistry, u32)> {
    use html5ever::parse_document;
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::RcDom;

    let Ok(dom) = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
    else {
        return None;
    };
    let mut registry = NodeRegistry::new();
    registry.register_tree(&dom.document);
    let document_id = registry.document_id()?;
    let element_id = registry.query_selector(document_id, selector)?;
    Some((registry, element_id))
}

fn all_elements(som: &Som) -> Vec<&Element> {
    fn visit<'a>(element: &'a Element, output: &mut Vec<&'a Element>) {
        output.push(element);
        if let Some(children) = &element.children {
            for child in children {
                visit(child, output);
            }
        }
        if let Some(shadow) = &element.shadow {
            for child in &shadow.elements {
                visit(child, output);
            }
        }
    }

    let mut output = Vec::new();
    for region in &som.regions {
        for element in &region.elements {
            visit(element, &mut output);
        }
    }
    output
}

fn summarize(tasks: &[TaskResult]) -> Summary {
    let mut summary = Summary {
        inputs_total: tasks.len(),
        ..Default::default()
    };
    for task in tasks {
        match task.cold.outcome {
            Outcome::Success => summary.success += 1,
            Outcome::Blocked => summary.blocked += 1,
            Outcome::Failed => summary.failed += 1,
            Outcome::Crash => summary.crash += 1,
            Outcome::Timeout => summary.timeout += 1,
        }
        if task.task_passed {
            summary.tasks_passed += 1;
        } else {
            summary.tasks_failed += 1;
        }
    }
    summary
}

fn evaluate_thresholds(
    tasks: &[TaskResult],
    summary: &Summary,
    thresholds: &Thresholds,
) -> ThresholdEvaluation {
    let mut violations = Vec::new();
    let classified =
        summary.success + summary.blocked + summary.failed + summary.crash + summary.timeout;
    if classified != summary.inputs_total || summary.inputs_total != tasks.len() {
        violations.push(format!(
            "outcome denominator mismatch: inputs={}, classified={}, tasks={}",
            summary.inputs_total,
            classified,
            tasks.len()
        ));
    }
    if thresholds.require_all_task_contracts && summary.tasks_failed > 0 {
        violations.push(format!("{} task contract(s) failed", summary.tasks_failed));
    }
    for task in tasks {
        if task.cold.outcome == Outcome::Success {
            if thresholds.require_cold_cache_miss && task.cold.cache_state != CacheState::Miss {
                violations.push(format!("{} cold sample was not a cache miss", task.id));
            }
            if thresholds.require_warm_cache_hit && task.warm.cache_state != CacheState::Hit {
                violations.push(format!("{} warm sample was not a cache hit", task.id));
            }
            if task.cold.wall_time_us > thresholds.max_cold_ms.saturating_mul(1_000) {
                violations.push(format!("{} exceeded cold latency threshold", task.id));
            }
            if task.warm.wall_time_us > thresholds.max_warm_ms.saturating_mul(1_000) {
                violations.push(format!("{} exceeded warm latency threshold", task.id));
            }
        }
    }
    ThresholdEvaluation {
        passed: violations.is_empty(),
        violations,
    }
}

fn environment() -> Environment {
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

#[cfg(unix)]
fn process_peak_rss_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the supplied rusage value on a zero return code.
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    // SAFETY: the successful call above initialized the value.
    let usage = unsafe { usage.assume_init() };
    #[cfg(target_os = "macos")]
    let bytes = usage.ru_maxrss as u64;
    #[cfg(not(target_os = "macos"))]
    let bytes = (usage.ru_maxrss as u64).saturating_mul(1_024);
    Some(bytes)
}

#[cfg(not(unix))]
fn process_peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(phase: Phase, outcome: Outcome, cache_state: CacheState, ms: u64) -> Sample {
        Sample {
            phase,
            outcome,
            cache_state,
            wall_time_us: ms * 1_000,
            process_peak_rss_bytes_at_sample_end: Some(1),
            final_url: None,
            http_status: Some(200),
            html_bytes: 100,
            som_bytes: 20,
            compression_ratio: Some(5.0),
            error: None,
        }
    }

    fn passing_task() -> TaskResult {
        TaskResult {
            id: "task".to_string(),
            category: "test".to_string(),
            requested_url: "http://fixture/task".to_string(),
            expected_outcome: Outcome::Success,
            task_passed: true,
            assertion_wall_time_us: 1,
            assertions: vec![Assertion {
                name: "contract".to_string(),
                passed: true,
                detail: "deterministic".to_string(),
            }],
            cold: sample(Phase::Cold, Outcome::Success, CacheState::Miss, 1),
            warm: sample(Phase::Warm, Outcome::Success, CacheState::Hit, 1),
        }
    }

    #[test]
    fn schema_serializes_version_and_cold_warm_labels() {
        let task = passing_task();
        let value = serde_json::to_value(&task).expect("serialize task");
        assert_eq!(value["cold"]["phase"], "cold");
        assert_eq!(value["warm"]["phase"], "warm");
        assert_eq!(SCHEMA_VERSION, "plasmate.benchmark.v1");
    }

    #[test]
    fn summary_uses_full_unfiltered_denominator() {
        let mut tasks = vec![passing_task()];
        for outcome in [
            Outcome::Blocked,
            Outcome::Failed,
            Outcome::Crash,
            Outcome::Timeout,
        ] {
            let mut task = passing_task();
            task.id = format!("{outcome:?}");
            task.cold.outcome = outcome.clone();
            task.warm.outcome = outcome;
            task.task_passed = false;
            tasks.push(task);
        }
        let summary = summarize(&tasks);
        assert_eq!(summary.inputs_total, 5);
        assert_eq!(
            summary.success + summary.blocked + summary.failed + summary.crash + summary.timeout,
            5
        );
    }

    #[test]
    fn threshold_gate_reports_latency_and_cache_regressions() {
        let mut task = passing_task();
        task.cold.wall_time_us = 20_000;
        task.warm.cache_state = CacheState::Miss;
        let tasks = vec![task];
        let evaluation = evaluate_thresholds(
            &tasks,
            &summarize(&tasks),
            &Thresholds {
                require_all_task_contracts: true,
                require_cold_cache_miss: true,
                require_warm_cache_hit: true,
                max_cold_ms: 10,
                max_warm_ms: 10,
            },
        );
        assert!(!evaluation.passed);
        assert!(evaluation
            .violations
            .iter()
            .any(|message| message.contains("cold latency")));
        assert!(evaluation
            .violations
            .iter()
            .any(|message| message.contains("warm sample")));
    }

    #[test]
    fn supervised_worker_timeout_is_a_real_reported_outcome() {
        let fixture = fixtures().remove(0);
        let task = classify_worker_output(
            &fixture,
            ProcessOutput {
                outcome: ProcessOutcome::TimedOut,
                stdout: Vec::new(),
                stderr: b"deadline".to_vec(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
            10_000,
        );
        assert_eq!(task.cold.outcome, Outcome::Timeout);
        assert!(!task.task_passed);
    }

    #[test]
    fn supervised_worker_signal_is_a_real_reported_crash() {
        let fixture = fixtures().remove(0);
        let task = classify_worker_output(
            &fixture,
            ProcessOutput {
                outcome: ProcessOutcome::Signaled { signal: 9 },
                stdout: Vec::new(),
                stderr: b"terminated".to_vec(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
            10_000,
        );
        assert_eq!(task.cold.outcome, Outcome::Crash);
        assert!(!task.task_passed);
    }

    #[test]
    fn worker_request_rejects_caller_controlled_destination() {
        let error = serde_json::from_slice::<BenchmarkWorkerRequest>(
            br#"{"fixture_index":0,"base_url":"http://127.0.0.1:65535"}"#,
        )
        .expect_err("worker requests must not accept a caller-provided destination");
        assert!(error.to_string().contains("unknown field `base_url`"));
    }

    #[tokio::test]
    async fn deterministic_suite_exercises_all_task_contracts() {
        let report = run_deterministic_suite(&BenchmarkOptions::default())
            .await
            .expect("run suite");
        assert_eq!(report.schema_version, SCHEMA_VERSION);
        assert_eq!(report.summary.inputs_total, 5);
        assert_eq!(report.summary.tasks_passed, 5);
        assert_eq!(
            report.summary.failed, 1,
            "expected HTTP error remains visible"
        );
        assert!(report.threshold_evaluation.passed);
    }
}
