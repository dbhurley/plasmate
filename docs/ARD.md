# Static Agentic Resource Discovery

Status: experimental, discovery only  
Result contract: `plasmate.ard.discovery.v1`  
Specification snapshot: ARD v0.9 draft/proposal dated May 28, 2026; checked July 19, 2026  
Catalog envelope accepted: `specVersion: "1.0"`

Plasmate implements a deliberately narrow static-discovery client for the
[official ARD v0.9 draft](https://agenticresourcediscovery.org/spec/) and its
[AI Catalog foundation](https://agenticresourcediscovery.org/ai_catalog_spec/).
The official draft separates discovery from invocation. Plasmate preserves
that boundary: the result is an inventory for operator review, not permission
to execute a capability.

## Interfaces

```bash
plasmate ard-discover https://example.com/ --output ard-report.json
```

The MCP server exposes the equivalent read-only `ard_discover` tool. Both
interfaces accept one operator-supplied HTTPS page or origin and one total
discovery wall deadline from 1 through 30,000 milliseconds. Every probe and
catalog fetch receives only the time remaining. Sources and catalogs that the
deadline prevents from starting are reported as `deadline_omitted`. A run with
no accepted catalog exits with code 2 in the CLI while still writing the full
report.

## Discovery scope

Every run reports all three source checks and their failures:

1. `https://<origin>/.well-known/ai-catalog.json`
2. HTML `<link rel="ai-catalog" href="...">`
3. `Agentmap: ...` in the origin's robots.txt

The well-known candidate is always attempted first. Remaining HTML and robots
candidates are deduplicated and ordered lexically. Catalog references must be
HTTPS and same-origin; redirects are revalidated and must remain same-origin.
Cross-origin references are reported as rejected and are never followed.

DNS-based ARD discovery is not implemented. Neither are registry `/search`,
federation, `/explore`, `/agents`, recursive catalog crawling, entry endpoint
fetching, installation, or invocation.

## Validation and limits

The static path uses a dedicated public-network-only HTTP client. It applies
Plasmate's centralized DNS/connect-time address checks, redirect checks, and
compressed/decoded body limits. It intentionally ignores
`PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK`, including when called through MCP.

- 3 redirects per resource
- 512 KiB decoded HTML or robots.txt
- 256 KiB decoded catalog
- 8 unique catalogs, with the well-known URI reserved first
- 8 HTML links and 8 Agentmap directives
- 128 entries per catalog
- 24 JSON levels, 16,384 JSON nodes, and 128 members/items per container
- 8 KiB maximum generic string; tighter field-specific limits
- 32 KiB inline `data`; 16 KiB `metadata` or `trustManifest`
- 512 KiB serialized standalone JSON report
- 512 KiB serialized MCP tool result after protocol metadata, structured-content handling, and JSON-string escaping

The report preserves complete source/catalog/entry denominators. If optional
payloads or diagnostics would exceed an output limit, it omits them in a
deterministic order and increments explicit omission counters. The MCP path
measures the complete modern protocol result rather than assuming that the
standalone report size is sufficient; hostile quotes and backslashes can grow
when the report is embedded as an MCP text string.

Catalogs require `specVersion`, `host`, and `entries`. Entries require a
syntactically valid domain-anchored `urn:air:` identifier, display name, media
type, and exactly one of HTTPS `url` or object `data`. Duplicate identifiers
are rejected deterministically. A publisher domain that differs from the
catalog host is retained with an unverified diagnostic because the official
draft includes legitimate cross-host publishing examples.

## Trust boundary

Catalog text is hostile input. Plasmate never turns descriptions, representative
queries, metadata, inline data, or trust manifests into instructions. Every
catalog and wrapped JSON field is labeled `untrusted_unverified`, with
verification `not_performed`. A claimed signature, DID, SPIFFE identity,
attestation, provenance record, score, or publisher domain is not proof of
identity, safety, compliance, relevance, or authorization.

Before separately fetching or invoking a discovered resource, an operator or
higher-trust component must approve its destination and independently verify
publisher identity, authentication, authorization, signatures, attestations,
and the resource protocol.
