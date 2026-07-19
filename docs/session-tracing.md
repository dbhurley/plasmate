# Session tracing and replay validation

Plasmate exposes an opt-in, memory-only `plasmate.trace.v1` action trace for
stateful MCP browser sessions. It is a debugging and validation surface, not a
video recording and not a macro player.

Enable tracing when the session is created:

```json
{"name":"open_page","arguments":{"url":"https://example.com","trace":true}}
```

Tracing is disabled by default. It cannot be enabled retroactively because
that could give a caller an incomplete history while implying otherwise.
Use `trace_status` to read the session-bound `trace_id` and retention counters,
`trace_export` to obtain retained events, and `trace_clear` to discard events
without resetting the monotonic sequence.

## Event contract

Each event contains:

- a session-scoped monotonic sequence and action kind;
- sanitized parameters and explicit redaction metadata;
- a target identity derived from the current session-owned SOM, including the
  domain-separated HMAC-SHA-256 replay fingerprint and its provenance;
- before/after origins plus keyed, opaque URL and semantic-state fingerprints;
- success/error outcome, coarse error class, and duration.

The recorder covers `open_page`, `navigate_to`, `click`, `type_text`,
`select_option`, `scroll`, `toggle`, `clear`, and `close_page`, including
failures for actions that resolve to a live traced session. A successful
`close_page` response carries `final_trace` because the owning in-memory
session and trace are destroyed by that action.

The state-mutating `evaluate`, `set_cookies`, and `clear_cookies` tools are
also traced, but only as coarse action/outcome events. Their source, results,
cookie names, values, domains, paths, and filters are omitted.

Typed text and selected values are never retained. Their event parameters
contain only a redaction marker and byte length; no reusable secret-derived
digest is exported. URL paths, queries, fragments, and user information are
replaced by a per-session keyed fingerprint; only the origin remains readable.
Target IDs and state fingerprints use the same random, non-exported per-session
key with separate HMAC-SHA-256 domains; raw SOM element IDs are not exported.
Traces never contain cookies, authorization
headers or tokens, arbitrary headers, raw/effective HTML, screenshots,
JavaScript evaluate source/results, or full MCP tool output. `evaluate`,
`set_cookies`, and `clear_cookies` produce coarse mutation events with no raw
input; `get_cookies` remains excluded because it is read-only.

## Hard bounds and retention

- 128 retained events per session;
- 48 KiB aggregate serialized event bytes;
- 4 KiB per event;
- 512 bytes per retained string;
- deterministic oldest-first eviction;
- session-lifetime, process-memory-only retention.

Status and export envelopes report eviction, oversize-drop, and manual-clear
counters. Closing the session removes the trace. There is no disk writer,
remote collector, or safe-capture override.

## Validation before replay

`replay_validate` accepts the current `session_id`, originating `trace_id`, and
event `sequence`. It refuses cross-session trace IDs, missing/evicted events,
page or origin drift, semantic-state drift, missing or ambiguous targets, and
targets that no longer expose the recorded action. Exact validation still
returns `confirmation_required` until `confirmed=true` is supplied.

This release is intentionally **validation-only**. Even after exact validation
and explicit confirmation, it returns a side-effect-free plan with
`execution_available=false`; it never invokes the recorded action. Execution
must wait for a separately reviewed dispatcher that can atomically bind
validation to a supervised, session-serialized mutation. Lifecycle actions
(`open_page` and `close_page`) are not replay candidates.

Known boundary: a failed `open_page` has no surviving session in which to
retain its trace. Invalid calls that do not resolve to a live session likewise
cannot be assigned to a session-scoped trace.
