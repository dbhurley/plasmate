#!/usr/bin/env python3
"""Fail-closed validation for production release tag workflows.

The live path validates the release event, tag/version relationship, Git
ancestry, and the latest GitHub Actions check run for every required context.
Tests can supply an API fixture without making network requests.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping


GITHUB_ACTIONS_APP_ID = 15368
REQUIRED_CHECKS = (
    "Minimum Rust 1.88",
    "test (ubuntu-latest)",
    "test (macos-latest)",
    "action-manifest",
    "Workflow trust policy",
    "Rust advisory audit",
    "npm audit (packages/som-parser-node)",
    "npm audit (sdk/node)",
    "npm audit (smoke)",
    "npm audit (integrations/vercel-ai)",
    "npm audit (website)",
    "Python dependency audit",
    "Go vulnerability audit",
)
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
REPOSITORY = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
STABLE_VERSION = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
TOML_TABLE = re.compile(r"^\s*\[([^]]+)]\s*(?:#.*)?$")
TOML_VERSION = re.compile(r'^\s*version\s*=\s*"([^"]+)"\s*(?:#.*)?$')


class PreflightError(RuntimeError):
    """A release authorization condition was not satisfied."""


def rust_package_version(cargo_toml: Path) -> str:
    """Read only ``package.version`` and reject ambiguous declarations."""

    section = ""
    versions: List[str] = []
    for line in cargo_toml.read_text(encoding="utf-8").splitlines():
        table = TOML_TABLE.match(line)
        if table is not None:
            section = table.group(1).strip()
            continue
        if section == "package":
            version = TOML_VERSION.match(line)
            if version is not None:
                versions.append(version.group(1))

    if len(versions) != 1 or not versions[0]:
        raise PreflightError(
            f"{cargo_toml}: expected exactly one non-empty package.version"
        )
    return versions[0]


def validate_release_event(event_name: str, ref_type: str) -> None:
    if event_name != "push":
        raise PreflightError(
            f"production releases require a push event, got {event_name!r}"
        )
    if ref_type != "tag":
        raise PreflightError(
            f"production releases require a tag ref, got {ref_type!r}"
        )


def expected_release_tag(version: str, tag: str) -> None:
    if STABLE_VERSION.fullmatch(version) is None:
        raise PreflightError(
            "production releases require a stable MAJOR.MINOR.PATCH Rust version; "
            f"got {version!r}"
        )
    expected = f"v{version}"
    if tag != expected:
        raise PreflightError(
            f"release tag must exactly match Rust package version: expected "
            f"{expected!r}, got {tag!r}"
        )


def _git(repository_root: Path, *arguments: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository_root), *arguments],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if check and result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "git command failed"
        raise PreflightError(detail)
    return result.stdout.strip()


def validate_repository_state(
    repository_root: Path, tag: str, event_sha: str, master_ref: str
) -> str:
    """Return the release commit after verifying checkout, tag, and ancestry."""

    if FULL_SHA.fullmatch(event_sha) is None:
        raise PreflightError("release event SHA must be a full lowercase commit SHA")

    tag_commit = _git(
        repository_root, "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}"
    )
    event_commit = _git(repository_root, "rev-parse", "--verify", f"{event_sha}^{{commit}}")
    checkout_commit = _git(repository_root, "rev-parse", "--verify", "HEAD^{commit}")
    _git(repository_root, "rev-parse", "--verify", f"{master_ref}^{{commit}}")

    if tag_commit != event_commit:
        raise PreflightError(
            f"tag {tag!r} resolves to {tag_commit}, not event SHA {event_commit}"
        )
    if checkout_commit != event_commit:
        raise PreflightError(
            f"checked-out HEAD {checkout_commit} does not match event SHA {event_commit}"
        )

    contained = subprocess.run(
        [
            "git",
            "-C",
            str(repository_root),
            "merge-base",
            "--is-ancestor",
            event_commit,
            master_ref,
        ],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    if contained.returncode == 1:
        raise PreflightError(
            f"release commit {event_commit} is not contained in {master_ref}"
        )
    if contained.returncode != 0:
        raise PreflightError(contained.stderr.strip() or "git ancestry check failed")
    return event_commit


def _check_rank(check: Mapping[str, Any]) -> tuple[int, str, str]:
    identifier = check.get("id")
    if not isinstance(identifier, int):
        raise PreflightError("required check run is missing an integer id")
    return (
        identifier,
        str(check.get("completed_at") or ""),
        str(check.get("started_at") or ""),
    )


def select_required_checks(
    check_runs: Iterable[Mapping[str, Any]], release_sha: str
) -> Dict[str, Mapping[str, Any]]:
    """Select the newest same-SHA GitHub Actions run for each required name."""

    candidates: Dict[str, List[Mapping[str, Any]]] = {
        name: [] for name in REQUIRED_CHECKS
    }
    for check in check_runs:
        name = check.get("name")
        app = check.get("app")
        if (
            name in candidates
            and isinstance(app, Mapping)
            and app.get("id") == GITHUB_ACTIONS_APP_ID
            and check.get("head_sha") == release_sha
        ):
            candidates[str(name)].append(check)

    selected: Dict[str, Mapping[str, Any]] = {}
    errors: List[str] = []
    for name in REQUIRED_CHECKS:
        matches = candidates[name]
        if not matches:
            errors.append(f"missing required check: {name}")
            continue
        latest = max(matches, key=_check_rank)
        selected[name] = latest
        status = latest.get("status")
        conclusion = latest.get("conclusion")
        if status != "completed" or conclusion != "success":
            errors.append(
                f"required check {name!r} is not successful "
                f"(status={status!r}, conclusion={conclusion!r}, id={latest.get('id')!r})"
            )

    if errors:
        raise PreflightError("\n".join(errors))
    return selected


def _load_check_fixture(path: Path) -> List[Mapping[str, Any]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(payload, dict):
        payload = payload.get("check_runs")
    if not isinstance(payload, list) or not all(isinstance(item, dict) for item in payload):
        raise PreflightError("check fixture must be a list or an object with check_runs")
    return payload


def fetch_check_runs(
    api_url: str, repository: str, release_sha: str, token: str
) -> List[Mapping[str, Any]]:
    if REPOSITORY.fullmatch(repository) is None:
        raise PreflightError(f"invalid GitHub repository name: {repository!r}")
    if not token:
        raise PreflightError("GITHUB_TOKEN is required for live check verification")

    base = api_url.rstrip("/")
    encoded_repository = "/".join(
        urllib.parse.quote(part, safe="") for part in repository.split("/", 1)
    )
    runs: List[Mapping[str, Any]] = []
    for page in range(1, 101):
        url = (
            f"{base}/repos/{encoded_repository}/commits/{release_sha}/check-runs"
            f"?per_page=100&page={page}"
        )
        request = urllib.request.Request(
            url,
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {token}",
                "User-Agent": "plasmate-release-preflight",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.load(response)
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
            raise PreflightError(f"GitHub check-runs request failed: {error}") from error
        page_runs = payload.get("check_runs") if isinstance(payload, dict) else None
        if not isinstance(page_runs, list) or not all(
            isinstance(item, dict) for item in page_runs
        ):
            raise PreflightError("GitHub check-runs response has an invalid shape")
        runs.extend(page_runs)
        if len(page_runs) < 100:
            return runs
    raise PreflightError("GitHub check-runs pagination exceeded 100 pages")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository-root", type=Path, default=Path.cwd())
    parser.add_argument("--cargo-toml", type=Path, default=Path("Cargo.toml"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--event-name", required=True)
    parser.add_argument("--ref-type", required=True)
    parser.add_argument(
        "--master-ref", default="refs/remotes/origin/master"
    )
    parser.add_argument("--checks-json", type=Path)
    parser.add_argument(
        "--api-url", default=os.environ.get("GITHUB_API_URL", "https://api.github.com")
    )
    return parser.parse_args()


def main() -> None:
    arguments = parse_arguments()
    root = arguments.repository_root.resolve()
    cargo_toml = arguments.cargo_toml
    if not cargo_toml.is_absolute():
        cargo_toml = root / cargo_toml

    try:
        validate_release_event(arguments.event_name, arguments.ref_type)
        version = rust_package_version(cargo_toml)
        expected_release_tag(version, arguments.tag)
        release_sha = validate_repository_state(
            root, arguments.tag, arguments.sha, arguments.master_ref
        )
        if arguments.checks_json is not None:
            check_runs = _load_check_fixture(arguments.checks_json)
        else:
            check_runs = fetch_check_runs(
                arguments.api_url,
                arguments.repository,
                release_sha,
                os.environ.get("GITHUB_TOKEN", ""),
            )
        selected = select_required_checks(check_runs, release_sha)
    except (OSError, PreflightError, json.JSONDecodeError) as error:
        raise SystemExit(f"release preflight failed: {error}") from error

    print(
        f"Release preflight passed for {arguments.tag} at {release_sha}; "
        f"validated {len(selected)} required GitHub Actions checks"
    )
    for name in REQUIRED_CHECKS:
        print(f"  {name}: check-run {selected[name]['id']}")


if __name__ == "__main__":
    main()
