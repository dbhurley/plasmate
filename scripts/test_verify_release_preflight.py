#!/usr/bin/env python3
"""Deterministic tests for production release authorization."""

from __future__ import annotations

import importlib.util
import re
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("verify-release-preflight.py")
ROOT = SCRIPT.resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("verify_release_preflight", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
PREFLIGHT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREFLIGHT)


def successful_checks(sha: str) -> list[dict[str, object]]:
    return [
        {
            "id": index,
            "name": name,
            "head_sha": sha,
            "status": "completed",
            "conclusion": "success",
            "app": {"id": PREFLIGHT.GITHUB_ACTIONS_APP_ID},
        }
        for index, name in enumerate(PREFLIGHT.REQUIRED_CHECKS, start=1)
    ]


class MetadataTests(unittest.TestCase):
    def test_reads_only_rust_package_version(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            cargo = Path(temporary) / "Cargo.toml"
            cargo.write_text(
                '[package]\nname = "plasmate"\nversion = "0.6.0-rc.1"\n\n'
                '[dependencies.foo]\nversion = "9"\n',
                encoding="utf-8",
            )
            self.assertEqual(PREFLIGHT.rust_package_version(cargo), "0.6.0-rc.1")

    def test_requires_tag_push_event(self) -> None:
        PREFLIGHT.validate_release_event("push", "tag")
        with self.assertRaisesRegex(PREFLIGHT.PreflightError, "push event"):
            PREFLIGHT.validate_release_event("workflow_dispatch", "branch")

    def test_tag_must_exactly_match_rust_version(self) -> None:
        PREFLIGHT.expected_release_tag("0.6.0", "v0.6.0")
        with self.assertRaisesRegex(PREFLIGHT.PreflightError, "expected 'v0.6.0'"):
            PREFLIGHT.expected_release_tag("0.6.0", "v0.6")

    def test_production_tag_rejects_prerelease_and_build_metadata(self) -> None:
        for version in ("0.6.0-rc.1", "0.6.0+build.7", "01.2.3", "0.6"):
            with self.subTest(version=version):
                with self.assertRaisesRegex(
                    PREFLIGHT.PreflightError, "stable MAJOR.MINOR.PATCH"
                ):
                    PREFLIGHT.expected_release_tag(version, f"v{version}")


class CheckSelectionTests(unittest.TestCase):
    SHA = "a" * 40

    def test_accepts_exact_required_successes(self) -> None:
        selected = PREFLIGHT.select_required_checks(successful_checks(self.SHA), self.SHA)
        self.assertEqual(tuple(selected), PREFLIGHT.REQUIRED_CHECKS)

    def test_newest_matching_check_wins_and_failure_fails_closed(self) -> None:
        checks = successful_checks(self.SHA)
        checks.append(
            {
                "id": 999,
                "name": PREFLIGHT.REQUIRED_CHECKS[0],
                "head_sha": self.SHA,
                "status": "completed",
                "conclusion": "failure",
                "app": {"id": PREFLIGHT.GITHUB_ACTIONS_APP_ID},
            }
        )
        with self.assertRaisesRegex(PREFLIGHT.PreflightError, "is not successful"):
            PREFLIGHT.select_required_checks(checks, self.SHA)

    def test_wrong_app_or_sha_cannot_satisfy_a_check(self) -> None:
        checks = successful_checks(self.SHA)[1:]
        checks.extend(
            [
                {
                    "id": 500,
                    "name": PREFLIGHT.REQUIRED_CHECKS[0],
                    "head_sha": self.SHA,
                    "status": "completed",
                    "conclusion": "success",
                    "app": {"id": 1},
                },
                {
                    "id": 501,
                    "name": PREFLIGHT.REQUIRED_CHECKS[0],
                    "head_sha": "b" * 40,
                    "status": "completed",
                    "conclusion": "success",
                    "app": {"id": PREFLIGHT.GITHUB_ACTIONS_APP_ID},
                },
            ]
        )
        with self.assertRaisesRegex(PREFLIGHT.PreflightError, "missing required check"):
            PREFLIGHT.select_required_checks(checks, self.SHA)


class RepositoryStateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.git("init", "-q", "-b", "master")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release@example.test")
        (self.root / "tracked").write_text("base\n", encoding="utf-8")
        self.git("add", "tracked")
        self.git("commit", "-q", "-m", "base")
        self.base = self.git("rev-parse", "HEAD")
        self.git("update-ref", "refs/remotes/origin/master", self.base)
        self.git("tag", "-a", "v0.6.0", "-m", "release", self.base)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def git(self, *arguments: str) -> str:
        return subprocess.check_output(
            ["git", "-C", str(self.root), *arguments], text=True
        ).strip()

    def test_accepts_annotated_tag_contained_in_master(self) -> None:
        release = PREFLIGHT.validate_repository_state(
            self.root,
            "v0.6.0",
            self.base,
            "refs/remotes/origin/master",
        )
        self.assertEqual(release, self.base)

    def test_rejects_release_commit_not_contained_in_master(self) -> None:
        (self.root / "tracked").write_text("candidate\n", encoding="utf-8")
        self.git("commit", "-q", "-am", "candidate")
        candidate = self.git("rev-parse", "HEAD")
        self.git("tag", "v0.6.1", candidate)
        with self.assertRaisesRegex(PREFLIGHT.PreflightError, "not contained"):
            PREFLIGHT.validate_repository_state(
                self.root,
                "v0.6.1",
                candidate,
                "refs/remotes/origin/master",
            )


class WorkflowPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (ROOT / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )

    def job(self, name: str) -> str:
        start_match = re.search(rf"^  {re.escape(name)}:\s*$", self.workflow, re.MULTILINE)
        assert start_match is not None
        next_match = re.search(
            r"^  [A-Za-z0-9_-]+:\s*$",
            self.workflow[start_match.end() :],
            re.MULTILINE,
        )
        if next_match is None:
            return self.workflow[start_match.start() :]
        end = start_match.end() + next_match.start()
        return self.workflow[start_match.start() : end]

    def test_only_tag_push_can_trigger_production_release(self) -> None:
        self.assertIn("tags: ['v*']", self.workflow)
        self.assertNotIn("workflow_dispatch:", self.workflow)

    def test_build_and_publishers_depend_on_preflight(self) -> None:
        self.assertIn("needs: preflight", self.job("build"))
        expected_needs = {
            "publish_crate": "needs: [preflight, build]",
            "docker": "needs: [preflight, build, publish_crate]",
            "release": "needs: [preflight, build, publish_crate, docker]",
        }
        for name, needs in expected_needs.items():
            job = self.job(name)
            self.assertIn(needs, job)
            self.assertIn("environment: release", job)

    def test_github_release_is_the_final_publication_job(self) -> None:
        self.assertIn("publish_crate", self.job("docker"))
        release = self.job("release")
        self.assertIn("publish_crate", release)
        self.assertIn("docker", release)

    def test_every_cargo_or_cross_release_operation_is_locked(self) -> None:
        command_lines = [
            line.strip()
            for line in self.workflow.splitlines()
            if line.strip().startswith(("cargo ", "cross "))
        ]
        self.assertTrue(command_lines)
        for line in command_lines:
            self.assertIn("--locked", line, line)

    def test_release_compiler_is_exactly_the_declared_msrv(self) -> None:
        self.assertEqual(self.workflow.count("toolchain: 1.88.0"), 3)
        self.assertNotIn("toolchain: stable", self.workflow)

    def test_preflight_has_read_only_check_access(self) -> None:
        preflight = self.job("preflight")
        self.assertIn("contents: read", preflight)
        self.assertIn("checks: read", preflight)
        self.assertNotIn("contents: write", preflight)

    def test_authorization_precedes_candidate_execution(self) -> None:
        preflight = self.job("preflight")
        authorization = preflight.index("Authorize tag commit and required checks")
        rust_install = preflight.index("Install Rust")
        metadata = preflight.index("cargo run --locked -- release-validate")
        package = preflight.index("cargo publish --locked --dry-run")
        self.assertLess(authorization, rust_install)
        self.assertLess(rust_install, metadata)
        self.assertLess(metadata, package)
        self.assertEqual(preflight.count("GITHUB_TOKEN:"), 1)


if __name__ == "__main__":
    unittest.main()
