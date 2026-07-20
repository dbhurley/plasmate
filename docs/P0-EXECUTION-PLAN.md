# Plasmate P0 Execution Plan

Status: active  
Milestone: v0.6 Reliability, Security, and Agentic-Web Compatibility  
Primary objective: restore a trustworthy engineering and runtime foundation before expanding protocol or browser features.

## Operating decisions

1. `master` must become a release branch, not an integration scratchpad. P0 changes are developed in isolated branches, verified independently, then integrated deliberately.
2. A green default test suite is necessary but insufficient. Required gates cover formatting, warnings, all features, adapters, smoke tests, security regressions, and crash containment.
3. Untrusted pages are hostile input. URLs, redirects, response bodies, scripts, DOM state, and tool output all require explicit limits and trust boundaries.
4. V8 is a native dependency capable of terminating its process. Heap limits alone are not containment; risky page execution needs a supervised process boundary.
5. Product claims must describe observable behavior. Filtered coverage, unpublished adapters, skipped module scripts, and unverified concurrency targets must not be presented as shipped capability.
6. P0 favors safe behavioral breaks over insecure compatibility. Local fixtures and unsafe development behavior require explicit opt-ins, never production defaults.

## Workstream A: Build integrity and release truth

Owner branch: `codex/p0-build-integrity`

### Deliverables

- Make `cargo fmt --all --check` pass.
- Make `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass without broad warning suppression.
- Make default and all-feature workspace tests pass, including the plugin target.
- Repair action-manifest conformance in a clean environment with a declared Python runtime and correct local package import paths.
- Make MCP smoke tests fail promptly when the binary exits, fails to start, or stops responding.
- Ensure smoke tests exercise the binary built by the same CI job.
- Add timeouts and cleanup to background servers used by CI.
- Require the full gate set in GitHub Actions on Linux and the platform-specific subset on macOS.
- Correct objective release drift:
  - ES modules remain incomplete until executable module tests pass.
  - Document the actual MCP tool surface.
  - Report both total-URL and non-blocked coverage.
  - Align checked-in package/server metadata or identify intentionally independent versions.
  - Add an unreleased changelog section for post-v0.5.1 work.
- Prevent automated scorecard updates from being mistaken for validated product builds.

### Acceptance criteria

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --all-features
./scripts/action-manifest-conformance.sh --quick
PLASMATE_BIN=./target/debug/plasmate python3 smoke/mcp-smoke.py
```

All commands must exit zero from a clean checkout. Negative smoke tests must exit non-zero within a bounded deadline and leave no child process running.

## Workstream B: Security boundaries

Owner branch: `codex/p0-security`

### Deliverables

- Centralize outbound URL policy and apply it to every network entry point.
- Permit only HTTP and HTTPS by default.
- Reject credentials in URLs unless a specific call surface explicitly supports them.
- Reject loopback, private, link-local, multicast, unspecified, reserved, and cloud-metadata destinations by default for literal and DNS-resolved addresses.
- Revalidate every redirect hop and cap redirect count.
- Add explicit, narrowly named development/test opt-ins for local fixtures.
- Bound compressed input, decompressed response body, script input, DOM/SOM output, and MCP tool output where each layer owns the data.
- Return structured, actionable policy errors without leaking secrets.
- Replace wildcard auth-bridge CORS with exact extension-origin validation.
- Require a short-lived capability token or nonce for bridge operations and compare it safely.
- Bind local bridges and unauthenticated protocol servers to loopback only.
- Refuse non-loopback exposure unless an authentication mechanism is configured.
- Add `SECURITY.md` with supported versions, reporting instructions, threat boundaries, and unsafe-development flags.

### Required regression cases

- `file:`, `ftp:`, `data:`, Unix-socket-style, and malformed URLs.
- IPv4 and IPv6 loopback/private/link-local literals.
- IPv4-in-IPv6 and unusual numeric-host representations supported by the URL parser.
- DNS resolving to a blocked address.
- Public URL redirecting to a blocked destination.
- Redirect loops and redirect-count exhaustion.
- Oversized fixed-length and chunked responses.
- Missing, invalid, and valid auth-bridge capability tokens.
- Missing, invalid, and valid bridge origins.
- Explicit local-fixture mode working without weakening the default.

### Acceptance criteria

- Security regression tests pass on Linux and macOS.
- Existing public-network fetch behavior remains functional.
- Default MCP/CLI network calls cannot reach localhost or private infrastructure.
- No wildcard browser origin can read or mutate cookie profiles.
- No server starts on a non-loopback address without an explicit authenticated or clearly unsafe development mode.

## Workstream C: Runtime containment

Owner branch: `codex/p0-runtime-containment`

### Deliverables

- Introduce a reusable parent-side worker supervisor.
- Run each risky JavaScript coverage page in a separate child process.
- Enforce a wall deadline and terminate the full child process group on expiry.
- Bound captured stdout and stderr so diagnostics cannot exhaust parent memory.
- Classify success, ordinary page error, timeout, signal/abort, resource exhaustion, malformed worker output, and launch failure.
- Continue a multi-URL run after a single worker failure.
- Preserve truthful exit semantics: individual page failures are report data; systemic setup failure makes the workflow fail.
- Record per-URL outcome, duration, exit status/signal, and a bounded diagnostic excerpt.
- Expose conservative worker heap/memory settings through validated configuration.
- Make the supervisor reusable for later MCP/session isolation without coupling it to coverage JSON.

### Required deterministic tests

- Worker success with valid structured output.
- Worker non-zero exit.
- Worker abort/signal.
- Worker hang and deadline enforcement.
- Worker stdout/stderr beyond capture limits.
- Worker malformed output.
- Parent continuation after each failure class.
- Child and descendant cleanup after cancellation.

### Acceptance criteria

- No deterministic crash/hang fixture terminates or indefinitely blocks the parent test process.
- The JS coverage workflow completes and publishes a truthful report even when individual pages crash.
- Normal non-JS/stateless fetch throughput does not pay a per-page process cost.
- Remaining in-process MCP/session V8 risk is documented as the first follow-up containment slice, not described as solved.

## Workstream D: Supply-chain integrity

Owner branch: `codex/p0-supply-chain`

### Deliverables

- Keep a committed lockfile for every npm root and use `npm ci` in CI.
- Declare and continuously compile-test the real Rust MSRV for default and
  all-feature builds.
- Fail on known Rust, npm, Python, and Go dependency vulnerabilities on pull
  requests, pushes to `master`, and a weekly schedule.
- Pin every remote GitHub Action to an immutable commit and default workflows
  to read-only permissions.
- Keep Python's multi-project audit input complete and reproducible with a
  hashed Linux/CPython 3.11 lock.
- Upload scheduled coverage as short-lived evidence instead of allowing an
  unattended workflow to write to protected `master`.
- Let Dependabot propose restrained weekly updates for every dependency root;
  never auto-merge dependency or Action changes.

### Acceptance criteria

- Production and complete npm audits report zero known vulnerabilities in all
  five lock roots.
- `cargo audit`, `pip-audit`, and `govulncheck` exit zero without ignore lists.
- The workflow pin validator finds no mutable remote Action references.
- Coverage workflows have only `contents: read` and cannot commit or push.

## Integration sequence

1. Integrate build integrity first because it defines the trustworthy verification gate.
2. Rebase security onto the resulting `master`; resolve behavior changes explicitly and run all gates.
3. Rebase runtime containment onto the resulting `master`; run all gates plus crash fixtures and a bounded coverage sample.
4. Update documentation only after integrated behavior is verified.
5. Integrate supply-chain policy before enabling repository branch protection.
6. Enable repository branch protection and required checks after the direct P0 integration sequence is complete.
7. Complete one exact-SHA release session, including two fresh isolated JS scorecards, before tagging v0.6.

Each integration requires:

- Review of the complete diff and test additions.
- Verification that the branch contains no unrelated generated artifacts or dependency churn.
- Targeted tests for the changed subsystem.
- Full formatting, clippy, default, and all-feature gates.
- A focused commit message describing behavior and risk, not merely files changed.
- A clean `master` worktree after integration.

## Operational acceptance: exact-SHA release session

Plasmate does not use an elapsed-time freeze as a release control. Release
authorization is a single bounded session tied to one immutable candidate SHA:

- `CI` and `Dependency Security` must be successful on the candidate.
- Dispatch `Coverage Scorecard (JS)` twice on that exact SHA. Each run builds
  independently, validates `plasmate.coverage.v2`, and uploads its own artifact.
- The production preflight requires the two newest `coverage_js` check runs to
  be successful, issued by GitHub Actions app `15368`, and completed within the
  preceding 24 hours. A newer failure blocks release.
- Branch, tag, and release-environment controls remain mandatory external
  evidence; workflow YAML cannot configure its own trust boundary.

### Run the session

Integrate all intended P0/P1/P2 changes, then select the current `master` SHA.
Do not combine checks or artifacts from different commits:

```bash
git fetch upstream master
candidate="$(git rev-parse upstream/master)"
test "$(git rev-parse HEAD)" = "$candidate"

gh workflow run ci.yml --ref master
gh workflow run security.yml --ref master
gh workflow run coverage-js.yml --ref master
gh workflow run coverage-js.yml --ref master
```

Wait for all four runs. Download both JS artifacts and validate each with the
candidate binary. Confirm their `environment.git_commit`, runner `head_sha`,
corpus digest, outcome partition, and zero infrastructure failures. Ordinary
site blocks and fetch failures remain observations, not infrastructure success.

### Capture external controls

Retain read-only snapshots of `master` protection, the stable-tag ruleset, the
`release` environment, its deployment policy, and secret names. Confirm that
`CARGO_REGISTRY_TOKEN` exists only in the protected environment. Never print a
secret value. If `master` moves, discard the session evidence and rerun the four
workflows on the new SHA; there is no waiting period.

Only after the two fresh JS checks, all 13 required checks, local release
validation, package dry-run, and external-control review pass should the
maintainer create the protected annotated tag. `verify-release-preflight.py`
rechecks those exact-SHA GitHub check runs before any candidate code executes.

## P0 completion definition

P0 is complete only when all of the following are true:

- Protected `master` requires the validated CI gates.
- Formatting, warning-free clippy, default tests, and all-feature tests pass.
- Action-manifest and MCP/CDP smoke paths pass and fail safely.
- Network defaults prevent SSRF and unbounded-body attacks.
- Authenticated browser state is not exposed to arbitrary local webpages.
- A crashing or hanging JS coverage page cannot crash or hang its parent run.
- Coverage and release claims use honest denominators and match shipped code.
- Package metadata and changelog state are coherent.
- Dependency scans cover every first-party lock or package root, all remote
  Actions are immutable, and scheduled evidence cannot write to `master`.
- A security policy and threat model are present.
- Two fresh isolated JS scorecards and all protected checks pass on one exact
  release SHA during the same bounded release session.

## Dependency path into P1 and P2

P1 begins after the P0 code gates are integrated; research and design may proceed earlier, but protocol/runtime changes must build on the secured network and worker primitives. Its order is modern MCP adapters, WebMCP discovery and safety, real ES modules, staged V8 compatibility, focused WPT, then the stateful agent CLI.

P2 begins after P1 protocol and trace contracts stabilize. Its order is Chrome/vision fallback, session trace and replay, coherent v0.6 publishing, reproducible task-success benchmarks, responsible crawl policy, and Agentic Resource Discovery metadata.

P0 deliberately does not promise 500 concurrent sessions, a full layout engine, hosted multi-tenancy, or complete Web Platform compatibility. Those claims require measured evidence after containment and conformance infrastructure exists.
