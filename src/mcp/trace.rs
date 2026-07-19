//! Privacy-safe, session-scoped MCP action traces and validation-only replay.
//!
//! Traces are intentionally not browser recordings. They contain bounded
//! action metadata and state fingerprints, never page bodies, cookies,
//! credentials, JavaScript source/results, screenshots, or tool output.

use std::collections::VecDeque;
use std::time::Duration;

use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use url::Url;
use uuid::Uuid;

use crate::som::types::{Element, Som};

pub const TRACE_SCHEMA: &str = "plasmate.trace.v1";
pub const MAX_TRACE_EVENTS: usize = 128;
pub const MAX_TRACE_BYTES: usize = 48 * 1024;
pub const MAX_TRACE_EVENT_BYTES: usize = 4 * 1024;
pub const MAX_TRACE_STRING_BYTES: usize = 512;
pub const MAX_TRACE_EXPORT_BYTES: usize = 64 * 1024;

const TRACEABLE_ACTIONS: &[&str] = &[
    "open_page",
    "navigate_to",
    "click",
    "type_text",
    "select_option",
    "scroll",
    "toggle",
    "clear",
    "close_page",
    "evaluate",
    "set_cookies",
    "clear_cookies",
];

#[derive(Debug, Clone, Serialize)]
pub struct TracePageState {
    pub url_fingerprint: Option<String>,
    pub origin: Option<String>,
    pub fingerprint: String,
    pub has_page: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayTarget {
    pub provenance: &'static str,
    pub target_id: String,
    pub stable_key: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceRedaction {
    pub policy: &'static str,
    pub value_handling: &'static str,
    pub redacted_fields: Vec<&'static str>,
    pub omitted_fields: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceEvent {
    pub schema: &'static str,
    pub sequence: u64,
    pub action: String,
    pub parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<ReplayTarget>,
    pub before: TracePageState,
    pub after: TracePageState,
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<&'static str>,
    pub duration_ms: u64,
    pub redaction: TraceRedaction,
}

#[derive(Debug, Clone)]
pub struct PendingTraceEvent {
    pub action: String,
    pub parameters: Value,
    pub target: Option<ReplayTarget>,
    pub before: TracePageState,
    pub after: TracePageState,
}

#[derive(Debug, Clone)]
pub struct TraceLog {
    enabled: bool,
    trace_id: String,
    fingerprint_key: [u8; 32],
    next_sequence: u64,
    events: VecDeque<TraceEvent>,
    retained_bytes: usize,
    evicted_events: u64,
    oversized_events: u64,
    cleared_events: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceStatus {
    pub schema: &'static str,
    pub enabled: bool,
    pub trace_id: String,
    pub retained_events: usize,
    pub retained_bytes: usize,
    pub max_events: usize,
    pub max_bytes: usize,
    pub next_sequence: u64,
    pub evicted_events: u64,
    pub oversized_events: u64,
    pub cleared_events: u64,
    pub retention: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceExport {
    pub schema: &'static str,
    pub trace_id: String,
    pub session_id: String,
    pub retained_events: usize,
    pub retained_bytes: usize,
    pub next_sequence: u64,
    pub evicted_events: u64,
    pub oversized_events: u64,
    pub cleared_events: u64,
    pub redaction_policy: &'static str,
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone)]
pub struct TraceAttempt {
    pub session_id: String,
    pub action: String,
    pub parameters: Value,
    pub target: Option<ReplayTarget>,
    pub before: TracePageState,
    pub detached_log: TraceLog,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayRequest {
    pub session_id: String,
    pub trace_id: String,
    pub sequence: u64,
    #[serde(default)]
    pub confirmed: bool,
}

impl Default for TraceLog {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceLog {
    pub fn new() -> Self {
        let mut fingerprint_key = [0_u8; 32];
        OsRng.fill_bytes(&mut fingerprint_key);
        Self {
            enabled: false,
            trace_id: Uuid::new_v4().to_string(),
            fingerprint_key,
            next_sequence: 1,
            events: VecDeque::new(),
            retained_bytes: 0,
            evicted_events: 0,
            oversized_events: 0,
            cleared_events: 0,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    pub fn page_state(&self, url: Option<&str>, som: Option<&Som>) -> TracePageState {
        page_state(&self.fingerprint_key, url, som)
    }

    pub fn closed_page_state(&self) -> TracePageState {
        closed_page_state(&self.fingerprint_key)
    }

    pub fn replay_target(
        &self,
        som: Option<&Som>,
        element_id: Option<&str>,
    ) -> Option<ReplayTarget> {
        replay_target(&self.fingerprint_key, som, element_id)
    }

    pub fn sanitized_parameters(&self, action: &str, arguments: &Value) -> Value {
        sanitized_parameters(&self.fingerprint_key, action, arguments)
    }

    pub fn status(&self) -> TraceStatus {
        TraceStatus {
            schema: TRACE_SCHEMA,
            enabled: self.enabled,
            trace_id: self.trace_id.clone(),
            retained_events: self.events.len(),
            retained_bytes: self.retained_bytes,
            max_events: MAX_TRACE_EVENTS,
            max_bytes: MAX_TRACE_BYTES,
            next_sequence: self.next_sequence,
            evicted_events: self.evicted_events,
            oversized_events: self.oversized_events,
            cleared_events: self.cleared_events,
            retention: "memory-only; removed when the browser session closes",
        }
    }

    pub fn export(&self, session_id: &str) -> TraceExport {
        TraceExport {
            schema: TRACE_SCHEMA,
            trace_id: self.trace_id.clone(),
            session_id: session_id.to_string(),
            retained_events: self.events.len(),
            retained_bytes: self.retained_bytes,
            next_sequence: self.next_sequence,
            evicted_events: self.evicted_events,
            oversized_events: self.oversized_events,
            cleared_events: self.cleared_events,
            redaction_policy: "secret values are omitted; page bodies, cookies, headers, JavaScript, screenshots, and tool output are omitted",
            events: self.events.iter().cloned().collect(),
        }
    }

    pub fn clear(&mut self) -> u64 {
        let cleared = self.events.len() as u64;
        self.cleared_events = self.cleared_events.saturating_add(cleared);
        self.events.clear();
        self.retained_bytes = 0;
        cleared
    }

    pub fn event(&self, sequence: u64) -> Option<&TraceEvent> {
        self.events.iter().find(|event| event.sequence == sequence)
    }

    pub fn append(&mut self, pending: PendingTraceEvent, result: &Value, duration: Duration) {
        if !self.enabled || !is_traceable_action(&pending.action) {
            return;
        }

        let (outcome, error_class) = classify_result(result);
        let redaction = redaction_for_action(&pending.action);
        let event = TraceEvent {
            schema: TRACE_SCHEMA,
            sequence: self.next_sequence,
            action: pending.action,
            parameters: pending.parameters,
            target: pending.target,
            before: pending.before,
            after: pending.after,
            outcome,
            error_class,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            redaction,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);

        let event_bytes = serialized_len(&event);
        if event_bytes > MAX_TRACE_EVENT_BYTES || event_bytes > MAX_TRACE_BYTES {
            self.oversized_events = self.oversized_events.saturating_add(1);
            return;
        }
        while self.events.len() >= MAX_TRACE_EVENTS
            || self.retained_bytes.saturating_add(event_bytes) > MAX_TRACE_BYTES
        {
            let Some(evicted) = self.events.pop_front() else {
                break;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(serialized_len(&evicted));
            self.evicted_events = self.evicted_events.saturating_add(1);
        }
        self.events.push_back(event);
        self.retained_bytes = self.retained_bytes.saturating_add(event_bytes);
    }
}

pub fn is_traceable_action(action: &str) -> bool {
    TRACEABLE_ACTIONS.contains(&action)
}

pub fn session_id_from_arguments(arguments: &Value) -> Option<&str> {
    arguments.get("session_id").and_then(Value::as_str)
}

pub fn open_trace_requested(action: &str, arguments: &Value) -> bool {
    action == "open_page"
        && arguments
            .get("trace")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

pub fn session_id_from_tool_result(result: &Value) -> Option<String> {
    let text = result
        .get("content")?
        .as_array()?
        .first()?
        .get("text")?
        .as_str()?;
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("session_id")?
        .as_str()
        .map(str::to_string)
}

pub fn attach_final_trace_export(result: &mut Value, export: TraceExport) {
    let Some(text) = result
        .get_mut("content")
        .and_then(Value::as_array_mut)
        .and_then(|content| content.first_mut())
        .and_then(|item| item.get_mut("text"))
    else {
        return;
    };
    let Some(raw) = text.as_str() else {
        return;
    };
    let Ok(mut payload) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    payload["final_trace"] = if serialized_len(&export) <= MAX_TRACE_EXPORT_BYTES {
        json!(export)
    } else {
        json!({
            "schema": TRACE_SCHEMA,
            "omitted": true,
            "reason": "bounded final trace export exceeded the MCP response ceiling"
        })
    };
    *text = Value::String(payload.to_string());
}

fn page_state(key: &[u8; 32], url: Option<&str>, som: Option<&Som>) -> TracePageState {
    let origin = url.and_then(url_origin);
    let url_fingerprint = url.map(|value| {
        format!(
            "plasmate-url:v1:{}",
            keyed_fingerprint(key, b"url", value.as_bytes())
        )
    });
    let som_bytes = som
        .and_then(|som| serde_json::to_vec(som).ok())
        .unwrap_or_default();
    TracePageState {
        url_fingerprint,
        origin,
        fingerprint: format!(
            "plasmate-state:v1:{}",
            keyed_fingerprint(key, b"som", &som_bytes)
        ),
        has_page: som.is_some(),
    }
}

fn closed_page_state(key: &[u8; 32]) -> TracePageState {
    TracePageState {
        url_fingerprint: None,
        origin: None,
        fingerprint: format!(
            "plasmate-state:v1:{}",
            keyed_fingerprint(key, b"som", b"closed")
        ),
        has_page: false,
    }
}

fn sanitized_parameters(key: &[u8; 32], action: &str, arguments: &Value) -> Value {
    match action {
        "open_page" | "navigate_to" => json!({
            "origin": arguments.get("url").and_then(Value::as_str).and_then(url_origin),
            "url_fingerprint": arguments.get("url").and_then(Value::as_str).map(|url| {
                format!("plasmate-url:v1:{}", keyed_fingerprint(key, b"url", url.as_bytes()))
            }),
        }),
        "type_text" => json!({
            "append": arguments.get("append").and_then(Value::as_bool).unwrap_or(false),
            "value": secret_summary(arguments.get("text").and_then(Value::as_str)),
        }),
        "select_option" => json!({
            "value": secret_summary(arguments.get("value").and_then(Value::as_str)),
        }),
        "evaluate" => json!({}),
        "set_cookies" => json!({
            "cookie_count": arguments
                .get("cookies")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
        }),
        "clear_cookies" => json!({
            "filter_present": arguments.get("name").is_some()
                || arguments.get("domain").is_some()
                || arguments.get("url").is_some(),
        }),
        "scroll" => json!({
            "direction": bounded_string(arguments.get("direction").and_then(Value::as_str).unwrap_or("down")),
            "pixels": arguments.get("pixels").and_then(Value::as_i64).unwrap_or(300),
            "targeted": arguments.get("element_id").is_some(),
        }),
        _ => json!({}),
    }
}

fn replay_target(
    key: &[u8; 32],
    som: Option<&Som>,
    element_id: Option<&str>,
) -> Option<ReplayTarget> {
    let element = find_element(som?, element_id?)?;
    Some(target_for_element(key, element))
}

pub fn validate_replay(
    log: &TraceLog,
    session_id: &str,
    request: &ReplayRequest,
    current: TracePageState,
    som: Option<&Som>,
) -> Value {
    if request.trace_id != log.trace_id() {
        return replay_refusal(
            request,
            "cross_session",
            "trace_id does not belong to this session",
        );
    }
    let Some(event) = log.event(request.sequence) else {
        return replay_refusal(
            request,
            "event_not_retained",
            "sequence is absent, evicted, or was cleared",
        );
    };
    if matches!(
        event.action.as_str(),
        "open_page" | "close_page" | "evaluate" | "set_cookies" | "clear_cookies"
    ) {
        return replay_refusal(
            request,
            "unsupported_action",
            "session lifecycle actions are not replay candidates",
        );
    }
    if current.origin != event.before.origin {
        return replay_refusal(
            request,
            "origin_drift",
            "current origin differs from the recorded pre-action origin",
        );
    }
    if current.url_fingerprint != event.before.url_fingerprint {
        return replay_refusal(
            request,
            "page_drift",
            "current keyed page URL fingerprint differs from the recorded pre-action URL",
        );
    }
    let resolved_target = if let Some(recorded) = &event.target {
        let matches = matching_targets(&log.fingerprint_key, som, recorded);
        if matches.is_empty() {
            return replay_refusal(
                request,
                "target_missing",
                "recorded target is not present in the current session state",
            );
        }
        if matches.len() > 1 {
            return replay_refusal(
                request,
                "target_ambiguous",
                "recorded target resolves to more than one current element",
            );
        }
        let element = matches[0];
        if !element_supports_action(element, &event.action) {
            return replay_refusal(
                request,
                "action_unavailable",
                "current target does not expose the recorded action",
            );
        }
        Some(target_for_element(&log.fingerprint_key, element))
    } else {
        None
    };
    if current.fingerprint != event.before.fingerprint {
        return replay_refusal(
            request,
            "state_drift",
            "current semantic state differs from the recorded pre-action state",
        );
    }

    if !request.confirmed {
        return json!({
            "schema": TRACE_SCHEMA,
            "status": "confirmation_required",
            "drift": null,
            "session_id": session_id,
            "trace_id": request.trace_id,
            "sequence": request.sequence,
            "action": event.action,
            "target": resolved_target,
            "side_effects": false,
            "execution": "validation_only",
            "execution_available": false,
            "message": "Exact validation passed; explicit confirmation is still required for this mutating action",
        });
    }

    json!({
        "schema": TRACE_SCHEMA,
        "status": "validated",
        "drift": null,
        "session_id": session_id,
        "trace_id": request.trace_id,
        "sequence": request.sequence,
        "action": event.action,
        "parameters": event.parameters,
        "target": resolved_target,
        "side_effects": false,
        "execution": "validation_only",
        "execution_available": false,
        "message": "Validation and confirmation contract passed; this slice returns a plan and does not execute it",
    })
}

fn replay_refusal(request: &ReplayRequest, drift: &str, message: &str) -> Value {
    json!({
        "schema": TRACE_SCHEMA,
        "status": "refused",
        "drift": drift,
        "trace_id": request.trace_id,
        "sequence": request.sequence,
        "side_effects": false,
        "execution": "validation_only",
        "execution_available": false,
        "message": message,
    })
}

fn target_for_element(key: &[u8; 32], element: &Element) -> ReplayTarget {
    ReplayTarget {
        provenance: "session-owned-som",
        target_id: format!(
            "plasmate-trace-id:v1:{}",
            keyed_fingerprint(key, b"target-id", element.id.as_bytes())
        ),
        stable_key: element_stable_key(key, element),
        role: element.role.as_str().to_string(),
    }
}

fn element_stable_key(key: &[u8; 32], element: &Element) -> String {
    let mut actions = element.actions.clone().unwrap_or_default();
    actions.sort();
    let action_value = if actions.is_empty() {
        None
    } else {
        Some(actions.join(","))
    };
    let attrs = element.attrs.as_ref();
    let parts = vec![
        Some(element.id.clone()),
        Some(element.role.as_str().to_string()),
        element.label.clone(),
        action_value,
        attr_string(attrs, "name").map(str::to_string),
        attr_string(attrs, "href").map(str::to_string),
        attr_string(attrs, "input_type").map(str::to_string),
        attr_string(attrs, "group").map(str::to_string),
        attr_string(attrs, "placeholder").map(str::to_string),
    ];
    let encoded = serde_json::to_vec(&parts).unwrap_or_else(|_| b"[]".to_vec());
    format!(
        "plasmate-trace-target:v1:{}",
        keyed_fingerprint(key, b"target", &encoded)
    )
}

fn matching_targets<'a>(
    key: &[u8; 32],
    som: Option<&'a Som>,
    target: &ReplayTarget,
) -> Vec<&'a Element> {
    let mut all = Vec::new();
    if let Some(som) = som {
        for region in &som.regions {
            collect_matching_targets(key, &region.elements, target, &mut all);
        }
    }
    all
}

fn collect_matching_targets<'a>(
    key: &[u8; 32],
    elements: &'a [Element],
    target: &ReplayTarget,
    output: &mut Vec<&'a Element>,
) {
    for element in elements {
        if element_stable_key(key, element) == target.stable_key
            && element.role.as_str() == target.role
        {
            output.push(element);
        }
        if let Some(children) = &element.children {
            collect_matching_targets(key, children, target, output);
        }
        if let Some(shadow) = &element.shadow {
            collect_matching_targets(key, &shadow.elements, target, output);
        }
    }
}

fn find_element<'a>(som: &'a Som, id: &str) -> Option<&'a Element> {
    for region in &som.regions {
        if let Some(element) = find_element_in(&region.elements, id) {
            return Some(element);
        }
    }
    None
}

fn find_element_in<'a>(elements: &'a [Element], id: &str) -> Option<&'a Element> {
    for element in elements {
        if element.id == id {
            return Some(element);
        }
        if let Some(found) = element
            .children
            .as_deref()
            .and_then(|children| find_element_in(children, id))
        {
            return Some(found);
        }
        if let Some(found) = element
            .shadow
            .as_ref()
            .and_then(|shadow| find_element_in(&shadow.elements, id))
        {
            return Some(found);
        }
    }
    None
}

fn element_supports_action(element: &Element, tool: &str) -> bool {
    let expected = match tool {
        "click" => Some("click"),
        "type_text" => Some("type"),
        "select_option" => Some("select"),
        "toggle" => Some("toggle"),
        "clear" => Some("clear"),
        "scroll" => return true,
        "navigate_to" => return true,
        _ => None,
    };
    expected.is_some_and(|expected| {
        element
            .actions
            .as_ref()
            .is_some_and(|actions| actions.iter().any(|action| action == expected))
    })
}

fn url_origin(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    match parsed.origin() {
        url::Origin::Tuple(scheme, host, port) => Some(format!("{scheme}://{host}:{port}")),
        url::Origin::Opaque(_) => None,
    }
}

fn secret_summary(value: Option<&str>) -> Value {
    match value {
        Some(value) => json!({
            "redacted": true,
            "byte_length": value.len(),
        }),
        None => Value::Null,
    }
}

fn redaction_for_action(action: &str) -> TraceRedaction {
    let redacted_fields = match action {
        "type_text" => vec!["text"],
        "select_option" => vec!["value"],
        "evaluate" => vec!["expression", "result"],
        "set_cookies" => vec!["cookies"],
        "clear_cookies" => vec!["name", "domain", "url"],
        _ => Vec::new(),
    };
    TraceRedaction {
        policy: TRACE_SCHEMA,
        value_handling: "omitted+byte_length",
        redacted_fields,
        omitted_fields: vec![
            "cookies",
            "authorization",
            "headers",
            "raw_html",
            "screenshots",
            "evaluate_source",
            "evaluate_result",
            "tool_output",
        ],
    }
}

fn classify_result(result: &Value) -> (&'static str, Option<&'static str>) {
    if !result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return ("success", None);
    }
    let message = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let class = if message.contains("not found") {
        "not_found"
    } else if message.contains("invalid argument") {
        "invalid_input"
    } else if message.contains("policy") || message.contains("blocked") {
        "policy"
    } else if message.contains("timeout") {
        "timeout"
    } else if message.contains("javascript") || message.contains("execution") {
        "execution"
    } else if message.contains("fetch") || message.contains("navigation") {
        "network"
    } else {
        "operation"
    };
    ("error", Some(class))
}

fn bounded_string(value: &str) -> String {
    if value.len() <= MAX_TRACE_STRING_BYTES {
        return value.to_string();
    }
    let mut end = MAX_TRACE_STRING_BYTES;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_string()
}

fn attr_string<'a>(attrs: Option<&'a Value>, key: &str) -> Option<&'a str> {
    attrs
        .and_then(|attrs| attrs.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn serialized_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn keyed_fingerprint(key: &[u8; 32], domain: &[u8], value: &[u8]) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else {
        return String::new();
    };
    mac.update(b"plasmate.trace.v1\0");
    mac.update(&(domain.len() as u64).to_be_bytes());
    mac.update(domain);
    mac.update(&(value.len() as u64).to_be_bytes());
    mac.update(value);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::som::types::{ElementRole, Region, RegionRole, SomMeta};
    use sha2::Digest;

    fn sample_som() -> Som {
        Som {
            som_version: "1.0".to_string(),
            url: "https://example.com/form".to_string(),
            title: "Form".to_string(),
            lang: "en".to_string(),
            regions: vec![Region {
                id: "r1".to_string(),
                role: RegionRole::Main,
                label: None,
                action: None,
                method: None,
                target: None,
                enctype: None,
                novalidate: None,
                accept_charset: None,
                autocomplete: None,
                elements: vec![Element {
                    id: "e1".to_string(),
                    role: ElementRole::Button,
                    html_id: Some("submit".to_string()),
                    text: Some("Submit".to_string()),
                    label: Some("Submit".to_string()),
                    actions: Some(vec!["click".to_string()]),
                    attrs: Some(json!({"test_id":"submit-button"})),
                    children: None,
                    hints: None,
                    shadow: None,
                }],
            }],
            meta: SomMeta {
                html_bytes: 100,
                som_bytes: 80,
                element_count: 1,
                interactive_count: 1,
            },
            structured_data: None,
        }
    }

    #[test]
    fn typed_and_selected_secrets_never_appear_in_serialized_parameters() {
        let log = TraceLog::new();
        let typed = log.sanitized_parameters(
            "type_text",
            &json!({"session_id":"s", "element_id":"e1", "text":"hunter2"}),
        );
        let selected = log.sanitized_parameters(
            "select_option",
            &json!({"session_id":"s", "element_id":"e2", "value":"private-plan"}),
        );
        let serialized = format!("{}{}", typed, selected);
        assert!(!serialized.contains("hunter2"));
        assert!(!serialized.contains("private-plan"));
        assert!(!serialized.contains("sha256"));
        assert_eq!(typed["value"]["byte_length"], 7);
        assert_eq!(selected["value"]["byte_length"], 12);
    }

    #[test]
    fn javascript_and_cookie_mutations_retain_no_raw_inputs() {
        let log = TraceLog::new();
        let evaluate = log.sanitized_parameters(
            "evaluate",
            &json!({"session_id":"s", "expression":"document.cookie='secret'"}),
        );
        let set = log.sanitized_parameters(
            "set_cookies",
            &json!({"session_id":"s", "cookies":[{
                "name":"auth_token", "value":"top-secret", "domain":"private.example"
            }]}),
        );
        let clear = log.sanitized_parameters(
            "clear_cookies",
            &json!({"session_id":"s", "name":"auth_token", "domain":"private.example"}),
        );
        let serialized = format!("{evaluate}{set}{clear}");
        assert!(!serialized.contains("document.cookie"));
        assert!(!serialized.contains("auth_token"));
        assert!(!serialized.contains("top-secret"));
        assert!(!serialized.contains("private.example"));
        assert_eq!(set["cookie_count"], 1);
        assert_eq!(clear["filter_present"], true);
    }

    #[test]
    fn url_paths_and_page_content_use_non_exported_session_key() {
        let mut log = TraceLog::new();
        log.set_enabled(true);
        let other_log = TraceLog::new();
        let raw_url = "https://user:secret@example.com/private/alice?token=abc#private";
        let params = log.sanitized_parameters("navigate_to", &json!({"url": raw_url}));
        let som = sample_som();
        let state = log.page_state(Some(raw_url), Some(&som));
        let target = log.replay_target(Some(&som), Some("e1")).unwrap();
        log.append(
            PendingTraceEvent {
                action: "navigate_to".to_string(),
                parameters: params,
                target: None,
                before: state.clone(),
                after: state.clone(),
            },
            &json!({"content":[]}),
            Duration::ZERO,
        );
        log.append(
            PendingTraceEvent {
                action: "click".to_string(),
                parameters: json!({}),
                target: Some(target),
                before: state.clone(),
                after: state.clone(),
            },
            &json!({"content":[]}),
            Duration::ZERO,
        );
        let serialized = serde_json::to_string(&log.export("session-a")).unwrap();
        let exported_key = hex::encode(log.fingerprint_key);
        assert!(!serialized.contains("/private/alice"));
        assert!(!serialized.contains("token=abc"));
        assert!(!serialized.contains("Submit"));
        assert!(!serialized.contains("submit-button"));
        assert!(!serialized.contains("\"e1\""));
        let plain_label_digest = hex::encode(Sha256::digest(b"Submit"));
        assert!(!serialized.contains(&plain_label_digest));
        assert!(!serialized.contains(&exported_key));
        assert_ne!(
            state.fingerprint,
            other_log.page_state(Some(raw_url), Some(&som)).fingerprint
        );
    }

    #[test]
    fn eviction_is_deterministic_and_sequence_never_rewinds() {
        let mut log = TraceLog::new();
        log.set_enabled(true);
        let state = log.closed_page_state();
        for _ in 0..(MAX_TRACE_EVENTS + 3) {
            log.append(
                PendingTraceEvent {
                    action: "scroll".to_string(),
                    parameters: json!({"direction":"down", "pixels":300}),
                    target: None,
                    before: state.clone(),
                    after: state.clone(),
                },
                &json!({"content":[]}),
                Duration::from_millis(1),
            );
        }
        let total = MAX_TRACE_EVENTS + 3;
        let retained = log.events.len();
        assert!(retained <= MAX_TRACE_EVENTS);
        assert_eq!(
            log.events.front().unwrap().sequence,
            (total - retained + 1) as u64
        );
        assert_eq!(log.status().evicted_events, (total - retained) as u64);
        assert_eq!(log.status().next_sequence, (MAX_TRACE_EVENTS + 4) as u64);
        assert!(serialized_len(&log.export("session-a")) <= MAX_TRACE_EXPORT_BYTES);
    }

    #[test]
    fn repeated_exports_are_byte_identical() {
        let log = TraceLog::new();
        let first = serde_json::to_vec(&log.export("session-a")).unwrap();
        let second = serde_json::to_vec(&log.export("session-a")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn failed_actions_store_only_a_coarse_error_class() {
        let som = sample_som();
        let mut log = TraceLog::new();
        log.set_enabled(true);
        let state = log.page_state(Some("https://example.com/form"), Some(&som));
        let target = log.replay_target(Some(&som), Some("e1"));
        log.append(
            PendingTraceEvent {
                action: "click".to_string(),
                parameters: json!({}),
                target,
                before: state.clone(),
                after: state,
            },
            &json!({
                "isError": true,
                "content": [{"type":"text", "text":"Navigation failed: bearer top-secret"}]
            }),
            Duration::from_millis(4),
        );
        let encoded = serde_json::to_string(&log.export("session-a")).unwrap();
        assert!(!encoded.contains("top-secret"));
        assert!(encoded.contains("network"));
        assert!(encoded.contains("error"));
    }

    #[test]
    fn replay_is_session_bound_detects_drift_and_never_executes() {
        let som = sample_som();
        let mut log = TraceLog::new();
        log.set_enabled(true);
        let state = log.page_state(Some("https://example.com/form"), Some(&som));
        let target = log.replay_target(Some(&som), Some("e1"));
        log.append(
            PendingTraceEvent {
                action: "click".to_string(),
                parameters: json!({}),
                target,
                before: state.clone(),
                after: state.clone(),
            },
            &json!({"content": []}),
            Duration::from_millis(2),
        );

        let wrong_trace = ReplayRequest {
            session_id: "session-b".to_string(),
            trace_id: "another-trace".to_string(),
            sequence: 1,
            confirmed: true,
        };
        assert_eq!(
            validate_replay(&log, "session-b", &wrong_trace, state.clone(), Some(&som))["drift"],
            "cross_session"
        );

        let request = ReplayRequest {
            session_id: "session-a".to_string(),
            trace_id: log.trace_id().to_string(),
            sequence: 1,
            confirmed: false,
        };
        let plan = validate_replay(&log, "session-a", &request, state.clone(), Some(&som));
        assert_eq!(plan["status"], "confirmation_required");
        assert_eq!(plan["side_effects"], false);
        assert_eq!(plan["execution_available"], false);

        let mut changed = som.clone();
        changed.title = "Changed".to_string();
        let stale = validate_replay(
            &log,
            "session-a",
            &request,
            log.page_state(Some("https://example.com/form"), Some(&changed)),
            Some(&changed),
        );
        assert_eq!(stale["drift"], "state_drift");

        let mut missing = som.clone();
        missing.regions[0].elements.clear();
        let missing_plan = validate_replay(
            &log,
            "session-a",
            &request,
            log.page_state(Some("https://example.com/form"), Some(&missing)),
            Some(&missing),
        );
        assert_eq!(missing_plan["drift"], "target_missing");

        let mut confirmed = request;
        confirmed.confirmed = true;
        let validated = validate_replay(&log, "session-a", &confirmed, state, Some(&som));
        assert_eq!(validated["status"], "validated");
        assert_eq!(validated["execution"], "validation_only");
        assert_eq!(validated["side_effects"], false);
    }

    #[test]
    fn clear_preserves_monotonic_sequence() {
        let mut log = TraceLog::new();
        log.set_enabled(true);
        let state = log.closed_page_state();
        log.append(
            PendingTraceEvent {
                action: "scroll".to_string(),
                parameters: json!({}),
                target: None,
                before: state.clone(),
                after: state.clone(),
            },
            &json!({"content":[]}),
            Duration::ZERO,
        );
        assert_eq!(log.clear(), 1);
        log.append(
            PendingTraceEvent {
                action: "scroll".to_string(),
                parameters: json!({}),
                target: None,
                before: state.clone(),
                after: state,
            },
            &json!({"content":[]}),
            Duration::ZERO,
        );
        assert_eq!(log.events.front().unwrap().sequence, 2);
        assert_eq!(log.status().cleared_events, 1);
    }
}
