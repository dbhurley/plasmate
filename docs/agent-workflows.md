# Supervised agent workflows

`plasmate agent-run` executes a bounded, versioned sequence of stateful MCP
browser tools. It is intentionally narrower than a general command runner: the
child is always the current Plasmate executable in MCP stdio mode, no shell or
program override exists, and every plan receives complete static and policy
validation before that child is spawned. After lifecycle negotiation, every
argument is checked against the child's advertised schema before the first tool
call, so drift cannot cause an earlier mutation.

Schema validation is fail-closed for unsupported assertion keywords and enforces
the object, scalar, numeric, string, and array constraints used by Plasmate's
MCP definitions. `format` is treated only as a JSON Schema annotation; it does
not add validation semantics.

```bash
plasmate agent-run \
  --plan examples/agent-workflow.json \
  --report agent-workflow-report.json \
  --dry-run
```

Remove `--dry-run` to execute. Add `--allow-evaluate` or
`--allow-cookie-writes` only for a plan that needs those sensitive tools. Each
page-mutating step must be approved outside the plan with a separate
`--confirm-step STEP_ID`; an agent-authored plan cannot approve itself. Unknown
and duplicate approvals fail validation, and one approved ID never authorizes
another. Category opt-ins are additional gates, not substitutes for approval.

## Plan contract

Plans use schema `plasmate.agent-workflow.v1`, contain 1–64 steps, begin with
exactly one `open_page`, and may use only the stateful browser allowlist. The
runner owns `session_id`, enables the requested bounded trace on `open_page`,
checks MCP's advertised tool list, and closes the session on every reachable
exit. Step and whole-workflow deadlines, line/output limits, stderr drainage,
fail-fast behavior, and an optional Linux address-space ceiling are explicit in
`limits`.

Secrets are accepted only as exact environment references:

```json
{"$secret":"PLASMATE_TEST_TOKEN"}
```

Resolved values are sent only to the child. Reports contain neither arguments,
tool payloads, stderr, secret names, nor values. Secret names are normalized to
a constant before the plan fingerprint is computed, and reports expose only
aggregate referenced/present/missing counts. `get_cookies` and trace export are
therefore usable without leaking their returned data into the report.

Expectations can assert the MCP tool error bit and optionally compare one JSON
Pointer in the first text result. Reports use
`plasmate.agent-workflow-report.v1` and always account for every declared step
as succeeded, failed, or skipped.

## Failure containment

The MCP child receives a dedicated process group, an empty inherited environment
with only `PATH`, temp-directory variables, TLS certificate paths,
`PLASMATE_ICU_DATA`, `RUST_BACKTRACE`, and Windows system-root variables
selectively restored, and bounded stdin/stdout/stderr
pipe handling. The stdout event queue holds only one response plus a terminal
event, so a multi-line flood receives OS-pipe backpressure instead of consuming
unbounded parent memory. Stdin is owned by a dedicated writer with a one-request
queue and per-write acknowledgement, so a child that stops reading cannot block
the coordinator beyond the same step/workflow deadline. A timeout, malformed or
oversized line, early exit, caller cancellation,
or normal completion terminates the entire descendant group. Failure reports
use stable classes such as `step_timeout`, `oversized_response`,
`malformed_response`, and `tool_schema_drift`; raw child diagnostics are never
copied into them.
