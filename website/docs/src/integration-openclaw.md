# OpenClaw Integration

[OpenClaw](https://openclaw.ai) integration for Plasmate — install it as a skill to give an OpenClaw agent structured SOM browsing.

Skill repo: [`plasmate-labs/skill-openclaw`](https://github.com/plasmate-labs/skill-openclaw)

## Installation

### 1. Install Plasmate

```bash
curl -fsSL https://plasmate.app/install.sh | sh
```

### 2. Install the skill

```bash
clawhub install plasmate
```

Or manually copy `integrations/openclaw/SKILL.md` to `~/.openclaw/skills/plasmate/SKILL.md`.

### 3. Install the `pf` wrapper

```bash
cp integrations/openclaw/scripts/pf /usr/local/bin/pf
chmod +x /usr/local/bin/pf
```

## Quick Start

Replace `web_fetch` calls with `pf`:

```bash
# Before
web_fetch https://docs.stripe.com/api

# After — structured SOM output, stats logged automatically
pf https://docs.stripe.com/api
```

`pf` wraps `plasmate fetch`, prints timing and estimated size statistics to stderr, and appends a stat entry to `~/.plasmate/fetch-stats.jsonl`.

## Output-size evidence

In the v0.5.1 observational benchmark snapshots, serialized SOM was smaller than
raw HTML by a median 9.98x across 83 successful non-JavaScript inputs out of 98
attempted, and by a median 9.32x across 82 successful JavaScript inputs out of
98 attempted. Results vary by page. These are byte ratios, not universal token,
cost, latency, or task-success guarantees. The wrapper's token fields are
estimates; use your model's tokenizer when evaluating an OpenClaw workflow.

## MCP Integration

For multi-step browsing, run Plasmate as an MCP server:

```bash
plasmate mcp
```

Add to your agent's MCP config:

```json
{
  "servers": {
    "plasmate": {
      "command": "plasmate",
      "args": ["mcp"],
      "transport": "stdio"
    }
  }
}
```

Available MCP tools: `fetch_page`, `extract_text`, `extract_links`, `cache_status`, `session_status`, `screenshot_page`, `open_page`, `navigate_to`, `click`, `type_text`, `select_option`, `scroll`, `toggle`, `clear`, `evaluate`, `close_page`, `get_cookies`, `set_cookies`, `clear_cookies`.

## CDP Mode (Puppeteer-compatible)

Run Plasmate as a CDP server to replace Chrome in existing Puppeteer/Playwright workflows:

```bash
plasmate serve --protocol cdp --port 9222
export BROWSER_WS_ENDPOINT="ws://127.0.0.1:9222"
```

## Viewing Fetch Stats

The `pf` wrapper logs every fetch:

```bash
python3 - << 'EOF'
import json, os
log = os.path.expanduser("~/.plasmate/fetch-stats.jsonl")
entries = [json.loads(l) for l in open(log) if l.strip()]
n = len(entries)
saved = sum(e.get("tokens_saved_est", 0) for e in entries)
print(f"{n} fetches | {saved:,} estimated tokens saved")
EOF
```

## Further Reading

- [MCP Integration](integration-mcp) — detailed MCP tool reference
- [AWP Protocol](awp) — native agent protocol
- [Authenticated Browsing](guide-authenticated-browsing) — cookie profiles for logged-in sites
