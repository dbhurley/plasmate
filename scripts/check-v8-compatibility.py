#!/usr/bin/env python3
"""Fail when Cargo metadata drifts from compatibility/v8.json."""

import json
import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "compatibility" / "v8.json"


def fail(message: str) -> None:
    print(f"v8 compatibility drift: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    if data.get("schema") != "plasmate.v8-compatibility.v1":
        fail("unsupported manifest schema")
    binding = data.get("binding", {})
    selected = binding.get("selected")
    if not isinstance(selected, str) or not re.fullmatch(r"\d+\.\d+\.\d+", selected):
        fail("binding.selected must be an exact semantic version")
    if binding.get("highest_api_compatible") != selected:
        fail("selected and highest_api_compatible differ")

    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    expected_requirement = f"={selected}"
    requirement = re.search(r'^v8\s*=\s*"([^"]+)"', cargo, re.MULTILINE)
    if requirement is None or requirement.group(1) != expected_requirement:
        fail(f"Cargo.toml v8 must be pinned to {expected_requirement}")
    minimum_rust = data.get("project", {}).get("minimum_rust", "").removesuffix(".0")
    rust_version = re.search(r'^rust-version\s*=\s*"([^"]+)"', cargo, re.MULTILINE)
    if rust_version is None or rust_version.group(1) != minimum_rust:
        fail(f"Cargo.toml rust-version must be {minimum_rust}")

    lock = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    locked = re.findall(
        r'\[\[package\]\]\nname = "v8"\nversion = "([^"]+)"', lock
    )
    if locked != [selected]:
        fail(f"Cargo.lock contains v8 versions {locked!r}, expected [{selected!r}]")

    if not data.get("supported_targets") or not data.get("verification", {}).get("commands"):
        fail("target matrix and verification commands must be non-empty")
    if not data.get("upgrade_gap", {}).get("blocking_change"):
        fail("upgrade gap must name the blocking change")
    if data.get("upgrade_gap", {}).get("severity") != "critical":
        fail("the V8 upgrade gap must remain classified as critical")
    print(f"v8 compatibility manifest matches Cargo metadata ({selected})")


if __name__ == "__main__":
    main()
