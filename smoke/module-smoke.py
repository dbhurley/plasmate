#!/usr/bin/env python3
"""Deterministic end-to-end smoke test for the native ES-module pipeline."""

import http.server
import os
import subprocess
import sys
import threading


PAGE = """<!doctype html>
<html><body>
<div id="module-result">pending</div>
<script>globalThis.moduleOrder = ['classic-before'];</script>
<script type="module" src="/main.js"></script>
<script>moduleOrder.push('classic-after');</script>
</body></html>"""

MAIN = """import { answer, increment } from './dependency.js';
increment();
document.getElementById('module-result').textContent =
  `module-ok:${answer}:${moduleOrder.join('|')}:${import.meta.url.endsWith('/main.js')}`;
"""

DEPENDENCY = """export let answer = 41;
export function increment() { answer += 1; }
"""


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/main.js":
            body, content_type = MAIN, "text/javascript"
        elif self.path == "/dependency.js":
            body, content_type = DEPENDENCY, "text/javascript"
        else:
            body, content_type = PAGE, "text/html; charset=utf-8"
        encoded = body.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_args):
        pass


def main():
    binary = os.environ.get("PLASMATE_BIN", "./target/release/plasmate")
    if not os.path.exists(binary):
        print(f"Binary not found: {binary}", file=sys.stderr)
        return 1

    server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f"http://127.0.0.1:{server.server_port}/page"
    env = os.environ.copy()
    env["PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK"] = "1"
    try:
        result = subprocess.run(
            [binary, "fetch", url],
            env=env,
            text=True,
            capture_output=True,
            timeout=60,
            check=False,
        )
    finally:
        server.shutdown()
        server.server_close()

    expected = "module-ok:42:classic-before|classic-after:true"
    if result.returncode != 0:
        print(result.stderr, file=sys.stderr)
        return result.returncode
    if expected not in result.stdout:
        print(f"Expected module marker not found: {expected}", file=sys.stderr)
        print(result.stdout[-4000:], file=sys.stderr)
        return 1
    print("ES module smoke: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
