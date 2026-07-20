---
title: "60+ Integrations: The Plasmate Ecosystem"
date: 2026-04-15
author: Plasmate Team
description: "Plasmate integrates with tools across AI frameworks, visual builders, automation platforms, and more. Learn how to add structured SOM browsing to LangChain, Vercel AI, and other tools."
keywords: ["AI agent web scraping", "LangChain web browser", "token compression", "MCP web browsing tool"]
---

# 60+ Integrations: The Plasmate Ecosystem

Building AI agents that browse the web? Raw HTML can include verbose markup,
scripts, styles, and other material that a particular agent workflow may not need.

Plasmate compiles HTML into a Semantic Object Model (SOM), a structured
representation of page regions, content, and supported actions. Output size is
page-dependent. In the v0.5.1 observational snapshots, the median serialized-byte
ratio was 9.98x across 83 successful non-JavaScript inputs out of 98 attempted
and 9.32x across 82 successful JavaScript inputs out of 98 attempted. These are
byte ratios, not universal token, cost, latency, or task-success guarantees.

Today, we are excited to announce a major milestone: **Plasmate now integrates with 60+ tools across 7 categories**, making it easier than ever to add intelligent web browsing to your AI stack.

## The Ecosystem at a Glance

Whether you are building with Python frameworks, visual workflow builders, or self-hosted LLMs, there is a Plasmate integration ready for you.

### AI Frameworks

The backbone of most agent architectures. Plasmate plugs directly into the frameworks you already use, giving your agents efficient web access without changing your existing code patterns.

- **[LangChain](https://github.com/nicholasquirk/plasmate-langchain)** - Drop-in tool for agents and chains. Use `PlasmateLoader` for document loading or `PlasmateTool` for agent browsing.
- **[LlamaIndex](https://github.com/nicholasquirk/plasmate-llamaindex)** - Native reader integration for RAG pipelines. Index web content with automatic SOM transformation.
- **[CrewAI](https://github.com/nicholasquirk/plasmate-crewai)** - Equip your AI crews with web research capabilities. Perfect for multi-agent research tasks.
- **[AutoGen](https://github.com/nicholasquirk/plasmate-autogen)** - Microsoft's multi-agent framework gets efficient web browsing. Ideal for collaborative agent systems.
- **[Haystack](https://github.com/nicholasquirk/plasmate-haystack)** - Custom component for deepset's NLP framework. Build search and QA pipelines with web data.
- **[DSPy](https://github.com/nicholasquirk/plasmate-dspy)** - Programmatic LLM pipelines with web retrieval. Great for structured extraction tasks.
- **[Semantic Kernel](https://github.com/nicholasquirk/plasmate-semantic-kernel)** - Microsoft's SDK for AI orchestration. Native plugin architecture support.
- **[Vercel AI SDK](https://github.com/nicholasquirk/plasmate/tree/main/integrations/vercel-ai)** - First-class TypeScript support for Next.js and React applications. Stream web content directly into your UI.

### Visual Builders

Not everyone wants to write code. These low-code platforms let you build AI workflows visually, and now they can browse the web efficiently.

- **[Langflow](https://github.com/nicholasquirk/plasmate-langflow)** - Drag-and-drop LangChain workflows with Plasmate nodes. Build web-aware agents without writing Python.
- **[Flowise](https://github.com/nicholasquirk/plasmate-flowise)** - Visual LLM workflow builder with Plasmate integration. Deploy chatbots that can research the web.
- **[Dify](https://github.com/nicholasquirk/plasmate-dify)** - Open-source LLMOps platform. Add web browsing to your Dify apps with a single node.

### Automation Platforms

Connect AI web browsing to your existing workflows. Trigger Plasmate from webhooks, schedules, or any of the thousands of apps these platforms support.

- **[n8n](https://github.com/nicholasquirk/plasmate-n8n)** - Self-hostable automation with a Plasmate node. Perfect for data pipelines and monitoring tasks.
- **[Zapier](https://github.com/nicholasquirk/plasmate-zapier)** - Connect Plasmate to 5,000+ apps. Trigger web scraping from emails, forms, or database updates.
- **[Make.com](https://github.com/nicholasquirk/plasmate-make)** - Visual automation with Plasmate modules. Build complex multi-step web research workflows.
- **[Activepieces](https://github.com/nicholasquirk/plasmate-activepieces)** - Open-source Zapier alternative with Plasmate pieces. Full control over your automation infrastructure.
- **[Temporal](https://github.com/nicholasquirk/plasmate-temporal)** - Durable workflow execution with Plasmate activities. Build reliable, long-running web scraping jobs.

### Web Scraping Tools

Already using a scraping framework? Plasmate integrates with the tools you know, adding semantic understanding on top of raw extraction.

- **[Scrapy](https://github.com/nicholasquirk/plasmate-scrapy)** - Middleware for Python's most popular scraping framework. Add SOM transformation to existing spiders.
- **[Crawl4AI](https://github.com/nicholasquirk/plasmate-crawl4ai)** - AI-first web crawler with native Plasmate support. Built for LLM-friendly output from day one.
- **[Firecrawl](https://github.com/nicholasquirk/plasmate-firecrawl)** - Modern web scraping API with Plasmate integration. Get clean markdown and SOM from any URL.
- **[ScrapeGraphAI](https://github.com/nicholasquirk/plasmate-scrapegraphai)** - Graph-based scraping with LLM intelligence. Plasmate provides the semantic layer.

### Databases and Storage

Store and query web content efficiently. These integrations help you build knowledge bases from web data.

- **[Supabase](https://github.com/nicholasquirk/plasmate-supabase)** - PostgreSQL with vector search. Store SOM documents and query them semantically.
- **[Prisma](https://github.com/nicholasquirk/plasmate-prisma)** - Type-safe database access for SOM data. Define schemas for your extracted web content.
- **[PlanetScale](https://github.com/nicholasquirk/plasmate-planetscale)** - Serverless MySQL for web content storage. Scale your web data infrastructure automatically.
- **[Airtable](https://github.com/nicholasquirk/plasmate-airtable)** - Spreadsheet-database hybrid for structured web data. Great for non-technical team collaboration.

### Developer Tools

Bring AI web browsing into your development environment. Read documentation, research APIs, and explore codebases from your editor.

- **[VS Code](https://marketplace.visualstudio.com/items?itemName=plasmate.plasmate-vscode)** - Extension for web research without leaving your editor. Fetch documentation directly into your workspace.
- **[Cursor](https://github.com/nicholasquirk/plasmate)** - MCP server integration for Cursor's AI features. Give your AI assistant efficient web access.
- **[Raycast](https://github.com/nicholasquirk/plasmate-raycast)** - Quick web lookups from your launcher. Perfect for fast research during development.
- **[GitHub Copilot](https://github.com/nicholasquirk/plasmate-copilot)** - Extend Copilot with web context. Research APIs and libraries while you code.

### Self-Hosted LLMs and Chat Interfaces

Running your own models? These integrations bring Plasmate to your self-hosted AI infrastructure.

- **[Open WebUI](https://github.com/nicholasquirk/plasmate-openwebui)** - Popular self-hosted chat interface with Plasmate tools. Give your local LLMs web access.
- **[OpenAI GPT Actions](https://github.com/nicholasquirk/plasmate-gpt-actions)** - Custom GPT with web browsing capabilities. Deploy Plasmate as an OpenAI-compatible endpoint.

## Getting Started

Adding Plasmate to your project takes minutes. Here are examples for two popular frameworks.

### LangChain (Python)

```python
from langchain.agents import initialize_agent, AgentType
from langchain_openai import ChatOpenAI
from plasmate_langchain import PlasmateTool

# Create the Plasmate browsing tool
plasmate = PlasmateTool()

# Initialize your agent with web browsing
agent = initialize_agent(
    tools=[plasmate],
    llm=ChatOpenAI(model="gpt-4"),
    agent=AgentType.OPENAI_FUNCTIONS,
)

# Your agent can now browse efficiently
result = agent.run("What are the top features of the new Python 3.13 release?")
```

### Vercel AI SDK (TypeScript)

```typescript
import { generateText } from 'ai';
import { openai } from '@ai-sdk/openai';
import { plasmate } from 'plasmate-ai';

const result = await generateText({
  model: openai('gpt-4-turbo'),
  tools: {
    browse: plasmate.tool(),
  },
  prompt: 'Research the latest trends in AI agent development',
});
```

Both examples show the same pattern: add Plasmate as a tool and pass structured
SOM into the framework. Integration changes and output size depend on the host
framework, page, and application.

## Why Output Size Matters

Removing unused markup can leave more of a model's context window available for
the task. It does not imply a fixed token or cost reduction: tokenization,
prompts, model pricing, retries, and page composition all affect the outcome.
Smaller serialized input also does not by itself guarantee lower end-to-end
latency or better task accuracy. Measure the full workflow on a representative
page corpus before making capacity or cost projections.

## Join the Ecosystem

We are building Plasmate in the open, and we would love your help growing the ecosystem.

- **[Star the repo](https://github.com/nicholasquirk/plasmate)** - Help us reach more developers
- **[Browse awesome-plasmate](https://github.com/nicholasquirk/awesome-plasmate)** - Curated list of integrations, tutorials, and projects
- **[Join our Discord](https://discord.gg/plasmate)** - Get help, share what you are building, and connect with the community
- **[Contribute an integration](https://github.com/nicholasquirk/plasmate/blob/main/CONTRIBUTING.md)** - Build support for your favorite tool

The future of AI agents is efficient, semantic web understanding. We are excited to have you along for the ride.

---

*Plasmate is open source under Apache-2.0. Try it today with `curl -fsSL https://plasmate.app/install.sh | bash`*
