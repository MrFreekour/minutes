#!/usr/bin/env python3
"""Assert every Apple/Tauri signing secret is confined to an environment.

Why this exists instead of environment-scoped secrets
-----------------------------------------------------
The stronger control is to hold these secrets as environment secrets and delete
the repository-scoped copies, so a workflow that never names an environment
cannot read them at all. That migration is not possible here: GitHub secrets
are write-only -- no API returns an existing value -- and the source material
(the Developer ID `.p12`, its password, the App Store Connect `.p8`, the Tauri
signing key) is not recoverable on the release machine. Deleting the repository
copies would be irreversible and would end the ability to sign anything.

So a repository-scoped secret is readable by any job that names it, and the
protection cannot come from GitHub. It comes from every reference sitting in a
job bound to an environment that confines it. That is checkable, and this
checks it.

What "confined" means, and what it does not
-------------------------------------------
The two signing environments protect differently and both are deliberate:
`production-release` carries a ref policy limiting it to `main` and `v*` tags
with no required reviewer, because it exists to keep the credentials out of
other workflows rather than to add a click to every release;
`signed-dev-acceptance` requires a named reviewer because its builds are
dispatched ad hoc. An earlier version of this file asserted both required a
reviewer. That was a hard-coded belief about remote state and it was false --
recorded here because a check that asserts something untrue is worse than no
check, and this one passed while doing it.

Evasions this refuses
---------------------
An independent reviewer walked signing material past the first version of this
guard four ways, each now covered by a negative control: a dynamic subscript
(`secrets[inputs.name]`) whose name cannot be resolved statically; a composite
action doing the same; `secrets: inherit` handing the whole context to a called
workflow this file cannot see; and a gated job publishing a secret as a job
output for an ungated job to read.

It does not stop someone who can both edit workflows and disable this check;
nothing in the repository can. It is a review aid with teeth, not a kernel
control.
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

# Environments that confine the signing credentials, and the protection each
# one actually carries. Verified against the GitHub API on 2026-08-04.
#
# An earlier version of this file asserted that every one of these required a
# human reviewer. That was a hard-coded belief about remote state, and it was
# false: `production-release` carries only a ref policy. release-macos.yml says
# so deliberately -- that environment exists to confine the credentials to one
# workflow, not to add a click to every release -- so the guard was wrong about
# the policy, not the workflow wrong about the guard.
#
# What matters for these secrets is that a job naming no environment can read
# them, and a job naming one of these cannot escape the confinement. Both
# protections below achieve that; they differ in what else they add.
SIGNING_ENVIRONMENTS = {
    # Ref policy: `main` and `v*` tags only. No required reviewer, by design.
    "production-release": "ref-policy",
    # Required reviewer (silverstein), plus a `main` ref policy.
    "signed-dev-acceptance": "required-reviewer",
}

SECRET_NAME = re.compile(r"^(APPLE_|TAURI_SIGNING_)")

# `secrets.NAME`, `secrets['NAME']`, `secrets["NAME"]`.
SECRET_REFERENCE = re.compile(
    r"secrets\s*(?:\.\s*([A-Za-z_][A-Za-z0-9_]*)"
    r"|\[\s*['\"]([^'\"]+)['\"]\s*\])"
)

# Serialising the whole context leaks every secret at once regardless of name,
# so it is refused anywhere rather than checked against the name pattern.
WHOLE_CONTEXT = re.compile(r"toJSON\s*\(\s*secrets\s*\)")

# `secrets[inputs.name]` and friends. The name is not knowable statically, so
# it cannot be matched against the guarded pattern and must be refused
# outright: an independent reviewer used exactly this to walk a signing secret
# past the previous version of this check.
DYNAMIC_SUBSCRIPT = re.compile(r"secrets\s*\[\s*(?!['\"])")

# `secrets: inherit` hands the entire secrets context to a called workflow,
# whose jobs are out of this file's sight.
INHERIT = re.compile(r"^\s*secrets\s*:\s*inherit\s*$", re.M)


def referenced_secrets(node):
    """Guarded secret names under `node`, plus the unresolvable forms.

    Returns (names, leaks_everything). `leaks_everything` covers the shapes
    where no name can be recovered -- whole-context serialisation and a
    dynamic subscript -- which must be refused rather than name-matched.
    """
    found = set()
    whole_context = False

    def walk(value):
        nonlocal whole_context
        if isinstance(value, str):
            if WHOLE_CONTEXT.search(value) or DYNAMIC_SUBSCRIPT.search(value):
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

        if INHERIT.search(path.read_text()):
            failures.append(
                f"{path}: `secrets: inherit` hands the whole secrets context to "
                "a called workflow, whose jobs this check cannot see"
            )

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

            # A gated job can launder a secret into an ungated one by
            # publishing it as an output. The approval then guards a value that
            # has already left the room.
            outputs = job.get("outputs")
            if isinstance(outputs, dict):
                output_names, output_leak = referenced_secrets(outputs)
                if output_names or output_leak:
                    failures.append(
                        f"{path}: job '{job_id}' publishes signing secret material "
                        "as a job output, where any downstream job can read it "
                        "without an approval"
                    )

            environment = environment_name(job)
            if environment is None:
                failures.append(
                    f"{path}: job '{job_id}' reads {', '.join(sorted(names))} "
                    "but names no environment, so nothing confines it"
                )
            elif environment not in SIGNING_ENVIRONMENTS:
                failures.append(
                    f"{path}: job '{job_id}' reads {', '.join(sorted(names))} "
                    f"behind environment '{environment}', which is not one of the "
                    "environments that confine signing credentials "
                    f"({', '.join(sorted(SIGNING_ENVIRONMENTS))})"
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
