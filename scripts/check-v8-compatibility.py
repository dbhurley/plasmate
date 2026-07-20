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
    icu_data = data.get("icu_data", {})
    icu_crate = icu_data.get("crate")
    icu_selected = icu_data.get("selected")
    if icu_crate != "deno_core_icudata":
        fail("icu_data.crate must be deno_core_icudata")
    if not isinstance(icu_selected, str) or not re.fullmatch(
        r"\d+\.\d+\.\d+", icu_selected
    ):
        fail("icu_data.selected must be an exact semantic version")
    if icu_data.get("icu_major") != 74:
        fail("v8 139.0.0 requires ICU 74 common data")
    if icu_data.get("registration_api") != "set_common_data_74":
        fail("v8 139.0.0 must use set_common_data_74")

    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    expected_requirement = f"={selected}"
    requirement = re.search(r'^v8\s*=\s*"([^"]+)"', cargo, re.MULTILINE)
    if requirement is None or requirement.group(1) != expected_requirement:
        fail(f"Cargo.toml v8 must be pinned to {expected_requirement}")
    icu_requirement = re.search(
        rf'^{re.escape(icu_crate)}\s*=\s*"([^"]+)"', cargo, re.MULTILINE
    )
    expected_icu_requirement = f"={icu_selected}"
    if (
        icu_requirement is None
        or icu_requirement.group(1) != expected_icu_requirement
    ):
        fail(
            f"Cargo.toml {icu_crate} must be pinned to {expected_icu_requirement}"
        )
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
    locked_icu = re.findall(
        rf'\[\[package\]\]\nname = "{re.escape(icu_crate)}"\nversion = "([^"]+)"',
        lock,
    )
    if locked_icu != [icu_selected]:
        fail(
            f"Cargo.lock contains {icu_crate} versions {locked_icu!r}, "
            f"expected [{icu_selected!r}]"
        )

    if not data.get("supported_targets") or not data.get("verification", {}).get("commands"):
        fail("target matrix and verification commands must be non-empty")
    if not data.get("upgrade_gap", {}).get("blocking_change"):
        fail("upgrade gap must name the blocking change")
    if data.get("upgrade_gap", {}).get("severity") != "critical":
        fail("the V8 upgrade gap must remain classified as critical")
    print(
        "v8 compatibility manifest matches Cargo metadata "
        f"(v8 {selected}, ICU data {icu_selected})"
    )


if __name__ == "__main__":
    main()
