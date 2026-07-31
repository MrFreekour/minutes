#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

// Updated only after a line-by-line review of the complete unsigned-build
// boundary and secret-bearing signing/notarization job.
const EXPECTED_PRE_SIGNING_BOUNDARY_SHA256 =
  "4a0e63562ee562d1eb9129550c7abc636e15b76ee06265663d214f6431e437e8";
const EXPECTED_SIGNING_JOB_SHA256 =
  "fa048e38bc4a3c51f45bbf8933df9986635e0dda16b33a182800335fdee6ccdb";
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

const workflowPath =
  process.argv[2] ?? ".github/workflows/signed-archive-acceptance.yml";
const source = readFileSync(workflowPath, "utf8");
const errors = [];

function requirePattern(pattern, message) {
  if (!pattern.test(source)) errors.push(message);
}

const triggerBlock = source.match(/^on:\n[\s\S]*?(?=\npermissions:)/m)?.[0];
if (triggerBlock !== EXPECTED_TRIGGER_BLOCK) {
  errors.push(
    "signed Archive acceptance must expose only the exact reviewed workflow_dispatch trigger",
  );
}

const signingJobMarker = "  sign-and-notarize:\n";
const signingJobStart = source.indexOf(signingJobMarker);
const preSigningBoundary =
  signingJobStart >= 0 ? source.slice(0, signingJobStart) : source;
const signingJobTail =
  signingJobStart >= 0
    ? source.slice(signingJobStart + signingJobMarker.length)
    : "";
const nextJobOffset = signingJobTail.search(/^  [a-z][a-z0-9-]*:\n/m);
const signingJob =
  signingJobStart < 0
    ? undefined
    : nextJobOffset >= 0
      ? signingJobTail.slice(0, nextJobOffset).replace(/\n$/, "")
      : signingJobTail;
const afterSigningJob =
  signingJobStart >= 0 && nextJobOffset >= 0
    ? signingJobTail.slice(nextJobOffset)
    : "";

const preSigningBoundaryHash = createHash("sha256")
  .update(preSigningBoundary)
  .digest("hex");
if (preSigningBoundaryHash !== EXPECTED_PRE_SIGNING_BOUNDARY_SHA256) {
  errors.push(
    "the complete Archive trigger, authorization, and unsigned-build boundary changed; review it and update its golden hash",
  );
}

requirePattern(
  /if: github\.ref == 'refs\/heads\/main' && github\.actor == 'silverstein'/,
  "Archive candidate authorization must run only from protected main under the repository owner",
);
requirePattern(
  /refs\/tags\/acceptance-\$\{\{ needs\.authorize-candidate\.outputs\.candidate_sha \}\}/,
  "Archive candidate checkout must be bound to its protected acceptance-<sha> tag",
);
requirePattern(
  /^  sign-and-notarize:[\s\S]*?^    environment: signed-dev-acceptance$/m,
  "the Archive secret-bearing job must use the reviewer-gated environment",
);
requirePattern(
  /^  build-unsigned:[\s\S]*?^  sign-and-notarize:/m,
  "Archive candidate code must build and run in a separate no-secret job",
);
requirePattern(
  /scripts\/archive-native-lifecycle-smoke\.sh "\$app"/,
  "the unsigned Archive candidate must prove native visible-window close-to-purge behavior",
);
requirePattern(
  /document_vault_smoke -- \\\n\s+"\$executable"/,
  "the unsigned Archive candidate must exercise its exact document and worker executable",
);

if (!signingJob) {
  errors.push("could not isolate the Archive secret-bearing signing job");
} else {
  if (!SECRET_CONTEXT_EXPRESSION.test(signingJob)) {
    errors.push("Archive signing job no longer consumes the protected secrets");
  }
  if (/uses:\s*actions\/checkout@/.test(signingJob)) {
    errors.push("the Archive secret-bearing job must never check out candidate source");
  }
  if (/^\s*uses:\s*\.\//m.test(signingJob)) {
    errors.push(
      "the Archive secret-bearing job must never execute a repository-local action",
    );
  }
  if (
    /^\s*(?:run:\s*)?(?:bash|sh|source|\.)\s+["']?(?:payload|signed)\//m.test(
      signingJob,
    ) ||
    /^\s*run:\s*["']?(?:payload|signed)\//m.test(signingJob)
  ) {
    errors.push(
      "the Archive secret-bearing job must never execute candidate-artifact content",
    );
  }

  const expectedSigningSteps = [
    "Download exact unsigned Archive candidate",
    "Verify provenance before unlocking credentials",
    "Import Developer ID identity into an ephemeral keychain",
    "Materialize App Store Connect key",
    "Sign Archive inside-out",
    "Verify identity, notarize, and staple exact app",
    "Remove ephemeral signing material",
    "Upload short-lived notarized Archive pilot",
  ];
  const signingSteps = [...signingJob.matchAll(/^      - name:\s*(.+)$/gm)].map(
    (match) => match[1].trim(),
  );
  if (JSON.stringify(signingSteps) !== JSON.stringify(expectedSigningSteps)) {
    errors.push("the Archive secret-bearing job step allowlist changed");
  }

  const signingJobHash = createHash("sha256").update(signingJob).digest("hex");
  if (signingJobHash !== EXPECTED_SIGNING_JOB_SHA256) {
    errors.push(
      "the complete Archive secret-bearing job changed; review it and update its golden hash",
    );
  }
}

if (SECRET_CONTEXT_EXPRESSION.test(preSigningBoundary)) {
  errors.push(
    "Archive signing secrets must not be exposed to candidate authorization or build jobs",
  );
}
if (SECRET_CONTEXT_EXPRESSION.test(afterSigningJob)) {
  errors.push("post-signing Archive jobs must not receive signing secrets");
}

requirePattern(
  /case "\$APPLE_SIGNING_IDENTITY" in[\s\S]*?"Developer ID Application:"\*/,
  "Archive distribution must require a Developer ID Application identity",
);
requirePattern(
  /test "\$team_id" = "63TMLKT8HN"/,
  "Archive signing must verify the exact Developer Team identity",
);
requirePattern(
  /test "\$identifier" = "com\.useminutes\.archive"/,
  "Archive signing must verify the production bundle identifier",
);
requirePattern(
  /xcrun notarytool submit[\s\S]*?--key "\$ARCHIVE_API_KEY_PATH"[\s\S]*?--wait/,
  "Archive signing must submit the exact app to Apple notarization and wait",
);
requirePattern(
  /xcrun stapler staple "\$app"[\s\S]*?xcrun stapler validate "\$app"[\s\S]*?spctl --assess --type execute/,
  "Archive signing must staple, validate, and pass Gatekeeper assessment",
);
requirePattern(
  /notarized=true\\nstapled=true/,
  "Archive provenance must record notarization and stapling",
);

for (const match of source.matchAll(/^\s*-?\s*uses:\s*([^\s#]+).*$/gm)) {
  const reference = match[1];
  if (!/@[0-9a-f]{40}$/.test(reference)) {
    errors.push(`Archive action is not pinned to a full commit SHA: ${reference}`);
  }
}

if (errors.length) {
  for (const error of errors) console.error(`${workflowPath}: ${error}`);
  process.exitCode = 1;
}
