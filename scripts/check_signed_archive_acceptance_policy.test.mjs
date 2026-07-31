#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const workflow = readFileSync(
  ".github/workflows/signed-archive-acceptance.yml",
  "utf8",
);
const directory = mkdtempSync(join(tmpdir(), "minutes-archive-signing-policy-"));

const mutations = [
  {
    name: "extra signing step",
    expected: "complete Archive secret-bearing job changed",
    source: workflow.replace(
      "      - name: Upload short-lived notarized Archive pilot",
      "      - run: env\n\n      - name: Upload short-lived notarized Archive pilot",
    ),
  },
  {
    name: "candidate execution after credential unlock",
    expected: "complete Archive secret-bearing job changed",
    source: workflow.replace(
      "          app=\"signed/Minutes Archive.app\"\n          while IFS=",
      "          app=\"signed/Minutes Archive.app\"\n          bash payload/candidate-controlled.sh\n          while IFS=",
    ),
  },
  {
    name: "secret before signing",
    expected: "Archive signing secrets must not be exposed",
    source: workflow.replace(
      "          CANDIDATE_SHA: ${{ inputs.candidate_sha }}",
      "          CANDIDATE_SHA: ${{ inputs.candidate_sha }}\n          STOLEN_CERT: ${{ secrets.APPLE_CERTIFICATE }}",
    ),
  },
  {
    name: "additional workflow trigger",
    expected: "only the exact reviewed workflow_dispatch trigger",
    source: workflow.replace(
      "\npermissions:\n",
      "\n  workflow_call:\n\npermissions:\n",
    ),
  },
  {
    name: "owner authorization removed",
    expected: "complete Archive trigger, authorization, and unsigned-build boundary changed",
    source: workflow.replace(
      "    if: github.ref == 'refs/heads/main' && github.actor == 'silverstein'",
      "    if: always()",
    ),
  },
  {
    name: "notarization removed",
    expected: "complete Archive secret-bearing job changed",
    source: workflow.replace(
      "          xcrun notarytool submit \"$notary_zip\" \\",
      "          echo skip-notarization \"$notary_zip\" \\",
    ),
  },
];

try {
  for (const mutation of mutations) {
    if (mutation.source === workflow) {
      throw new Error(`fixture mutation did not apply: ${mutation.name}`);
    }
    const fixture = join(directory, `${mutation.name.replaceAll(" ", "-")}.yml`);
    writeFileSync(fixture, mutation.source);
    const result = spawnSync(
      process.execPath,
      ["scripts/check_signed_archive_acceptance_policy.mjs", fixture],
      { encoding: "utf8" },
    );
    if (result.status === 0) {
      throw new Error(`Archive policy accepted ${mutation.name}`);
    }
    if (!result.stderr.includes(mutation.expected)) {
      throw new Error(
        `Archive policy rejected ${mutation.name} for the wrong reason:\n${result.stderr}`,
      );
    }
  }
} finally {
  rmSync(directory, { recursive: true, force: true });
}
