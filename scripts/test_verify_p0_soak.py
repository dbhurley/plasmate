from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from datetime import date, timedelta
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "verify-p0-soak.py"
SHA = "a" * 40
WORKFLOW_NAMES = {
    "ci": "CI",
    "security": "Dependency Security",
    "js": "Coverage Scorecard (JS)",
}


def run(day: date, run_id: int, **overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "databaseId": run_id,
        "attempt": 1,
        "event": "schedule",
        "headSha": SHA,
        "startedAt": f"{day.isoformat()}T04:15:00Z",
        "updatedAt": f"{day.isoformat()}T05:15:00Z",
        "conclusion": "success",
        "url": f"https://github.com/plasmate-labs/plasmate/actions/runs/{run_id}",
    }
    value.update(overrides)
    return value


class VerifyP0SoakTests(unittest.TestCase):
    def invoke(
        self,
        ci: list[dict[str, object]],
        security: list[dict[str, object]],
        js: list[dict[str, object]],
        *,
        frozen_at: str = "2026-07-19T23:59:00Z",
        verified_at: str = "2026-07-27T00:00:00Z",
    ) -> subprocess.CompletedProcess[str]:
        start = date(2026, 7, 20)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, value in (("ci", ci), ("security", security), ("js", js)):
                normalized = []
                for item in value:
                    copy = dict(item)
                    copy.setdefault("workflowName", WORKFLOW_NAMES[name])
                    normalized.append(copy)
                (root / f"{name}.json").write_text(
                    json.dumps(normalized), encoding="utf-8"
                )
            return subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--sha",
                    SHA,
                    "--frozen-at",
                    frozen_at,
                    "--start-date",
                    start.isoformat(),
                    "--verified-at",
                    verified_at,
                    "--ci",
                    str(root / "ci.json"),
                    "--security",
                    str(root / "security.json"),
                    "--js",
                    str(root / "js.json"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

    def test_accepts_seven_days_and_one_js_run(self) -> None:
        start = date(2026, 7, 20)
        ci = [run(start + timedelta(days=offset), 100 + offset) for offset in range(7)]
        security = [
            run(start + timedelta(days=offset), 200 + offset) for offset in range(7)
        ]
        result = self.invoke(ci, security, [run(start + timedelta(days=6), 300)])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["passed"])
        first_run = report["daily_workflows"][0]["days"]["2026-07-20"][0]
        self.assertEqual(first_run["database_id"], 100)
        self.assertEqual(first_run["attempt"], 1)
        self.assertEqual(
            first_run["url"],
            "https://github.com/plasmate-labs/plasmate/actions/runs/100",
        )

    def test_rejects_missing_daily_run(self) -> None:
        start = date(2026, 7, 20)
        ci = [run(start + timedelta(days=offset), 100 + offset) for offset in range(6)]
        security = [
            run(start + timedelta(days=offset), 200 + offset) for offset in range(7)
        ]
        result = self.invoke(ci, security, [run(start, 300)])
        self.assertEqual(result.returncode, 1)
        self.assertIn("ci: no successful run for 2026-07-26", result.stdout)

    def test_rejects_wrong_sha_failed_and_push_only_runs(self) -> None:
        start = date(2026, 7, 20)
        good = [run(start + timedelta(days=offset), 100 + offset) for offset in range(7)]
        bad_ci = good[:-1] + [run(start + timedelta(days=6), 999, headSha="b" * 40)]
        bad_security = good[:-1] + [
            run(start + timedelta(days=6), 998, conclusion="failure")
        ]
        result = self.invoke(bad_ci, bad_security, [run(start, 300, event="push")])
        self.assertEqual(result.returncode, 1)
        self.assertIn("ci: no successful run for 2026-07-26", result.stdout)
        self.assertIn("security: no successful run for 2026-07-26", result.stdout)
        self.assertIn("js: no successful run", result.stdout)

    def test_rejects_incomplete_168_hour_window(self) -> None:
        start = date(2026, 7, 20)
        daily = [run(start + timedelta(days=offset), 100 + offset) for offset in range(7)]
        result = self.invoke(
            daily,
            daily,
            [run(start, 300)],
            verified_at="2026-07-26T23:59:59Z",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("168-hour window is incomplete", result.stdout)

    def test_rejects_run_completed_after_verification_cutoff(self) -> None:
        start = date(2026, 7, 20)
        daily = [run(start + timedelta(days=offset), 100 + offset) for offset in range(7)]
        daily[-1] = run(
            start + timedelta(days=6),
            999,
            updatedAt="2026-07-27T00:00:01Z",
        )
        result = self.invoke(daily, daily, [run(start, 300)])
        self.assertEqual(result.returncode, 1)
        self.assertIn("ci: no successful run for 2026-07-26", result.stdout)
        self.assertIn("security: no successful run for 2026-07-26", result.stdout)

    def test_rejects_run_url_with_a_different_database_id(self) -> None:
        start = date(2026, 7, 20)
        daily = [run(start + timedelta(days=offset), 100 + offset) for offset in range(7)]
        daily[-1] = run(
            start + timedelta(days=6),
            999,
            url="https://github.com/plasmate-labs/plasmate/actions/runs/998",
        )
        result = self.invoke(daily, daily, [run(start, 300)])
        self.assertEqual(result.returncode, 1)
        self.assertIn("ci: no successful run for 2026-07-26", result.stdout)

    def test_rejects_wrong_host_and_repository_run_urls(self) -> None:
        start = date(2026, 7, 20)
        for bad_url in (
            "https://example.com/plasmate-labs/plasmate/actions/runs/999",
            "https://github.com/other/plasmate/actions/runs/999",
        ):
            with self.subTest(url=bad_url):
                daily = [
                    run(start + timedelta(days=offset), 100 + offset)
                    for offset in range(7)
                ]
                daily[-1] = run(start + timedelta(days=6), 999, url=bad_url)
                result = self.invoke(daily, daily, [run(start, 300)])
                self.assertEqual(result.returncode, 1)
                self.assertIn("ci: no successful run for 2026-07-26", result.stdout)

    def test_rejects_non_positive_or_boolean_attempt(self) -> None:
        start = date(2026, 7, 20)
        for attempt in (0, True):
            with self.subTest(attempt=attempt):
                daily = [
                    run(start + timedelta(days=offset), 100 + offset)
                    for offset in range(7)
                ]
                daily[-1] = run(
                    start + timedelta(days=6), 999, attempt=attempt
                )
                result = self.invoke(daily, daily, [run(start, 300)])
                self.assertEqual(result.returncode, 1)
                self.assertIn("ci: no successful run for 2026-07-26", result.stdout)

    def test_rejects_run_from_a_different_workflow(self) -> None:
        start = date(2026, 7, 20)
        daily = [run(start + timedelta(days=offset), 100 + offset) for offset in range(7)]
        daily[-1] = run(
            start + timedelta(days=6),
            999,
            workflowName="Different workflow",
        )
        result = self.invoke(daily, daily, [run(start, 300)])
        self.assertEqual(result.returncode, 1)
        self.assertIn("ci: no successful run for 2026-07-26", result.stdout)
        self.assertIn("security: no successful run for 2026-07-26", result.stdout)

    def test_rejects_a_start_date_before_the_freeze(self) -> None:
        start = date(2026, 7, 20)
        daily = [run(start + timedelta(days=offset), 100 + offset) for offset in range(7)]
        daily[0] = run(start, 999, startedAt="2026-07-20T00:00:00Z")
        result = self.invoke(
            daily,
            daily,
            [run(start + timedelta(days=1), 300)],
            frozen_at="2026-07-20T00:01:00Z",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("--start-date must begin at or after --frozen-at", result.stdout)


if __name__ == "__main__":
    unittest.main()
