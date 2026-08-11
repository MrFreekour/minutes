#!/usr/bin/env python3
"""Assert the sherpa archive still packages a working engine.

`Release CLI Binaries` runs only on tags and workflow_dispatch, so nothing on a
pull request builds the sherpa archive. A change that stops it shipping a
usable engine therefore stays invisible until someone cuts a release.

That is not hypothetical. #685 moved sherpa into a dlopened cdylib outside the
workspace, which meant `-p minutes-cli --features engine-sherpa` stopped
emitting any sherpa shared library. The archive step still copied `*.so` out of
the CLI's target directory, matched nothing, and would have failed the next
tagged release. Every pull-request gate was green throughout.

An earlier version of this guard matched substrings against a slice of the raw
file. Adversarial review showed it accepted a workflow that deleted the copy,
pointed at the old target directory, commented out the plugin build while
leaving the words in the comment, or verified the build-tree plugin instead of
the packaged one. It also rejected correct alternatives such as
`--manifest-path` and `[[ -f ... ]]`. So this parses the workflow, strips
comments before matching, and keys on the step id rather than its name.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

try:
    import yaml
except ModuleNotFoundError:  # pragma: no cover - environment problem, not logic
    print(
        "PyYAML is required by this check. Install it with `pip install pyyaml`.",
        file=sys.stderr,
    )
    raise SystemExit(2)

REPO = Path(__file__).resolve().parent.parent
WORKFLOW = REPO / ".github/workflows/release-cli.yml"
WORKFLOW_NAME = ".github/workflows/release-cli.yml"
STEP_ID = "sherpa-archive"
LOADER = "scripts/verify_sherpa_plugin_loads.py"
PLUGIN = "libminutes_sherpa.so"
# Every library the archive must contain. The plugin resolves the sherpa pair
# through its own $ORIGIN, so an archive missing them ships a plugin that
# cannot load on a machine that lacks a system copy.
REQUIRED_LIBS = [PLUGIN, "libsherpa-onnx-c-api.so", "libonnxruntime.so"]


def archive_script(workflow_text: str) -> str | None:
    """The `run:` body of the archive step, found by id rather than by name."""
    try:
        document = yaml.safe_load(workflow_text)
    except yaml.YAMLError:
        # A workflow that does not parse cannot run, so this is a failure to
        # report rather than an exception to die on.
        return None
    if not isinstance(document, dict):
        return None
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            if isinstance(step, dict) and step.get("id") == STEP_ID:
                return step.get("run") or ""
    return None


def strip_comments(script: str) -> str:
    """Drop comment lines so commented-out code cannot satisfy a check.

    Only whole-line comments are removed. A trailing `#` inside a shell line
    could be quoted, and guessing wrong there would silently weaken matching.
    """
    return "\n".join(
        line for line in script.splitlines() if not line.lstrip().startswith("#")
    )


def check(workflow_text: str) -> list[str]:
    script = archive_script(workflow_text)
    if script is None:
        return [
            f"no step with `id: {STEP_ID}` in {WORKFLOW_NAME}. The Linux sherpa "
            "archive is the only shipping sherpa engine today, and this id is "
            "what keeps its packaging gated."
        ]

    live = strip_comments(script)
    failures: list[str] = []

    # Excluded from the workspace, so it is never built as a side effect of
    # anything else. Both the subshell-cd and --manifest-path forms are fine.
    builds_plugin = re.search(
        r"cd\s+crates/sherpa-plugin\b[^\n]*cargo\s+build", live
    ) or re.search(
        r"cargo\s+build[^\n]*--manifest-path[^\n]*crates/sherpa-plugin", live
    )
    if not builds_plugin:
        failures.append(
            "the archive step must build crates/sherpa-plugin; since #685 the "
            "CLI build emits no sherpa artifacts of its own"
        )

    # The plugin has to be copied into the archive, not merely mentioned.
    if not re.search(rf"cp\s[^\n]*{re.escape(PLUGIN)}[^\n]*\"\$out", live) and not re.search(
        rf"cp\s[^\n]*\$plugin_dir[^\n]*\"\$out", live
    ):
        failures.append(
            f"the archive step must copy {PLUGIN} into the archive directory"
        )

    # Where the libraries come from, asserted positively. The negative form of
    # this check (ban the old CLI target path next to a `*.so` glob) was walked
    # straight past by simply repointing `plugin_dir` at the old directory,
    # which is the most likely way this actually regresses.
    if not re.search(r"crates/sherpa-plugin/target", live):
        failures.append(
            "the archive step must take its sherpa libraries from "
            "crates/sherpa-plugin/target; the CLI target directory has not "
            "contained any since #685, and looking there matches nothing "
            "while exiting 0"
        )

    # An empty find-and-copy still exits 0, so absence must be asserted. The
    # assertion has to be a real file test naming the library, not any nearby
    # occurrence of the name.
    for lib in REQUIRED_LIBS:
        tested_directly = re.search(
            rf"(test\s+-f|\[\[?\s+-f)\s[^\n]*{re.escape(lib)}", live
        )
        # `for lib in a b c; do test -f "$out/$lib"` covers each name too, but
        # only when the body actually tests a file.
        loop = re.search(r"for\s+\w+\s+in\s([^\n;]*)", live)
        tested_in_loop = bool(
            loop
            and lib in loop.group(1)
            and re.search(r"(test\s+-f|\[\[?\s+-f)", live)
        )
        if not (tested_directly or tested_in_loop):
            failures.append(
                f"the archive step must assert {lib} is present in the archive; "
                "a find-and-copy that matches nothing still exits 0"
            )

    # A packaged CLI runs `--version` happily with an unloadable plugin, since
    # the plugin is dlopened lazily and only when sherpa is selected. The check
    # must also target the packaged copy: verifying the build-tree plugin
    # proves nothing about what shipped.
    # Join backslash continuations so the loader's actual argument is
    # inspected. An earlier version scanned a window that ran to the end of the
    # script and was satisfied by the "$out" in the later `tar` line, which
    # meant it accepted verifying the build-tree plugin.
    logical: list[str] = []
    buffer = ""
    for line in live.splitlines():
        buffer = f"{buffer} {line.strip()}" if buffer else line.strip()
        if buffer.endswith("\\"):
            buffer = buffer[:-1]
            continue
        logical.append(buffer)
        buffer = ""
    if buffer:
        logical.append(buffer)

    invocations = [command for command in logical if LOADER in command]
    if not invocations:
        failures.append(
            f"the archive step must run {LOADER} against the packaged plugin; "
            "a binary smoke test cannot reach the plugin at all"
        )
    elif not any(
        "$out" in command and "$plugin_dir" not in command
        for command in invocations
    ):
        failures.append(
            f"{LOADER} must be pointed at the plugin inside the archive "
            '("$out"), not at the copy in the build tree, which proves '
            "nothing about what shipped"
        )

    return failures


MUTATIONS: list[tuple[str, object]] = [
    ("drops the plugin build", lambda s: s.replace(
        "( cd crates/sherpa-plugin && cargo build --release --locked )", "")),
    ("comments out the plugin build", lambda s: s.replace(
        "( cd crates/sherpa-plugin && cargo build --release --locked )",
        "# ( cd crates/sherpa-plugin && cargo build --release --locked )")),
    ("deletes the plugin copy", lambda s: s.replace(
        '          cp "$plugin_dir/libminutes_sherpa.so" "$out/"\n', "")),
    ("comments out the load verification", lambda s: s.replace(
        f'              "$GITHUB_WORKSPACE/{LOADER}" \\',
        f'              # "$GITHUB_WORKSPACE/{LOADER}" \\')),
    ("repoints plugin_dir at the CLI target dir", lambda s: s.replace(
        'plugin_dir="crates/sherpa-plugin/target/release"',
        'plugin_dir="target/${{ matrix.target }}/release"')),
    ("verifies the build-tree plugin instead of the packaged one", lambda s: s.replace(
        '"$GITHUB_WORKSPACE/$out/libminutes_sherpa.so" )',
        '"$GITHUB_WORKSPACE/$plugin_dir/libminutes_sherpa.so" )')),
    ("guts the presence loop", lambda s: s.replace(
        '            test -f "$out/$lib" \\\n'
        '              || { echo "::error::$lib missing from the sherpa archive"; exit 1; }\n',
        "            :\n")),
    ("removes the step id", lambda s: s.replace(f"        id: {STEP_ID}\n", "")),
]

# Correct alternatives that must NOT be rejected, because a guard that only
# accepts one spelling gets worked around rather than satisfied.
ACCEPTABLE: list[tuple[str, object]] = [
    ("renames the step but keeps its id", lambda s: s.replace(
        "- name: Build sherpa-enabled Linux archive",
        "- name: Package the sherpa engine for Linux")),
    ("builds the plugin with --manifest-path", lambda s: s.replace(
        "( cd crates/sherpa-plugin && cargo build --release --locked )",
        "cargo build --release --locked --manifest-path crates/sherpa-plugin/Cargo.toml")),
]


def self_test() -> int:
    workflow_text = WORKFLOW.read_text(encoding="utf8")
    live = check(workflow_text)
    if live:
        print("self-test aborted: the committed workflow already fails", file=sys.stderr)
        for failure in live:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    failed = 0
    for label, mutate in MUTATIONS:
        mutated = mutate(workflow_text)
        if mutated == workflow_text:
            print(f"self-test FAILED: mutation that {label} changed nothing", file=sys.stderr)
            failed += 1
            continue
        if not check(mutated):
            print(f"self-test FAILED: guard accepted a workflow that {label}", file=sys.stderr)
            failed += 1
        else:
            print(f"self-test ok: rejected a workflow that {label}")

    for label, mutate in ACCEPTABLE:
        mutated = mutate(workflow_text)
        if mutated == workflow_text:
            print(f"self-test FAILED: variant that {label} changed nothing", file=sys.stderr)
            failed += 1
            continue
        found = check(mutated)
        if found:
            print(
                f"self-test FAILED: guard rejected a correct workflow that {label}: {found}",
                file=sys.stderr,
            )
            failed += 1
        else:
            print(f"self-test ok: accepted a correct workflow that {label}")

    return 0 if failed == 0 else 1


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    failures = check(WORKFLOW.read_text(encoding="utf8"))
    if failures:
        print(f"{WORKFLOW_NAME}: sherpa packaging contract broken", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print("sherpa packaging contract holds")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
