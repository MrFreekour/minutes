#!/usr/bin/env node

import { chmod, lstat, readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const automaticPackageFile = /^(?:readme|license|licence|copying|notice|changes|changelog|history)(?:\.|$)/i;

async function normalizeEntry(target, executableFiles) {
  let stat;
  try {
    stat = await lstat(target);
  } catch (error) {
    if (error?.code === "ENOENT") return;
    throw error;
  }

  if (stat.isSymbolicLink()) return;
  if (stat.isDirectory()) {
    await chmod(target, 0o755);
    for (const entry of await readdir(target)) {
      await normalizeEntry(path.join(target, entry), executableFiles);
    }
    return;
  }
  if (stat.isFile()) {
    await chmod(target, executableFiles.has(path.resolve(target)) ? 0o755 : 0o644);
  }
}

function packageBins(packageJson) {
  if (typeof packageJson.bin === "string") return [packageJson.bin];
  if (packageJson.bin && typeof packageJson.bin === "object") {
    return Object.values(packageJson.bin).filter((value) => typeof value === "string");
  }
  return [];
}

export async function normalizeNpmPackPermissions(packageDirectory) {
  const directory = path.resolve(packageDirectory);
  const packageJson = JSON.parse(await readFile(path.join(directory, "package.json"), "utf8"));
  const bins = packageBins(packageJson);
  const executableFiles = new Set(bins.map((file) => path.resolve(directory, file)));
  const topLevel = await readdir(directory);
  const entries = new Set([
    "package.json",
    ...(Array.isArray(packageJson.files) ? packageJson.files : []),
    ...(typeof packageJson.main === "string" ? [packageJson.main] : []),
    ...bins,
    ...topLevel.filter((file) => automaticPackageFile.test(file)),
  ]);

  for (const entry of entries) {
    if (typeof entry !== "string" || /[*?{}[\]]/.test(entry)) {
      throw new Error(`cannot normalize npm files pattern ${JSON.stringify(entry)}`);
    }
    const target = path.resolve(directory, entry);
    if (target !== directory && !target.startsWith(`${directory}${path.sep}`)) {
      throw new Error(`npm package entry escapes package directory: ${entry}`);
    }
    await normalizeEntry(target, executableFiles);
  }
}

const invokedPath = process.argv[1] ? path.resolve(process.argv[1]) : undefined;
if (invokedPath === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 3) {
    throw new Error("usage: node scripts/normalize_npm_pack_permissions.mjs <package-directory>");
  }
  await normalizeNpmPackPermissions(process.argv[2]);
}
