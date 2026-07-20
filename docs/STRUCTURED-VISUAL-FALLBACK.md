# Structured-first visual fallback

Status: bounded read-only MCP inspection
Result contract: `plasmate.structured-inspection.v1`
Design sources checked: July 19, 2026

The `inspect_page` MCP tool makes Plasmate's product position explicit:
structured page state is the default; pixels are a selective fallback.

The design was checked against Playwright MCP's [structured snapshot
introduction](https://playwright.dev/mcp/introduction), [screenshot
guidance](https://playwright.dev/mcp/tools/screenshots), and [vision
mode](https://playwright.dev/mcp/vision-mode) on July 19, 2026. Those primary
documents likewise reserve screenshots and coordinate tools for content not
represented by structured accessibility state. Chrome's [WebMCP overview](https://developer.chrome.com/docs/ai/webmcp),
[declarative API](https://developer.chrome.com/docs/ai/webmcp/declarative-api),
and [imperative API](https://developer.chrome.com/docs/ai/webmcp/imperative-api)
were checked the same day; they reinforce the direction toward typed,
site-declared structure.

Stagehand's official [`observe()`
documentation](https://docs.stagehand.dev/v3/basics/observe), checked July 19,
2026, separately emphasizes returning structured candidate actions that can be
validated before `act()`. Plasmate does not copy Stagehand's model-driven
selector planning, but the inspect-then-authorize separation informs this
read-only fallback contract.

## Contract

Every successful inspection puts a bounded compact SOM in the first text
content block. It includes source URLs, original and returned element counts,
interactive counts, region identity, stable element IDs, roles, bounded
text/labels, actions, and explicit omission counters.

Page JavaScript is disabled by default. Setting `javascript=true` explicitly
opts into Plasmate's existing in-process V8 pipeline before the screenshot
subprocess is started. That pipeline can improve dynamic-page structure, but it
still carries the documented residual risk that a fatal V8 failure can end the
long-lived MCP server. Chrome process containment does not isolate this earlier
pipeline execution.

`visual_mode` has exactly three values:

- `never`: never starts Chrome. It can recommend a screenshot and names why.
- `auto` (default): starts Chrome only when deterministic signals say the SOM
  is insufficient.
- `always`: the caller explicitly requests a screenshot; the trigger is
  reported as `explicit_always_mode`.

Current auto signals are deliberately small and testable:

- `meaningful_structure_empty`
- `meaningful_structure_near_empty`
- `canvas_heavy_structure`
- `image_map_or_image_control_evidence`

A normal semantic page produces no screenshot in `auto`. Signals are
recommendations about representational coverage, not semantic conclusions.

## Browser and output safety

The page is fetched once through Plasmate's centralized outbound policy and
processed by the SOM pipeline, with JavaScript only on explicit opt-in. Chrome
never navigates to the caller URL. It receives a generated top-level file in a
Plasmate-owned temporary directory; the already-fetched effective HTML is escaped into a
`sandbox` iframe's `srcdoc` and receives a first-parsed CSP with
`default-src 'none'`, `script-src 'none'`, `frame-src 'none'`, and narrowly
scoped inline-style/data-image allowances.

The Chrome command also includes `--disable-javascript`. HTTP(S) subresources
are sent to `http://127.0.0.1:9` with `--proxy-bypass-list=<-loopback>`, and
Plasmate never adds `--allow-file-access-from-files`, `--disable-web-security`,
or `--no-sandbox`. Chromium's current source documents that file URLs cannot
read other file URLs by default and that `--allow-file-access-from-files` is
the unsafe developer override; see Chromium's
[`content_switches.cc`](https://chromium.googlesource.com/chromium/src/+/master/content/public/common/content_switches.cc).
The sandbox, inner CSP, restrictive Chromium default, and dead proxy are
defense in depth. Tests compare rendered pixels to prove that a script and a
cross-file SVG cannot change the hardened screenshot.

Chrome is spawned in a dedicated process group (`process_group(0)` on Unix;
`CREATE_NEW_PROCESS_GROUP` on Windows). Success, timeout, output overflow, and
error paths terminate the whole tree and wait for the root process. Plasmate
only accepts a PNG after its complete `IEND` trailer is present.

- viewport width: 320–1,920 pixels
- viewport height: 200–1,080 pixels
- screenshot process timeout: 100–10,000 ms (default 5,000)
- raw PNG: at most 192 KiB
- compact SOM: at most 256 returned elements and 32 regions
- complete legacy and modern MCP result: at most 512 KiB after base64,
  protocol metadata, structured-content handling, and JSON-string escaping

If Chrome is missing, times out, fails, or produces an oversized image, the
tool still returns the SOM plus a typed `visual.failure`. If the combined
envelope is too large, the image is omitted first and the SOM remains. Raw
Chrome stderr is never returned by this tool.

## Trust and interpretation boundary

SOM fields and screenshot pixels are untrusted page data, not instructions.
Plasmate does not run a vision model, infer meaning from pixels, click by
coordinate, or claim that an image is safe. A consuming model may inspect the
optional image under its own authorization and prompt-injection controls. Any
subsequent mutation must use a separately authorized action surface.
