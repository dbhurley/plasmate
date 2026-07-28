# Plasmate Integration Directory - Draft Thread

> Evidence review: 2026-07-28. This draft intentionally makes no numeric
> integration, token, cost, speed, memory, or compatibility claim. Verify every
> linked adapter and package before publishing.

## Tweet 1 (Hook)

Plasmate has a growing directory of adapters and examples across the AI
ecosystem.

The common contract is structured SOM output for agent web access.

Here's what the ecosystem looks like (thread)

**[Image: Plasmate logo with integration logos arranged around it - LangChain, Vercel AI, CrewAI, n8n, Scrapy, etc. arranged in a wheel pattern]**

---

## Tweet 2 (The Problem)

Why does this matter?

Raw HTML can contain runtime and presentation markup that a given agent task
does not need. SOM makes the retained semantic structure explicit.

The retained v0.5.1 non-JavaScript snapshot measured a 9.98x median
serialized-byte ratio over 83 successful inputs from 98 attempts. That is
historical byte evidence—not token, cost, latency, information-equivalence, or
task-success evidence.

---

## Tweet 3 (AI Frameworks)

AI framework adapters and examples include:

- LangChain - document loaders and tools to evaluate
- LlamaIndex - native data connectors
- CrewAI - web browsing for agent crews
- AutoGen - multi-agent web research
- Vercel AI SDK - one-liner MCP integration
- Haystack, DSPy, Semantic Kernel

All use the same SOM output format. Learn once, use everywhere.

**[Image: Code snippet showing LangChain PlasmateFetchTool - 5 lines of code to add web browsing]**

---

## Tweet 4 (Browser Automation)

Browser automation adapters and examples include:

- Browser Use: structured SOM context alongside Browser Use
- Scrapy: spider middleware for SOM extraction
- Crawl4AI: structured scraping at scale
- Firecrawl: adapter for web-research workflows

Measure configured Browser Use and SOM workflows on the same pages, tokenizer,
model, and tasks before making a cost or quality comparison.

---

## Tweet 5 (No-Code/Low-Code)

No-code and automation directory entries include:

- n8n - native Plasmate node
- Zapier - web parsing actions
- Make.com - scenario components
- Langflow - visual agent builder
- Flowise - drag-and-drop chains
- Dify - workflow blocks
- Activepieces - automation pieces

Build web-aware AI workflows without writing code.

**[Image: n8n workflow canvas showing Plasmate node connected to OpenAI and Slack nodes]**

---

## Tweet 6 (Developer Tools)

Developer-tool directory entries include:

- VS Code extension
- Cursor integration
- Raycast commands
- GitHub Copilot extension

Native MCP can connect to clients that support the same transport and server
configuration. Verify client-specific support.

One config line:
```
"plasmate": { "command": "plasmate", "args": ["mcp"] }
```

---

## Tweet 7 (SDKs)

SDKs and clients include:

- Node.js (npm install plasmate)
- Python (pip install plasmate)
- Go (go get github.com/nickel-org/plasmate-go)
- Rust (cargo install plasmate)

Full TypeScript types, async/await, query helpers for traversing SOM documents.

All SDKs spawn `plasmate mcp` and communicate via JSON-RPC over stdio. Zero network config.

---

## Tweet 8 (Performance)

Plasmate runs without installing a full visual browser. Latency, memory,
throughput, binary size, and deployment fit depend on the build, page,
JavaScript, cache, runner, and concurrency. Profile the intended workload.

---

## Tweet 9 (Call to Action)

Try it in 30 seconds:

```bash
curl -fsSL https://plasmate.app/install.sh | sh
plasmate fetch https://news.ycombinator.com | jq
```

Star the repo: github.com/plasmate-labs/plasmate

Community integration directory: github.com/plasmate-labs/awesome-plasmate

We're building the browser engine for the agentic web. Join us.

**[Image: Terminal screenshot showing Plasmate fetch output - clean SOM JSON structure with regions, elements, and compression stats]**
