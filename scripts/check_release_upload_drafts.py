#!/usr/bin/env python3
"""Require release asset upload actions to preserve prepared draft releases.

softprops/action-gh-release creates or updates the release for a tag. If an
asset-upload step omits ``draft: true``, it can publish a release while other
platform lanes are still running. This guard covers every release workflow so
adding a new upload step cannot silently reintroduce that failure mode.
"""

from __future__ import annotations

import copy
import sys
from pathlib import Path

import yaml


REPO = Path(__file__).resolve().parent.parent
WORKFLOW_DIR = REPO / ".github/workflows"
ACTION_PREFIX = "softprops/action-gh-release@"


def release_workflows() -> list[Path]:
    return sorted(WORKFLOW_DIR.glob("release-*.yml"))


def check_document(document: object, source: str) -> list[str]:
    failures: list[str] = []
    if not isinstance(document, dict):
        return [f"{source}: workflow is not a mapping"]

    jobs = document.get("jobs")
    if not isinstance(jobs, dict):
        return [f"{source}: workflow has no jobs mapping"]

    for job_name, job in jobs.items():
        if not isinstance(job, dict):
            continue
        for index, step in enumerate(job.get("steps") or []):
            if not isinstance(step, dict):
                continue
            uses = step.get("uses")
            if not isinstance(uses, str) or not uses.startswith(ACTION_PREFIX):
                continue
            with_values = step.get("with")
            if not isinstance(with_values, dict) or with_values.get("draft") is not True:
                label = step.get("name") or f"step {index + 1}"
                failures.append(
                    f"{source}: job {job_name!r}, {label!r} must set "
                    "`with: draft: true`; otherwise an asset upload can publish "
                    "the release before every platform lane finishes"
                )
    return failures


def load(path: Path) -> object:
    return yaml.safe_load(path.read_text(encoding="utf8"))


def check_repo() -> list[str]:
    failures: list[str] = []
    for path in release_workflows():
        failures.extend(check_document(load(path), str(path.relative_to(REPO))))
    return failures


def self_test() -> int:
    found = 0
    failed = 0
    for path in release_workflows():
        document = load(path)
        jobs = document.get("jobs") if isinstance(document, dict) else None
        if not isinstance(jobs, dict):
            continue
        for job_name, job in jobs.items():
            if not isinstance(job, dict):
                continue
            for index, step in enumerate(job.get("steps") or []):
                if not isinstance(step, dict):
                    continue
                uses = step.get("uses")
                if not isinstance(uses, str) or not uses.startswith(ACTION_PREFIX):
                    continue
                found += 1
                mutated = copy.deepcopy(document)
                mutated_step = mutated["jobs"][job_name]["steps"][index]
                mutated_step.setdefault("with", {}).pop("draft", None)
                if check_document(mutated, str(path.relative_to(REPO))):
                    print(f"self-test ok: rejected missing draft flag in {path.name}:{job_name}")
                else:
                    print(
                        f"self-test FAILED: accepted missing draft flag in {path.name}:{job_name}",
                        file=sys.stderr,
                    )
                    failed += 1
    if found == 0:
        print("self-test FAILED: found no release upload actions", file=sys.stderr)
        return 1
    return 1 if failed else 0


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if sys.argv[1:]:
        print("usage: check_release_upload_drafts.py [--self-test]", file=sys.stderr)
        return 2

    failures = check_repo()
    for failure in failures:
        print(failure, file=sys.stderr)
    if failures:
        return 1
    print("release_upload_draft_policy=passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
