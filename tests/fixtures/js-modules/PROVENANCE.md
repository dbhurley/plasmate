# Module conformance fixture provenance

These fixtures are original, minimal Plasmate fixtures whose behavioral
assertions are derived from the Web Platform Tests module-script coverage.
The source snapshot is pinned to WPT commit
`ff372ea4a4b10d75830d4bad6445b2904d00a537` (checked 2026-07-20); this avoids
quietly moving conformance evidence when WPT `master` changes.

- `html/semantics/scripting-1/the-script-element/module/`
- `html/semantics/scripting-1/the-script-element/moving-between-documents/`
- `html/webappapis/dynamic-markup-insertion/opening-the-input-stream/`

The machine-readable case-to-test map, upstream source hashes, and local
fixture hashes are in [`wpt-corpus.json`](wpt-corpus.json). CI runs
`python3 scripts/check-wpt-module-corpus.py` to fail if a mapped assertion is
renamed, a local fixture changes without review, or provenance becomes
unpinned. Maintainers can additionally supply a checkout of that exact WPT
commit with `--upstream-root` to verify the recorded upstream hashes.

Upstream: <https://github.com/web-platform-tests/wpt/tree/ff372ea4a4b10d75830d4bad6445b2904d00a537/html/semantics/scripting-1/the-script-element/module>

Standards references:

- WHATWG MIME Sniffing, JavaScript MIME types and essence matching:
  <https://mimesniff.spec.whatwg.org/#javascript-mime-type>
- WHATWG HTML, the `script` element `type` classifier:
  <https://html.spec.whatwg.org/multipage/scripting.html#attr-script-type>
- WHATWG HTML, fetching a single module script and MIME enforcement:
  <https://html.spec.whatwg.org/multipage/webappapis.html#fetch-a-single-module-script>

WPT is distributed under the W3C 3-clause BSD License. No upstream source text
is copied here; only observable cases were reduced into local deterministic
fixtures. The suite intentionally covers Plasmate's documented subset rather
than implying full WPT conformance.
