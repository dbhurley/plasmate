<p align="center">
  <img src="website/brand/plasmate-mark.png" alt="Plasmate" width="80" />
</p>

<h1 align="center">Plasmate</h1>

<p align="center">
  The browser engine for agents.<br/>
  HTML in. Semantic Object Model out.
</p>

<p align="center">
  <a href="https://plasmate.app">Website</a> &middot;
  <a href="https://docs.plasmate.app">Docs</a> &middot;
  <a href="https://plasmate.app/compare">Benchmarks</a> &middot;
  <a href="https://crates.io/crates/plasmate">Crates.io</a> &middot;
  <a href="https://www.npmjs.com/package/plasmate">npm</a> &middot;
  <a href="https://pypi.org/project/plasmate/">PyPI</a>
</p>

<p align="center">
  <img src="https://github.com/plasmate-labs/plasmate/actions/workflows/release.yml/badge.svg" alt="CI" />
  <img src="https://img.shields.io/crates/v/plasmate" alt="crates.io" />
  <img src="https://img.shields.io/npm/v/plasmate" alt="npm" />
  <img src="https://img.shields.io/badge/integrations-60%2B-brightgreen" alt="60+ Integrations" />
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License" />
</p>

---

Dependency and CI trust policy, local audit commands, and scorecard promotion
rules are documented in [Supply-chain policy](docs/SUPPLY-CHAIN.md).

Plasmate compiles HTML into a **Semantic Object Model (SOM)**, a structured representation that LLMs can reason about directly. It runs JavaScript via V8, supports Puppeteer via CDP, and produces output that is 10-800x smaller than raw HTML.

| | Plasmate | Lightpanda | Chrome |
|---|---|---|---|
| **Per page** | **4-5 ms** | 23 ms | 252 ms |
| **Memory (100 pages)** | **~30 MB** | ~2.4 GB | ~20 GB |
| **Binary** | **43 MB** | 59-111 MB | 300-500 MB |
| **Output** | **SOM (10-800x smaller)** | Raw HTML | Raw HTML |
| **License** | **Apache-2.0** | AGPL-3.0 | Chromium |

## Install

```bash
curl -fsSL https://plasmate.app/install.sh | sh
```

Or via package managers:

```bash
cargo install plasmate       # Rust
npm install -g plasmate      # Node.js
pip install plasmate         # Python
```

## Quick Start

### Fetch a page and get structured output

```bash
plasmate fetch https://news.ycombinator.com
```

Returns SOM JSON: structured regions, interactive elements with stable IDs, and content, typically 10x smaller than the raw HTML.

### Start a CDP server (Puppeteer compatible)

```bash
plasmate serve --protocol cdp --host 127.0.0.1 --port 9222
```

Then connect with Puppeteer:

```javascript
import puppeteer from 'puppeteer-core';

const browser = await puppeteer.connect({
  browserWSEndpoint: 'ws://127.0.0.1:9222',
  protocolTimeout: 10000,
});

const page = await browser.newPage();
await page.goto('https://example.com');

const title = await page.evaluate(() => document.title);
console.log(title);

await browser.close();
```

### Start an AWP server (native protocol)

```bash
plasmate serve --protocol awp --host 127.0.0.1 --port 9222
```

AWP has 7 methods: `navigate`, `snapshot`, `click`, `type`, `scroll`, `select`, `extract`. That's the entire protocol.

### Run a supervised stateful workflow

```bash
plasmate agent-run --plan examples/agent-workflow.json \
  --report agent-workflow-report.json --dry-run
```

Plans are versioned and bounded; mutating steps require separate
`--confirm-step <id>` approvals, while JavaScript evaluation and cookie writes
also require category opt-ins. See [the workflow contract](docs/agent-workflows.md)
for schemas, secret references, containment, and execution examples.

### Run as an MCP tool server (Model Context Protocol)

```bash
plasmate mcp
```

This exposes Plasmate over stdio as MCP tools:
- `fetch_page` - get structured SOM from any URL
- `extract_text` - get clean readable text
- `extract_links` - get deduplicated links from a page
- `ard_discover` - inspect bounded static ARD v0.9 draft catalog signals without invoking them
- `crawl_policy` - evaluate RFC 9309 robots.txt policy without changing fetch behavior
- `inspect_page` - return bounded SOM first, with deterministic optional visual fallback
- `cache_status` - inspect MCP SOM cache reuse and restorable page-state entries
- `session_status` - inspect active sessions, loaded URLs, HTML/SOM/node inventory
- `trace_status` - inspect bounded action-trace retention for one session
- `trace_export` - export privacy-safe `plasmate.trace.v1` events
- `trace_clear` - discard retained events without resetting their sequence
- `replay_validate` - validate a retained action without executing it
- `screenshot_page` - capture a page screenshot, with SOM fallback
- `open_page` - start an interactive session (returns session_id, SOM, cache_restored)
- `navigate_to` - navigate an existing session
- `evaluate` - run JavaScript in the page context
- `click` - click elements by SOM element ID
- `type_text` - type into an input or textarea
- `select_option` - select a dropdown option
- `scroll` - scroll the page or an element
- `toggle` - toggle checkbox, radio, or details state
- `clear` - clear an input or textarea
- `close_page` - end a session
- `get_cookies` - read session cookies
- `set_cookies` - add or update session cookies
- `clear_cookies` - remove session cookies

For an authenticated local Streamable HTTP endpoint, set a capability token
and select the HTTP transport explicitly:

```bash
PLASMATE_MCP_HTTP_TOKEN="$(openssl rand -hex 32)" \
  plasmate mcp --transport http --host 127.0.0.1 --port 9272
```

The endpoint is `http://127.0.0.1:9272/mcp`. Every request must include
`Authorization: Bearer <token>`. The server implements JSON response mode for
the stateful MCP `2025-11-25` transport and the stateless `2026-07-28` release
candidate. It deliberately returns HTTP 405 for GET rather than claiming an
SSE notification stream it does not implement. See
[`docs/mcp-streamable-http.md`](docs/mcp-streamable-http.md) for lifecycle,
headers, and browser-Origin policy.

Example Claude Desktop config:

```json
{
  "mcpServers": {
    "plasmate": {
      "command": "plasmate",
      "args": ["mcp"]
    }
  }
}
```

## For AI Agents

Plasmate is purpose-built for AI agent pipelines. Several ways to wire it in:

### MCP (Claude Desktop, Cursor, VS Code Copilot, Windsurf)

Add to your MCP config and every tool call automatically uses Plasmate:

```json
{
  "mcpServers": {
    "plasmate": {
      "command": "plasmate",
      "args": ["mcp"]
    }
  }
}
```

Config file locations:
- **Claude Desktop** — `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS)
- **Cursor** — `~/.cursor/mcp.json`
- **VS Code Copilot** — `.vscode/mcp.json` (workspace) or user settings
- **Windsurf** — `~/.codeium/windsurf/mcp_config.json`

Once connected, 26 tools are available: `fetch_page`, `extract_text`, `extract_links`, `ard_discover`, `crawl_policy`, `inspect_page`, `cache_status`, `session_status`, `trace_status`, `trace_export`, `trace_clear`, `replay_validate`, `screenshot_page`, `open_page`, `navigate_to`, `click`, `type_text`, `select_option`, `scroll`, `toggle`, `clear`, `evaluate`, `close_page`, `get_cookies`, `set_cookies`, `clear_cookies`.

**Tip:** use `selector="main"` to strip nav/footer, `selector="interactive"`
to return only actionable elements, or `selector="action:click"` to build a
compact click-target menu before the LLM sees the content.
Use `cache_status` after repeated fetches to inspect local MCP SOM cache hits,
misses, selector entries, effective-HTML entries, and avoided HTML work.
Use `session_status` before long interactive runs to inspect active browser
session count, capacity, loaded URLs, raw/effective HTML sizes, SOM sizes,
node-map counts, structured data presence, age, and idle time. Stateful
`open_page` and `navigate_to` return `cache_restored=true` when they reuse a
content-hash-validated cache entry with both SOM and effective HTML.
Set `trace=true` on `open_page` to opt into bounded, memory-only action tracing,
then use `trace_status`, `trace_export`, and side-effect-free
`replay_validate`. Typed values and page bodies are never exported. See
[`docs/session-tracing.md`](docs/session-tracing.md) for the privacy contract,
bounds, drift classes, and validation-only limitation.

Before a multi-page crawl, use `crawl_policy` or the CLI equivalent:

```bash
plasmate crawl-policy https://example.com/private --product-token Plasmate
```

The versioned report distinguishes an unavailable robots file (4xx, access
permitted by RFC 9309) from an unreachable one (network/5xx, access denied).
It is advisory metadata, not authorization, and does not silently alter
ordinary `fetch` or `fetch_page`. See [docs/CRAWL-POLICY.md](docs/CRAWL-POLICY.md).

Use `inspect_page` when an agent may need pixels. Its default `auto` mode
always returns a bounded compact SOM and only attaches a screenshot for named
signals such as near-empty structure, canvas-heavy content, or image-map/image
controls. `never` recommends without capture; `always` is an explicit capture
request. Page JavaScript is off by default; `javascript=true` explicitly opts
into the existing in-process V8 crash boundary before screenshot isolation.
Rendering uses a sandboxed/CSP-constrained document with JavaScript
disabled, no unsafe file-access or browser-sandbox switches, a dead network
proxy, and process-tree cleanup. Plasmate does not interpret the image with a vision model. See
[docs/STRUCTURED-VISUAL-FALLBACK.md](docs/STRUCTURED-VISUAL-FALLBACK.md).

### Static ARD discovery (draft)

Inspect the three static Agentic Resource Discovery signals supported by
Plasmate—the well-known catalog, an HTML `rel="ai-catalog"` link, and a
robots.txt `Agentmap` directive:

```bash
plasmate ard-discover https://example.com/ --output ard-report.json
```

The versioned `plasmate.ard.discovery.v1` report validates bounded catalog
envelopes and labels all publisher metadata, inline data, trust manifests,
attestations, and signatures as untrusted and unverified. It does not search a
registry, invoke entries, fetch entry endpoints, follow nested catalogs, or
verify identity. HTTPS and same-origin catalog discovery references are
mandatory, and the ARD path cannot use Plasmate's private-network development
override. Standalone reports and fully wrapped MCP tool results each have an
independently measured 512 KiB serialized bound. See
[docs/ARD.md](docs/ARD.md) for limits and protocol scope.

### Vercel AI SDK

Use Plasmate via the AI SDK's built-in MCP client (AI SDK v4+):

```bash
npm install ai @ai-sdk/openai
```

```ts
import { experimental_createMCPClient as createMCPClient, generateText } from 'ai'
import { Experimental_StdioMCPTransport as StdioMCPTransport } from 'ai/mcp-stdio'
import { openai } from '@ai-sdk/openai'

const mcp = await createMCPClient({
  transport: new StdioMCPTransport({
    command: 'plasmate',
    args: ['mcp'],
  }),
})

const { text } = await generateText({
  model: openai('gpt-4o'),
  tools: await mcp.tools(),
  maxSteps: 5,
  prompt: 'Summarize the top 3 stories on news.ycombinator.com',
})

await mcp.close()
```

This wires the full Plasmate MCP tool set directly into any Vercel AI SDK agent. See [Vercel AI SDK MCP docs](https://ai-sdk.dev/docs/ai-sdk-core/tools-and-tool-calling#mcp-tools) for details.

### LLM context

- Machine-readable summary: [`https://plasmate.app/llms.txt`](https://plasmate.app/llms.txt)
- Codebase guide for AI coding agents: [`AGENTS.md`](./AGENTS.md)
- Listed on [MCP Registry](https://registry.modelcontextprotocol.io) as the first browser/web tool


## What is SOM?

The DOM was built for rendering. SOM was built for reasoning.

```
Wikipedia homepage:
  DOM  → 47,000 tokens
  SOM  → 4,500 tokens (10.4x compression)

accounts.google.com:
  DOM  → ~300,000 tokens
  SOM  → ~350 tokens (864x compression)
```

SOM strips layout, styling, scripts, SVGs, and boilerplate. It keeps structure, content, and interactive elements with stable IDs that agents can reference in actions.

## Token Compression (38-site benchmark)

| Site | HTML | SOM | Compression |
|---|---|---|---|
| accounts.google.com | 1.2 MB | 1.4 KB | **864x** |
| x.com | 239 KB | 1.5 KB | **159x** |
| linear.app | 2.2 MB | 21 KB | **105x** |
| bing.com | 157 KB | 1.7 KB | **93x** |
| google.com | 194 KB | 2.6 KB | **74x** |
| vercel.com | 941 KB | 22 KB | **43x** |
| ebay.com | 831 KB | 33 KB | **25x** |
| Wikipedia | 1.7 MB | 70 KB | **25x** |

Median compression: **10.2x** across 38 sites. [Full results](https://plasmate.app/compare).

## JavaScript Support

Plasmate embeds V8 and executes page JavaScript, including:

- Inline and external `<script>` tags
- `fetch()` and `XMLHttpRequest` with real HTTP requests
- `setTimeout` / `setInterval` with timer draining
- DOM mutations (createElement, appendChild, textContent, innerHTML, etc.)
- DOMContentLoaded and load events
- Promise resolution and microtask pumping

The JS pipeline runs during `plasmate fetch` and CDP `page.goto()`. The resulting DOM mutations are serialized back to HTML before SOM compilation, so JS-rendered content is captured.

## CDP Compatibility

Plasmate passes [Lightpanda's Puppeteer benchmark](https://github.com/lightpanda-io/demo) (campfire-commerce). Supported CDP methods:

- `page.goto()`, `page.content()`, `page.title()`
- `page.evaluate()`, `page.waitForFunction()`
- `browser.newPage()`, `browser.createBrowserContext()`
- `Runtime.evaluate`, `Runtime.callFunctionOn`
- `DOM.getDocument`, `DOM.querySelector`, `DOM.querySelectorAll`
- `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`
- Target management (create, attach, close)
- `Plasmate.getSom`, `Plasmate.getStructuredData`, `Plasmate.getInteractiveElements`, `Plasmate.getMarkdown`

`Plasmate.getInteractiveElements` returns a full-tree action menu, including
nested and shadow-root targets. It accepts optional `role`, `action`, `label`,
`exact`, `enabledOnly`, replay lookup (`value` plus `by`, or direct `id`,
`cacheKey`, `htmlId`, `testId`), and `offset`/`limit` params so Puppeteer/
Playwright clients can request, for example, one page of enabled click targets
before calling an LLM. Each target includes SOM role names, deterministic
`cache_key`, `html_id`, `test_id`, `enabled`, and `blocked_reason` when
available.

`DOM.querySelector` / `DOM.querySelectorAll` walk the SOM-backed node tree in
document order and understand stable replay selectors such as `#html_id`,
`#som_id`, `[data-testid="..."]`, `[data-test-id="..."]`, common attributes
such as `[name="..."]`, `[href="..."]`, `[type="..."]`,
`[aria-label="..."]`, `[aria-labelledby="..."]`, `[role="..."]`, SOM role
names, text, and labels. Matching for text and labels is case-insensitive.
CDP DOM nodes expose replay attributes such as `data-plasmate-id`,
`data-som-role`, HTML id, test id, ARIA label, href/name/type, and
disabled/readonly/required flags when available. `Accessibility.getFullAXTree`
is also SOM-backed and includes nested and shadow-root elements with backend
node ids plus disabled/readonly properties when available.

CDP is a compatibility layer. AWP is the native protocol, designed for agents rather than debuggers.

## Architecture

```
HTML → Network (reqwest) → HTML Parser (html5ever)
  → JS Pipeline (V8: scripts, fetch, XHR, timers, DOM mutations)
    → DOM Serialization → SOM Compiler → JSON output
```

- **Network**: reqwest with TLS, HTTP/2, redirects, compression; cookie jar supported, cookie APIs and proxy configuration are still limited
- **JS Runtime**: V8 with DOM shim (80+ methods), blocking fetch bridge
- **SOM Compiler**: semantic region detection, element ID generation, interactive element preservation, smart truncation, deduplication
- **Protocols**: AWP (native, 7 methods) and CDP (Puppeteer compatibility)

## Build from Source

```bash
git clone https://github.com/plasmate-labs/plasmate.git
cd plasmate
cargo build --release
./target/release/plasmate fetch https://example.com
```

Requirements: Rust 1.88+ for both default and `plugins` builds. V8 is fetched
automatically by rusty_v8. CI checks the declared minimum against default and
all-feature builds so dependency updates cannot silently raise it.

## Docker

Prebuilt multi-arch images (linux/amd64 and linux/arm64) are published to GHCR:

```bash
# Server mode (CDP or AWP)
docker run --rm -p 9222:9222 ghcr.io/plasmate-labs/plasmate:latest

# One-shot fetch
docker run --rm ghcr.io/plasmate-labs/plasmate:latest fetch https://example.com
```

Build locally:

```bash
docker build -t plasmate .
docker run --rm -p 9222:9222 plasmate
```

## Tests

```bash
cargo test --workspace    # all workspace tests
./scripts/action-manifest-conformance.sh
```

## Benchmarks

Run the built-in benchmark against cached pages:

```bash
cargo run --release -- bench --urls bench/urls.txt
```

Or test against live sites:

```bash
plasmate fetch https://en.wikipedia.org/wiki/Rust_(programming_language) | jq '.regions | length'
```

See [plasmate.app/compare](https://plasmate.app/compare) for the full comparison with Lightpanda and Chrome.

## Roadmap

See [docs.plasmate.app/roadmap](https://docs.plasmate.app/roadmap) for the full roadmap.

**v0.5 (current):**
- [x] Proxy support (HTTP, HTTPS, SOCKS5 with auth)
- [x] Proxy rotation (pool management, sticky sessions)
- [x] Iframe support
- [x] Shadow DOM support (declarative shadow DOM)
- [x] Native bounded ES-module core (same-origin static URL imports; see [scope and limitations](docs/ES-MODULES.md))
- [ ] Broader module compatibility (import maps, CORS graphs, dynamic import, import attributes, top-level await)
- [x] MCP cache/session observability for repeated agent workflows
- [ ] Parallel sessions at scale (500+ concurrent)

## Ecosystem

Plasmate has **60+ integrations** across the AI and developer ecosystem:

| Category | Integrations |
|----------|--------------|
| **AI Frameworks** | [LangChain](https://github.com/plasmate-labs/langchain-plasmate), [LlamaIndex](https://github.com/plasmate-labs/llamaindex-plasmate), [CrewAI](https://github.com/plasmate-labs/crewai-plasmate), [AutoGen](https://github.com/plasmate-labs/autogen-plasmate), [Haystack](https://github.com/plasmate-labs/haystack-plasmate), [DSPy](https://github.com/plasmate-labs/dspy-plasmate), [Semantic Kernel](https://github.com/plasmate-labs/semantic-kernel-plasmate), [Vercel AI](https://github.com/plasmate-labs/plasmate/tree/master/integrations/vercel-ai) |
| **Visual Builders** | [Langflow](https://github.com/plasmate-labs/langflow-plasmate), [Flowise](https://github.com/plasmate-labs/flowise-plasmate), [Dify](https://github.com/plasmate-labs/dify-plasmate) |
| **Automation** | [n8n](https://github.com/plasmate-labs/n8n-nodes-plasmate), [Zapier](https://github.com/plasmate-labs/zapier-plasmate), [Make.com](https://github.com/plasmate-labs/make-plasmate), [Activepieces](https://github.com/plasmate-labs/activepieces-plasmate), [Temporal](https://github.com/plasmate-labs/temporal-plasmate) |
| **Web Scraping** | [Scrapy](https://github.com/plasmate-labs/scrapy-plasmate), [Crawl4AI](https://github.com/plasmate-labs/crawl4ai-plasmate), [Firecrawl](https://github.com/plasmate-labs/firecrawl-plasmate), [ScrapeGraphAI](https://github.com/plasmate-labs/scrapegraphai-plasmate) |
| **Databases** | [Supabase](https://github.com/plasmate-labs/supabase-plasmate), [Prisma](https://github.com/plasmate-labs/prisma-plasmate), [PlanetScale](https://github.com/plasmate-labs/planetscale-plasmate), [Airtable](https://github.com/plasmate-labs/airtable-plasmate) |
| **Developer Tools** | [VS Code](https://github.com/plasmate-labs/vscode-plasmate), [Cursor](https://github.com/plasmate-labs/cursor-plasmate), [Raycast](https://github.com/plasmate-labs/raycast-plasmate), [GitHub Copilot](https://github.com/plasmate-labs/copilot-plasmate) |
| **Self-Hosted LLMs** | [Open WebUI](https://github.com/plasmate-labs/openwebui-plasmate), [OpenAI GPT Actions](https://github.com/plasmate-labs/openai-gpt-plasmate) |

See [awesome-plasmate](https://github.com/plasmate-labs/awesome-plasmate) for the full list.

## License

Apache-2.0. See [LICENSE](LICENSE).

Built by [Plasmate Labs](https://plasmate.app).
