#!/usr/bin/env python3
"""Assert every Apple/Tauri signing secret is reachable only behind an approved
environment gate.

Why this exists instead of environment-scoped secrets
-----------------------------------------------------
The stronger control is to hold these secrets as environment secrets and delete
the repository-scoped copies, so a workflow that never names an environment
cannot read them at all. That migration is not currently possible: GitHub
secrets are write-only -- no API returns an existing value -- and the source
material for these particular secrets (the Developer ID `.p12`, its password,
the App Store Connect `.p8`, the Tauri signing key) is not recoverable on the
release machine. Deleting the repository copies would therefore be irreversible
and would end the ability to sign anything.

So the boundary is enforced here instead. A repository-scoped secret is
readable by any job that names it, which means the protection cannot come from
GitHub and has to come from review: every reference must sit in a job bound to
an environment that requires a human reviewer. That is checkable, and this
checks it.

What this does and does not stop
--------------------------------
It stops a job from quietly gaining access to signing material without an
approval gate -- the accidental case, and the deliberate case that hopes nobody
reads the diff. It does not stop someone who can both edit workflows and
disable this check; nothing in the repository can. It is a review aid with
teeth, not a kernel control.
"""

import re
import sys
from pathlib import Path

try:
    import yaml
except ModuleNotFoundError:  # pragma: no cover - environment problem, not logic
    # Fail loudly. A guard that cannot parse must never be mistaken for a guard
    # that found nothing to complain about.
    print(
        "PyYAML is required by this check. Install it with `pip install pyyaml`.",
        file=sys.stderr,
    )
    raise SystemExit(1)

WORKFLOW_DIR = Path(".github/workflows")

# Environments that gate on a human reviewer. Adding a name here widens the
# boundary, so it is a deliberate, reviewable edit rather than a lookup.
APPROVED_ENVIRONMENTS = frozenset({
    "production-release",
    "signed-dev-acceptance",
})

SECRET_NAME = re.compile(r"^(APPLE_|TAURI_SIGNING_)")

# `secrets.NAME`, `secrets['NAME']`, `secrets["NAME"]`.
SECRET_REFERENCE = re.compile(
    r"secrets\s*(?:\.\s*([A-Za-z_][A-Za-z0-9_]*)"
    r"|\[\s*['\"]([^'\"]+)['\"]\s*\])"
)

# Serialising the whole context leaks every secret at once regardless of name,
# so it is refused anywhere rather than checked against the name pattern.
WHOLE_CONTEXT = re.compile(r"toJSON\s*\(\s*secrets\s*\)")


def referenced_secrets(node):
    """Every guarded secret name appearing anywhere under `node`."""
    found = set()
    whole_context = False

    def walk(value):
        nonlocal whole_context
        if isinstance(value, str):
            if WHOLE_CONTEXT.search(value):
                whole_context = True
            for dotted, bracketed in SECRET_REFERENCE.findall(value):
                name = dotted or bracketed
                if SECRET_NAME.match(name):
                    found.add(name)
        elif isinstance(value, dict):
            for key, item in value.items():
                walk(key)
                walk(item)
        elif isinstance(value, list):
            for item in value:
                walk(item)

    walk(node)
    return found, whole_context


def environment_name(job):
    """The environment a job is bound to, or None.

    Accepts both spellings GitHub allows: a bare string, and a mapping with a
    `name` key. A mapping whose name is an expression cannot be resolved
    statically, so it is returned verbatim and will fail the membership test --
    deliberately, because an expression could evaluate to an ungated
    environment.
    """
    environment = job.get("environment")
    if environment is None:
        return None
    if isinstance(environment, str):
        return environment
    if isinstance(environment, dict):
        return environment.get("name")
    return None


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    # An explicit directory is what the mutation test drives; without it the
    # check runs against the real workflows.
    workflow_dir = Path(argv[0]) if argv else WORKFLOW_DIR
    if not workflow_dir.is_dir():
        print(f"no workflow directory at {workflow_dir}", file=sys.stderr)
        return 1

    failures = []
    checked_jobs = 0
    gated_jobs = 0

    for path in sorted(workflow_dir.glob("*.yml")) + sorted(
        workflow_dir.glob("*.yaml")
    ):
        try:
            document = yaml.safe_load(path.read_text())
        except yaml.YAMLError as error:
            failures.append(f"{path}: could not be parsed: {error}")
            continue
        if not isinstance(document, dict):
            continue

        jobs = document.get("jobs")
        if not isinstance(jobs, dict):
            continue

        # Anything outside `jobs` -- defaults, top-level env, reusable-workflow
        # `with:` blocks -- can carry a secret reference that belongs to no job
        # and therefore to no environment. Refuse it outright.
        outside = {key: value for key, value in document.items() if key != "jobs"}
        names, whole_context = referenced_secrets(outside)
        if whole_context:
            failures.append(
                f"{path}: the whole secrets context is serialised outside any job"
            )
        for name in sorted(names):
            failures.append(
                f"{path}: {name} is referenced outside any job, so no environment gates it"
            )

        for job_id, job in jobs.items():
            if not isinstance(job, dict):
                continue
            checked_jobs += 1
            names, whole_context = referenced_secrets(job)
            if whole_context:
                failures.append(
                    f"{path}: job '{job_id}' serialises the whole secrets context"
                )
            if not names:
                continue

            environment = environment_name(job)
            if environment is None:
                failures.append(
                    f"{path}: job '{job_id}' reads {', '.join(sorted(names))} "
                    "but names no environment, so it needs no approval"
                )
            elif environment not in APPROVED_ENVIRONMENTS:
                failures.append(
                    f"{path}: job '{job_id}' reads {', '.join(sorted(names))} "
                    f"behind environment '{environment}', which is not an "
                    "approved reviewer-gated environment"
                )
            else:
                gated_jobs += 1

    if failures:
        print("Signing secret scope check failed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(
        f"signing_secret_scope=passed jobs_checked={checked_jobs} "
        f"secret_bearing_jobs_gated={gated_jobs}"
    )
    if gated_jobs == 0:
        # Nothing referenced a signing secret. That is either a repository with
        # no signing, or a check that has stopped seeing what it guards -- and
        # the second is indistinguishable from the first at a glance.
        print(
            "warning: no job referenced a guarded signing secret; "
            "verify the name pattern still matches",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
