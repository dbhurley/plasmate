#!/usr/bin/env python3
"""Deterministic line-oriented MCP fixture for agent workflow containment tests."""

import json
import os
import pathlib
import subprocess
import sys
import time


MODE = sys.argv[1]
LOG = pathlib.Path(sys.argv[2])
# Mirror plasmate::process_supervisor::prepare_current_process so the fixture
# closes the same parent/exec process-group race as the real MCP child.
try:
    os.setpgid(0, 0)
except PermissionError:
    pass
TOOLS = [
    "open_page",
    "navigate_to",
    "click",
    "type_text",
    "select_option",
    "scroll",
    "toggle",
    "clear",
    "evaluate",
    "get_cookies",
    "set_cookies",
    "clear_cookies",
    "trace_status",
    "trace_export",
    "trace_clear",
    "close_page",
]
PROPERTIES = {
    "open_page": {"url": {"type": "string"}, "trace": {"type": "boolean"}},
    "navigate_to": {"session_id": {"type": "string"}, "url": {"type": "string"}},
    "click": {"session_id": {"type": "string"}, "element_id": {"type": "string"}},
    "type_text": {"session_id": {"type": "string"}, "element_id": {"type": "string"}, "text": {"type": "string"}, "append": {"type": "boolean"}},
    "select_option": {"session_id": {"type": "string"}, "element_id": {"type": "string"}, "value": {"type": "string"}},
    "scroll": {"session_id": {"type": "string"}, "direction": {"type": "string"}, "pixels": {"type": "integer"}, "element_id": {"type": "string"}},
    "toggle": {"session_id": {"type": "string"}, "element_id": {"type": "string"}},
    "clear": {"session_id": {"type": "string"}, "element_id": {"type": "string"}},
    "evaluate": {"session_id": {"type": "string"}, "expression": {"type": "string"}},
    "get_cookies": {"session_id": {"type": "string"}, "url": {"type": "string"}},
    "set_cookies": {"session_id": {"type": "string"}, "cookies": {"type": "array", "items": {"type": "object", "properties": {}}}},
    "clear_cookies": {"session_id": {"type": "string"}, "name": {"type": "string"}, "domain": {"type": "string"}, "url": {"type": "string"}},
    "trace_status": {"session_id": {"type": "string"}},
    "trace_export": {"session_id": {"type": "string"}},
    "trace_clear": {"session_id": {"type": "string"}},
    "close_page": {"session_id": {"type": "string"}},
}


def definition(name):
    properties = PROPERTIES[name]
    required = ["url"] if name == "open_page" else ["session_id"]
    schema = {"type": "object", "properties": properties, "required": required}
    if name == "open_page" and MODE == "schema_unsupported":
        schema["oneOf"] = []
    if name == "open_page" and MODE == "secret_max_length":
        schema["properties"]["url"] = {"type": "string", "maxLength": 30}
    return {"name": name, "inputSchema": schema}


def send(value):
    print(json.dumps(value, separators=(",", ":")), flush=True)


def record(name):
    with LOG.open("a", encoding="utf-8") as stream:
        stream.write(name + "\n")
        stream.flush()


for raw in sys.stdin:
    request = json.loads(raw)
    method = request.get("method")
    if method == "notifications/initialized":
        continue
    request_id = request.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"protocolVersion": "2025-11-25"}})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": [definition(name) for name in TOOLS]}})
        if MODE == "stdin_backpressure":
            marker = str(LOG.with_suffix(".descendant"))
            subprocess.Popen([
                sys.executable,
                "-c",
                "import pathlib,sys,time; time.sleep(3); pathlib.Path(sys.argv[1]).write_text('leaked')",
                marker,
            ])
            # Keep stdin open but stop reading it. A large next request must be
            # bounded by the parent's write deadline, then the whole group must
            # be terminated before the descendant can write its marker.
            time.sleep(10.0)
    elif method == "tools/call":
        name = request["params"]["name"]
        record(name)
        if MODE == "early_exit" and name == "trace_status":
            raise SystemExit(17)
        if MODE == "timeout" and name == "trace_status":
            time.sleep(3.0)
        if MODE == "descendant_timeout" and name == "trace_status":
            marker = str(LOG.with_suffix(".descendant"))
            subprocess.Popen([
                sys.executable,
                "-c",
                "import pathlib,sys,time; time.sleep(3); pathlib.Path(sys.argv[1]).write_text('leaked')",
                marker,
            ])
            time.sleep(5.0)
        if MODE == "malformed" and name == "trace_status":
            print("{not-json", flush=True)
            continue
        if MODE == "oversized" and name == "trace_status":
            print("x" * 8192, flush=True)
            continue
        if MODE == "short_line_flood" and name == "trace_status":
            # Exercise bounded queue backpressure. The first line is already
            # an invalid JSON-RPC response; the rest must never accumulate in
            # the parent while it tears down this process group.
            for _ in range(100_000):
                print("{}", flush=True)
            continue
        payload = {"ok": True}
        if name == "open_page":
            payload = {
                "session_id": "fixture-session",
                "sentinel_present": "PLASMATE_UNRELATED_SENTINEL" in os.environ,
            }
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {"content": [{"type": "text", "text": json.dumps(payload)}]},
        })
