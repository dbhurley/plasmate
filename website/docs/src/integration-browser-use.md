# Browser Use Integration

Use Plasmate as the browser backend for [Browser Use](https://github.com/browser-use/browser-use), replacing Chrome + Playwright with structured SOM output. Output size and token use depend on the page and model tokenizer.

Browser Use is an open-source AI browser agent framework. Plasmate's adapter
uses the Semantic Object Model instead of a DOM-derived page representation,
retaining supported interactive elements while stripping layout noise.

Source: [`integrations/browser-use/`](https://github.com/plasmate-labs/plasmate/tree/master/integrations/browser-use)

## Installation

```bash
pip install plasmate-browser-use
```

Requires the `plasmate` binary on your PATH:

```bash
curl -fsSL https://plasmate.app/install.sh | sh
```

## Quick Start

```python
import asyncio
from plasmate_browser_use import PlasmateBrowser

async def main():
    async with PlasmateBrowser() as browser:
        # Navigate and get SOM state
        state = await browser.navigate("https://news.ycombinator.com")
        print(state.text)  # What the LLM sees

        # Click an element by its index
        link = state.interactive_elements[0]
        state = await browser.click(link.index)

        # Type into a form field
        inputs = [el for el in state.interactive_elements if el.role == "text_input"]
        if inputs:
            state = await browser.type_text(inputs[0].index, "hello world")

asyncio.run(main())
```

## How It Works

`PlasmateBrowser` wraps a Plasmate MCP subprocess. When you call `navigate()`, `click()`, or `type_text()`, it sends AWP commands to Plasmate and returns SOM output formatted for Browser Use compatibility.

| | Browser Use (Playwright) | Browser Use (Plasmate) |
|---|---|---|
| **Backend** | Chrome via Playwright | Plasmate MCP subprocess |
| **Output to LLM** | Raw DOM tree | SOM (Semantic Object Model) |
| **Context representation** | DOM and optional visual state | Structured SOM text |
| **Interactive elements** | `[backend_node_id]<tag>` | `[N] role "label"` |
| **Dependencies** | Chrome, Playwright | `plasmate` binary only |
| **Runtime** | Full browser process | Native Plasmate subprocess |

SOM replaces DOM screenshots with structured text. Instead of parsing a raw DOM tree with thousands of nodes, the LLM sees a compact summary with numbered interactive elements:

```
[Tab] Hacker News
[URL] https://news.ycombinator.com

--- navigation "Main menu" ---
  [1] link "Hacker News" -> /
  [2] link "new" -> /newest
  [3] link "past" -> /front

--- main ---
  [4] link "Show HN: Something Cool" -> https://example.com
  142 points by someone
  [5] link "89 comments" -> /item?id=12345678

```

The lightweight extractor package also exposes `extract_action_plan()` and
`extract_action_plan_async()` for agents that want only reusable action
targets. Those targets carry `enabled`, disabled/inert `blocked_reason`, `required`,
`description`, `placeholder`, `group`, `current`, `controls`, and `haspopup`
context when Plasmate emits it, so Browser Use agents can skip unavailable
controls and understand popup/controlled-panel targets before spending a
browser action.

For repetitive workflows, scope targets before prompting:

```python
from plasmate_browser_use import PlasmateExtractor

extractor = PlasmateExtractor()
plan = extractor.extract_action_plan("https://example.com/settings")
buttons = extractor.find_action_targets_by_role(
    "https://example.com/settings",
    "button",
    enabled_only=True,
)
clicks = extractor.find_action_targets_by_action(
    "https://example.com/settings",
    "click",
    enabled_only=True,
)
```

## Output-size evidence

In the v0.5.1 observational benchmark snapshots, serialized SOM was smaller than
raw HTML by a median 9.98x across 83 successful non-JavaScript inputs out of 98
attempted, and by a median 9.32x across 82 successful JavaScript inputs out of
98 attempted. Results vary by page. These are serialized-byte ratios, not
universal token, cost, latency, or task-success guarantees; measure the complete
Browser Use workflow with your target pages and model.

## API Reference

### `PlasmateBrowser`

```python
PlasmateBrowser(
    binary="plasmate",   # Path to plasmate binary
    timeout=30,          # Response timeout in seconds
    budget=None,         # Optional SOM token budget
)
```

| Method | Description | Returns |
|--------|-------------|---------|
| `navigate(url)` | Open a URL in a persistent session | `PageState` |
| `click(element_index)` | Click element by its `[N]` index | `PageState` |
| `type_text(element_index, text)` | Type into an input/textarea | `PageState` |
| `get_state()` | Get current page state as SOM | `PageState` |
| `screenshot()` | Returns `None` (no visual rendering) | `None` |
| `close()` | Close session and shut down process | -  |

### `PageState`

| Field | Type | Description |
|-------|------|-------------|
| `url` | `str` | Current page URL |
| `title` | `str` | Page title |
| `text` | `str` | SOM text for the LLM |
| `interactive_elements` | `list[InteractiveElement]` | All clickable/typeable elements |
| `selector_map` | `dict[int, InteractiveElement]` | Index -> element lookup |
| `som` | `dict` | Raw SOM dict |
| `som_tokens` | `int` | Estimated token count |
| `html_bytes` | `int` | Original HTML size |
| `som_bytes` | `int` | SOM output size |

### `InteractiveElement`

| Field | Type | Description |
|-------|------|-------------|
| `index` | `int` | Integer index (`[N]` in SOM text) |
| `som_id` | `str` | Original SOM element ID |
| `role` | `str` | `link`, `button`, `text_input`, etc. |
| `text` | `str` | Display text or label |
| `attrs` | `dict` | Role-specific attributes |

## Known Limitations

- **No screenshots** -  Plasmate has no visual rendering. Agents that rely on screenshots (CAPTCHA, visual layout) should use the Playwright backend.
- **No coordinate-based clicking** -  all interactions use SOM element IDs, not pixel coordinates.
- **No file uploads** -  the `upload_file` action is not supported.
- **No scroll position** -  Plasmate renders the full page. No concept of viewport or scroll.
- **Single tab** -  each `PlasmateBrowser` instance maintains one session. Create multiple instances for multi-tab workflows.
