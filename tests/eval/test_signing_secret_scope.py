#!/usr/bin/env python3
"""Mutation tests for scripts/check_signing_secret_scope.py.

Each fixture is a way a signing secret could become readable without a human
approval gate. A guard that passes every one of these is not a guard, so the
suite asserts refusal AND asserts the stated reason, which stops a fixture from
passing because the parser choked on unrelated grounds.
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GUARD = Path(__file__).resolve().parents[2] / "scripts" / "check_signing_secret_scope.py"

GATED = """
name: Gated
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    environment: signed-dev-acceptance
    steps:
      - env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
        run: echo signing
"""

REFUSALS = [
    (
        "no environment at all",
        "names no environment",
        """
name: Ungated
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    steps:
      - env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
        run: echo signing
""",
    ),
    (
        "environment that requires no reviewer",
        "not an approved reviewer-gated environment",
        """
name: Wrong environment
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    environment: Preview
    steps:
      - env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
        run: echo signing
""",
    ),
    (
        "bracket subscript evades a dotted-name pattern",
        "names no environment",
        """
name: Bracket
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    steps:
      - env:
          STOLEN: ${{ secrets['APPLE_CERTIFICATE'] }}
        run: echo signing
""",
    ),
    (
        "whole secrets context serialised inside a gated job",
        "serialises the whole secrets context",
        """
name: Context dump
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    environment: signed-dev-acceptance
    steps:
      - env:
          EVERYTHING: ${{ toJSON(secrets) }}
        run: echo signing
""",
    ),
    (
        "secret hoisted to top-level env, belonging to no job",
        "referenced outside any job",
        """
name: Top level
on:
  workflow_dispatch:
env:
  APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
jobs:
  sign:
    runs-on: macos-latest
    environment: signed-dev-acceptance
    steps:
      - run: echo signing
""",
    ),
    (
        "environment name is an expression that cannot be resolved statically",
        "not an approved reviewer-gated environment",
        """
name: Expression environment
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    environment:
      name: ${{ inputs.environment }}
    steps:
      - env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
        run: echo signing
""",
    ),
    (
        "tauri signing key is guarded too, not just APPLE_*",
        "names no environment",
        """
name: Tauri key
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    steps:
      - env:
          KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
        run: echo signing
""",
    ),
]

ACCEPTANCES = [
    ("gated job", GATED),
    (
        "environment given as a mapping, which GitHub also allows",
        """
name: Mapping environment
on:
  workflow_dispatch:
jobs:
  sign:
    runs-on: macos-latest
    environment:
      name: production-release
      url: https://example.invalid
    steps:
      - env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
        run: echo signing
""",
    ),
    (
        "unguarded secret in an ungated job is none of this check's business",
        """
name: Other secret
on:
  workflow_dispatch:
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - env:
          TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: echo building
""",
    ),
]


def run_guard(workflow_source):
    with tempfile.TemporaryDirectory() as directory:
        (Path(directory) / "fixture.yml").write_text(workflow_source)
        return subprocess.run(
            [sys.executable, str(GUARD), directory],
            capture_output=True,
            text=True,
        )


class SigningSecretScopeTests(unittest.TestCase):
    def test_refusals(self):
        for label, expected_reason, source in REFUSALS:
            with self.subTest(case=label):
                result = run_guard(source)
                self.assertNotEqual(result.returncode, 0, f"guard accepted: {label}")
                self.assertIn(
                    expected_reason,
                    result.stderr,
                    f"guard refused '{label}' for the wrong reason:\n{result.stderr}",
                )

    def test_acceptances(self):
        for label, source in ACCEPTANCES:
            with self.subTest(case=label):
                result = run_guard(source)
                self.assertEqual(
                    result.returncode,
                    0,
                    f"guard refused a legitimate workflow ({label}):\n{result.stderr}",
                )

    def test_real_workflows_pass(self):
        result = subprocess.run(
            [sys.executable, str(GUARD)],
            capture_output=True,
            text=True,
            cwd=GUARD.parents[1],
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        # A guard that sees nothing passes vacuously; assert it is actually
        # watching the signing jobs that exist today.
        self.assertIn("secret_bearing_jobs_gated=", result.stdout)
        gated = int(result.stdout.split("secret_bearing_jobs_gated=")[1].split()[0])
        self.assertGreater(gated, 0, "no secret-bearing job found; the guard sees nothing")


if __name__ == "__main__":
    unittest.main()
