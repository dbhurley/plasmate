# Changelog

## Unreleased

- Post-v0.5.1 work remains unreleased and is being brought back behind required formatting, all-feature lint, test, conformance, and smoke gates.
- Current development includes expanded SOM action metadata, cache/session observability, proxy and cookie surfaces, iframe and Shadow DOM extraction, and broader SDK/adapter conformance.
- Added opt-in, bounded, memory-only `plasmate.trace.v1` session action traces plus four MCP tools for status, privacy-safe export, clearing, and exact validation-before-replay. Replay is validation-only and never executes an action in this release.
- Added the reproducible `plasmate.agent-task-benchmark.v1` release gate. Six compiled, loopback-only scenarios exercise real supervised MCP navigation, approved actions, trace export, validation-only replay refusal, and expected/unexpected failure containment with complete outcome and step denominators, fixture/executable provenance, an independent validator, and no model judgment or public-network dependency.
- Added bounded, static Agentic Resource Discovery v0.9 draft inspection through `plasmate ard-discover` and the read-only `ard_discover` MCP tool. The implementation checks the well-known, HTML link, and robots.txt signals; rejects unsafe or cross-origin catalog fetches; validates catalog envelopes; and labels all discovered data and trust claims as untrusted and unverified. Registry search, DNS discovery, nested crawling, endpoint fetching, invocation, and trust/signature verification are not included.
- Added native V8 execution for a bounded ES-module core: inline/external roots,
  same-origin static URL imports, live bindings, cycles, evaluate-once caching,
  strict scope, deferred ordering, and `import.meta.url`. Bare specifiers,
  import maps/attributes, cross-origin CORS, dynamic import, and top-level await
  remain explicitly unsupported; see `docs/ES-MODULES.md`.
- Added read-only Rust, npm, Python, Go, and GitHub Actions supply-chain gates,
  immutable Action pins, restrained Dependabot coverage, and artifact-only
  scheduled coverage evidence.
- Replaced the elapsed-time release soak with a bounded exact-SHA release
  session: production preflight now requires all protected checks plus the two
  newest isolated JS scorecards to have succeeded within 24 hours.
- Added a Vercel AI dependency lock and migrated its MCP client to supported AI
  SDK 6 APIs; refreshed vulnerable npm transitive dependencies to clean audits.
- Updated the optional Wasmtime runtime to the patched 36.x line with a minimal
  feature set and refreshed vulnerable TLS, QUIC, and test-only Rust transitives.
- Corrected the documented Rust minimum to the verified 1.88 dependency floor
  and added default/all-feature MSRV checks.
- Removed the Browser Use package's unused `browser-use` convenience extra so
  installing Plasmate no longer pulls an upstream graph pinned to vulnerable
  `pypdf` releases.
- Added the versioned `plasmate.crawl-policy.v1` RFC 9309 evaluator through `plasmate crawl-policy` and the read-only `crawl_policy` MCP tool. It uses a public-only, same-origin robots request; combines exact product-token groups with wildcard fallback; implements longest-rule selection, wildcard/end anchoring, percent-encoding semantics, and conservative unavailable/unreachable handling; and leaves ordinary fetch behavior unchanged.
- Added the read-only `inspect_page` MCP tool. It returns a bounded compact SOM first and uses deterministic `never`/`auto`/`always` visual fallback. Page JavaScript is off by default; explicit opt-in retains the documented in-process V8 risk before screenshot isolation. Screenshots render only already-fetched HTML in a JavaScript-disabled, sandboxed/CSP-constrained Chrome document with restrictive file defaults, a dead network proxy, process-tree cleanup, and hard dimensions/time/image/envelope bounds. Typed visual failure never discards structure, and Plasmate does not perform vision-model interpretation.

## v0.5.1 (2026-04-05)

- Added selector-aware MCP fetching and the `extract_links` tool.
- Added custom fetch headers, additional CLI formats/selectors, SOM diff support, and hardened V8 fetch bridging.
- Added the Vercel AI SDK integration and MCP Registry manifest.

## v0.5.0 (2026-03-28)

- Added direct HTML-to-SOM compilation with `plasmate compile`.
- Added `html_id`, details/summary semantics, expanded ARIA state, consent-banner filtering, and authenticated-browsing documentation.

## v0.3.0 (2026-03-22)

### SPA Rendering Bridge
- Live DOM bridge: V8 JavaScript mutations now flow into the real rcdom tree
- NodeRegistry with bidirectional V8-DOM bindings (14 native callbacks)
- CSS selector engine for querySelector/querySelectorAll
- SOM recompiled from JS-modified DOM tree after script execution
- DOM shim expanded: createTreeWalker, createRange, Observer stubs, navigator, window APIs

### Screenshot Support
- `plasmate screenshot <url>` CLI command
- Page.captureScreenshot CDP method (returns SOM fallback until renderer lands)
- Screenshot support in AWP and MCP protocols

### Parallel Sessions
- SessionManager for concurrent page processing (up to 50 sessions)
- CDP multi-target support with independent page contexts
- Thread-safe session storage with idle timeout and memory tracking

### Network & Security
- Network request interception (block, modify, mock responses)
- TLS fingerprint configuration (cipher suites, version control)
- CDP cookie jar (Network.getCookies, setCookies, deleteCookies, clearBrowserCookies)

### Plugin System
- Wasm plugin runtime (wasmtime-based)
- 8 plugin types: page_transform, request_intercept, response_transform, dom_mutate, som_post_process, auth_provider, cache_strategy, analytics

### Coverage & Benchmarks
- 100-URL benchmark suite (98 sites tested)
- HTML coverage: 95.9% parseable-site success (blocked sites excluded)
- JS coverage: 95.9% parseable-site success (blocked sites excluded)
- Median SOM compression: 9.05x
- Nightly HTML + weekly JS coverage CI

### Other
- Browser-realistic HTTP headers for anti-bot compatibility
- URL/URLSearchParams polyfill improvements
- External module script handling
- Pre/post JS SOM comparison (keep best result)

## v0.1.1 (2026-03-18)

### Added
- Cookie-based auth profiles for authenticated browsing
- `--profile` flag on serve for authenticated sessions
- Extension CLI bridge server with AES-256-GCM encryption at rest
- Privacy policy page for Chrome Web Store submission
- `/api/wait` endpoint for agent-driven auth flow
- Top-100 coverage scorecard with configurable JS safety budgets
- Functional MutationObserver for SPA framework support

### Fixed
- Browser-realistic HTTP headers to avoid anti-bot blocking
- Pre/post JS SOM comparison to prevent JS content degradation
- Robust URL/URLSearchParams polyfills
- Strip macOS quarantine flag in install script

## v0.1.0 (2026-03-17)

- Initial release
- Headless browser engine with Semantic Object Model (SOM)
- CDP-compatible WebSocket server
- AWP and MCP protocol support
- JavaScript execution via V8
- HTML parsing and rendering pipeline
