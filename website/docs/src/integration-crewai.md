# CrewAI Integration

Give your CrewAI agents structured SOM pages instead of raw HTML scraping. The resulting context size depends on the page and model tokenizer.

Source: [`integrations/crewai/`](https://github.com/plasmate-labs/plasmate/tree/master/integrations/crewai)

## Installation

```bash
pip install plasmate crewai crewai-tools
```

## Quick Start

```python
from crewai import Agent, Task, Crew
from plasmate.integrations.crewai import PlasmateWebTool

# Create the Plasmate browsing tool
browse = PlasmateWebTool()

# Create an agent with web access
researcher = Agent(
    role="Web Researcher",
    goal="Find and summarize information from the web",
    backstory="Expert at extracting key information from web pages.",
    tools=[browse],
)

# Define a task
task = Task(
    description="Research the top stories on Hacker News and summarize them.",
    expected_output="A bullet-point summary of the top 5 stories.",
    agent=researcher,
)

# Run the crew
crew = Crew(agents=[researcher], tasks=[task])
result = crew.kickoff()
print(result)
```

## Available Tools

### `PlasmateWebTool`

Fetches a URL and returns SOM text. Use it in place of `ScrapeWebsiteTool` when your workflow benefits from semantic regions and indexed actions.

```python
from plasmate.integrations.crewai import PlasmateWebTool

tool = PlasmateWebTool()
# Agents invoke it automatically when they need web content
```

### `PlasmateBrowseTool`

Persistent browser session with navigate, click, and type actions for multi-step workflows.

```python
from plasmate.integrations.crewai import PlasmateBrowseTool

tool = PlasmateBrowseTool()
# Supports: navigate(url), click(index), type(index, text)
```

## Why Plasmate for CrewAI?

| | ScrapeWebsiteTool | PlasmateWebTool |
|---|---|---|
| **Output** | Raw HTML/text | Structured SOM |
| **Context representation** | Raw HTML/text | Structured SOM |
| **Interactive elements** | Lost | Indexed `[N]` |
| **Multi-step browsing** | ❌ | ✅ |
| **Dependencies** | requests + beautifulsoup | `plasmate` binary |

SOM removes non-semantic markup, but token and cost differences vary with the
pages, prompts, and model tokenizer. Benchmark the full crew workflow before
planning context or spend.

## Links

- [CrewAI Docs](https://docs.crewai.com)
- [Plasmate Python SDK](sdk-python)
