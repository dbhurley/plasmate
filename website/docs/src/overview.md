# Plasmate Documentation

Plasmate is an open-source, agent-native headless browser engine written in Rust.

It compiles HTML into a **Semantic Object Model (SOM)** - a structured, token-efficient representation that AI agents can reason about directly. It replaces CDP with a purpose-built **Agent Web Protocol (AWP)**.

## Quick Links

| Document | Description |
|----------|-------------|
| [MCP Registry](https://registry.modelcontextprotocol.io/)
Listed on the official MCP Registry. Install in Claude or Cursor with one click.

[Quick Start](quickstart) | Install Plasmate and run your first fetch in 60 seconds |
| [Product Spec](spec) | Full architecture, market analysis, and technical vision |
| [AWP Protocol](awp) | Agent Web Protocol draft specification |
| [AWP MVP v0.1](awp-mvp) | The seven-method foundational core; the current wire handler adds extension groups |
| [SOM Reference](som) | Semantic Object Model structure and element addressing |
| [Roadmap (v0.2)](roadmap) | V8 integration, CDP compatibility, SOM cache, parallel sessions |
| [Brand Guide](brand) | Colors, typography, voice, and the pixie dust system |

## Evidence policy

- Deterministic loopback release benchmarks retain task outcomes, cold/warm
  cache state, latency, memory high-water mark, and build/runner provenance.
- Observational public-web reports retain the complete attempted-input
  denominator, including blocked and failed inputs.
- Output reduction is page-, configuration-, corpus-, and serialization-dependent;
  byte ratios are not universal token, cost, latency, or task-success claims.
- Plasmate is Apache-2.0 licensed.

## Architecture at a Glance

```
Agent (Python/JS/Rust)
  |
  |-- AWP (seven foundational methods plus current extensions)
  |-- CDP (legacy compatibility)
  |
Plasmate Engine
  |-- HTML Parser (html5ever)
  |-- SOM Compiler (regions, elements, budgets)
  |-- V8 JS Runtime (script execution, DOM shim)
  |-- Network Layer (reqwest, connection pooling, HTTP/2)
  |-- Session Manager (cookie persistence, navigation history)
```

## Source

- GitHub: [plasmate-labs/plasmate](https://github.com/plasmate-labs/plasmate)
- License: Apache 2.0
- Website: [plasmate.app](https://plasmate.app)
