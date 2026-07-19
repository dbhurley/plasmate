#!/usr/bin/env python3
"""Tests for the dependency-only TOML reader used by the security gate."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-python-audit-input.py")
SPEC = importlib.util.spec_from_file_location("check_python_audit_input", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class DependencyArraysTests(unittest.TestCase):
    def test_extracts_all_audited_sections_and_ignores_unrelated_arrays(self) -> None:
        source = '''
[build-system]
requires = ["hatchling>=1"]

[project]
dependencies = [
  "httpx>=0.27", # inline comments are legal
  "pydantic>=2; python_version >= '3.9'",
]
keywords = ["not", "dependencies"]

[project.optional-dependencies]
dev = ["pytest>=8"]
docs = [
  "sphinx",
]
'''
        self.assertEqual(
            CHECKER.dependency_arrays(source),
            {
                "build-system.requires": ["hatchling>=1"],
                "project.dependencies": [
                    "httpx>=0.27",
                    "pydantic>=2; python_version >= '3.9'",
                ],
                "project.optional-dependencies.dev": ["pytest>=8"],
                "project.optional-dependencies.docs": ["sphinx"],
            },
        )

    def test_rejects_non_string_dependency_entries(self) -> None:
        with self.assertRaisesRegex(ValueError, "array of strings"):
            CHECKER.dependency_arrays("[project]\ndependencies = [123]\n")

    def test_rejects_unclosed_dependency_arrays(self) -> None:
        with self.assertRaisesRegex(ValueError, "malformed dependency array"):
            CHECKER.dependency_arrays('[project]\ndependencies = ["httpx"\n')

    def test_normalizes_pep_503_package_names(self) -> None:
        self.assertEqual(CHECKER.package_name("PyTest_Asyncio>=0.23"), "pytest-asyncio")


if __name__ == "__main__":
    unittest.main()
