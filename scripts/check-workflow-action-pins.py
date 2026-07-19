#!/usr/bin/env python3
"""Require immutable SHAs and readable version comments for remote actions."""

from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
USE = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)(?:\s+#\s*(.+))?$")
SHA = re.compile(r"^[0-9a-f]{40}$")
errors: list[str] = []
checked = 0

for workflow in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
    for line_number, line in enumerate(
        workflow.read_text(encoding="utf-8").splitlines(), start=1
    ):
        match = USE.match(line)
        if match is None:
            continue
        action, comment = match.groups()
        if action.startswith("./") or action.startswith("docker://"):
            continue
        checked += 1
        if "@" not in action:
            errors.append(f"{workflow}:{line_number}: action has no ref")
            continue
        _, reference = action.rsplit("@", 1)
        if SHA.fullmatch(reference) is None:
            errors.append(
                f"{workflow}:{line_number}: {reference!r} is not a full commit SHA"
            )
        if not comment:
            errors.append(f"{workflow}:{line_number}: pinned action needs a version comment")

if errors:
    raise SystemExit("\n".join(errors))

print(f"Validated {checked} remote action pins")
