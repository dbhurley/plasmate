---
title: "Plasmate vs Firecrawl: Web Scraping for AI Agents Compared"
description: "Compare Plasmate and Firecrawl for LLM-ready web scraping across output structure, local deployment, MCP support, and page-dependent output size."
---

# Plasmate vs Firecrawl

Comparing two approaches to making web content digestible for AI agents.

## What Each Tool Does

**Plasmate** is a browser engine purpose-built for AI agents. It fetches web pages and compiles them into SOM (Semantic Object Model), a structured JSON format that captures meaning, not markup. Plasmate runs locally as a CLI, Docker container, or MCP server. Apache-2.0 open source.

**Firecrawl** is a hosted web scraping API that converts websites to LLM-ready markdown. It handles JavaScript rendering, crawling, and outputs clean markdown. Firecrawl offers a cloud API with usage-based pricing.

Both tools solve the same core problem: web pages are too noisy for LLMs. They diverge in how they solve it.

---

## Feature Comparison

| Feature | Plasmate | Firecrawl |
|---------|----------|-----------|
| **Output format** | SOM (structured JSON) | Markdown |
| **Output-size behavior** | Page-dependent structured SOM | Page-dependent markdown |
| **Deployment** | Local CLI, Docker, self-hosted | Cloud API |
| **JavaScript execution** | Yes (V8 engine) | Yes |
| **Pricing** | Free (open source) | API pricing (usage-based) |
| **Protocol support** | MCP, CDP, AWP | Hosted API and documented integrations |
| **License / terms** | Apache-2.0 | Review the current open-source and hosted-service terms |
| **Structured data** | SOM roles, regions, forms, and actions | Markdown and configured extraction formats |
| **Action annotations** | Yes (click, type, select) | No |
| **Self-hosted option** | Native local runtime | Review current Firecrawl self-hosting support and feature coverage |

---

## Output Format: SOM vs Markdown

The key architectural difference is output format.

**Firecrawl** produces markdown. Markdown is readable and works well for content extraction, but it's fundamentally unstructured. A link in markdown is just `[text](url)`. You don't know if it's navigation, a button, or a content link. Forms become plain text.

**Plasmate** produces SOM, a JSON structure with semantic roles, regions, and explicit action annotations:

```json
{
  "role": "link",
  "text": "Sign Up",
  "attrs": { "href": "/signup" },
  "actions": ["click"],
  "region": "navigation"
}
```

For agents that need to act on pages (clicking, filling forms, navigating), SOM provides the structured data that markdown cannot. For pure text extraction, markdown may suffice.

---

## Output size: why measurement matters

Raw HTML, markdown, and SOM have different schemas, and byte counts do not map
to a fixed number of model tokens or dollars. In the v0.5.1 Plasmate
observational snapshots, serialized SOM was smaller than raw HTML by a median
9.98x across 83 successful non-JavaScript inputs out of 98 attempted and by a
median 9.32x across 82 successful JavaScript inputs out of 98 attempted.
Firecrawl was not measured in those runs. These page-corpus byte ratios are not
universal token, cost, latency, or task-success guarantees. Compare both outputs
on the same pages with the tokenizer and pricing used by your application.

---

## Deployment Model

**Plasmate** runs locally by default:

```bash
# CLI
plasmate fetch https://example.com

# Docker
docker run -p 9222:9222 plasmate/browser

# MCP server (for Claude Code, Cursor, etc.)
plasmate mcp
```

No hosted-service API key is required, and URLs can remain on your
infrastructure. Local capacity and latency still depend on the page, network,
hardware, JavaScript work, and configured concurrency.

**Firecrawl** is API-first:

```bash
curl -X POST https://api.firecrawl.dev/v0/scrape \
  -H "Authorization: Bearer fc-..." \
  -d '{"url": "https://example.com"}'
```

This managed path requires credentials and sends requested URLs through the
service. End-to-end latency depends on both the service and the target site.

---

## Protocol Support

**Plasmate** supports multiple protocols:
- **MCP** (Model Context Protocol): First-class integration with Claude Code, Cursor, and other MCP clients
- **CDP** (Chrome DevTools Protocol): Drop-in replacement for Puppeteer/Playwright workflows
- **AWP** (Agent Web Protocol): Purpose-built WebSocket protocol for agents

**Firecrawl** exposes a hosted API and integrations. Check its current
documentation for the protocols and agent frameworks supported by the version
you plan to deploy.

---

## Code Examples

### Fetch a page and extract content

**Plasmate (CLI):**
```bash
plasmate fetch https://news.ycombinator.com
```

**Plasmate (Python with AWP):**
```python
import asyncio
import websockets
import json

async def fetch_page():
    async with websockets.connect("ws://127.0.0.1:9222") as ws:
        await ws.send(json.dumps({
            "id": 1,
            "method": "page.navigate",
            "params": {"url": "https://news.ycombinator.com"}
        }))
        result = json.loads(await ws.recv())
        return result["result"]["som"]

som = asyncio.run(fetch_page())
print(f"Title: {som['title']}")
print(f"Regions: {len(som['regions'])}")
```

**Firecrawl (Python):**
```python
import requests

response = requests.post(
    "https://api.firecrawl.dev/v0/scrape",
    headers={"Authorization": "Bearer fc-..."},
    json={"url": "https://news.ycombinator.com"}
)
result = response.json()
print(result["data"]["markdown"])
```

### With Claude Code (MCP)

**Plasmate:** Native MCP support. Claude Code can browse the web directly:

```bash
# Start MCP server
plasmate mcp

# In Claude Code, Plasmate tools are available automatically
```

**Firecrawl:** Requires custom MCP wrapper or manual API calls in code.

---

## When to Use Plasmate

- **Local-first workflows**: No API keys, no external dependencies, no data egress
- **Structured-context agents**: SOM exposes semantic roles and actions; measure token use on your pages
- **Agent automation**: Structured actions (click, type, select) for tool-use agents
- **MCP integration**: First-class support for Claude Code, Cursor, and other MCP clients
- **Self-hosted requirements**: Run entirely on your infrastructure
- **Self-managed volume**: No hosted per-page charge, but you operate and size the runtime

## When to Use Firecrawl

- **Managed prototypes**: Use an API instead of operating a local service
- **Managed service preference**: Let someone else handle infrastructure
- **Markdown output needed**: If your pipeline expects markdown specifically
- **Crawling features**: Firecrawl has built-in site crawling and sitemap handling
- **No local resources**: When you can't or don't want to run local processes

---

## Summary

Plasmate and Firecrawl take different approaches to the same problem.

Firecrawl is a hosted API that produces markdown. Quick to integrate, no infrastructure to manage, usage-based pricing.

Plasmate is a local tool that produces structured SOM and is open source.
Firecrawl provides managed markdown extraction. Suitability and output size
depend on the target pages and downstream workflow.

If you need structured action metadata, MCP integration, or a local-first
runtime, evaluate Plasmate. If you prefer managed markdown extraction, evaluate
Firecrawl. Compare both on representative pages before choosing.

---

## Get Started with Plasmate

```bash
# Install
curl -fsSL https://plasmate.app/install.sh | sh

# Fetch your first page
plasmate fetch https://example.com

# Start MCP server for Claude Code
plasmate mcp
```

[Read the docs](https://plasmate.app/docs/overview) | [View on GitHub](https://github.com/plasmate-labs/plasmate)
