# Responsible crawl-policy evaluation

Status: advisory evaluator, not a fetch gate
Result contract: `plasmate.crawl-policy.v1`
Standard snapshot: RFC 9309, checked July 19, 2026

Plasmate exposes crawl policy as an explicit decision report:

```bash
plasmate crawl-policy https://example.com/articles/1 \
  --product-token Plasmate \
  --output crawl-policy.json
```

MCP exposes the same operation as the read-only, strict-schema
`crawl_policy` tool. Ordinary `fetch`, `fetch_page`, navigation, and session
behavior is deliberately unchanged. A caller that is crawling multiple pages
must evaluate and enforce the returned decision in its own workflow.

## Normative behavior

The implementation follows the [IETF Robots Exclusion Protocol, RFC
9309](https://www.rfc-editor.org/rfc/rfc9309.html), checked July 19, 2026. It
also uses [Google's public parser behavior
documentation](https://developers.google.com/crawling/docs/robots-txt/robots-txt-spec),
checked July 19, 2026, as an interoperability reference for equally specific
group merging and field handling. RFC 9309 remains authoritative for the
versioned decision.

- `User-agent` field names and product-token matching are case-insensitive.
  Caller product tokens contain only ASCII letters, `_`, and `-`, as required
  by RFC 9309's `identifier` grammar.
- Every group with an exact product-token match is combined. `*` groups are
  combined only when no exact group exists. Substrings are not matches: a
  group for `Mate` does not apply to the `Plasmate` product token.
- Matching starts at the first path octet and includes the query component.
  `*` matches any sequence and terminal `$` anchors the end.
- Percent-encoded unreserved ASCII octets are decoded. Reserved and non-ASCII
  octets remain canonical uppercase percent encodings for comparison.
- The longest matching allow/disallow path wins. `Allow` wins an equal-length
  tie. Empty `Disallow` has no restrictive effect. `/robots.txt` is implicitly
  allowed.
- Comments, a leading UTF-8 BOM, malformed lines, and parseable rules around
  malformed lines are handled independently. A valid line may occupy the full
  500 KiB body floor and is still parsed and matched; only its report copy is
  truncated at 8 KiB and labeled `pattern_truncated`. At most 65,536 rules are
  retained, enough to cover the theoretical maximum number of minimum-length
  rules within the 500 KiB document bound.

`Crawl-delay`, `Sitemap`, `Agentmap`, and every other nonstandard record are
returned only as bounded advisory metadata. They never change the RFC
allow/disallow result.

## Network and failure semantics

The evaluator derives only `/<origin>/robots.txt`, fetches it through a fresh
public-network-only client, and identifies the HTTP request with the explicit
product token. The client ignores `PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK`,
revalidates every DNS result and redirect, and rejects cross-origin redirects
before requesting them.

That redirect rule is a deliberate security divergence. RFC 9309 recommends
following at least five consecutive redirects even when authority changes;
Plasmate follows up to five only while they remain same-origin, then reports a
conservative unreachable/deny result rather than expanding the SSRF surface.

- 2xx: `available`; parse the bounded body.
- 4xx: `unavailable`; RFC 9309 permits access.
- 5xx, timeout, DNS/connection error, invalid or excessive redirect chain:
  `unreachable`; deny access.
- compressed or decoded body above 500 KiB: `invalid_too_large`; deny
  conservatively rather than buffering or partially interpreting excess data.

The report includes one-source check denominators, byte counts, parsing
denominators, selected-group and considered-rule denominators, the winning
rule, and a trust label. Standalone JSON is capped at 128 KiB. The MCP handler
measures both the legacy and fully adapted modern result, including JSON-string
escaping, against that same complete-envelope ceiling.

## Trust boundary and limitations

Robots policy is voluntary advisory metadata. An `Allow` result is not
authentication, authorization, consent, a license, or evidence that an action
is safe. Callers must still satisfy site terms, legal obligations, credentials,
rate limits, data-minimization rules, and user authorization.

The evaluator does not cache policy, schedule revisits, enforce crawl rate,
modify normal fetches, inspect page-level robots meta tags, or treat
nonstandard directives as permission. The output records the standards
snapshot so future changes can produce a new explicit contract.
