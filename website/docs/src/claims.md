# Public Claim & Evidence Registry

Plasmate separates product facts, historical observations, compatibility
scope, and ecosystem directory entries. Quantitative claims are allowed only
when a retained artifact supports the exact metric, denominator, provenance,
and limitations.

The machine-readable source of truth is
[`claims/evidence.v1.json`](https://github.com/plasmate-labs/plasmate/blob/master/claims/evidence.v1.json).
The governing methodology is
[`docs/BENCHMARKING.md`](https://github.com/plasmate-labs/plasmate/blob/master/docs/BENCHMARKING.md).

## Currently allowed quantitative wording

- **Non-JavaScript historical snapshot:** The retained v0.5.1 public-web
  snapshot generated July 19, 2026 recorded a 9.98x median serialized-byte
  ratio over 83 successful inputs from 98 attempted URLs.
- **JavaScript-enabled historical snapshot:** The retained v0.5.1 public-web
  snapshot generated June 7, 2026 recorded a 9.32x median serialized-byte ratio
  over 82 successful inputs from 98 attempted URLs.

Both are historical observational results. They lack the commit, corpus digest,
and runner provenance required of current evidence. They measure raw HTML bytes
divided by serialized SOM bytes over successful inputs. They do not establish
token savings, model cost, latency, memory, information equivalence, or task
success. Blocked and failed inputs remain in each 98-URL attempted denominator.

## Compatibility wording

Use:

> Plasmate provides a CDP compatibility layer for the supported Puppeteer/CDP
> workflows and methods documented in the README.

Do not imply full CDP, Puppeteer, Playwright, Chromium, or web-platform
compatibility. A benchmark-fixture pass validates that fixture and its listed
assertions, not an entire ecosystem.

## Ecosystem wording

The README and documentation provide an integration directory. Inclusion does
not establish current maintenance, end-to-end testing, or compatibility with
every upstream version. Plasmate does not publish a numeric integration count
until a dated manifest defines inclusion and verification criteria.

## Retired claims

Do not publish universal claims such as “10x token compression,” “17x average
token reduction,” “10–100x fewer tokens,” “50x faster than Chrome,” fixed
cross-product memory comparisons, or universal cost savings. Those claims
require a named tokenizer or model, comparable workload, build and runner
metadata, cache state, complete outcomes, and retained evidence.

## How to add a claim

1. Retain the report or conformance artifact in a stable public location.
2. Record the exact metric and all attempted, successful, blocked, failed,
   crashed, and timed-out denominators.
3. Record version, commit, dirty state, corpus digest, configuration, build
   profile, runner, cache state, and measurement date.
4. Add allowed wording and limitations to the machine-readable registry.
5. Run the release validation and relevant benchmark validator before
   publishing.

## Externally controlled surfaces

As checked on July 28, 2026, the live GitHub repository description still says
“10x token compression,” and the repository topics include
`token-compression`. Those settings are not changed by a source commit. Replace
the description with the wording in the machine-readable registry and replace
that topic with `structured-output` or `semantic-browser`.

The live crates.io, npm, and PyPI summaries did not contain the retired numeric
claims at review time. Future source-metadata improvements reach those
registries only through normal versioned package releases.
