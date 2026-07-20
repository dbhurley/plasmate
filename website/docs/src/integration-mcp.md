# Native MCP Server

Plasmate's native MCP server exposes structured page fetching, inspection, and
stateful interaction over stdio. Its source is
[`src/mcp/`](https://github.com/plasmate-labs/plasmate/tree/master/src/mcp), and
the authoritative tool registration is
[`src/mcp/server.rs`](https://github.com/plasmate-labs/plasmate/blob/master/src/mcp/server.rs).

The checked-in v0.6.0 Registry metadata is a next-release candidate. Do not
publish or install its OCI declaration until the newly labeled v0.6.0 image is
built and anonymously pullable. The npm and PyPI packages named `plasmate` are
client SDKs, not native server executables.

## Install the native engine

```bash
curl -fsSL https://plasmate.app/install.sh | sh
# or
cargo install plasmate
```

## Configure an MCP client

Use this stdio configuration with Claude Desktop, Cursor, Windsurf, or another
MCP-compatible client:

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

For Claude Code:

```bash
claude mcp add plasmate -- plasmate mcp
```

The server communicates over stdin/stdout using MCP JSON-RPC. Run it directly
for protocol debugging:

```bash
plasmate mcp
```

## Tool surface

The native server advertises its current tools through MCP `tools/list`; clients
should discover them rather than depend on a hard-coded count. The surface
includes:

- stateless fetch, text/link extraction, ARD discovery, crawl-policy, and page
  inspection;
- cache, session, trace, and validation-only replay operations;
- screenshots and persistent page sessions;
- navigation, evaluation, click/type/select/scroll/toggle/clear interactions;
- cookie read, write, and clear operations.

Sensitive mutations remain subject to the server's explicit policy and
workflow authorization controls.

## Output-size expectations

SOM removes presentation and runtime markup, but output reduction varies by
page, selector, JavaScript mode, budget, and corpus. Retained v0.5.1
observational snapshots recorded median serialized-byte ratios of 9.98x over 83
successful non-JavaScript inputs and 9.32x over 82 successful JavaScript inputs,
from 98 attempted URLs per run. These are byte ratios—not universal token,
cost, latency, or task-success guarantees. See the
[benchmark policy](https://github.com/plasmate-labs/plasmate/blob/master/docs/BENCHMARKING.md).
