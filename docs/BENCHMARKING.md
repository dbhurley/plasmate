# Benchmarking Plasmate

Plasmate uses two evidence classes. Deterministic product-contract benchmarks are release gates. Public-web coverage is observational and must never hide blocked, failed, crashed, or timed-out inputs.

## Deterministic release benchmark

Run the versioned suite from the repository root:

```bash
cargo run -- benchmark-v1 --output benchmark-v1.json
cargo run --release -- benchmark-v1 --output benchmark-v1-release.json
cargo run --release -- benchmark-v1 --js --output benchmark-v1-js.json
```

`benchmark-v1` starts ephemeral loopback-only fixture servers and makes no public-network requests. Only the crate-private fixture client receives the explicit private-network policy; ordinary CLI, MCP, daemon, AWP, CDP, script, and JavaScript fetch paths remain fail-closed for localhost and private addresses. Each supervised worker creates its own fixed fixture server, and its strict input schema has no destination field, so caller-controlled URLs cannot reach that exception. It exercises five product contracts:

1. A navigation link preserves its action, label, and destination, then produces the expected navigation action.
2. A labeled form control exposes the expected input action and accepts the fixture value through the DOM action layer.
3. Main-content text remains observable in the SOM, and JavaScript mode must expose the script-mutated document title.
4. An HTTP redirect preserves the final URL and resulting document.
5. An HTTP 500 remains visibly classified as a failed input while the expected-error task assertion passes.

Each page input runs twice. The cold sample must miss the SOM cache; the warm sample must hit it. Both samples record end-to-end fetch-and-compile wall time, cache state, JS state, final URL, HTTP status, HTML bytes, serialized SOM bytes, compression ratio, and `process_peak_rss_bytes_at_sample_end` where the operating system exposes it. Action and extraction assertions run after those samples and record their own `assertion_wall_time_us`. The RSS field is explicitly the cumulative process high-water mark observed at sample end, not an isolated allocation or delta.

With JavaScript disabled, the deterministic suite runs in-process and cannot truthfully classify a native process crash. With `--js`, every complete fixture task—including its cold and warm samples—runs in a separate process under the shared supervisor. A signal is reported as `crash`, a hard wall deadline as `timeout`, output is bounded, and the coordinator continues to later tasks. The default worker deadline is 15 seconds; release runners may tune it explicitly. The report's `execution_isolation` and worker-limit fields make that distinction machine-readable.

The report also records the Plasmate version, Git commit and dirty state, Rust compiler, build profile, operating system, and architecture. `schema_version` is `plasmate.benchmark.v1`; consumers must reject an unknown major schema rather than silently interpreting it as v1.

### Outcome denominator

`summary.inputs_total` is always the complete task input count. The following fields partition that denominator without exclusions:

- `success`
- `blocked`
- `failed`
- `crash`
- `timeout`

Expected-error fixtures remain in their observed outcome bucket. A separate `task_passed` value says whether the observed result satisfied that fixture's contract. This prevents a handled HTTP error from disappearing from the outcome totals.

The command exits 2 when a regression threshold fails. Default thresholds require all task contracts, cold misses, warm hits, and a two-second ceiling for each successful cold and warm local sample. Latency ceilings can be made stricter for a stable release runner:

```bash
cargo run --release -- benchmark-v1 \
  --max-cold-ms 250 \
  --max-warm-ms 100 \
  --output benchmark-v1-release.json
```

Do not compare debug and release reports as if their latency were equivalent. Do not treat byte compression as task success.

## Deterministic agent task benchmark

The agent task suite is a separate release gate for the supervised workflow
product surface:

```bash
cargo run --locked -- agent-task-benchmark-v1 \
  --output agent-task-benchmark-v1.json
cargo run --locked -- agent-task-benchmark-validate \
  --input agent-task-benchmark-v1.json
```

`plasmate.agent-task-benchmark.v1` runs six checked-in scenarios through a
real supervised Plasmate MCP stdio child. The child uses the ordinary MCP
session, action, trace, and replay-validation implementations. The inputs are
compiled into the binary from `benchmarks/agent-workflow-v1/`; an ephemeral
loopback server is the only network destination. The runner gives only that
child the private-network development opt-in, and neither the command nor its
internal plan builder accepts a caller-controlled URL. No public service,
model call, or model judgment is part of the required gate.

The scenarios cover navigation, approved typing plus state observation,
privacy-safe trace export, cross-session replay refusal, an expected tool error
that permits the workflow to continue, and fail-fast containment of an
unexpected tool error. A healthy report therefore has six passing **task
contracts** but five succeeded workflow outcomes and one deliberately failed
workflow outcome. Reporting six successful workflows would be false.

`summary.tasks_total` is the complete scenario denominator. Exactly one of
`observed_succeeded`, `observed_failed`, `observed_crash`, and
`observed_timeout` accounts for every scenario. Task-contract pass/fail is a
separate complete partition, because an expected failure-containment scenario
can pass its contract while its observed workflow outcome is `failed`. Step
totals are also partitioned into succeeded, failed, and skipped rows.

The report includes the ordered scenario descriptors, a canonical manifest
digest, a length-framed digest over the manifest and exact HTML fixture bytes,
plan fingerprints, executable digest, Git/compiler/build/OS/architecture
provenance, and every redacted step outcome. The validator recomputes the
compiled corpus identity and all denominators, rejects unknown schema majors,
and confirms that the release gate agrees with the task rows.

Per-scenario wall time is observational. It is not a microbenchmark, has no
release threshold, and must not be compared across runner classes. Optional
live evaluations involving public sites or models must use a separate schema
and artifact; they must never replace, alter, or be averaged into this required
deterministic gate.

## Optional public-web methodology

Public-web runs answer a different question: how Plasmate behaved against a named corpus from a named environment at a particular time. They are not deterministic release gates.

Every published public-web report must include:

- `schema_version: plasmate.coverage.v2`;
- `corpus.sha256`, computed over the exact ordered URL sequence selected after
  comment/blank-line removal and `max_urls` truncation using the canonicalization
  named in the report, plus `corpus.ordered_input_urls` so the digest can be
  independently recomputed from the artifact;
- the complete input denominator;
- mutually exclusive `summary.outcomes` success, blocked, failed, crash, and
  timeout counts whose sum equals the complete denominator;
- both the overall success rate and any explicitly labeled filtered rate;
- per-input single-pass fetch and pipeline wall latency, explicitly labeled with
  `cache_state: not_measured` and separate observed-sample counts;
- JS and external-script settings;
- response HTML and SOM bytes for successful inputs;
- peak RSS and worker termination evidence where available;
- Plasmate, Git commit/dirty state, compiler, build profile, OS, architecture,
  and runner metadata where available;
- per-input results sufficient to audit every aggregate.

The public scorecard currently executes each selected URL once and does not use
the SOM cache, so it must not label samples `cold` or `warm` or claim cache-hit
evidence. Its `measurement.cache.collected` value is therefore `false` and its
latencies are observational single-pass wall times. Controlled cold/warm cache
evidence belongs to `benchmark-v1`, which performs and verifies both samples.
`summary.compression_samples` is the exact successful-input denominator for the
reported compression distribution.

For compatibility, v2 retains the original top-level `plasmate_version`, legacy
`summary.ok`, `summary.blocked`, and overlapping `summary.failed` aggregates.
New analysis must use `summary.outcomes`: its `failed` bucket excludes the
separate crash and timeout buckets. Consumers must reject unknown coverage schema
majors instead of guessing their denominator semantics. `coverage-validate`
checks these structural invariants; observed site failures remain valid evidence
and do not make validation fail.

A site blocked by authentication, robots policy, anti-automation measures, geography, or network policy remains `blocked` in the all-input denominator. It must not be removed, reclassified as absent, or implied to have succeeded. Compression statistics may be computed over successful inputs, but the report must state that denominator next to the statistic.

For trend comparisons, use reports with the same schema, corpus digest, JS mode,
cache measurement policy, build profile, and runner class. A change to any of
those properties begins a new comparison series. Do not compare the public
single-pass latencies with `benchmark-v1` cold/warm measurements.

## Benchmark claims

Marketing and release notes may cite a benchmark only when the report is retained as a build artifact and the exact metric denominator is stated. Preferred claims describe task-contract pass rate, total-input availability, latency distribution, peak memory, and crash rate. Compression is supporting evidence, not the product-quality headline.
