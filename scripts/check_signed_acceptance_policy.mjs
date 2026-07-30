#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

// Updated only after a line-by-line review of the complete unsigned-build
// boundary and secret-bearing job. Raw regex matches are intentionally not the
// authority: these goldens make comment/dead-step duplication fail closed.
const EXPECTED_SIGNING_JOB_SHA256 =
  "f29213abf408b5e69a381bfbefef7db4896368c1369d8acc38f09f8442c41aad";
const EXPECTED_PRE_SIGNING_BOUNDARY_SHA256 =
  "f532d4e841a4be222ccec24b029a4a8f413dcfe6f4688c621475dace9c54c2e6";
const EXPECTED_TRIGGER_BLOCK = `on:
  workflow_dispatch:
    inputs:
      candidate_sha:
        description: Full SHA protected by the matching acceptance-<sha> tag
        required: true
        type: string
`;
const SECRET_CONTEXT_EXPRESSION =
  /\$\{\{(?:(?!\}\})[\s\S])*?\bsecrets\b(?:(?!\}\})[\s\S])*?\}\}/i;

const signingJobFixture = process.argv[2] === "--signing-job-fixture";
const workflowPath = signingJobFixture
  ? process.argv[3]
  : process.argv[2] ?? ".github/workflows/signed-dev-acceptance.yml";
const source = readFileSync(workflowPath, "utf8");
const errors = [];

function requirePattern(pattern, message) {
  if (!pattern.test(source)) errors.push(message);
}

if (!signingJobFixture) {
  const preSigningBoundary = source.split(/^  sign-reviewed-artifact:\n/m, 1)[0];
  const preSigningBoundaryHash = createHash("sha256")
    .update(preSigningBoundary)
    .digest("hex");
  if (preSigningBoundaryHash !== EXPECTED_PRE_SIGNING_BOUNDARY_SHA256) {
    errors.push(
      "the complete trigger, authorization, and unsigned-build boundary changed; review it and update its golden hash",
    );
  }

  const triggerBlock = source.match(/^on:\n[\s\S]*?(?=\npermissions:)/m)?.[0];
  if (triggerBlock !== EXPECTED_TRIGGER_BLOCK) {
    errors.push("signed acceptance must expose only the exact reviewed workflow_dispatch trigger");
  }
  requirePattern(
    /if: github\.ref == 'refs\/heads\/main' && github\.actor == 'silverstein'/,
    "candidate authorization must run only from protected main under the repository owner",
  );
  requirePattern(
    /refs\/tags\/acceptance-\$\{\{ needs\.authorize-candidate\.outputs\.candidate_sha \}\}/,
    "candidate checkout must be bound to its protected acceptance-<sha> tag",
  );
  requirePattern(
    /^  sign-reviewed-artifact:[\s\S]*?^    environment: signed-dev-acceptance$/m,
    "the secret-bearing signing job must use the reviewer-gated environment",
  );
  requirePattern(
    /^  build-unsigned:[\s\S]*?^  sign-reviewed-artifact:/m,
    "candidate code must build in a separate job before signing credentials are available",
  );
  requirePattern(
    /graph_worker_bundle="\$app\/Contents\/XPCServices\/com\.useminutes\.graph-worker\.xpc"[\s\S]*?test ! -e "\$app\/Contents\/MacOS\/minutes-graph-worker"[\s\S]*?--entitlements payload\/graph-worker-entitlements\.plist[\s\S]*?--identifier com\.useminutes\.graph-worker[\s\S]*?--sign "\$MINUTES_DEV_SIGNING_IDENTITY" "\$graph_worker_bundle"/,
    "the nested graph XPC service must receive only its dedicated App Sandbox identity",
  );
  requirePattern(
    /expected = \{"com\.apple\.security\.app-sandbox": True\}[\s\S]*?actual != expected/,
    "signed acceptance must enforce the graph helper's exact one-key entitlement allowlist",
  );
  requirePattern(
    /minutes-graph-worker\.cdhash[\s\S]*?observed_cdhash[\s\S]*?test "\$expected_cdhash" = "\$observed_cdhash"/,
    "signed acceptance must seal and verify the graph helper's exact CodeDirectory hash",
  );
  requirePattern(
    /MINUTES_GRAPH_WORKER_CDHASH_V1=[\s\S]*?contents\.count\(marker\) != 1[\s\S]*?invalid prior parent graph-worker seal[\s\S]*?os\.fsync/,
    "signed acceptance must bind one exact graph-worker hash into the parent before outer signing",
  );
  requirePattern(
    /signed parent is not bound to the exact graph worker/,
    "signed acceptance must verify the final parent-to-helper binding",
  );
}

const signingJob = source.match(
  /^  sign-reviewed-artifact:\n([\s\S]*)$/m,
)?.[1];
if (!signingJob) {
  errors.push("could not isolate the secret-bearing signing job");
} else {
  if (!signingJobFixture && !SECRET_CONTEXT_EXPRESSION.test(signingJob)) {
    errors.push("signing job no longer consumes the expected protected secrets");
  }
  if (/uses:\s*actions\/checkout@/.test(signingJob)) {
    errors.push("the secret-bearing job must never check out or execute candidate source");
  }
  if (/^\s*uses:\s*\.\//m.test(signingJob)) {
    errors.push("the secret-bearing job must never execute a repository-local action");
  }
  if (
    /^\s*(?:run:\s*)?(?:bash|sh|source|\.)\s+["']?(?:payload|signed)\//m.test(
      signingJob,
    ) ||
    /^\s*run:\s*["']?(?:payload|signed)\//m.test(signingJob)
  ) {
    errors.push("the secret-bearing job must never execute candidate-artifact content");
  }

  const expectedSigningSteps = [
    "Download exact unsigned candidate",
    "Verify artifact provenance before unlocking the identity",
    "Import Developer ID identity into an ephemeral keychain",
    "Sign nested executables and outer app inside-out",
    "Verify exact Team identity and package sealed app",
    "Remove ephemeral signing material",
    "Upload short-lived signed acceptance artifact",
  ];
  const signingSteps = [...signingJob.matchAll(/^      - name:\s*(.+)$/gm)].map(
    (match) => match[1].trim(),
  );
  if (JSON.stringify(signingSteps) !== JSON.stringify(expectedSigningSteps)) {
    errors.push("the secret-bearing job step allowlist changed");
  }
  if (!signingJobFixture) {
    const signingJobHash = createHash("sha256").update(signingJob).digest("hex");
    if (signingJobHash !== EXPECTED_SIGNING_JOB_SHA256) {
      errors.push(
        "the complete secret-bearing job changed; review it and update its golden hash",
      );
    }
  }
}

if (!signingJobFixture) {
  const beforeSigningJob = source.split(/^  sign-reviewed-artifact:\n/m, 1)[0];
  if (SECRET_CONTEXT_EXPRESSION.test(beforeSigningJob)) {
    errors.push("signing secrets must not be exposed to candidate authorization or build jobs");
  }

  for (const match of source.matchAll(/^\s*-?\s*uses:\s*([^\s#]+).*$/gm)) {
    const reference = match[1];
    if (!/@[0-9a-f]{40}$/.test(reference)) {
      errors.push(`action is not pinned to a full commit SHA: ${reference}`);
    }
  }
}

if (errors.length) {
  for (const error of errors) console.error(`${workflowPath}: ${error}`);
  process.exitCode = 1;
}
