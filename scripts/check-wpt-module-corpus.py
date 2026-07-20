#!/usr/bin/env python3
"""Validate the pinned, reduced WPT module corpus without network access."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


SCHEMA = "plasmate.wpt-module-corpus.v1"
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
GIT_SHA = re.compile(r"[0-9a-f]{40}\Z")
WPT_PREFIX = "html/semantics/scripting-1/the-script-element/module/"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def validate(root: Path, manifest_path: Path, upstream_root: Path | None) -> list[str]:
    errors: list[str] = []
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"cannot read manifest {manifest_path}: {error}"]

    if manifest.get("schema") != SCHEMA:
        fail(errors, f"schema must be {SCHEMA!r}")
    upstream = manifest.get("upstream", {})
    commit = upstream.get("commit", "")
    if not GIT_SHA.fullmatch(commit):
        fail(errors, "upstream.commit must be a full lowercase 40-character Git SHA")
    if upstream.get("repository") != "https://github.com/web-platform-tests/wpt":
        fail(errors, "upstream.repository must name the canonical WPT repository")

    cases = manifest.get("cases")
    if not isinstance(cases, list) or len(cases) < 6:
        fail(errors, "cases must contain at least six focused module behaviors")
        cases = []
    seen_ids: set[str] = set()
    seen_paths: set[str] = set()
    for index, case in enumerate(cases):
        label = f"cases[{index}]"
        case_id = case.get("id", "")
        if not case_id or case_id in seen_ids:
            fail(errors, f"{label}.id must be non-empty and unique")
        seen_ids.add(case_id)
        upstream_path = case.get("upstream_path", "")
        if not upstream_path.startswith(WPT_PREFIX) or ".." in Path(upstream_path).parts:
            fail(errors, f"{label}.upstream_path must stay inside the WPT module corpus")
        if upstream_path in seen_paths:
            fail(errors, f"{label}.upstream_path is duplicated")
        seen_paths.add(upstream_path)
        expected_upstream = case.get("upstream_sha256", "")
        if not SHA256.fullmatch(expected_upstream):
            fail(errors, f"{label}.upstream_sha256 must be a lowercase SHA-256")
        if upstream_root is not None:
            source = upstream_root / upstream_path
            if not source.is_file():
                fail(errors, f"{label} upstream source is missing: {source}")
            elif digest(source) != expected_upstream:
                fail(errors, f"{label} upstream source hash drifted at pinned commit {commit}")
        relationship = case.get("relationship")
        if relationship not in {"reduced_behavior", "documented_scope_boundary"}:
            fail(errors, f"{label}.relationship is not recognized")
        assertions = case.get("local_assertions")
        if not isinstance(assertions, list) or not assertions:
            fail(errors, f"{label}.local_assertions must be non-empty")
            continue
        for assertion in assertions:
            if not isinstance(assertion, str) or "::" not in assertion:
                fail(errors, f"{label} has malformed local assertion {assertion!r}")
                continue
            relative, test_name = assertion.rsplit("::", 1)
            local_file = root / relative
            if not local_file.is_file():
                fail(errors, f"{label} local assertion file is missing: {relative}")
                continue
            marker = re.compile(rf"\bfn\s+{re.escape(test_name)}\s*\(")
            if not marker.search(local_file.read_text(encoding="utf-8")):
                fail(errors, f"{label} local assertion drifted: {assertion}")

    fixtures = manifest.get("local_fixtures")
    if not isinstance(fixtures, list) or not fixtures:
        fail(errors, "local_fixtures must be non-empty")
        fixtures = []
    for index, fixture in enumerate(fixtures):
        relative = fixture.get("path", "")
        expected = fixture.get("sha256", "")
        path = root / relative
        if not path.is_file():
            fail(errors, f"local_fixtures[{index}] is missing: {relative}")
        elif not SHA256.fullmatch(expected):
            fail(errors, f"local_fixtures[{index}].sha256 is malformed")
        elif digest(path) != expected:
            fail(errors, f"local fixture hash drifted without manifest review: {relative}")

    provenance = root / "tests/fixtures/js-modules/PROVENANCE.md"
    if not provenance.is_file():
        fail(errors, "module corpus provenance document is missing")
    else:
        text = provenance.read_text(encoding="utf-8")
        if commit and commit not in text:
            fail(errors, "provenance document does not name the pinned WPT commit")
        if "wpt-corpus.json" not in text:
            fail(errors, "provenance document does not name the machine-readable manifest")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--manifest", default="tests/fixtures/js-modules/wpt-corpus.json"
    )
    parser.add_argument(
        "--upstream-root",
        help="optional WPT checkout at the manifest commit; verifies upstream hashes",
    )
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    manifest_path = root / args.manifest
    upstream_root = Path(args.upstream_root).resolve() if args.upstream_root else None
    errors = validate(root, manifest_path, upstream_root)
    if errors:
        for error in errors:
            print(f"WPT module corpus error: {error}", file=sys.stderr)
        return 1
    print(f"WPT module corpus is pinned and internally consistent: {manifest_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
