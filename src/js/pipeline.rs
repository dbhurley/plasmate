//! Page processing pipeline: fetch HTML -> extract scripts -> execute JS -> compile SOM.
//!
//! This is the core of Plasmate's page understanding. Unlike Lightpanda and Chrome
//! which build a full DOM then convert it afterward, we go:
//!   HTML -> (optional JS execution) -> SOM
//! in a single pass with no intermediate rendering.

use std::time::Instant;
use tracing::debug;

use super::dom_bridge::NodeRegistry;
use super::extract;
use super::extract::ScriptKind;
use super::modules;
use super::runtime::{JsExecutionReport, JsRuntime, RuntimeConfig};
use super::script_fetch;
use super::worker::{self, JsWorkerOptions, PreparedPageRequest, PreparedPageResponse};
use crate::plugin::PluginManager;
use crate::som::compiler;
use crate::som::types::Som;

/// Result of the full page processing pipeline.
#[derive(Debug)]
pub struct PageResult {
    /// The compiled SOM.
    pub som: Som,
    /// The page URL (after redirects).
    pub url: String,
    /// Pipeline timing breakdown.
    pub timing: PipelineTiming,
    /// JS execution report (if JS was enabled).
    pub js_report: Option<JsExecutionReport>,
    /// The effective HTML after JS execution (post-JS DOM serialized to HTML).
    /// For CDP Runtime.evaluate, this is the HTML that will be used to bootstrap a fresh DOM.
    pub effective_html: String,
    /// WebMCP tools discovered in the current top-level page. Tool metadata is
    /// untrusted web content and invocation remains explicitly discovery-only.
    pub webmcp: crate::webmcp::WebMcpCatalog,
}

/// Timing breakdown for the pipeline stages.
#[derive(Debug, Clone)]
pub struct PipelineTiming {
    /// Time to extract scripts from HTML.
    pub extract_scripts_us: u128,
    /// Time spent executing JS.
    pub js_execution_us: u128,
    /// Time to compile HTML to SOM.
    pub som_compile_us: u128,
    /// Total pipeline time (excluding network fetch).
    pub total_us: u128,
}

/// Configuration for the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Whether to execute JavaScript before compiling SOM.
    pub execute_js: bool,
    /// Whether to fetch external <script src="..."> files.
    pub fetch_external_scripts: bool,
    /// Limits for external script fetching.
    pub external_script_limits: script_fetch::ScriptFetchLimits,
    /// Limits for native ES-module graph acquisition.
    pub module_limits: modules::ModuleLimits,
    /// JS runtime configuration.
    pub js_config: RuntimeConfig,
    /// Max timer drain threshold in ms (execute short setTimeout callbacks).
    pub timer_drain_ms: u64,
    /// Put V8 in a supervised child process. Public async CLI/server paths must
    /// leave this enabled; isolated workers set it false to avoid nesting.
    pub isolate_js: bool,
    /// Hard wall deadline for the complete V8 child operation.
    pub js_worker_timeout_ms: u64,
    /// Maximum worker stdout bytes. The serialized DOM and runtime discovery
    /// catalog must fit inside this bound.
    pub js_worker_output_bytes: usize,
    /// Optional Linux address-space ceiling. Zero disables it because V8
    /// reserves substantial virtual address space.
    pub js_worker_memory_bytes: u64,
    /// Test/operator override for the worker executable.
    pub js_worker_executable: Option<std::path::PathBuf>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            execute_js: true,
            fetch_external_scripts: false, // Off by default for sync API; async API enables it
            external_script_limits: script_fetch::ScriptFetchLimits::default(),
            module_limits: modules::ModuleLimits::default(),
            js_config: RuntimeConfig::default(),
            timer_drain_ms: 100,
            isolate_js: true,
            js_worker_timeout_ms: worker::DEFAULT_WORKER_TIMEOUT_MS,
            js_worker_output_bytes: worker::DEFAULT_WORKER_OUTPUT_BYTES,
            js_worker_memory_bytes: 0,
            js_worker_executable: None,
        }
    }
}

fn worker_options(config: &PipelineConfig) -> JsWorkerOptions {
    JsWorkerOptions {
        executable: config.js_worker_executable.clone(),
        timeout: std::time::Duration::from_millis(config.js_worker_timeout_ms),
        max_stdout_bytes: config.js_worker_output_bytes,
        max_stderr_bytes: worker::DEFAULT_WORKER_STDERR_BYTES,
        memory_limit_bytes: config.js_worker_memory_bytes,
    }
}

fn containment_report(
    classic_scripts: usize,
    module_graph: &modules::ModuleGraph,
    error: &worker::JsWorkerError,
) -> JsExecutionReport {
    let total = classic_scripts.saturating_add(module_graph.root_count);
    let module_diagnostics = if module_graph.root_count > 0 || !module_graph.diagnostics.is_empty()
    {
        Some(modules::ModuleExecutionDiagnostics::from_graph(
            module_graph,
        ))
    } else {
        None
    };
    JsExecutionReport {
        total,
        succeeded: 0,
        failed: total,
        errors: vec![("<process-containment>".to_string(), error.to_string())],
        module_diagnostics,
        containment_failure: Some(error.containment_failure()),
    }
}

/// Wire the DOM bridge into a JsRuntime: parse HTML with html5ever, register the tree,
/// and inject the NodeRegistry into V8. The bridge callbacks are available via
/// `__plasmate_dom` but document methods are NOT overridden yet — the V8 JS DOM shim
/// remains the primary DOM for script execution. The registry provides a parallel
/// rcdom tree that can be used for direct SOM compilation in the future once all
/// bridge callbacks (innerHTML, className, etc.) are fully wired.
fn wire_dom_bridge(runtime: &mut JsRuntime, html: &str) {
    use html5ever::parse_document;
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::RcDom;

    // Parse HTML into an rcdom tree
    let dom = match parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
    {
        Ok(dom) => dom,
        Err(e) => {
            debug!("DOM bridge: html5ever parse failed, skipping bridge: {}", e);
            return;
        }
    };

    // Create registry and register the full tree
    let mut registry = NodeRegistry::new();
    registry.register_tree(&dom.document);

    debug!(
        node_count = registry.node_count(),
        doc_id = ?registry.document_id(),
        body_id = ?registry.body_id(),
        "DOM bridge: registered tree"
    );

    // Inject the registry into V8 (sets __plasmate_dom with native callbacks).
    // We do NOT call __plasmate_bridge_enable() yet because the bridge callbacks
    // are incomplete (missing getInnerHTML, setInnerHTML, getClassName native
    // implementations). Enabling the bridge would override document methods to
    // use the rcdom tree while V8's JS shim tree has the actual content, causing
    // query mismatches. For now, the registry is available but passive.
    runtime.inject_dom_bridge(registry);
}

/// JavaScript source that installs timer/rAF shims so callbacks are queued
/// rather than dropped. Must run AFTER the DOM shim but BEFORE page scripts.
const TIMER_SETUP_JS: &str = r#"
var __plasmate_timers = [];
var __plasmate_raf_queue = [];
var _origSetTimeout = typeof setTimeout !== 'undefined' ? setTimeout : null;
setTimeout = function(fn, delay) {
    if (typeof fn === 'function') __plasmate_timers.push(fn);
    return __plasmate_timers.length;
};
requestAnimationFrame = function(fn) {
    if (typeof fn === 'function') __plasmate_raf_queue.push(fn);
    return __plasmate_raf_queue.length;
};
"#;

/// JavaScript source that drains one pass of queued timer/rAF callbacks.
/// Run AFTER all page scripts execute but BEFORE SOM re-compilation.
const TIMER_DRAIN_JS: &str = r#"
(function() {
    if (typeof __plasmate_timers !== 'undefined' && __plasmate_timers.length > 0) {
        var timers = __plasmate_timers.splice(0);
        for (var i = 0; i < timers.length; i++) {
            try { timers[i](); } catch(e) {}
        }
    }
    if (typeof __plasmate_raf_queue !== 'undefined' && __plasmate_raf_queue.length > 0) {
        var rafs = __plasmate_raf_queue.splice(0);
        for (var i = 0; i < rafs.length; i++) {
            try { rafs[i](Date.now()); } catch(e) {}
        }
    }
})();
"#;

/// Install timer/rAF shims so SPA frameworks can queue deferred callbacks.
fn install_timer_shims(runtime: &mut JsRuntime) {
    let _ = runtime.execute_in_context(TIMER_SETUP_JS, "timer-setup");
}

/// Drain one pass of queued timer/rAF callbacks (catches React/Vue deferred rendering).
fn drain_timer_queue(runtime: &mut JsRuntime) {
    let _ = runtime.execute_in_context(TIMER_DRAIN_JS, "timer-drain");
    runtime.pump_microtasks();
}

/// After JS execution, serialize the DOM preferring V8's JS serialization (which
/// captures all mutations including innerHTML) but falling back to the NodeRegistry
/// if V8 serialization fails. Also drains the registry from thread-local storage.
fn serialize_post_js(runtime: &mut JsRuntime) -> Option<String> {
    // V8's serialize_dom captures all JS-side mutations (innerHTML, textContent, etc.)
    // and is more complete than the registry for now. Use it as primary.
    let v8_html = runtime.serialize_dom().ok();

    // Drain the registry from thread-local storage so it doesn't leak.
    // In future, when all DOM callbacks are wired, the registry serialization
    // could be preferred as it avoids the parse-serialize-reparse round-trip.
    let registry = runtime.take_registry();
    if let Some(ref reg) = registry {
        debug!(
            node_count = reg.node_count(),
            "DOM bridge: registry drained after JS"
        );
    }

    // Use V8 serialization. Fall back to registry if V8 failed.
    if let Some(ref html) = v8_html {
        if !html.is_empty() && html != "undefined" {
            return v8_html;
        }
    }

    // Fallback: try registry serialization
    if let Some(reg) = registry {
        match reg.serialize_document() {
            Ok(html) if !html.is_empty() => {
                debug!(
                    html_len = html.len(),
                    "DOM bridge: fallback to registry serialization"
                );
                return Some(html);
            }
            _ => {}
        }
    }

    None
}

/// Execute network-prepared scripts inside an already isolated worker.
///
/// External classic scripts and native module graphs are acquired in the
/// parent before this boundary so network deadlines remain independently
/// enforceable. Page-origin fetch/XHR remains available through the existing
/// security-checked bridge in the worker.
pub fn run_prepared_page(
    request: PreparedPageRequest,
) -> Result<PreparedPageResponse, PipelineError> {
    let started = Instant::now();
    let mut effective_html = request.html.clone();
    let mut runtime = JsRuntime::new(request.runtime_config);
    if request.inject_fetch_bridge {
        runtime.inject_fetch_bridge(reqwest::Client::new());
    }
    runtime.bootstrap_dom(&request.html, &request.url);
    runtime.install_webmcp_discovery();
    wire_dom_bridge(&mut runtime, &request.html);
    if request.install_timer_shims {
        install_timer_shims(&mut runtime);
    }

    let mut js_report = None;
    if !request.classic_scripts.is_empty()
        || !request.module_graph.roots.is_empty()
        || !request.module_graph.diagnostics.is_empty()
    {
        let mut report = runtime.execute_page_scripts(&request.classic_scripts);
        let module_report = runtime.execute_module_graph(&request.module_graph);
        report.total += module_report.roots;
        report.succeeded += module_report.roots_evaluated;
        report.failed += module_report.roots_failed;
        if module_report.roots > 0 || !module_report.diagnostics.is_empty() {
            report.module_diagnostics = Some(module_report);
        }
        if request.install_timer_shims {
            drain_timer_queue(&mut runtime);
        }
        runtime.pump_microtasks();
        runtime.fire_dom_content_loaded();
        runtime.pump_microtasks();
        if request.timer_drain_ms > 0 {
            runtime.drain_timers(request.timer_drain_ms);
            runtime.pump_microtasks();
        }
        runtime.fire_load();
        runtime.pump_microtasks();
        js_report = Some(report);
        if let Some(serialized) = serialize_post_js(&mut runtime) {
            if !serialized.is_empty() && serialized != "undefined" {
                effective_html = serialized;
            }
        }
    }
    let runtime_capture = runtime.collect_webmcp_registrations();
    Ok(PreparedPageResponse {
        effective_html,
        js_report,
        runtime_capture,
        execution_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    })
}

async fn execute_prepared_page(
    request: PreparedPageRequest,
    config: &PipelineConfig,
) -> Result<PreparedPageResponse, worker::JsWorkerError> {
    // Unit tests exercise the V8 implementation directly. Production async
    // paths and integration tests cross the real process boundary.
    if !config.isolate_js || cfg!(test) {
        return run_prepared_page(request).map_err(|error| worker::JsWorkerError::Worker {
            code: "page_execution".to_string(),
            message: error.to_string(),
        });
    }
    worker::execute_page(request, worker_options(config)).await
}

fn execute_prepared_page_sync(
    request: PreparedPageRequest,
    config: &PipelineConfig,
) -> Result<PreparedPageResponse, worker::JsWorkerError> {
    if !config.isolate_js || cfg!(test) {
        return run_prepared_page(request).map_err(|error| worker::JsWorkerError::Worker {
            code: "page_execution".to_string(),
            message: error.to_string(),
        });
    }
    worker::execute_page_sync(request, worker_options(config))
}

/// Process a page through the full pipeline (async version with external script fetching).
pub async fn process_page_async(
    html: &str,
    url: &str,
    config: &PipelineConfig,
    client: &reqwest::Client,
) -> Result<PageResult, PipelineError> {
    let pipeline_start = Instant::now();

    let mut js_report = None;
    let mut extract_us = 0u128;
    let mut js_us = 0u128;
    let mut effective_html = std::borrow::Cow::Borrowed(html);
    let mut webmcp_runtime_capture = None;

    if config.execute_js {
        let t0 = Instant::now();
        let scripts = extract::extract_scripts(html);
        extract_us = t0.elapsed().as_micros();

        // Resolve external scripts (fetch from network)
        let t1 = Instant::now();
        let resolved = if config.fetch_external_scripts {
            script_fetch::resolve_scripts(&scripts, url, client, &config.external_script_limits)
                .await
        } else {
            scripts
                .iter()
                .filter(|s| s.is_inline)
                .filter(|s| s.kind == ScriptKind::Classic)
                .map(|s| script_fetch::ResolvedScript {
                    source: s.source.clone(),
                    label: s.label.clone(),
                    index: s.index,
                })
                .collect()
        };
        let module_graph = modules::resolve_module_graph(
            &scripts,
            url,
            client,
            &config.module_limits,
            config.fetch_external_scripts,
        )
        .await;

        let exec_scripts: Vec<(String, String)> = resolved
            .iter()
            .filter(|s| !s.source.is_empty())
            .map(|s| (s.source.clone(), s.label.clone()))
            .collect();

        let request = PreparedPageRequest {
            protocol_version: worker::WORKER_PROTOCOL_VERSION.to_string(),
            html: html.to_string(),
            url: url.to_string(),
            classic_scripts: exec_scripts,
            module_graph: module_graph.clone(),
            runtime_config: config.js_config.clone(),
            timer_drain_ms: config.timer_drain_ms,
            install_timer_shims: false,
            inject_fetch_bridge: true,
        };
        match execute_prepared_page(request, config).await {
            Ok(response) => {
                js_report = response.js_report;
                effective_html = std::borrow::Cow::Owned(response.effective_html);
                webmcp_runtime_capture = Some(response.runtime_capture);
                js_us = t1.elapsed().as_micros();
                debug!(
                    external_fetched = resolved
                        .iter()
                        .filter(|s| !s.label.starts_with("inline"))
                        .count(),
                    js_ms = js_us / 1000,
                    "isolated JS execution complete"
                );
            }
            Err(error) => {
                tracing::warn!(code = error.code(), %error, "isolated JS execution failed; compiling source HTML fallback");
                js_report = Some(containment_report(
                    resolved
                        .iter()
                        .filter(|script| !script.source.is_empty())
                        .count(),
                    &module_graph,
                    &error,
                ));
                js_us = t1.elapsed().as_micros();
            }
        }
    }

    let t2 = Instant::now();
    let effective_html_owned = effective_html.into_owned();
    let webmcp = crate::webmcp::discover(&effective_html_owned, url, webmcp_runtime_capture);
    let som = compiler::compile(&effective_html_owned, url)
        .map_err(|e| PipelineError::SomCompile(e.to_string()))?;
    let som_us = t2.elapsed().as_micros();

    let total_us = pipeline_start.elapsed().as_micros();

    Ok(PageResult {
        som,
        url: url.to_string(),
        timing: PipelineTiming {
            extract_scripts_us: extract_us,
            js_execution_us: js_us,
            som_compile_us: som_us,
            total_us,
        },
        js_report,
        effective_html: effective_html_owned,
        webmcp,
    })
}

/// Process a page through the full pipeline.
///
/// This is the main entry point for converting fetched HTML into a SOM.
/// If JS execution is enabled:
/// 1. The source HTML is parsed into a JS DOM tree
/// 2. Inline scripts are extracted and executed in V8
/// 3. DOMContentLoaded and load events are fired
/// 4. The resulting DOM tree is serialized back to HTML
/// 5. The serialized HTML (with JS modifications) is compiled to SOM
pub fn process_page(
    html: &str,
    url: &str,
    config: &PipelineConfig,
) -> Result<PageResult, PipelineError> {
    process_page_with_client(html, url, config, None)
}

/// Process a page through the full pipeline with an optional HTTP client.
///
/// When a client is provided, the restricted JS fetch()/XMLHttpRequest bridge
/// is enabled. The bridge intentionally does not copy caller cookies, proxy,
/// custom TLS, or default headers. Without a client, it returns stub responses.
pub fn process_page_with_client(
    html: &str,
    url: &str,
    config: &PipelineConfig,
    client: Option<&reqwest::Client>,
) -> Result<PageResult, PipelineError> {
    let pipeline_start = Instant::now();

    let mut js_report = None;
    let mut extract_us = 0u128;
    let mut js_us = 0u128;
    let mut effective_html = std::borrow::Cow::Borrowed(html);
    let mut webmcp_runtime_capture = None;

    // Phase 1: Extract scripts (if JS enabled)
    if config.execute_js {
        let t0 = Instant::now();
        let scripts = extract::extract_scripts(html);
        extract_us = t0.elapsed().as_micros();

        let inline_scripts: Vec<(String, String)> = scripts
            .iter()
            .filter(|s| s.is_inline && s.kind == ScriptKind::Classic)
            .map(|s| (s.source.clone(), s.label.clone()))
            .collect();
        let module_graph = modules::inline_module_graph(&scripts, url, &config.module_limits);

        let t1 = Instant::now();
        let request = PreparedPageRequest {
            protocol_version: worker::WORKER_PROTOCOL_VERSION.to_string(),
            html: html.to_string(),
            url: url.to_string(),
            classic_scripts: inline_scripts,
            module_graph: module_graph.clone(),
            runtime_config: config.js_config.clone(),
            timer_drain_ms: config.timer_drain_ms,
            install_timer_shims: false,
            inject_fetch_bridge: client.is_some(),
        };
        match execute_prepared_page_sync(request, config) {
            Ok(response) => {
                js_report = response.js_report;
                effective_html = std::borrow::Cow::Owned(response.effective_html);
                webmcp_runtime_capture = Some(response.runtime_capture);
            }
            Err(error) => {
                tracing::warn!(code = error.code(), %error, "isolated sync JS execution failed; compiling source HTML fallback");
                js_report = Some(containment_report(
                    scripts
                        .iter()
                        .filter(|script| script.is_inline && script.kind == ScriptKind::Classic)
                        .count(),
                    &module_graph,
                    &error,
                ));
            }
        }
        js_us = t1.elapsed().as_micros();
    }

    let t2 = Instant::now();
    let effective_html_owned = effective_html.into_owned();
    let webmcp = crate::webmcp::discover(&effective_html_owned, url, webmcp_runtime_capture);
    let som = compiler::compile(&effective_html_owned, url)
        .map_err(|e| PipelineError::SomCompile(e.to_string()))?;
    let som_us = t2.elapsed().as_micros();

    let total_us = pipeline_start.elapsed().as_micros();

    Ok(PageResult {
        som,
        url: url.to_string(),
        timing: PipelineTiming {
            extract_scripts_us: extract_us,
            js_execution_us: js_us,
            som_compile_us: som_us,
            total_us,
        },
        js_report,
        effective_html: effective_html_owned,
        webmcp,
    })
}

/// Process a page with Wasm plugin hooks at each pipeline stage.
///
/// Identical to `process_page_async` but also runs:
/// - `post_parse` after JS execution, before SOM compilation
/// - `post_som` after SOM compilation
pub async fn process_page_async_with_plugins(
    html: &str,
    url: &str,
    config: &PipelineConfig,
    client: &reqwest::Client,
    plugins: &mut PluginManager,
) -> Result<PageResult, PipelineError> {
    let pipeline_start = Instant::now();

    let mut js_report = None;
    let mut extract_us = 0u128;
    let mut js_us = 0u128;
    let mut effective_html = std::borrow::Cow::Borrowed(html);
    let mut webmcp_runtime_capture = None;

    if config.execute_js {
        let t0 = Instant::now();
        let scripts = extract::extract_scripts(html);
        extract_us = t0.elapsed().as_micros();

        let t1 = Instant::now();
        let resolved = if config.fetch_external_scripts {
            script_fetch::resolve_scripts(&scripts, url, client, &config.external_script_limits)
                .await
        } else {
            scripts
                .iter()
                .filter(|s| s.is_inline)
                .filter(|s| s.kind == ScriptKind::Classic)
                .map(|s| script_fetch::ResolvedScript {
                    source: s.source.clone(),
                    label: s.label.clone(),
                    index: s.index,
                })
                .collect()
        };
        let module_graph = modules::resolve_module_graph(
            &scripts,
            url,
            client,
            &config.module_limits,
            config.fetch_external_scripts,
        )
        .await;

        let exec_scripts: Vec<(String, String)> = resolved
            .iter()
            .filter(|s| !s.source.is_empty())
            .map(|s| (s.source.clone(), s.label.clone()))
            .collect();

        let request = PreparedPageRequest {
            protocol_version: worker::WORKER_PROTOCOL_VERSION.to_string(),
            html: html.to_string(),
            url: url.to_string(),
            classic_scripts: exec_scripts,
            module_graph: module_graph.clone(),
            runtime_config: config.js_config.clone(),
            timer_drain_ms: config.timer_drain_ms,
            install_timer_shims: true,
            inject_fetch_bridge: true,
        };
        match execute_prepared_page(request, config).await {
            Ok(response) => {
                js_report = response.js_report;
                effective_html = std::borrow::Cow::Owned(response.effective_html);
                webmcp_runtime_capture = Some(response.runtime_capture);
                js_us = t1.elapsed().as_micros();
            }
            Err(error) => {
                tracing::warn!(code = error.code(), %error, "isolated plugin JS execution failed; running plugins on source HTML fallback");
                js_report = Some(containment_report(
                    resolved
                        .iter()
                        .filter(|script| !script.source.is_empty())
                        .count(),
                    &module_graph,
                    &error,
                ));
                js_us = t1.elapsed().as_micros();
            }
        }
    }

    // Plugin hook: post_parse (between JS execution and SOM compilation).
    let effective_html_owned = if plugins.has_hook(crate::plugin::Hook::PostParse) {
        plugins
            .run_post_parse(&effective_html)
            .map_err(|e| PipelineError::Plugin(e.to_string()))?
    } else {
        effective_html.into_owned()
    };
    let webmcp = crate::webmcp::discover(&effective_html_owned, url, webmcp_runtime_capture);

    let t2 = Instant::now();
    let som = compiler::compile(&effective_html_owned, url)
        .map_err(|e| PipelineError::SomCompile(e.to_string()))?;
    let som_us = t2.elapsed().as_micros();

    // Plugin hook: post_som (after SOM compilation).
    let som = plugins
        .run_post_som(som)
        .map_err(|e| PipelineError::Plugin(e.to_string()))?;

    let total_us = pipeline_start.elapsed().as_micros();

    Ok(PageResult {
        som,
        url: url.to_string(),
        timing: PipelineTiming {
            extract_scripts_us: extract_us,
            js_execution_us: js_us,
            som_compile_us: som_us,
            total_us,
        },
        js_report,
        effective_html: effective_html_owned,
        webmcp,
    })
}

/// Sync version of `process_page_async_with_plugins`.
pub fn process_page_with_plugins(
    html: &str,
    url: &str,
    config: &PipelineConfig,
    plugins: &mut PluginManager,
) -> Result<PageResult, PipelineError> {
    let pipeline_start = Instant::now();

    let mut js_report = None;
    let mut extract_us = 0u128;
    let mut js_us = 0u128;
    let mut effective_html = std::borrow::Cow::Borrowed(html);
    let mut webmcp_runtime_capture = None;

    if config.execute_js {
        let t0 = Instant::now();
        let scripts = extract::extract_scripts(html);
        extract_us = t0.elapsed().as_micros();

        let inline_scripts: Vec<(String, String)> = scripts
            .iter()
            .filter(|s| s.is_inline && s.kind == ScriptKind::Classic)
            .map(|s| (s.source.clone(), s.label.clone()))
            .collect();
        let module_graph = modules::inline_module_graph(&scripts, url, &config.module_limits);

        let t1 = Instant::now();
        let request = PreparedPageRequest {
            protocol_version: worker::WORKER_PROTOCOL_VERSION.to_string(),
            html: html.to_string(),
            url: url.to_string(),
            classic_scripts: inline_scripts,
            module_graph: module_graph.clone(),
            runtime_config: config.js_config.clone(),
            timer_drain_ms: config.timer_drain_ms,
            install_timer_shims: true,
            inject_fetch_bridge: false,
        };
        match execute_prepared_page_sync(request, config) {
            Ok(response) => {
                js_report = response.js_report;
                effective_html = std::borrow::Cow::Owned(response.effective_html);
                webmcp_runtime_capture = Some(response.runtime_capture);
            }
            Err(error) => {
                tracing::warn!(code = error.code(), %error, "isolated sync plugin JS execution failed; running plugins on source HTML fallback");
                js_report = Some(containment_report(
                    scripts
                        .iter()
                        .filter(|script| script.is_inline && script.kind == ScriptKind::Classic)
                        .count(),
                    &module_graph,
                    &error,
                ));
            }
        }
        js_us = t1.elapsed().as_micros();
    }

    let effective_html_owned = if plugins.has_hook(crate::plugin::Hook::PostParse) {
        plugins
            .run_post_parse(&effective_html)
            .map_err(|e| PipelineError::Plugin(e.to_string()))?
    } else {
        effective_html.into_owned()
    };
    let webmcp = crate::webmcp::discover(&effective_html_owned, url, webmcp_runtime_capture);

    let t2 = Instant::now();
    let som = compiler::compile(&effective_html_owned, url)
        .map_err(|e| PipelineError::SomCompile(e.to_string()))?;
    let som_us = t2.elapsed().as_micros();

    let som = plugins
        .run_post_som(som)
        .map_err(|e| PipelineError::Plugin(e.to_string()))?;

    let total_us = pipeline_start.elapsed().as_micros();

    Ok(PageResult {
        som,
        url: url.to_string(),
        timing: PipelineTiming {
            extract_scripts_us: extract_us,
            js_execution_us: js_us,
            som_compile_us: som_us,
            total_us,
        },
        js_report,
        effective_html: effective_html_owned,
        webmcp,
    })
}

/// Errors from the page processing pipeline.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("SOM compilation failed: {0}")]
    SomCompile(String),
    #[error("JS execution failed: {0}")]
    JsExecution(String),
    #[error("plugin error: {0}")]
    Plugin(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_no_js() {
        let config = PipelineConfig {
            execute_js: false,
            ..Default::default()
        };
        let html = r#"<html><body><h1>Hello</h1><p>World</p></body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();
        assert_eq!(result.som.title, "");
        assert!(result.js_report.is_none());
        assert!(result.timing.js_execution_us == 0);
    }

    #[test]
    fn classic_scripts_run_before_deferred_modules_in_document_order() {
        // Locally reduced from the WPT module-script ordering cases; see the
        // adjacent fixture provenance document.
        let html = include_str!("../../tests/fixtures/js-modules/ordering.html");
        let result =
            process_page(html, "https://example.com/page", &PipelineConfig::default()).unwrap();
        assert!(
            result
                .effective_html
                .contains("classic-before,classic-after,module:https://example.com/page"),
            "{}",
            result.effective_html
        );
        let report = result.js_report.unwrap();
        assert_eq!(report.total, 3);
        assert_eq!(report.failed, 0, "{report:#?}");
        assert_eq!(
            report.module_diagnostics.unwrap().version,
            modules::MODULE_DIAGNOSTICS_VERSION
        );
    }

    #[test]
    fn import_maps_are_reported_instead_of_silently_ignored() {
        let result = process_page(
            r#"<script type="importmap">{"imports":{"pkg":"./pkg.js"}}</script>"#,
            "https://example.com/page",
            &PipelineConfig::default(),
        )
        .unwrap();
        let modules = result
            .js_report
            .and_then(|report| report.module_diagnostics)
            .expect("module diagnostics");
        assert!(modules
            .diagnostics
            .iter()
            .any(|item| item.code == "unsupported-import-maps"));
    }

    #[test]
    fn test_pipeline_with_js() {
        let config = PipelineConfig::default();
        let html = r#"<html><head>
            <title>Test</title>
            <script>document.title = 'JS Title';</script>
        </head><body><h1>Hello</h1></body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();
        assert!(result.js_report.is_some());
        let report = result.js_report.unwrap();
        assert_eq!(report.succeeded, 1);
    }

    #[test]
    fn test_pipeline_timing() {
        let config = PipelineConfig::default();
        let html = r#"<html><body><script>var x = 1;</script><p>Hello</p></body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();
        assert!(result.timing.total_us > 0);
        assert!(result.timing.som_compile_us > 0);
    }

    #[test]
    fn test_pipeline_js_error_nonfatal() {
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <script>undefinedFunction();</script>
            <script>var ok = true;</script>
            <h1>Content</h1>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();
        let report = result.js_report.unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(report.succeeded, 1);
        // SOM should still compile
        assert!(result.som.meta.element_count > 0);
    }

    #[test]
    fn test_pipeline_real_world_js_patterns() {
        let config = PipelineConfig::default();
        // Simulate common JS patterns found on real sites
        let html = r#"<html><head>
            <script>
                // Analytics
                window.dataLayer = window.dataLayer || [];
                function gtag(){ dataLayer.push(arguments); }
                gtag('js', new Date());
                gtag('config', 'GA-123');
            </script>
            <script>
                // DOM ready pattern
                document.addEventListener('DOMContentLoaded', function() {
                    document.title = 'Loaded';
                });
            </script>
            <script>
                // Feature detection
                var hasTouch = 'ontouchstart' in window;
                var hasFetch = typeof fetch === 'function';
            </script>
        </head><body>
            <nav><a href="/">Home</a></nav>
            <main><h1>Page</h1><p>Content</p></main>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();
        let report = result.js_report.unwrap();
        assert_eq!(
            report.succeeded, 3,
            "All 3 common JS patterns should execute: {:?}",
            report.errors
        );
        assert_eq!(report.failed, 0);
    }

    // =========================================================================
    // JS Rendering Pipeline Tests
    // =========================================================================

    #[test]
    fn test_pipeline_js_creates_elements() {
        // Test that JS-created elements appear in the SOM
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div id="root"></div>
            <script>
                var root = document.getElementById('root');
                var p = document.createElement('p');
                p.id = 'dynamic-para';
                p.textContent = 'Created by JavaScript';
                root.appendChild(p);
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        // Serialize SOM to JSON and check for dynamic content
        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("Created by JavaScript"),
            "SOM should contain JS-created content"
        );
    }

    #[test]
    fn test_pipeline_innerhtml_rendering() {
        // Test that innerHTML modifications appear in SOM
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div id="app"></div>
            <script>
                document.getElementById('app').innerHTML = '<h1>Rendered Title</h1><p>Rendered paragraph</p>';
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("Rendered Title"),
            "SOM should contain innerHTML content"
        );
        assert!(
            som_json.contains("Rendered paragraph"),
            "SOM should contain innerHTML paragraph"
        );
    }

    #[test]
    fn test_pipeline_getelementbyid_modify() {
        // Test that modifying existing elements via getElementById works
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <p id="target">Original text</p>
            <script>
                document.getElementById('target').textContent = 'Modified by JS';
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("Modified by JS"),
            "SOM should reflect JS text modification"
        );
        assert!(
            !som_json.contains("Original text"),
            "Original text should be replaced"
        );
    }

    #[test]
    fn test_pipeline_timer_based_rendering() {
        // Test that setTimeout(fn, 0) callbacks are executed
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div id="container"></div>
            <script>
                setTimeout(function() {
                    var el = document.createElement('span');
                    el.textContent = 'Timer rendered';
                    document.getElementById('container').appendChild(el);
                }, 0);
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("Timer rendered"),
            "Timer callback should have executed"
        );
    }

    #[test]
    fn test_pipeline_domcontentloaded_handler() {
        // Test that DOMContentLoaded handlers execute
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div id="target"></div>
            <script>
                document.addEventListener('DOMContentLoaded', function() {
                    document.getElementById('target').innerHTML = '<strong>Loaded!</strong>';
                });
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("Loaded!"),
            "DOMContentLoaded handler should have executed"
        );
    }

    #[test]
    fn test_pipeline_react_style_app() {
        // Simulate a React-style single page app with JS rendering
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div id="root"></div>
            <script>
                function App() {
                    var div = document.createElement('div');
                    div.className = 'app-container';

                    var header = document.createElement('header');
                    var h1 = document.createElement('h1');
                    h1.textContent = 'My React App';
                    header.appendChild(h1);
                    div.appendChild(header);

                    var main = document.createElement('main');
                    var p = document.createElement('p');
                    p.textContent = 'Welcome to the application';
                    main.appendChild(p);

                    var btn = document.createElement('button');
                    btn.textContent = 'Get Started';
                    btn.id = 'cta-button';
                    main.appendChild(btn);
                    div.appendChild(main);

                    return div;
                }

                document.getElementById('root').appendChild(App());
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        // Check that all React-rendered content is in the SOM
        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("My React App"),
            "Heading should be present"
        );
        assert!(
            som_json.contains("Welcome to the application"),
            "Content should be present"
        );
        assert!(som_json.contains("Get Started"), "Button should be present");

        // Check interactive element count (button should be counted)
        assert!(
            result.som.meta.interactive_count > 0,
            "Should have interactive elements"
        );
    }

    #[test]
    fn test_pipeline_document_write() {
        // Test that document.write works
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <script>
                document.write('<p id="written">Written content</p>');
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("Written content"),
            "document.write content should appear"
        );
    }

    #[test]
    fn test_pipeline_no_regression_static_html() {
        // Ensure static HTML without JS still works correctly
        let config = PipelineConfig::default();
        let html = r#"<!DOCTYPE html>
<html lang="en">
<head><title>Static Page</title></head>
<body>
<nav>
    <a href="/">Home</a>
    <a href="/about">About</a>
</nav>
<main>
    <h1>Welcome</h1>
    <p>This is static content with no JavaScript.</p>
    <form action="/search" method="GET">
        <input type="text" placeholder="Search...">
        <button type="submit">Search</button>
    </form>
</main>
<footer>
    <p>Copyright 2026</p>
</footer>
</body>
</html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        // SOM should have proper structure
        assert_eq!(result.som.title, "Static Page");
        assert!(!result.som.regions.is_empty());

        // Should have navigation, main, form regions
        assert!(
            result.som.regions.iter().any(|r| !r.elements.is_empty()),
            "Regions should have elements"
        );
    }

    #[test]
    fn test_pipeline_queryselector_in_script() {
        // Test that querySelector works in scripts
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div class="container">
                <p class="message">Initial</p>
            </div>
            <script>
                var msg = document.querySelector('.container .message');
                if (msg) {
                    msg.textContent = 'Found and modified';
                }
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("Found and modified"),
            "querySelector should find element"
        );
    }

    #[test]
    fn test_pipeline_multiple_scripts_state_persistence() {
        // Test that state persists across multiple script blocks
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div id="output"></div>
            <script>
                var items = [];
                items.push('first');
            </script>
            <script>
                items.push('second');
            </script>
            <script>
                items.push('third');
                document.getElementById('output').textContent = items.join(', ');
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(
            som_json.contains("first, second, third"),
            "State should persist across script blocks"
        );
    }

    #[test]
    fn test_pipeline_link_creation() {
        // Test that dynamically created links appear in SOM with correct attributes
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <nav id="nav"></nav>
            <script>
                var nav = document.getElementById('nav');
                var links = [
                    {href: '/home', text: 'Home'},
                    {href: '/products', text: 'Products'},
                    {href: '/contact', text: 'Contact'}
                ];
                links.forEach(function(link) {
                    var a = document.createElement('a');
                    a.href = link.href;
                    a.textContent = link.text;
                    nav.appendChild(a);
                });
            </script>
        </body></html>"#;
        let result = process_page(html, "https://example.com", &config).unwrap();

        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(som_json.contains("/home"), "Home link should be present");
        assert!(
            som_json.contains("/products"),
            "Products link should be present"
        );
        assert!(
            som_json.contains("Contact"),
            "Contact text should be present"
        );
        assert!(
            result.som.meta.interactive_count >= 3,
            "Should have at least 3 interactive links"
        );
    }

    #[test]
    fn test_process_page_with_client_no_client() {
        // Test process_page_with_client with None client (fallback to stub)
        let config = PipelineConfig::default();
        let html = r#"<html><body>
            <div id="result"></div>
            <script>
                // fetch without bridge should return stub response
                fetch('/api/data').then(function(r) {
                    document.getElementById('result').textContent = r.ok ? 'OK' : 'FAIL';
                });
            </script>
        </body></html>"#;
        let result = process_page_with_client(html, "https://example.com", &config, None).unwrap();

        // The stub fetch returns ok:true, so result should be "OK"
        let som_json = serde_json::to_string(&result.som).unwrap();
        assert!(som_json.contains("OK"), "Stub fetch should return ok");
    }
}
