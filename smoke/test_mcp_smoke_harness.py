"""Failure-path tests for the MCP smoke-test process harness."""

import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
SMOKE = ROOT / "smoke" / "mcp-smoke.py"


def run_smoke(binary: str, rpc_timeout: str = "0.25"):
    env = os.environ.copy()
    env.update(
        {
            "PLASMATE_BIN": binary,
            "MCP_SMOKE_RPC_TIMEOUT": rpc_timeout,
            "MCP_SMOKE_OVERALL_TIMEOUT": "1",
        }
    )
    started = time.monotonic()
    result = subprocess.run(
        [sys.executable, str(SMOKE)],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=5,
        check=False,
    )
    return result, time.monotonic() - started


class McpSmokeHarnessTests(unittest.TestCase):
    def test_dead_binary_fails_promptly(self):
        dead_binary = shutil.which("false")
        self.assertIsNotNone(dead_binary)
        result, elapsed = run_smoke(dead_binary)

        self.assertNotEqual(result.returncode, 0)
        self.assertLess(elapsed, 3)
        self.assertRegex(result.stderr, r"MCP (process exited|stdout closed)")

    def test_silent_binary_hits_rpc_deadline(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            binary = Path(temp_dir) / "silent-mcp"
            binary.write_text("#!/bin/sh\nsleep 30\n", encoding="utf-8")
            binary.chmod(0o755)

            result, elapsed = run_smoke(str(binary))

        self.assertNotEqual(result.returncode, 0)
        self.assertLess(elapsed, 3)
        self.assertIn("Timed out waiting for MCP response", result.stderr)


if __name__ == "__main__":
    unittest.main()
