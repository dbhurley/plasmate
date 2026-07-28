# Archived March 2026 Benchmark Note

> **Historical evidence only.** This note describes exploratory Plasmate v0.1.0
> work from March 2026. The original per-run artifacts, commit, corpus digest,
> runner detail, and complete public-web outcomes were not retained to today's
> evidence standard. Historical latency, output-size, token-estimate, and site
> comparison figures have therefore been retired.

## What the campfire-commerce fixture checked

A local Puppeteer script navigated to a fixture product page, waited for
JavaScript-rendered product price and XHR-provided reviews, and extracted
structured data. The historical run reported that the fixture assertions
passed. The exercised path included:

- the CDP WebSocket endpoint;
- `page.goto()` through the JavaScript pipeline;
- `page.waitForFunction()` polling;
- `page.evaluate()` for the fixture's DOM queries;
- XHR/fetch through the V8 bridge; and
- serialization of the JavaScript-mutated document.

This is evidence for that fixture and those assertions only. It is not evidence
of full CDP, Puppeteer, Playwright, Chromium, or web-platform compatibility, and
it is not a current latency or reliability benchmark.

## Current evidence

Use the [public claim and evidence registry](claims) for currently allowed
wording. New benchmark work must follow the
[benchmark policy](https://github.com/plasmate-labs/plasmate/blob/master/docs/BENCHMARKING.md)
and retain independently validatable reports with complete denominators and
provenance.
