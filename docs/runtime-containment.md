# Runtime containment

Fatal V8 failures abort the process and cannot be recovered by Rust error
handling. Ordinary page JavaScript and one-shot stateful evaluations therefore
run in a supervised Plasmate child rather than the CLI, MCP, daemon, AWP, CDP,
or embedding process that owns the request.

## Ordinary page and session execution

The page pipeline resolves bounded external classic scripts and native module
graphs before entering V8, then sends the source HTML and prepared graph to
`__js-worker` over the versioned `plasmate.js-worker.v1` JSON protocol. This
boundary covers:

- synchronous and asynchronous public page-pipeline helpers;
- `plasmate fetch` and daemon fetches;
- stateless and stateful MCP page loads;
- AWP/CDP navigation and `Page.setContent`;
- MCP evaluate, click, type, select, scroll, toggle, and clear operations; and
- CDP `Runtime.evaluate` and `Runtime.callFunctionOn`.

The default worker has a 15-second hard wall deadline, a 16 MiB stdout bound,
a 256 KiB stderr bound, the runtime's 64 MiB V8 heap limit, and no operating-
system address-space limit. V8 reserves substantial virtual address space, so a
non-zero Linux address-space ceiling must be validated against the pinned V8
build. Requests are bounded at 32 MiB. Output pipes continue to drain after a
bound is reached, preventing a noisy child from deadlocking its parent.

Every child gets a dedicated process group. The group and any descendants are
terminated on timeout, caller cancellation, normal completion, and coordinator
shutdown. A child signal, non-zero exit, timeout, spawn failure, output
violation, or protocol violation becomes a typed `JsWorkerError` with a stable
code such as `js_worker_timeout` or `js_worker_crash`.

The worker starts with an empty environment. Only `PLASMATE_ICU_DATA` and the
explicit `PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK` development opt-in are copied
when present. Auth tokens, cloud credentials, proxy credentials, and unrelated
application secrets are not ambiently exposed to page-controlled code.

Page rendering does not discard all useful output when V8 fails. The parent
compiles the original HTML to SOM and adds a typed `containment_failure` to the
otherwise compatible `JsExecutionReport`. Imperative WebMCP capture is absent
in that fallback, while bounded declarative discovery still runs over the
source HTML. Stateful mutation/evaluation failures leave the session's last
good SOM, effective HTML, and WebMCP catalog unchanged.

The worker returns only serializable state: effective HTML, execution
diagnostics, and bounded WebMCP registration metadata. Function callbacks and
V8 handles never cross the boundary. Wasm post-parse and post-SOM plugin hooks
remain in the parent and run on either the worker result or the source-HTML
fallback.

External classic scripts and native module graphs are still acquired in the
parent with the caller-provided asynchronous client. Page-originated `fetch`
and `XMLHttpRequest` retain the pre-existing restricted bridge behavior: they
use a fresh, public-network-checked blocking client and do not inherit caller
cookies, proxy settings, custom TLS configuration, or default headers.
Authenticated page-side XHR through caller client state is therefore not
supported; worker isolation does not newly remove that capability.

An embedding executable that is not named `plasmate` must install a sibling
`plasmate` binary or set `PLASMATE_JS_WORKER_EXECUTABLE` to an exact compatible
binary. If no worker can be resolved, the typed spawn failure follows the same
source-HTML fallback rather than executing untrusted JavaScript in-process.
Direct `JsRuntime` construction and the explicit `PipelineConfig {
isolate_js: false, .. }` escape hatch remain in-process and are only for an
already-supervised worker or trusted tests.

## Coverage execution

Coverage retains one supervised child process per URL. The coverage child
performs fetch, V8 execution, and SOM compilation as one unit and therefore
disables a redundant nested JS worker. Its coordinator enforces:

- `--timeout` as the hard wall timeout for each URL worker;
- `--js-heap-mb` as the V8 heap budget inside the worker;
- `--worker-output-kb` as bounded stdout and stderr capture;
- `--worker-memory-mb` as an optional Linux address-space limit; and
- process-group cleanup for completion, timeout, and cancellation.

Worker timeouts, signals, and non-zero exits are recorded as per-URL failures,
and the remaining URLs continue. Coordinator launch and worker-protocol errors
are recorded in `summary.infrastructure_failures`; the report is written before
the CLI exits non-zero for those systemic failures. `summary.success_percent`
uses all input URLs, while `parsed_percent` retains the historical metric that
excludes blocked sites.
