# Module conformance fixture provenance

These fixtures are original, minimal Plasmate fixtures whose behavioral
assertions are derived from the Web Platform Tests module-script coverage:

- `html/semantics/scripting-1/the-script-element/module/`
- `html/semantics/scripting-1/the-script-element/moving-between-documents/`
- `html/webappapis/dynamic-markup-insertion/opening-the-input-stream/`

Upstream: <https://github.com/web-platform-tests/wpt/tree/master/html/semantics/scripting-1/the-script-element/module>

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
