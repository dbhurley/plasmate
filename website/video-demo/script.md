# Plasmate Measurement Demo Script

**Duration:** 60-90 seconds  
**Style:** Screen recording with voiceover  
**Tone:** Technical but accessible, confident

---

## HOOK (0:00 - 0:10)

**[Screen: Terminal, dark theme]**

> "What does your AI agent actually need from a web page?"

**[Type command: `plasmate fetch https://linear.app`]**

---

## THE PROBLEM (0:10 - 0:25)

**[Screen: Split view - messy HTML on left, token counter spinning on right]**

> "Right now, when your AI reads a webpage, it's drowning in HTML noise."

**[Show: Wall of HTML tags scrolling]**

> "Raw HTML contains rendering and runtime markup. Some agent workflows need
> only the supported semantic content and actions."

**[Show: raw bytes, serialized SOM bytes, and the exact command used]**

---

## THE SOLUTION (0:25 - 0:45)

**[Screen: Terminal]**

> "Plasmate compiles the page into a Semantic Object Model."

**[Run: `curl -s https://linear.app | wc -c` showing ~2.2MB]**

> "First measure the raw response for this captured input."

**[Run: `plasmate fetch https://linear.app | wc -c` showing ~21KB]**

> "Then measure serialized SOM from the same captured input. This is a byte
> comparison for one page state, not proof of equal information or model
> performance."

**[Run: `plasmate fetch https://linear.app | head -30`]**

> "The output exposes supported headlines, links, content, and actions. Test
> whether it retains what your task needs."

---

## THE RESULT (0:45 - 1:00)

**[Screen: Side-by-side comparison graphic]**

> "We call it the Semantic Object Model - SOM for short. It's like a DOM, but built for AI."

**[Show: input URL or fixture digest, version/commit, JavaScript and cache
settings, raw bytes, serialized SOM bytes, and task assertions]**

> "Bytes are only supporting evidence. Measure tokens with your model's
> tokenizer, latency on your runner, and task success with complete outcomes."

---

## CALL TO ACTION (1:00 - 1:15)

**[Screen: Terminal with install command]**

> "Get started in seconds."

**[Type: `pip install plasmate`]**

> "Or run it as an MCP server for Claude, Cursor, or any AI tool."

**[Type: `plasmate mcp`]**

**[Screen: plasmate.app homepage]**

> "Plasmate. The browser engine for AI agents."

**[End card: plasmate.app logo, GitHub stars]**

---

## Recording Notes

- Use a clean terminal theme (dark background, readable font)
- Pre-load the Linear.app response so demo doesn't depend on network
- Keep typing speed moderate - let viewers follow along
- Pause briefly after each stat reveal for impact
