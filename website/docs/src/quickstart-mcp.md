# Install the Native MCP Server

Install the native Plasmate engine first:

```bash
curl -fsSL https://plasmate.app/install.sh | sh
# or
cargo install plasmate
```

The npm and PyPI packages named `plasmate` are client SDKs; they do not install
the native MCP server executable.

## Claude Code

```bash
claude mcp add plasmate -- plasmate mcp
```

## Cursor, Claude Desktop, Windsurf, and other MCP clients

```json
{
  "mcpServers": {
    "plasmate": {
      "command": "plasmate",
      "args": ["mcp"]
    }
  }
}
```

Restart clients that do not reload MCP configuration automatically.

## Verify it works

Ask your agent to fetch `https://example.com`. A successful response contains a
structured SOM with regions, content, and interactive elements rather than raw
HTML.

The native server advertises its current capabilities with MCP `tools/list`.
Discover that surface instead of depending on a fixed tool count. It includes
stateless fetch and extraction, discovery and inspection, cache/session/trace
status, screenshots, stateful page interaction, cookies, and validation-only
replay.

SOM output size varies by page, selector, JavaScript mode, budget, and corpus.
Do not infer universal token, cost, latency, or task-success savings from a
single page. See the
[benchmark policy](https://github.com/plasmate-labs/plasmate/blob/master/docs/BENCHMARKING.md).

## v0.6.0 candidate status

The checked-in OCI Registry declaration is for the next v0.6.0 candidate. It is
not installable until the immutable, newly labeled v0.6.0 GHCR image is built
and anonymously pullable. The old v0.5.1 image must not be rebuilt or treated
as label-compliant.
