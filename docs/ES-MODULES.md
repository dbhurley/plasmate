# ES modules

Plasmate executes a deliberately bounded ES-module core with V8's native
module compiler and evaluator. It does not transform module source into classic
scripts.

## Supported

- Inline and external `<script type="module">` roots.
- Static relative, root-relative, and absolute same-origin HTTP(S) imports.
- Native import/export bindings, strict module scope, cycles, and evaluate-once
  URL caching.
- Deferred root execution in document order after classic scripts.
- `import.meta.url` using the document URL for inline roots and the final
  same-origin response URL for external modules.
- Same-origin redirects, strict JavaScript response MIME checking, and bounded
  compressed and decoded UTF-8 source bodies.

JavaScript response types use the complete, ASCII-insensitive JavaScript MIME
type essence list from [WHATWG MIME Sniffing section 4.6](https://mimesniff.spec.whatwg.org/#javascript-mime-type).
This follows the HTML Standard's [single-module fetch](https://html.spec.whatwg.org/multipage/webappapis.html#fetch-a-single-module-script)
requirement that JavaScript modules have a JavaScript MIME type. Parameters such
as `charset=utf-8` are permitted on a response `Content-Type`; the HTML
`script[type]` classifier instead performs the standard's exact
[JavaScript MIME type essence match](https://html.spec.whatwg.org/multipage/scripting.html#attr-script-type).

External roots and dependencies require the existing external-script option.
Production module pages must use HTTPS. HTTP is accepted only when the explicit
`PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK=1` local-fixture/development escape hatch
is active.

## Limits and diagnostics

The default graph limit is 64 modules, depth 16, 1 MiB per compressed or
decoded module, 8 MiB aggregate decoded source, five redirects, five seconds
per fetch, and 15 seconds for graph acquisition. Module compilation and every
root evaluation share the runtime's single execution deadline; it is not
renewed per root. Diagnostics use the version `plasmate.js-modules.v1`; their
count, URLs, and messages are bounded, and URLs exclude credentials, query
strings, and fragments. Transport and resolution messages do not echo raw URLs.

## Explicitly unsupported

- Bare package specifiers and import maps.
- Cross-origin module graphs and CORS module loading.
- Dynamic `import()` (rejected by the native V8 host callback).
- Import attributes / JSON modules.
- Top-level `await`.

These cases produce diagnostics or evaluation failures; they are not silently
treated as successful module execution.

## Conformance scope

The focused corpus in `tests/fixtures/js-modules/` derives behavioral cases
from the Web Platform Tests module-script suites. It validates ordering,
strictness, static bindings, cycles, duplicate evaluation, `import.meta.url`,
syntax/runtime/fetch failures, MIME enforcement, origin and redirect policy,
and graph budgets. This is a targeted compatibility corpus, not a claim of full
Web Platform conformance.
