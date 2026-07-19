# Runtime containment

JavaScript coverage pages run in one supervised child process per URL. This is
required because fatal V8 failures abort the process and cannot be recovered by
Rust error handling.

The coverage coordinator enforces these boundaries:

- `--timeout` is a hard wall timeout for each worker.
- `--js-heap-mb` sets the V8 heap budget inside the worker.
- `--worker-output-kb` bounds captured stdout and stderr while continuing to
  drain both pipes.
- `--worker-memory-mb` optionally applies a Linux address-space limit. It is
  disabled by default because V8 reserves substantial virtual address space;
  deployments should validate a non-zero value against their V8 build.
- Worker process groups and descendants are terminated on timeout, completion,
  or coordinator cancellation.

Worker timeouts, signals, and non-zero exits are recorded as per-URL failures,
and the remaining URLs continue. Coordinator launch and worker-protocol errors
are recorded in `summary.infrastructure_failures`; the report is written before
the CLI exits non-zero for those systemic failures. `summary.success_percent`
uses all input URLs, while `parsed_percent` retains the historical metric that
excludes blocked sites.

This boundary currently protects the batch coverage path only. MCP, AWP, CDP,
daemon, and ordinary `fetch` JavaScript execution still occur in their owning
processes. Those paths should adopt the reusable `process_supervisor` primitive
before Plasmate claims process isolation for interactive or long-lived sessions.
