#!/usr/bin/env node

import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sdkDirectory = path.join(root, "crates", "sdk");
const mcpDirectory = path.join(root, "crates", "mcp");
const npmCommand = "npm";

function runNpm(args, cwd, { capture = false } = {}) {
  const result = spawnSync(npmCommand, args, {
    cwd,
    encoding: "utf8",
    shell: process.platform === "win32",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`npm ${args.join(" ")} failed with exit code ${result.status ?? "unknown"}`);
  }
  return result.stdout ?? "";
}

async function readJson(file) {
  return JSON.parse(await readFile(file, "utf8"));
}

const args = process.argv.slice(2);
const sdkReady = args.includes("--sdk-ready");
if (args.some((argument) => argument !== "--sdk-ready")) {
  throw new Error("usage: node scripts/install_mcp_dependencies.mjs [--sdk-ready]");
}

const sdkPackage = await readJson(path.join(sdkDirectory, "package.json"));
const mcpPackage = await readJson(path.join(mcpDirectory, "package.json"));
const mcpLock = await readJson(path.join(mcpDirectory, "package-lock.json"));
const pinnedVersion = mcpPackage.dependencies?.["minutes-sdk"];

if (pinnedVersion !== sdkPackage.version) {
  console.log(`Installing MCP dependencies from the published minutes-sdk ${pinnedVersion}.`);
  runNpm(["ci"], mcpDirectory);
  process.exit(0);
}

const lockedSdk = mcpLock.packages?.["node_modules/minutes-sdk"];
const expectedResolved = `https://registry.npmjs.org/minutes-sdk/-/minutes-sdk-${sdkPackage.version}.tgz`;
if (
  mcpLock.packages?.[""]?.dependencies?.["minutes-sdk"] !== sdkPackage.version ||
  lockedSdk?.version !== sdkPackage.version ||
  lockedSdk.resolved !== expectedResolved ||
  typeof lockedSdk.integrity !== "string" ||
  !lockedSdk.integrity.startsWith("sha512-")
) {
  throw new Error(`MCP lockfile does not contain a complete exact minutes-sdk ${sdkPackage.version} pin`);
}

const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "minutes-mcp-local-sdk-"));
try {
  if (!sdkReady) {
    runNpm(["ci"], sdkDirectory);
    runNpm(["run", "build"], sdkDirectory);
  }

  const packOutput = runNpm(
    ["pack", "--json", "--pack-destination", temporaryDirectory],
    sdkDirectory,
    { capture: true },
  );
  const [packed] = JSON.parse(packOutput);
  if (!packed?.filename || !packed?.integrity) throw new Error("npm pack returned no filename or integrity");
  if (packed.integrity !== lockedSdk.integrity) {
    throw new Error(
      `local minutes-sdk ${sdkPackage.version} integrity does not match the MCP lockfile\n` +
        `  lockfile: ${lockedSdk.integrity}\n  local:    ${packed.integrity}`,
    );
  }

  const tarball = path.join(temporaryDirectory, packed.filename);
  const cacheDirectory = path.join(temporaryDirectory, "npm-cache");
  runNpm(["cache", "add", tarball, "--cache", cacheDirectory], root);
  runNpm(["ci", "--cache", cacheDirectory], mcpDirectory);
  console.log(`Installed MCP dependencies from the exact local minutes-sdk ${sdkPackage.version} tarball.`);
} finally {
  await rm(temporaryDirectory, { recursive: true, force: true });
}
