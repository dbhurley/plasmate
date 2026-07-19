#!/usr/bin/env python3
"""Fail when a first-party Python dependency is absent from the audit input.

This intentionally avoids ``tomllib`` so contributors can run it with the
Python 3.9 floor supported by the Python packages.  It parses only the TOML
string arrays relevant to dependency auditing; unsupported or malformed
dependency declarations fail closed.
"""

from __future__ import annotations

import ast
import re
from pathlib import Path
from typing import Dict, Iterable, List


ROOT = Path(__file__).resolve().parents[1]
PROJECTS = (
    "sdk/python/pyproject.toml",
    "packages/som-parser-python/pyproject.toml",
    "integrations/browser-use/pyproject.toml",
    "integrations/langchain/pyproject.toml",
    "tools/awp-cdp-proxy/pyproject.toml",
)
FIRST_PARTY = {"plasmate", "som-parser"}
ARRAY_ASSIGNMENT = re.compile(r"^\s*([A-Za-z0-9_-]+)\s*=\s*(\[.*)$")
TABLE = re.compile(r"^\s*\[([^]]+)]\s*(?:#.*)?$")


def package_name(requirement: str) -> str:
    match = re.match(r"\s*([A-Za-z0-9_.-]+)", requirement)
    if match is None:
        raise ValueError(f"cannot parse requirement: {requirement!r}")
    return re.sub(r"[-_.]+", "-", match.group(1)).lower()


def _parse_string_array(source: str, origin: str) -> List[str]:
    try:
        value = ast.literal_eval(source)
    except (SyntaxError, ValueError) as error:
        raise ValueError(f"{origin}: malformed dependency array: {error}") from error
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ValueError(f"{origin}: dependency value must be an array of strings")
    return value


def dependency_arrays(text: str, origin: str = "pyproject.toml") -> Dict[str, List[str]]:
    """Extract dependency arrays from the TOML sections covered by the audit."""

    section = ""
    arrays: Dict[str, List[str]] = {}
    lines = text.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        table = TABLE.match(line)
        if table:
            section = table.group(1).strip()
            index += 1
            continue

        assignment = ARRAY_ASSIGNMENT.match(line)
        if assignment is None:
            index += 1
            continue

        key, expression = assignment.groups()
        relevant = (
            (section == "build-system" and key == "requires")
            or (section == "project" and key == "dependencies")
            or section == "project.optional-dependencies"
        )
        if not relevant:
            index += 1
            continue

        while True:
            try:
                parsed = _parse_string_array(expression, f"{origin}:{index + 1}")
                break
            except ValueError as error:
                # An incomplete multiline Python/TOML list raises this form of
                # SyntaxError. Other malformed values must fail immediately.
                if "was never closed" not in str(error) and "unexpected EOF" not in str(error):
                    raise
                index += 1
                if index >= len(lines):
                    raise
                expression += "\n" + lines[index]

        arrays[f"{section}.{key}"] = parsed
        index += 1

    return arrays


def declared_dependencies(projects: Iterable[Path]) -> set[str]:
    declared: set[str] = set()
    for path in projects:
        arrays = dependency_arrays(path.read_text(encoding="utf-8"), str(path))
        for requirements in arrays.values():
            declared.update(package_name(item) for item in requirements)
    return declared


def main() -> None:
    input_path = ROOT / "security/python-audit-requirements.in"
    audited = {
        package_name(line)
        for line in input_path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }
    declared = declared_dependencies(ROOT / relative for relative in PROJECTS)
    missing = sorted(declared - FIRST_PARTY - audited)
    if missing:
        raise SystemExit(
            "Python dependencies missing from security/python-audit-requirements.in: "
            + ", ".join(missing)
        )
    print(f"Python audit input covers {len(declared - FIRST_PARTY)} external dependencies")


if __name__ == "__main__":
    main()
