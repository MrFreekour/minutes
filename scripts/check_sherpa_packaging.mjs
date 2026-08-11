#!/usr/bin/env node

// The sherpa archive is built only by `Release CLI Binaries`, which runs on
// tags and workflow_dispatch. Nothing on a pull request exercises it, so a
// change that stops the archive shipping a working engine stays invisible
// until someone cuts a release.
//
// That is not hypothetical. #685 moved sherpa into a dlopened cdylib outside
// the workspace, which meant `-p minutes-cli --features engine-sherpa` stopped
// emitting any sherpa shared library. The archive step still copied `*.so` out
// of the CLI's target directory, found nothing, and would have failed the next
// tagged release. Every pull-request gate was green throughout.
//
// So this guard asserts the packaging contract statically, on every PR: the
// archive must build the plugin crate, ship the plugin next to the binary
// along with the sherpa libraries it resolves through `$ORIGIN`, and prove the
// result actually loads outside the build tree.

import { readFileSync } from "node:fs";

const WORKFLOW = ".github/workflows/release-cli.yml";
const LOADER = "scripts/verify_sherpa_plugin_loads.py";

/** Extract the `run:` body of the sherpa archive step. */
function sherpaArchiveStep(workflow) {
  const marker = "- name: Build sherpa-enabled Linux archive";
  const start = workflow.indexOf(marker);
  if (start === -1) return null;
  // The step ends where the next step at the same indentation begins.
  const rest = workflow.slice(start + marker.length);
  const next = rest.indexOf("\n      - name:");
  return next === -1 ? rest : rest.slice(0, next);
}

function checkWorkflow(workflow) {
  const failures = [];
  const step = sherpaArchiveStep(workflow);
  if (step === null) {
    return [
      `${WORKFLOW} has no "Build sherpa-enabled Linux archive" step; the ` +
        "Linux sherpa artifact is the only shipping sherpa engine today",
    ];
  }

  // Excluded from the workspace, so it never gets built as a side effect.
  if (!/cd crates\/sherpa-plugin[\s\S]*?cargo build --release/.test(step)) {
    failures.push(
      "the archive step must build crates/sherpa-plugin; since #685 the CLI " +
        "build emits no sherpa artifacts of its own",
    );
  }

  // The engine is unreachable without the plugin beside the binary, because
  // that is one of the paths sherpa_plugin::candidate_paths searches.
  if (!step.includes("libminutes_sherpa.so")) {
    failures.push(
      "the archive step must copy libminutes_sherpa.so into the archive",
    );
  }

  // Copying from the CLI's target dir is the stale pattern that broke: it
  // silently matches nothing now that sherpa-rs is not in that graph.
  if (/find "target\/\$\{\{ matrix\.target \}\}\/release"[^\n]*\*\.so/.test(step)) {
    failures.push(
      "the archive step copies *.so out of the CLI target dir, which no " +
        "longer contains any sherpa library; copy from the plugin's target dir",
    );
  }

  // An empty `find` copy loop exits 0, so absence has to be asserted rather
  // than inferred from the copy succeeding. Both names must appear inside a
  // presence check, not merely somewhere in the step.
  const presenceAssertions = step.match(/test -f[^\n]*\n?[^\n]*/g) ?? [];
  const asserted = presenceAssertions.join("\n") + "\n" + (step.match(/for lib in [^\n]*/g) ?? []).join("\n");
  for (const lib of ["libminutes_sherpa.so", "libsherpa-onnx-c-api.so"]) {
    if (!asserted.includes(lib)) {
      failures.push(
        `the archive step must assert ${lib} is present; a find-and-copy that ` +
          "matches nothing still exits 0",
      );
    }
  }

  // A packaged CLI runs `--version` happily with an unloadable plugin, because
  // the plugin is dlopened lazily and only when sherpa is selected.
  if (!step.includes(LOADER)) {
    failures.push(
      `the archive step must run ${LOADER} against the packaged plugin, ` +
        "outside the build tree; a binary smoke test cannot reach it",
    );
  }

  return failures;
}

function selfTest() {
  const workflow = readFileSync(WORKFLOW, "utf8");
  const live = checkWorkflow(workflow);
  if (live.length > 0) {
    console.error("self-test aborted: the committed workflow already fails");
    for (const failure of live) console.error(`  - ${failure}`);
    return 1;
  }

  // Each mutation is a real regression this guard exists to catch, and the
  // guard has to fail on every one of them or it is decoration.
  const mutations = [
    [
      "drops the plugin build",
      (w) => w.replace("( cd crates/sherpa-plugin && cargo build --release --locked )", ""),
    ],
    [
      "stops shipping the plugin",
      (w) => w.replaceAll("libminutes_sherpa.so", "libminutes_placeholder.so"),
    ],
    [
      "drops the load verification",
      (w) => w.replaceAll(LOADER, "scripts/does_not_verify.py"),
    ],
    [
      "removes the step entirely",
      (w) => w.replace("- name: Build sherpa-enabled Linux archive", "- name: Something else"),
    ],
  ];

  let failed = 0;
  for (const [label, mutate] of mutations) {
    const found = checkWorkflow(mutate(workflow));
    if (found.length === 0) {
      console.error(`self-test FAILED: guard accepted a workflow that ${label}`);
      failed += 1;
    } else {
      console.log(`self-test ok: rejected a workflow that ${label}`);
    }
  }
  return failed === 0 ? 0 : 1;
}

function main() {
  if (process.argv.includes("--self-test")) return selfTest();

  const failures = checkWorkflow(readFileSync(WORKFLOW, "utf8"));
  if (failures.length > 0) {
    console.error(`${WORKFLOW}: sherpa packaging contract broken`);
    for (const failure of failures) console.error(`  - ${failure}`);
    return 1;
  }
  console.log("sherpa packaging contract holds");
  return 0;
}

process.exit(main());
