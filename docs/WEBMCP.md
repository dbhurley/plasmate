# WebMCP compatibility

Plasmate exposes WebMCP discovery as a progressive extension to its Semantic
Object Model. The internal contract is versioned as `plasmate.webmcp.v1`.
Discovered tools appear in the `webmcp` member of stateful MCP page responses;
the existing SOM `regions` member is unchanged.

## Supported in this slice

- Declarative `<form toolname tooldescription>` discovery.
- Input, textarea, select, required, label, `toolparamdescription`, option,
  radio, checkbox, number, multiple-select, and `toolautosubmit` metadata.
- Imperative `document.modelContext.registerTool()` discovery for inline and
  resolved external scripts executed by the normal page pipeline.
- Name, title, description, input schema, optional output schema, origin,
  top-frame ownership, read-only/mutating hints, confirmation requirements,
  and explicit untrusted-content classification.
- Structural schema limits, bounded proposed inputs and outputs, and a basic
  object/array/type/required/enum/const/anyOf/oneOf/allOf validator for future
  invocation work. Unsupported constraint keywords are rejected rather than
  presented as validated.
- A 256 KiB aggregate serialized-catalog ceiling. After stable name/id sorting
  and duplicate removal, excess tools are omitted from the end with an explicit
  warning. The same ceiling is enforced before a catalog enters page-state
  cache storage.
- Imperative registration metadata crosses the JavaScript process boundary as
  bounded JSON only. V8 handles and page callbacks never leave the supervised
  worker; a worker failure falls back to declarative discovery over source HTML.
- Secure HTTPS top-level origins. Cross-origin iframe documents are not fetched
  or exposed, even when their iframe declares `allow="tools"`.
- Disabling both API shapes when page code assigns `document.domain`.
- A case-insensitive marker fast path keeps ordinary pages out of the second
  html5ever parse when there are no imperative registrations or plausible
  declarative form annotations.

## Intentionally unsupported

All tools currently report `availability: "discovery_only"`.

Plasmate's page pipeline drops the V8 isolate after scripts run. Later MCP and
CDP interactions reconstruct a new DOM without re-running the application's
scripts. Executing an imperative callback in that new context would lose page
state, event listeners, cancellation, and callback identity. Re-running scripts
could duplicate application side effects. Both choices would violate WebMCP's
page-context lifecycle.

Declarative invocation is also deferred. Chrome brings the visible form into
focus, populates it, provides active-form styling and lifecycle events, lets the
user submit unless `toolautosubmit` is present, and marks agent submission on a
`SubmitEvent`. Plasmate does not yet retain a visible DOM session that can honor
those semantics. It therefore does not silently submit a form or invent a
headless substitute.

AbortSignal unregistration, `toolchange` events, cross-origin `fromOrigins` /
`exposedTo` discovery, and page-side `executeTool()` are not emulated. The
discovery shim only implements the registration behavior needed to capture
top-level metadata. Page code that depends on those APIs requires a retained
browser-compatible context in a later slice.

`exposedTo` metadata is accepted only as an exact HTTPS origin: no credentials,
non-root path, query, or fragment. It is validated for forward compatibility
but does not grant cross-origin access in this discovery-only slice.

Page-state cache entries persist the bounded WebMCP catalog alongside the SOM
and effective HTML, so imperative registrations from external scripts survive
an exact page-state cache hit. Legacy entries without a catalog recover only
declarative tools and carry an explicit partial-discovery warning.
Cache status accounts for stored and served WebMCP bytes separately from SOM
bytes.

Repeated radio fields with the same name merge their choice values. Repeated
text and checkbox fields collapse deterministically to the first compatible
property instead of producing empty, unsatisfiable choice arrays. This is a
Plasmate discovery normalization, not a claim that Chrome has standardized all
malformed or repeated-control edge cases.

Invocation should be added only after Plasmate retains or supervises an owning
page context. It must look up the tool inside that exact session, validate the
bounded input there, request confirmation for every mutating or unknown action,
and return bounded output labeled as untrusted web content.

## Trust and permissions

Tool descriptions, schemas, and outputs are page-controlled data. They remain
`untrusted_web_content` even if a page omits or clears its own
`untrustedContentHint`. Read-only declarations only relax the default
confirmation recommendation; they do not upgrade page content to trusted.

The custom runtime has one stable top-level origin and notices page assignments
to `document.domain`, but it cannot verify the HTTP `Origin-Agent-Cluster` and
Permissions-Policy response headers yet. The contract reports this limitation
and keeps invocation disabled. Plasmate does not expose cross-origin iframe
tools by default or implement a network WebMCP bridge.

## Specification sources

- [Chrome WebMCP overview](https://developer.chrome.com/docs/ai/webmcp)
- [Chrome imperative API](https://developer.chrome.com/docs/ai/webmcp/imperative-api)
- [Chrome declarative API](https://developer.chrome.com/docs/ai/webmcp/declarative-api)
- [Chrome agent security guidance](https://developer.chrome.com/docs/agents/security)
- [WebMCP standards proposal](https://github.com/webmachinelearning/webmcp)

The API is an origin trial and remains subject to change. Plasmate's versioned
contract insulates callers from experimental field changes while retaining the
source schema and explicit compatibility status.
