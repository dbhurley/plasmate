use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use plasmate::js::runtime::RuntimeConfig;
use plasmate::js::worker::{self, EvaluationRequest, JsWorkerError, JsWorkerOptions};
use serial_test::serial;

fn fixture_path() -> PathBuf {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/js_worker_fixture.rs");
            let mut output = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("js-worker-fixture");
            if cfg!(windows) {
                output.set_extension("exe");
            }
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
            let compilation = std::process::Command::new(rustc)
                .args(["--edition=2021", "--crate-name", "js_worker_fixture"])
                .arg(&source)
                .arg("-o")
                .arg(&output)
                .output()
                .expect("failed to launch rustc for JS worker fixture");
            assert!(
                compilation.status.success(),
                "fixture compilation failed: {}",
                String::from_utf8_lossy(&compilation.stderr)
            );
            output
        })
        .clone()
}

fn request(url: &str) -> EvaluationRequest {
    EvaluationRequest {
        protocol_version: worker::WORKER_PROTOCOL_VERSION.to_string(),
        html: "<p>safe fallback</p>".to_string(),
        url: url.to_string(),
        expression: "1 + 1".to_string(),
        return_effective_html: false,
        runtime_config: RuntimeConfig::default(),
    }
}

fn options() -> JsWorkerOptions {
    JsWorkerOptions {
        executable: Some(fixture_path()),
        timeout: Duration::from_secs(5),
        max_stdout_bytes: 4096,
        max_stderr_bytes: 4096,
        memory_limit_bytes: 0,
    }
}

fn real_worker_options() -> JsWorkerOptions {
    JsWorkerOptions {
        executable: Some(PathBuf::from(env!("CARGO_BIN_EXE_plasmate"))),
        ..Default::default()
    }
}

#[tokio::test]
#[serial]
async fn normal_worker_response_round_trips() {
    let response = worker::evaluate(request("https://fixture.invalid/__fixture_ok__"), options())
        .await
        .unwrap();
    assert_eq!(response.result, "ok");
}

#[tokio::test]
#[serial]
async fn worker_abort_is_contained_and_typed() {
    let error = worker::evaluate(
        request("https://fixture.invalid/__fixture_abort__"),
        options(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        JsWorkerError::Crashed { .. } | JsWorkerError::Exit { .. }
    ));
}

#[tokio::test]
#[serial]
async fn worker_hang_hits_hard_wall_deadline() {
    let mut options = options();
    options.timeout = Duration::from_millis(100);
    let started = Instant::now();
    let error = worker::evaluate(request("https://fixture.invalid/__fixture_hang__"), options)
        .await
        .unwrap_err();
    assert!(matches!(error, JsWorkerError::Timeout { .. }));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
#[serial]
async fn worker_output_is_drained_but_bounded() {
    let mut options = options();
    options.max_stdout_bytes = 128;
    let error = worker::evaluate(
        request("https://fixture.invalid/__fixture_output__"),
        options,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, JsWorkerError::OutputLimit));
}

#[tokio::test]
#[serial]
async fn unrelated_parent_environment_is_not_inherited() {
    let previous = std::env::var_os("PLASMATE_TEST_SECRET");
    std::env::set_var("PLASMATE_TEST_SECRET", "must-not-cross-boundary");
    let result = worker::evaluate(
        request("https://fixture.invalid/__fixture_env__"),
        options(),
    )
    .await;
    match previous {
        Some(value) => std::env::set_var("PLASMATE_TEST_SECRET", value),
        None => std::env::remove_var("PLASMATE_TEST_SECRET"),
    }
    assert_eq!(result.unwrap().result, "ok");
}

#[tokio::test]
#[serial]
async fn page_pipeline_keeps_static_som_after_worker_crash() {
    let html = "<html><head><title>Fallback survives</title></head><body><main>Structured fallback</main><script>while (true) {}</script></body></html>";
    let config = plasmate::js::pipeline::PipelineConfig {
        fetch_external_scripts: false,
        js_worker_executable: Some(fixture_path()),
        ..Default::default()
    };
    let result = plasmate::js::pipeline::process_page_async(
        html,
        "https://fixture.invalid/__fixture_abort__",
        &config,
        &reqwest::Client::new(),
    )
    .await
    .unwrap();

    assert_eq!(result.som.title, "Fallback survives");
    let failure = result
        .js_report
        .and_then(|report| report.containment_failure)
        .expect("typed containment failure");
    assert!(matches!(
        failure.kind,
        worker::JsContainmentFailureKind::Crash | worker::JsContainmentFailureKind::Exit
    ));
}

#[tokio::test]
#[serial]
async fn real_worker_executes_javascript_and_returns_mutated_dom() {
    let mut request = request("https://example.com/");
    request.html = "<html><body><p id='state'>before</p></body></html>".to_string();
    request.expression =
        "document.getElementById('state').textContent = 'after'; 'complete'".to_string();
    request.return_effective_html = true;
    let response = worker::evaluate(request, real_worker_options())
        .await
        .unwrap();
    assert_eq!(response.result, "complete");
    assert!(response.effective_html.unwrap().contains("after"));
}

#[tokio::test]
#[serial]
async fn real_infinite_script_cannot_hang_test_coordinator() {
    let mut request = request("https://example.com/");
    request.expression = "while (true) {}".to_string();
    let mut options = real_worker_options();
    options.timeout = Duration::from_millis(500);
    let started = Instant::now();
    let error = worker::evaluate(request, options).await.unwrap_err();
    assert!(matches!(error, JsWorkerError::Timeout { .. }));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
#[serial]
async fn real_javascript_result_cannot_overrun_parent_output_bound() {
    let mut request = request("https://example.com/");
    request.expression = "'x'.repeat(1024 * 1024)".to_string();
    let mut options = real_worker_options();
    options.max_stdout_bytes = 1024;
    let error = worker::evaluate(request, options).await.unwrap_err();
    assert!(matches!(error, JsWorkerError::OutputLimit));
}

#[tokio::test]
#[serial]
async fn real_page_worker_applies_page_script_dom_mutation() {
    let html = "<html><body><main id='app'>before</main><script>document.getElementById('app').textContent = 'after';</script></body></html>";
    let config = plasmate::js::pipeline::PipelineConfig {
        fetch_external_scripts: false,
        js_worker_executable: Some(PathBuf::from(env!("CARGO_BIN_EXE_plasmate"))),
        ..Default::default()
    };
    let result = plasmate::js::pipeline::process_page_async(
        html,
        "https://example.com/",
        &config,
        &reqwest::Client::new(),
    )
    .await
    .unwrap();

    assert!(result.effective_html.contains("after"));
    assert!(
        result
            .js_report
            .as_ref()
            .is_some_and(|report| report.succeeded == 1 && report.containment_failure.is_none()),
        "report={:#?}; html={}",
        result.js_report,
        result.effective_html
    );
    assert!(serde_json::to_string(&result.som)
        .unwrap()
        .contains("after"));
}

#[test]
#[serial]
fn synchronous_public_pipeline_uses_real_process_boundary() {
    let html = "<html><body><main id='app'>before</main><script>document.getElementById('app').textContent = 'sync-after';</script></body></html>";
    let config = plasmate::js::pipeline::PipelineConfig {
        js_worker_executable: Some(PathBuf::from(env!("CARGO_BIN_EXE_plasmate"))),
        ..Default::default()
    };
    let result =
        plasmate::js::pipeline::process_page(html, "https://example.com/", &config).unwrap();
    assert!(result.effective_html.contains("sync-after"));
    assert!(
        result
            .js_report
            .as_ref()
            .is_some_and(|report| report.containment_failure.is_none()),
        "report={:#?}; html={}",
        result.js_report,
        result.effective_html
    );
}

#[test]
#[serial]
fn missing_worker_is_typed_and_preserves_sync_static_som() {
    let html = "<html><head><title>Still structured</title></head><body><main>fallback</main><script>document.title = 'lost';</script></body></html>";
    let config = plasmate::js::pipeline::PipelineConfig {
        js_worker_executable: Some(PathBuf::from("/definitely/not/a/plasmate-worker")),
        ..Default::default()
    };
    let result =
        plasmate::js::pipeline::process_page(html, "https://example.com/", &config).unwrap();
    assert_eq!(result.som.title, "Still structured");
    let failure = result
        .js_report
        .and_then(|report| report.containment_failure)
        .expect("typed missing-worker failure");
    assert_eq!(failure.kind, worker::JsContainmentFailureKind::Spawn);
    assert_eq!(failure.code, "js_worker_spawn");
}

#[test]
#[serial]
fn cdp_stateful_evaluation_uses_supervised_worker() {
    let mut target = plasmate::cdp::session::CdpTarget::new().unwrap();
    target.current_url = Some("https://example.com/".to_string());
    target.effective_html =
        Some("<html><head><title>Worker title</title></head><body></body></html>".to_string());
    let value = target.evaluate_js("document.title").unwrap();
    assert_eq!(value, serde_json::Value::String("Worker title".to_string()));
}
