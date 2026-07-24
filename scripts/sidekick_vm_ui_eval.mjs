#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testFiles = [
  "scripts/test/native_sidekick_acceptance.test.mjs",
  "scripts/test/native_sidekick_ui_acceptance.test.mjs",
  "scripts/test/tauri_frontend_startup.test.mjs",
  "scripts/test/tauri_startup_bindings.test.mjs",
];
const boundSources = [
  "scripts/sidekick_engine_eval.sh",
  "scripts/sidekick_vm_ui_eval.mjs",
  ...testFiles,
  "scripts/run_native_sidekick_acceptance.mjs",
  "scripts/run_native_sidekick_ui_acceptance.mjs",
  "tauri/src/index.html",
  "tauri/src/sidekick.html",
  "tauri/src/sidekick-acceptance-marker.html",
  "tauri/src-tauri/src/commands.rs",
];

function parseArgs(argv) {
  let output = path.join(root, "target/sidekick-eval/live-sidekick-vm-ui-eval.json");
  for (let index = 2; index < argv.length; index += 1) {
    if (argv[index] !== "--out") throw new Error(`unknown argument: ${argv[index]}`);
    const supplied = argv[++index];
    if (!supplied) throw new Error("--out requires a path");
    output = path.resolve(root, supplied);
  }
  return { output };
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function tapCount(output, label) {
  const match = output.match(new RegExp(`^# ${label} (\\d+)$`, "m"));
  if (!match) throw new Error(`node:test did not report a ${label} count`);
  return Number.parseInt(match[1], 10);
}

function sourceDigest() {
  const hash = createHash("sha256");
  for (const relative of boundSources) {
    const bytes = fs.readFileSync(path.join(root, relative));
    hash.update(relative);
    hash.update("\0");
    hash.update(sha256(bytes));
    hash.update("\0");
  }
  return hash.digest("hex");
}

const { output } = parseArgs(process.argv);
const result = spawnSync(process.execPath, ["--test", ...testFiles], {
  cwd: root,
  encoding: "utf8",
  maxBuffer: 2 * 1024 * 1024,
  timeout: 30_000,
});
if (result.error) throw result.error;

const tap = result.stdout ?? "";
const tests = tapCount(tap, "tests");
const passed = tapCount(tap, "pass");
const failed = tapCount(tap, "fail");
const cancelled = tapCount(tap, "cancelled");
const skipped = tapCount(tap, "skipped");
const todo = tapCount(tap, "todo");
const report = {
  schema_version: "1",
  passed:
    result.status === 0 &&
    tests > 0 &&
    passed === tests &&
    failed === 0 &&
    cancelled === 0 &&
    skipped === 0 &&
    todo === 0,
  source_sha256: sourceDigest(),
  runner_exit_code: result.status,
  tests,
  test_files: testFiles.length,
  assertions: {
    passed,
    failed,
    cancelled,
    skipped,
    todo,
  },
  coverage: {
    production_sidekick_markup_and_handlers: true,
    production_main_window_startup: true,
    production_acceptance_evaluators: true,
    headless_event_order_and_reload_recovery: true,
    startup_reference_error_regression: true,
    native_webview: false,
    signed_app: false,
    native_screen_permission_adapter: false,
    native_microphone_capture: false,
    real_cloud_provider: false,
    release_ready_from_this_report_alone: false,
  },
};

fs.mkdirSync(path.dirname(output), { recursive: true });
fs.writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
console.error(`sidekick_vm_ui_eval_artifact=${output}`);

if (!report.passed) {
  const diagnostics = `${tap}\n${result.stderr ?? ""}`.slice(-16 * 1024);
  process.stderr.write(diagnostics);
  process.exit(1);
}
