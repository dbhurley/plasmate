//! Secure acquisition and representation of native V8 ES-module graphs.
//!
//! This module intentionally implements a browser-compatible core, not a
//! general-purpose package loader. Only same-origin URL imports are accepted;
//! bare specifiers, import maps, dynamic import, import attributes, and
//! cross-origin CORS are reported as unsupported.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE, LOCATION};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use super::extract::{is_javascript_mime_essence, ScriptBlock, ScriptKind};
use crate::network::security::OutboundUrlPolicy;

pub const MODULE_DIAGNOSTICS_VERSION: &str = "plasmate.js-modules.v1";
pub(crate) const MAX_MODULE_DIAGNOSTICS: usize = 128;

#[derive(Debug, Clone)]
pub struct ModuleLimits {
    pub max_modules: usize,
    pub max_depth: usize,
    pub max_module_bytes: usize,
    pub max_total_bytes: usize,
    pub max_redirects: usize,
    pub fetch_timeout_ms: u64,
    pub graph_timeout_ms: u64,
}

impl Default for ModuleLimits {
    fn default() -> Self {
        Self {
            max_modules: 64,
            max_depth: 16,
            max_module_bytes: 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024,
            max_redirects: 5,
            fetch_timeout_ms: 5_000,
            graph_timeout_ms: 15_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModuleSource {
    /// Canonical cache identity. Inline roots use a synthetic fragment so
    /// multiple elements remain distinct module records.
    pub url: String,
    /// URL used to resolve this module's static requests.
    pub resolution_url: String,
    /// Browser-visible `import.meta.url`.
    pub import_meta_url: String,
    pub source: String,
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ModuleGraph {
    /// Number of module script elements encountered, including invalid roots.
    pub root_count: usize,
    pub roots: Vec<String>,
    pub sources: HashMap<String, ModuleSource>,
    /// Request URL to final response URL. Every runtime lookup is canonicalized
    /// through this map before selecting the single compiled module object.
    pub aliases: HashMap<String, String>,
    pub diagnostics: Vec<ModuleDiagnostic>,
}

impl ModuleGraph {
    fn push_diagnostic(&mut self, value: ModuleDiagnostic) {
        if self.diagnostics.len() < MAX_MODULE_DIAGNOSTICS {
            self.diagnostics.push(value);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleDiagnostic {
    pub url: String,
    pub phase: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleExecutionDiagnostics {
    pub version: String,
    pub roots: usize,
    pub modules_loaded: usize,
    pub roots_evaluated: usize,
    pub roots_failed: usize,
    pub diagnostics: Vec<ModuleDiagnostic>,
}

impl ModuleExecutionDiagnostics {
    pub fn from_graph(graph: &ModuleGraph) -> Self {
        Self {
            version: MODULE_DIAGNOSTICS_VERSION.to_string(),
            roots: graph.root_count,
            modules_loaded: graph.sources.len(),
            roots_evaluated: 0,
            roots_failed: graph.root_count.saturating_sub(graph.roots.len()),
            diagnostics: graph.diagnostics.clone(),
        }
    }
}

#[derive(Debug)]
struct PendingModule {
    url: String,
    source: Option<String>,
    depth: usize,
}

/// Resolve module roots and their static dependency graph before entering V8.
/// Holding V8 state across network awaits would make server futures non-Send.
pub async fn resolve_module_graph(
    scripts: &[ScriptBlock],
    page_url: &str,
    client: &Client,
    limits: &ModuleLimits,
    fetch_external: bool,
) -> ModuleGraph {
    let mut graph = ModuleGraph {
        root_count: scripts
            .iter()
            .filter(|script| script.kind == ScriptKind::Module)
            .count(),
        ..Default::default()
    };
    for script in scripts
        .iter()
        .filter(|script| script.kind == ScriptKind::ImportMap)
    {
        graph.push_diagnostic(diagnostic(
            &script.label,
            "resolve",
            "unsupported-import-maps",
            "import maps are not supported",
        ));
    }
    let policy = OutboundUrlPolicy::from_environment();
    let page = match eligible_page_url(page_url, policy) {
        Ok(url) => url,
        Err(message) => {
            for script in scripts.iter().filter(|s| s.kind == ScriptKind::Module) {
                graph.push_diagnostic(diagnostic(
                    &script.label,
                    "eligibility",
                    "ineligible-page-url",
                    &message,
                ));
            }
            return graph;
        }
    };
    let origin = origin_key(&page);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(limits.graph_timeout_ms);
    let mut queue = VecDeque::new();
    let mut root_limit_reported = false;

    for script in scripts.iter().filter(|s| s.kind == ScriptKind::Module) {
        if graph.roots.len() >= limits.max_modules {
            if !root_limit_reported {
                graph.push_diagnostic(diagnostic(
                    page.as_str(),
                    "budget",
                    "module-count-exceeded",
                    "module roots exceeded the configured module count",
                ));
                root_limit_reported = true;
            }
            continue;
        }
        let (url, source) = if script.is_inline {
            let mut url = page.clone();
            url.set_fragment(Some(&format!("plasmate-inline-module-{}", script.index)));
            (url, Some(script.source.clone()))
        } else {
            if script.label.trim().is_empty() {
                graph.push_diagnostic(diagnostic(
                    page.as_str(),
                    "resolve",
                    "empty-module-src",
                    "module script has an empty src attribute",
                ));
                continue;
            }
            match resolve_specifier(&page, &script.label, &origin) {
                Ok(url) => (url, None),
                Err((code, message)) => {
                    graph.push_diagnostic(diagnostic(&script.label, "resolve", code, &message));
                    continue;
                }
            }
        };
        graph.roots.push(url.to_string());
        queue.push_back(PendingModule {
            url: url.to_string(),
            source,
            depth: 0,
        });
    }

    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;
    while let Some(pending) = queue.pop_front() {
        if seen.contains(&pending.url) {
            continue;
        }
        if tokio::time::Instant::now() >= deadline {
            graph.push_diagnostic(diagnostic(
                &pending.url,
                "fetch",
                "graph-deadline-exceeded",
                "module graph acquisition exceeded its wall deadline",
            ));
            break;
        }
        if seen.len() >= limits.max_modules {
            graph.push_diagnostic(diagnostic(
                &pending.url,
                "budget",
                "module-count-exceeded",
                "module graph exceeded the configured module count",
            ));
            break;
        }
        if pending.depth > limits.max_depth {
            graph.push_diagnostic(diagnostic(
                &pending.url,
                "budget",
                "module-depth-exceeded",
                "module graph exceeded the configured dependency depth",
            ));
            seen.insert(pending.url);
            continue;
        }

        let is_inline = pending.source.is_some();
        let (module_url, source) = match pending.source {
            Some(source) => {
                if source.len() > limits.max_module_bytes {
                    graph.push_diagnostic(diagnostic(
                        &pending.url,
                        "budget",
                        "module-bytes-exceeded",
                        "inline module exceeded the per-module byte limit",
                    ));
                    seen.insert(pending.url);
                    continue;
                }
                (pending.url.clone(), source)
            }
            None if !fetch_external => {
                graph.push_diagnostic(diagnostic(
                    &pending.url,
                    "fetch",
                    "external-module-fetch-disabled",
                    "external module fetching is disabled for this pipeline",
                ));
                seen.insert(pending.url);
                continue;
            }
            None => {
                match fetch_module(client, &pending.url, &origin, policy, limits, deadline).await {
                    Ok(fetched) => fetched,
                    Err((code, message)) => {
                        graph.push_diagnostic(diagnostic(&pending.url, "fetch", code, &message));
                        seen.insert(pending.url);
                        continue;
                    }
                }
            }
        };

        graph
            .aliases
            .insert(pending.url.clone(), module_url.clone());
        if graph.sources.contains_key(&module_url) {
            seen.insert(pending.url);
            continue;
        }
        if total_bytes.saturating_add(source.len()) > limits.max_total_bytes {
            graph.push_diagnostic(diagnostic(
                &module_url,
                "budget",
                "aggregate-bytes-exceeded",
                "module graph exceeded the aggregate source byte limit",
            ));
            seen.insert(pending.url);
            continue;
        }
        total_bytes += source.len();
        let inspection_remaining =
            match deadline.checked_duration_since(tokio::time::Instant::now()) {
                Some(remaining) => remaining,
                None => {
                    graph.push_diagnostic(diagnostic(
                        &module_url,
                        "compile",
                        "graph-deadline-exceeded",
                        "module graph acquisition exceeded its wall deadline",
                    ));
                    break;
                }
            };
        let requests = match tokio::time::timeout(
            inspection_remaining,
            inspect_module_source(
                source.clone(),
                module_url.clone(),
                limits.max_modules.saturating_add(1),
            ),
        )
        .await
        {
            Ok(Ok(requests)) => requests,
            Ok(Err(message)) => {
                graph.push_diagnostic(diagnostic(
                    &module_url,
                    "compile",
                    "module-syntax-error",
                    &message,
                ));
                seen.insert(pending.url);
                continue;
            }
            Err(_) => {
                graph.push_diagnostic(diagnostic(
                    &module_url,
                    "compile",
                    "graph-deadline-exceeded",
                    "module compile-only inspection exceeded the graph wall deadline",
                ));
                break;
            }
        };
        let referrer = Url::parse(&module_url).expect("module URL was validated");
        let mut imports = Vec::new();
        for request in requests
            .into_iter()
            .take(limits.max_modules.saturating_add(1))
        {
            if request.has_attributes {
                graph.push_diagnostic(diagnostic(
                    &module_url,
                    "resolve",
                    "unsupported-import-attributes",
                    "module import attributes are not supported",
                ));
                continue;
            }
            let specifier = request.specifier;
            match resolve_specifier(&referrer, &specifier, &origin) {
                Ok(url) => {
                    let target = url.to_string();
                    imports.push(target.clone());
                    queue.push_back(PendingModule {
                        url: target,
                        source: None,
                        depth: pending.depth + 1,
                    });
                }
                Err((code, message)) => {
                    graph.push_diagnostic(diagnostic(&pending.url, "resolve", code, &message))
                }
            }
        }
        seen.insert(pending.url.clone());
        graph.sources.insert(
            module_url.clone(),
            ModuleSource {
                url: module_url.clone(),
                resolution_url: if is_inline {
                    page.to_string()
                } else {
                    module_url.clone()
                },
                import_meta_url: if is_inline {
                    page.to_string()
                } else {
                    module_url
                },
                source,
                imports,
            },
        );
    }

    graph
}

/// Build a graph from inline roots without network access for synchronous
/// callers. Static dependencies are diagnosed as unavailable.
pub fn inline_module_graph(
    scripts: &[ScriptBlock],
    page_url: &str,
    limits: &ModuleLimits,
) -> ModuleGraph {
    let mut graph = ModuleGraph {
        root_count: scripts
            .iter()
            .filter(|script| script.kind == ScriptKind::Module)
            .count(),
        ..Default::default()
    };
    for script in scripts
        .iter()
        .filter(|script| script.kind == ScriptKind::ImportMap)
    {
        graph.push_diagnostic(diagnostic(
            &script.label,
            "resolve",
            "unsupported-import-maps",
            "import maps are not supported",
        ));
    }
    let policy = OutboundUrlPolicy::from_environment();
    let page = match eligible_page_url(page_url, policy) {
        Ok(url) => url,
        Err(message) => {
            for script in scripts.iter().filter(|s| s.kind == ScriptKind::Module) {
                graph.push_diagnostic(diagnostic(
                    &script.label,
                    "eligibility",
                    "ineligible-page-url",
                    &message,
                ));
            }
            return graph;
        }
    };
    let origin = origin_key(&page);
    let mut total = 0usize;
    let mut root_limit_reported = false;
    for script in scripts.iter().filter(|s| s.kind == ScriptKind::Module) {
        if graph.roots.len() >= limits.max_modules {
            if !root_limit_reported {
                graph.push_diagnostic(diagnostic(
                    page.as_str(),
                    "budget",
                    "module-count-exceeded",
                    "module roots exceeded the configured module count",
                ));
                root_limit_reported = true;
            }
            continue;
        }
        if !script.is_inline {
            if script.label.trim().is_empty() {
                graph.push_diagnostic(diagnostic(
                    page.as_str(),
                    "resolve",
                    "empty-module-src",
                    "module script has an empty src attribute",
                ));
                continue;
            }
            match resolve_specifier(&page, &script.label, &origin) {
                Ok(target) => {
                    graph.roots.push(target.to_string());
                    graph.push_diagnostic(diagnostic(
                        target.as_str(),
                        "fetch",
                        "external-module-fetch-disabled",
                        "external module fetching is unavailable in the synchronous pipeline",
                    ));
                }
                Err((code, message)) => {
                    graph.push_diagnostic(diagnostic(&script.label, "resolve", code, &message));
                }
            }
            continue;
        }
        let mut url = page.clone();
        url.set_fragment(Some(&format!("plasmate-inline-module-{}", script.index)));
        let url = url.to_string();
        graph.roots.push(url.clone());
        if script.source.len() > limits.max_module_bytes
            || total.saturating_add(script.source.len()) > limits.max_total_bytes
        {
            graph.push_diagnostic(diagnostic(
                &url,
                "budget",
                "module-bytes-exceeded",
                "inline module exceeded the configured source budget",
            ));
            continue;
        }
        total += script.source.len();
        let requests = match inspect_module_source_blocking(
            &script.source,
            &url,
            limits.max_modules.saturating_add(1),
        ) {
            Ok(requests) => requests,
            Err(message) => {
                graph.push_diagnostic(diagnostic(&url, "compile", "module-syntax-error", &message));
                continue;
            }
        };
        let mut imports = Vec::new();
        for request in requests
            .into_iter()
            .take(limits.max_modules.saturating_add(1))
        {
            if request.has_attributes {
                graph.push_diagnostic(diagnostic(
                    &url,
                    "resolve",
                    "unsupported-import-attributes",
                    "module import attributes are not supported",
                ));
                continue;
            }
            let specifier = request.specifier;
            match resolve_specifier(&page, &specifier, &origin) {
                Ok(target) => {
                    imports.push(target.to_string());
                    graph.push_diagnostic(diagnostic(
                        &url,
                        "fetch",
                        "external-module-fetch-disabled",
                        "a static dependency requires the asynchronous external-module pipeline",
                    ));
                }
                Err((code, message)) => {
                    graph.push_diagnostic(diagnostic(&url, "resolve", code, &message))
                }
            }
        }
        graph.sources.insert(
            url.clone(),
            ModuleSource {
                url: url.clone(),
                resolution_url: page.to_string(),
                import_meta_url: page.to_string(),
                source: script.source.clone(),
                imports,
            },
        );
        graph.aliases.insert(url.clone(), url);
    }
    graph
}

fn eligible_page_url(value: &str, policy: OutboundUrlPolicy) -> Result<Url, String> {
    let url = policy.validate_url_syntax(value)?;
    if url.scheme() != "https" && !policy.allows_private_network() {
        return Err("module execution requires HTTPS (HTTP is available only in explicit unsafe local-fixture mode)".into());
    }
    Ok(url)
}

fn resolve_specifier(
    referrer: &Url,
    specifier: &str,
    expected_origin: &(String, String, u16),
) -> Result<Url, (&'static str, String)> {
    if !(specifier.starts_with('/')
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with("http://")
        || specifier.starts_with("https://"))
    {
        return Err((
            "unsupported-bare-specifier",
            "bare module specifiers are not supported".into(),
        ));
    }
    let mut url = referrer.join(specifier).map_err(|_| {
        (
            "invalid-specifier",
            "module specifier could not be parsed".into(),
        )
    })?;
    url.set_fragment(None);
    if origin_key(&url) != *expected_origin {
        return Err((
            "cross-origin-module",
            "cross-origin module URLs are not allowed".into(),
        ));
    }
    Ok(url)
}

async fn fetch_module(
    client: &Client,
    initial: &str,
    expected_origin: &(String, String, u16),
    policy: OutboundUrlPolicy,
    limits: &ModuleLimits,
    graph_deadline: tokio::time::Instant,
) -> Result<(String, String), (&'static str, String)> {
    let mut current = policy.validate_url(initial).await.map_err(|_| {
        (
            "url-policy",
            "module URL failed outbound security policy".into(),
        )
    })?;
    let per_fetch_deadline =
        tokio::time::Instant::now() + Duration::from_millis(limits.fetch_timeout_ms);
    let deadline = std::cmp::min(per_fetch_deadline, graph_deadline);

    for redirect_count in 0..=limits.max_redirects {
        if origin_key(&current) != *expected_origin {
            return Err((
                "cross-origin-module",
                "cross-origin module URLs are not allowed".into(),
            ));
        }
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or_else(|| {
                (
                    "module-fetch-timeout",
                    "module fetch deadline exceeded".into(),
                )
            })?;
        let response = tokio::time::timeout(
            remaining,
            client
                .get(current.clone())
                .header("Accept", "text/javascript, application/javascript")
                .header("Sec-Fetch-Dest", "script")
                .header("Sec-Fetch-Mode", "same-origin")
                .send(),
        )
        .await
        .map_err(|_| ("module-fetch-timeout", "module request timed out".into()))?
        .map_err(|_| ("module-fetch-failed", "module request failed".into()))?;

        if response.status().is_redirection() {
            if redirect_count == limits.max_redirects {
                return Err((
                    "redirect-limit-exceeded",
                    format!("module exceeded {} redirects", limits.max_redirects),
                ));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    (
                        "invalid-redirect",
                        "module redirect did not include a valid Location".into(),
                    )
                })?;
            let next = current.join(location).map_err(|_| {
                (
                    "invalid-redirect",
                    "module redirect Location could not be parsed".into(),
                )
            })?;
            if origin_key(&next) != *expected_origin {
                return Err((
                    "cross-origin-redirect",
                    "module redirect to a cross-origin URL was blocked".into(),
                ));
            }
            current = policy.validate_url(next.as_str()).await.map_err(|_| {
                (
                    "url-policy",
                    "redirect URL failed outbound security policy".into(),
                )
            })?;
            continue;
        }
        if !response.status().is_success() {
            return Err((
                "http-status",
                format!("module request returned HTTP {}", response.status()),
            ));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !is_javascript_content_type(content_type) {
            return Err((
                "invalid-module-mime",
                "module response MIME type is not JavaScript".into(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > limits.max_module_bytes as u64)
        {
            return Err((
                "module-bytes-exceeded",
                "module Content-Length exceeded the per-module byte limit".into(),
            ));
        }
        let encoding = response
            .headers()
            .get(CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mut wire = Vec::new();
        let mut stream = response.bytes_stream();
        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .ok_or_else(|| ("module-fetch-timeout", "module body timed out".into()))?;
            let Some(chunk) = tokio::time::timeout(remaining, stream.next())
                .await
                .map_err(|_| ("module-fetch-timeout", "module body timed out".into()))?
            else {
                break;
            };
            let chunk =
                chunk.map_err(|_| ("module-fetch-failed", "module response body failed".into()))?;
            if wire.len().saturating_add(chunk.len()) > limits.max_module_bytes {
                return Err((
                    "module-bytes-exceeded",
                    "module compressed body exceeded the per-module byte limit".into(),
                ));
            }
            wire.extend_from_slice(&chunk);
        }
        let decoded = crate::network::fetch::decode_limited_body_async(
            wire,
            encoding,
            limits.max_module_bytes,
        )
        .await
        .map_err(|_| {
            (
                "module-decode-failed",
                "module body decompression failed or exceeded the decoded byte limit".into(),
            )
        })?;
        let source = String::from_utf8(decoded).map_err(|_| {
            (
                "invalid-module-encoding",
                "module source is not UTF-8".into(),
            )
        })?;
        return Ok((current.to_string(), source));
    }
    Err((
        "redirect-limit-exceeded",
        "module redirect limit exceeded".into(),
    ))
}

fn is_javascript_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(is_javascript_mime_essence)
}

fn origin_key(url: &Url) -> (String, String, u16) {
    (
        url.scheme().to_string(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default().unwrap_or(0),
    )
}

fn diagnostic(url: &str, phase: &str, code: &str, message: &str) -> ModuleDiagnostic {
    let safe_url = Url::parse(url)
        .map(|mut parsed| {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        })
        .unwrap_or_default();
    ModuleDiagnostic {
        url: bounded(&safe_url, 512),
        phase: bounded(phase, 32),
        code: bounded(code, 64),
        message: bounded(message, 1024),
    }
}

fn bounded(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

struct InspectedRequest {
    specifier: String,
    has_attributes: bool,
}

async fn inspect_module_source(
    source: String,
    url: String,
    max_requests: usize,
) -> Result<Vec<InspectedRequest>, String> {
    tokio::task::spawn_blocking(move || inspect_module_source_blocking(&source, &url, max_requests))
        .await
        .map_err(|error| format!("module inspection worker failed: {error}"))?
}

fn inspect_module_source_blocking(
    source: &str,
    url: &str,
    max_requests: usize,
) -> Result<Vec<InspectedRequest>, String> {
    super::runtime::init_platform();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let source_text = v8::String::new(scope, source)
        .ok_or_else(|| "V8 could not allocate module source".to_string())?;
    let name = v8::String::new(scope, url)
        .ok_or_else(|| "V8 could not allocate module URL".to_string())?;
    let origin = v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        0,
        None,
        false,
        false,
        true,
        None,
    );
    let mut compiler_source = v8::script_compiler::Source::new(source_text, Some(&origin));
    let tc = &mut v8::TryCatch::new(scope);
    let module =
        v8::script_compiler::compile_module(tc, &mut compiler_source).ok_or_else(|| {
            tc.exception()
                .map(|exception| bounded(&exception.to_rust_string_lossy(tc), 1024))
                .unwrap_or_else(|| "V8 rejected the module source".into())
        })?;
    let requests = module.get_module_requests();
    let request_count = requests.length().min(max_requests);
    let mut result = Vec::with_capacity(request_count);
    for index in 0..request_count {
        let request = requests
            .get(tc, index)
            .and_then(|value| v8::Local::<v8::ModuleRequest>::try_from(value).ok())
            .ok_or_else(|| "V8 returned malformed module request metadata".to_string())?;
        result.push(InspectedRequest {
            specifier: request.get_specifier().to_rust_string_lossy(tc),
            has_attributes: request.get_import_attributes().length() != 0,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::extract::ScriptBlock;
    use super::*;

    #[test]
    fn v8_inspector_finds_only_real_static_imports() {
        let requests = inspect_module_source_blocking(
            r#"
            import value from './value.js';
            import './side.js';
            export { answer } from "/answer.js";
            const text = "import './not-real.js'";
            const escaped = "import \"./escaped-not-real.js\"";
            // import './also-not-real.js'
            /* export * from './not-real-either.js' */
            const regex = /import\s+['\"]\.\/regex-false-positive\.js/;
            import.meta.url;
            "#,
            "https://example.com/main.js",
            16,
        )
        .unwrap();
        assert_eq!(
            requests
                .into_iter()
                .map(|request| request.specifier)
                .collect::<Vec<_>>(),
            vec!["./value.js", "./side.js", "/answer.js"]
        );
    }

    #[test]
    fn resolver_rejects_bare_and_cross_origin_specifiers() {
        let base = Url::parse("https://example.com/path/main.js").unwrap();
        let origin = origin_key(&base);
        assert_eq!(
            resolve_specifier(&base, "pkg", &origin).unwrap_err().0,
            "unsupported-bare-specifier"
        );
        assert_eq!(
            resolve_specifier(&base, "https://other.example/x.js", &origin)
                .unwrap_err()
                .0,
            "cross-origin-module"
        );
        assert_eq!(
            resolve_specifier(&base, "../x.js", &origin)
                .unwrap()
                .as_str(),
            "https://example.com/x.js"
        );
    }

    #[test]
    fn javascript_module_content_type_uses_exact_whatwg_essence_list() {
        for essence in super::super::extract::JAVASCRIPT_MIME_TYPE_ESSENCES {
            assert!(
                is_javascript_content_type(essence),
                "JavaScript MIME essence was rejected: {essence}"
            );
            assert!(
                is_javascript_content_type(&essence.to_ascii_uppercase()),
                "ASCII-insensitive JavaScript MIME essence was rejected: {essence}"
            );
            assert!(
                is_javascript_content_type(&format!("{essence}; charset=utf-8")),
                "JavaScript Content-Type parameters were rejected: {essence}"
            );
        }
        for value in [
            "text/plain",
            "text/javascript1.6",
            "text/javascript+json",
            "application/json",
            "",
            "; charset=utf-8",
        ] {
            assert!(
                !is_javascript_content_type(value),
                "non-JavaScript Content-Type was accepted: {value}"
            );
        }
    }

    #[test]
    fn inline_budget_is_diagnosed() {
        let scripts = vec![ScriptBlock {
            source: "export const oversized = true;".into(),
            label: "inline-0".into(),
            is_inline: true,
            index: 0,
            kind: ScriptKind::Module,
        }];
        let limits = ModuleLimits {
            max_module_bytes: 4,
            ..Default::default()
        };
        let graph = inline_module_graph(&scripts, "https://example.com", &limits);
        assert!(graph
            .diagnostics
            .iter()
            .any(|item| item.code == "module-bytes-exceeded"));
    }

    #[test]
    fn module_roots_are_bounded_but_report_the_full_denominator() {
        let scripts = (0..3)
            .map(|index| ScriptBlock {
                source: format!("globalThis.root{index} = true;"),
                label: format!("inline-{index}"),
                is_inline: true,
                index,
                kind: ScriptKind::Module,
            })
            .collect::<Vec<_>>();
        let limits = ModuleLimits {
            max_modules: 2,
            ..Default::default()
        };
        let graph = inline_module_graph(&scripts, "https://example.com", &limits);
        assert_eq!(graph.root_count, 3);
        assert_eq!(graph.roots.len(), 2);
        assert_eq!(graph.sources.len(), 2);
        assert!(graph
            .diagnostics
            .iter()
            .any(|item| item.code == "module-count-exceeded"));

        let mut runtime = super::super::runtime::JsRuntime::new(Default::default());
        let execution = runtime.execute_module_graph(&graph);
        assert_eq!(execution.roots, 3);
        assert_eq!(execution.roots_evaluated, 2);
        assert_eq!(execution.roots_failed, 1);
    }

    #[test]
    fn diagnostics_are_utf8_byte_bounded_and_strip_sensitive_url_parts() {
        let item = diagnostic(
            "https://user:password@example.com/module.js?token=secret#fragment",
            "fetch",
            "failure",
            &"🦀".repeat(600),
        );
        assert_eq!(item.url, "https://example.com/module.js");
        assert!(item.message.len() <= 1024);
        assert!(std::str::from_utf8(item.message.as_bytes()).is_ok());

        let base = Url::parse("https://example.com/main.js").unwrap();
        let origin = origin_key(&base);
        let (code, message) = resolve_specifier(
            &base,
            "https://user:password@other.example/dep.js?token=secret#fragment",
            &origin,
        )
        .unwrap_err();
        let cross_origin = diagnostic(
            "https://user:password@other.example/dep.js?token=secret#fragment",
            "resolve",
            code,
            &message,
        );
        let malformed = diagnostic(
            "not a URL?token=secret#fragment",
            "fetch",
            "module-fetch-failed",
            "module request failed",
        );
        let serialized = serde_json::to_string(&[item, cross_origin, malformed]).unwrap();
        for secret in ["user", "password", "token=secret", "fragment"] {
            assert!(
                !serialized.contains(secret),
                "serialized module diagnostics leaked {secret}: {serialized}"
            );
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn graph_accepts_same_origin_redirect_canonicalizes_aliases_and_rejects_attacks() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            while let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(Duration::from_secs(2), listener.accept()).await
            {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut request = [0u8; 2048];
                let count = stream.read(&mut request).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..count]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let response = match path {
                    "/alias-a.js" | "/alias-b.js" => {
                        "HTTP/1.1 302 Found\r\nLocation: /final.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                    }
                    "/final.js" => {
                        let body = "globalThis.redirectModuleRuns = (globalThis.redirectModuleRuns || 0) + 1; export const ok = true;";
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
                    }
                    "/plain.js" => {
                        let body = "export const nope = true;";
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
                    }
                    "/cross.js" => format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/secret.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        address.port().saturating_add(1)
                    ),
                    _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        std::env::set_var("PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK", "1");
        let base = format!("http://{address}/page");
        let scripts = ["/alias-a.js", "/alias-b.js", "/plain.js", "/cross.js", ""]
            .iter()
            .enumerate()
            .map(|(index, src)| ScriptBlock {
                source: String::new(),
                label: (*src).into(),
                is_inline: false,
                index,
                kind: ScriptKind::Module,
            })
            .collect::<Vec<_>>();
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .gzip(false)
            .brotli(false)
            .deflate(false)
            .build()
            .unwrap();
        let graph =
            resolve_module_graph(&scripts, &base, &client, &ModuleLimits::default(), true).await;
        std::env::remove_var("PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK");
        server.abort();

        assert_eq!(graph.sources.len(), 1, "{:#?}", graph.diagnostics);
        let final_url = format!("http://{address}/final.js");
        assert_eq!(
            graph.aliases.get(&format!("http://{address}/alias-a.js")),
            Some(&final_url)
        );
        assert_eq!(
            graph.aliases.get(&format!("http://{address}/alias-b.js")),
            Some(&final_url)
        );
        assert!(graph
            .diagnostics
            .iter()
            .any(|item| item.code == "invalid-module-mime"));
        assert!(graph
            .diagnostics
            .iter()
            .any(|item| item.code == "cross-origin-redirect"));
        assert!(graph
            .diagnostics
            .iter()
            .any(|item| item.code == "empty-module-src"));
        let mut runtime = super::super::runtime::JsRuntime::new(Default::default());
        let execution = runtime.execute_module_graph(&graph);
        assert_eq!(execution.roots_evaluated, 2, "{:#?}", execution.diagnostics);
        assert_eq!(execution.roots_failed, 3, "{:#?}", execution.diagnostics);
        assert_eq!(runtime.eval("redirectModuleRuns").unwrap(), "1");
    }
}
