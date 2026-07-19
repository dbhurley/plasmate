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

## Optional public-web methodology

Public-web runs answer a different question: how Plasmate behaved against a named corpus from a named environment at a particular time. They are not deterministic release gates.

Every published public-web report must include:

- the immutable input list or its cryptographic digest;
- the complete input denominator;
- mutually exclusive success, blocked, failed, crash, and timeout counts;
- both the overall success rate and any explicitly labeled filtered rate;
- cold and warm wall latency, with cache state;
- JS and external-script settings;
- response HTML and SOM bytes for successful inputs;
- peak RSS and worker termination evidence where available;
- Plasmate, Git, compiler, OS, architecture, and machine metadata;
- per-input results sufficient to audit every aggregate.

A site blocked by authentication, robots policy, anti-automation measures, geography, or network policy remains `blocked` in the all-input denominator. It must not be removed, reclassified as absent, or implied to have succeeded. Compression statistics may be computed over successful inputs, but the report must state that denominator next to the statistic.

For trend comparisons, use reports with the same schema, input-list digest, JS mode, cache policy, build profile, and runner class. A change to any of those properties begins a new comparison series.

## Benchmark claims

Marketing and release notes may cite a benchmark only when the report is retained as a build artifact and the exact metric denominator is stated. Preferred claims describe task-contract pass rate, total-input availability, latency distribution, peak memory, and crash rate. Compression is supporting evidence, not the product-quality headline.
