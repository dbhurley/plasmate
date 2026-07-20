#!/usr/bin/env python3
"""Verify P0 soak evidence exported from GitHub Actions.

This script is intentionally read-only and network-free. It validates `gh run
list --json ...` output captured for the CI, dependency-security, and weekly JS
workflows. GitHub repository settings remain separate evidence.
"""

from __future__ import annotations

import argparse
import json
import re
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit


SHA = re.compile(r"^[0-9a-f]{40}$")
ACCEPTED_EVENTS = {"schedule", "workflow_dispatch"}
SOAK_DAYS = 7
WORKFLOW_NAMES = {
    "ci": "CI",
    "security": "Dependency Security",
    "js": "Coverage Scorecard (JS)",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sha", required=True, help="frozen 40-character commit SHA")
    parser.add_argument("--frozen-at", required=True, help="freeze time (UTC RFC 3339)")
    parser.add_argument("--start-date", required=True, help="first UTC day (YYYY-MM-DD)")
    parser.add_argument(
        "--verified-at", required=True, help="evidence export time (UTC RFC 3339)"
    )
    parser.add_argument("--ci", type=Path, required=True, help="CI run-list JSON")
    parser.add_argument(
        "--security", type=Path, required=True, help="dependency-security run-list JSON"
    )
    parser.add_argument("--js", type=Path, required=True, help="JS coverage run-list JSON")
    parser.add_argument("--output", type=Path, help="write the verification report as JSON")
    return parser.parse_args()


def load_runs(path: Path) -> list[dict[str, Any]]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: cannot read run-list JSON: {error}") from error
    if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
        raise ValueError(f"{path}: expected a JSON array of run objects")
    return value


def parse_utc_instant(value: str, label: str) -> datetime:
    try:
        instant = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError(f"{label} must be an RFC 3339 timestamp") from error
    if instant.tzinfo is None or instant.utcoffset() != timedelta(0):
        raise ValueError(f"{label} must include the UTC offset Z or +00:00")
    return instant.astimezone(timezone.utc)


def run_url_is_consistent(url: str, run_id: int) -> bool:
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or parsed.hostname != "github.com"
        or parsed.port is not None
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        return False
    match = re.fullmatch(
        r"/plasmate-labs/plasmate/actions/runs/([1-9][0-9]*)/?", parsed.path
    )
    return match is not None and int(match.group(1)) == run_id


def successful_days(
    runs: list[dict[str, Any]],
    workflow_name: str,
    sha: str,
    frozen_at: datetime,
    verified_at: datetime,
) -> dict[date, list[dict[str, Any]]]:
    days: dict[date, list[dict[str, Any]]] = {}
    for run in runs:
        if (
            run.get("workflowName") != workflow_name
            or run.get("headSha") != sha
            or run.get("conclusion") != "success"
            or run.get("event") not in ACCEPTED_EVENTS
        ):
            continue
        started_at = run.get("startedAt")
        updated_at = run.get("updatedAt")
        run_id = run.get("databaseId")
        attempt = run.get("attempt")
        url = run.get("url")
        if (
            not isinstance(started_at, str)
            or not isinstance(updated_at, str)
            or not isinstance(run_id, int)
            or isinstance(run_id, bool)
            or run_id < 1
            or not isinstance(attempt, int)
            or isinstance(attempt, bool)
            or attempt < 1
            or not isinstance(url, str)
            or not run_url_is_consistent(url, run_id)
        ):
            continue
        try:
            started = parse_utc_instant(started_at, "startedAt")
            completed = parse_utc_instant(updated_at, "updatedAt")
        except ValueError:
            continue
        if (
            started < frozen_at
            or started > verified_at
            or completed < started
            or completed > verified_at
        ):
            continue
        utc_day = started.date()
        days.setdefault(utc_day, []).append(
            {
                "database_id": run_id,
                "attempt": attempt,
                "event": run["event"],
                "url": url,
                "started_at": started.isoformat(),
                "completed_at": completed.isoformat(),
            }
        )
    return days


def verify_daily(
    name: str,
    runs: list[dict[str, Any]],
    sha: str,
    frozen_at: datetime,
    verified_at: datetime,
    expected_days: list[date],
) -> tuple[dict[str, Any], list[str]]:
    observed = successful_days(
        runs, WORKFLOW_NAMES[name], sha, frozen_at, verified_at
    )
    missing = [day.isoformat() for day in expected_days if day not in observed]
    evidence = {
        "workflow": name,
        "days": {
            day.isoformat(): sorted(
                observed.get(day, []), key=lambda run: run["database_id"]
            )
            for day in expected_days
        },
        "missing_days": missing,
    }
    errors = [f"{name}: no successful run for {day}" for day in missing]
    return evidence, errors


def main() -> int:
    args = parse_args()
    errors: list[str] = []

    if SHA.fullmatch(args.sha) is None:
        errors.append("--sha must be a lowercase 40-character hexadecimal commit SHA")
    try:
        start = date.fromisoformat(args.start_date)
    except ValueError:
        errors.append("--start-date must be YYYY-MM-DD")
        start = date.min
    else:
        if start.isoformat() != args.start_date:
            errors.append("--start-date must be YYYY-MM-DD")
    try:
        frozen_at = parse_utc_instant(args.frozen_at, "--frozen-at")
    except ValueError as error:
        errors.append(str(error))
        frozen_at = datetime.min.replace(tzinfo=timezone.utc)
    try:
        verified_at = parse_utc_instant(args.verified_at, "--verified-at")
    except ValueError as error:
        errors.append(str(error))
        verified_at = datetime.min.replace(tzinfo=timezone.utc)

    start_instant = datetime.combine(start, datetime.min.time(), tzinfo=timezone.utc)
    end_instant = start_instant + timedelta(days=SOAK_DAYS)
    if frozen_at > start_instant:
        errors.append("--start-date must begin at or after --frozen-at")
    if verified_at < end_instant:
        errors.append(
            f"168-hour window is incomplete; verify at or after {end_instant.isoformat()}"
        )

    inputs: dict[str, list[dict[str, Any]]] = {}
    for name, path in (("ci", args.ci), ("security", args.security), ("js", args.js)):
        try:
            inputs[name] = load_runs(path)
        except ValueError as error:
            errors.append(str(error))
            inputs[name] = []

    expected_days = [start + timedelta(days=offset) for offset in range(SOAK_DAYS)]
    daily_evidence: list[dict[str, Any]] = []
    for name in ("ci", "security"):
        evidence, daily_errors = verify_daily(
            name, inputs[name], args.sha, frozen_at, verified_at, expected_days
        )
        daily_evidence.append(evidence)
        errors.extend(daily_errors)

    js_days = successful_days(
        inputs["js"], WORKFLOW_NAMES["js"], args.sha, frozen_at, verified_at
    )
    js_runs = {
        day.isoformat(): sorted(runs, key=lambda run: run["database_id"])
        for day, runs in sorted(js_days.items())
        if day in expected_days
    }
    if not js_runs:
        errors.append("js: no successful run in the acceptance window")

    report = {
        "schema": "plasmate.p0-soak-verification.v1",
        "frozen_sha": args.sha,
        "frozen_at": frozen_at.isoformat(),
        "start_date_utc": start.isoformat(),
        "verified_at": verified_at.isoformat(),
        "days": SOAK_DAYS,
        "daily_workflows": daily_evidence,
        "js_workflow_runs": js_runs,
        "passed": not errors,
        "errors": errors,
    }
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
