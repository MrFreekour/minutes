#!/usr/bin/env node

/**
 * Isolated filesystem authorizer for the MCP process_audio surface.
 *
 * This process is deliberately a detached process-group leader. Potentially
 * blocking realpath/open/stat calls happen here, never in the long-lived MCP
 * event loop. After retaining and re-attesting one exact source inode, the
 * helper starts the Minutes CLI in this same process group and maps only that
 * source descriptor to child fd 3. The MCP parent owns the deadline, output
 * budgets, and kill(-pgid) supervision for this complete tree.
 */

import { spawn } from "node:child_process";
import {
  closeSync,
  constants,
  existsSync,
  fstatSync,
  lstatSync,
  openSync,
  realpathSync,
  statSync,
  writeSync,
  type BigIntStats,
} from "node:fs";
import { createInterface } from "node:readline";
import {
  basename,
  dirname,
  extname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";

const REQUEST_MAX_BYTES = 64 * 1024;
const ROOT_UPDATE_MAX_BYTES = 16 * 1024;
const MAX_PATH_CHARS = 16 * 1024;
const MAX_TITLE_CHARS = 16 * 1024;
const MAX_ALLOWED_ROOTS = 16;
const MAX_EXTENSIONS = 16;
const MAX_AUDIO_BYTES = 2 * 1024 * 1024 * 1024;
const AUDIO_FORMATS = new Set(["wav"]);

type HelperRequest = {
  schemaVersion: 1;
  filePath: string;
  allowedDirs: string[];
  audioExts: string[];
  initialMeetingsRoot: string;
  requestedTitle?: string;
  contentType: "meeting" | "memo";
  language?: string;
  cliBinary: string;
  maxBytes: number;
  extraEnv?: Record<string, string>;
};

type RootAttestation = {
  canonicalPath: string;
  identity: string;
};

type SourceExpectation = {
  parentIdentity: string;
  leafFingerprint: string;
};

function fail(): never {
  process.exit(70);
}

function isBoundedString(value: unknown, max: number): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= max &&
    !value.includes("\0")
  );
}

function parseRequest(line: string): HelperRequest {
  if (Buffer.byteLength(line) > REQUEST_MAX_BYTES) fail();
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    fail();
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) fail();
  const request = value as Record<string, unknown>;
  const allowedKeys = new Set([
    "schemaVersion",
    "filePath",
    "allowedDirs",
    "audioExts",
    "initialMeetingsRoot",
    "requestedTitle",
    "contentType",
    "language",
    "cliBinary",
    "maxBytes",
    "extraEnv",
  ]);
  if (Object.keys(request).some((key) => !allowedKeys.has(key))) fail();
  if (
    request.schemaVersion !== 1 ||
    !isBoundedString(request.filePath, MAX_PATH_CHARS) ||
    !Array.isArray(request.allowedDirs) ||
    request.allowedDirs.length < 1 ||
    request.allowedDirs.length > MAX_ALLOWED_ROOTS ||
    request.allowedDirs.some((entry) => !isBoundedString(entry, MAX_PATH_CHARS)) ||
    !Array.isArray(request.audioExts) ||
    request.audioExts.length < 1 ||
    request.audioExts.length > MAX_EXTENSIONS ||
    request.audioExts.some(
      (entry) => typeof entry !== "string" || !/^\.[a-z0-9]{1,10}$/.test(entry)
    ) ||
    request.audioExts.length !== 1 ||
    request.audioExts[0] !== ".wav" ||
    !isBoundedString(request.initialMeetingsRoot, MAX_PATH_CHARS) ||
    (request.requestedTitle !== undefined &&
      !isBoundedString(request.requestedTitle, MAX_TITLE_CHARS)) ||
    (request.contentType !== "meeting" && request.contentType !== "memo") ||
    (request.language !== undefined &&
      (typeof request.language !== "string" ||
        !/^[A-Za-z0-9_-]{1,32}$/.test(request.language))) ||
    !isBoundedString(request.cliBinary, MAX_PATH_CHARS) ||
    !Number.isSafeInteger(request.maxBytes) ||
    (request.maxBytes as number) < 1 ||
    (request.maxBytes as number) > MAX_AUDIO_BYTES ||
    (request.extraEnv !== undefined &&
      (!request.extraEnv ||
        typeof request.extraEnv !== "object" ||
        Array.isArray(request.extraEnv) ||
        Object.keys(request.extraEnv).length > 32 ||
        Object.entries(request.extraEnv).some(
          ([key, entry]) =>
            !/^[A-Z0-9_]{1,128}$/.test(key) ||
            typeof entry !== "string" ||
            entry.length > MAX_PATH_CHARS ||
            entry.includes("\0")
        )))
  ) {
    fail();
  }
  return request as HelperRequest;
}

function identity(info: BigIntStats): string {
  return `${info.dev}:${info.ino}`;
}

function fingerprint(info: BigIntStats): string {
  return [
    info.dev,
    info.ino,
    info.size,
    info.mtimeNs,
    info.ctimeNs,
    info.birthtimeNs,
    info.mode,
    info.nlink,
  ].join(":");
}

function isWithin(candidate: string, root: string): boolean {
  const rootWithSep = root.endsWith(sep) ? root : root + sep;
  return candidate === root || candidate.startsWith(rootWithSep);
}

function canonicalRoot(root: string): string {
  const absolute = resolve(root);
  return existsSync(absolute) ? realpathSync(absolute) : absolute;
}

function attestRoot(root: string): RootAttestation {
  let cursor = resolve(root);
  const missing: string[] = [];
  while (!existsSync(cursor)) {
    const parent = dirname(cursor);
    if (parent === cursor) fail();
    missing.unshift(basename(cursor));
    cursor = parent;
  }
  const ancestorPath = realpathSync(cursor);
  const ancestor = statSync(ancestorPath, { bigint: true });
  if (!ancestor.isDirectory()) fail();
  return {
    canonicalPath:
      missing.length === 0 ? ancestorPath : join(ancestorPath, ...missing),
    identity:
      (missing.length === 0 ? "present:" : "absent:") +
      identity(ancestor) +
      ":" +
      missing.join("/"),
  };
}

function validateSource(
  requestedPath: string,
  allowedRoots: string[],
  allowedExts: string[],
  meetingsRoot: string
): string {
  const source = realpathSync(requestedPath);
  if (!allowedExts.includes(extname(source).toLowerCase())) fail();
  const roots = allowedRoots.map(canonicalRoot);
  if (!roots.some((root) => isWithin(source, root))) fail();
  const retainedRelative = relative(meetingsRoot, source);
  if (
    retainedRelative === "" ||
    (!isAbsolute(retainedRelative) &&
      !retainedRelative.split(/[\\/]+/).some((component) => component === ".."))
  ) {
    fail();
  }
  return source;
}

function captureExpectation(source: string): SourceExpectation {
  const parent = dirname(source);
  const before = statSync(parent, { bigint: true });
  const lexical = lstatSync(source, { bigint: true });
  const live = statSync(source, { bigint: true });
  const after = statSync(parent, { bigint: true });
  if (
    !before.isDirectory() ||
    !after.isDirectory() ||
    realpathSync(parent) !== parent ||
    identity(before) !== identity(after) ||
    lexical.isSymbolicLink() ||
    !lexical.isFile() ||
    lexical.nlink !== 1n ||
    !live.isFile() ||
    live.nlink !== 1n ||
    realpathSync(source) !== source ||
    fingerprint(lexical) !== fingerprint(live)
  ) {
    fail();
  }
  return {
    parentIdentity: identity(before),
    leafFingerprint: fingerprint(live),
  };
}

function requireExpectation(source: string, expected: SourceExpectation): void {
  const actual = captureExpectation(source);
  if (
    actual.parentIdentity !== expected.parentIdentity ||
    actual.leafFingerprint !== expected.leafFingerprint
  ) {
    fail();
  }
}

function sanitizeTitle(value: string): string {
  return (
    value
      .replace(/[\u0000-\u001f\u007f]/g, " ")
      .replace(/[\\/]/g, " ")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 200) || "Untitled Recording"
  );
}

function safeTitle(source: string, callerPath: string, requested?: string): string {
  const fallback = sanitizeTitle(basename(source, extname(source)));
  if (
    requested === undefined ||
    requested.includes(source) ||
    requested.includes(callerPath) ||
    requested.includes("/") ||
    requested.includes("\\")
  ) {
    return fallback;
  }
  const candidate = sanitizeTitle(requested);
  return candidate.includes(source) || candidate.includes(callerPath)
    ? fallback
    : candidate;
}

function sameRoot(left: RootAttestation, right: RootAttestation): boolean {
  return left.canonicalPath === right.canonicalPath && left.identity === right.identity;
}

async function main(): Promise<void> {
  if (process.platform !== "linux" && process.platform !== "darwin") fail();

  let inputBytes = 0;
  process.stdin.on("data", (chunk: Buffer | string) => {
    inputBytes += Buffer.byteLength(chunk);
    if (inputBytes > REQUEST_MAX_BYTES + ROOT_UPDATE_MAX_BYTES + 2) fail();
  });
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  const iterator = lines[Symbol.asyncIterator]();
  const first = await iterator.next();
  if (first.done || typeof first.value !== "string") fail();
  const request = parseRequest(first.value);

  const initialRoot = attestRoot(request.initialMeetingsRoot);
  const source = validateSource(
    request.filePath,
    request.allowedDirs,
    request.audioExts,
    initialRoot.canonicalPath
  );
  const expected = captureExpectation(source);
  requireExpectation(source, expected);
  const sourceFd = openSync(source, constants.O_RDONLY | constants.O_NOFOLLOW);
  let childStarted = false;
  try {
    const initialInfo = fstatSync(sourceFd, { bigint: true });
    if (
      !initialInfo.isFile() ||
      initialInfo.nlink !== 1n ||
      initialInfo.size > BigInt(request.maxBytes) ||
      fingerprint(initialInfo) !== expected.leafFingerprint
    ) {
      fail();
    }
    requireExpectation(source, expected);

    writeSync(
      3,
      JSON.stringify({ status: "authorized", byteLength: Number(initialInfo.size) }) +
        "\n"
    );
    const second = await iterator.next();
    if (
      second.done ||
      typeof second.value !== "string" ||
      Buffer.byteLength(second.value) > ROOT_UPDATE_MAX_BYTES
    ) {
      fail();
    }
    let update: unknown;
    try {
      update = JSON.parse(second.value);
    } catch {
      fail();
    }
    if (
      !update ||
      typeof update !== "object" ||
      Array.isArray(update) ||
      Object.keys(update).join("\0") !== "finalMeetingsRoot" ||
      !isBoundedString(
        (update as Record<string, unknown>).finalMeetingsRoot,
        MAX_PATH_CHARS
      )
    ) {
      fail();
    }
    const finalRoot = attestRoot(
      (update as { finalMeetingsRoot: string }).finalMeetingsRoot
    );
    if (!sameRoot(initialRoot, finalRoot)) fail();
    const finalSource = validateSource(
      source,
      request.allowedDirs,
      request.audioExts,
      finalRoot.canonicalPath
    );
    if (finalSource !== source) fail();
    requireExpectation(source, expected);
    const finalInfo = fstatSync(sourceFd, { bigint: true });
    if (
      !finalInfo.isFile() ||
      finalInfo.nlink !== 1n ||
      finalInfo.size !== initialInfo.size ||
      fingerprint(finalInfo) !== expected.leafFingerprint
    ) {
      fail();
    }

    const format = extname(source).slice(1).toLowerCase();
    if (!AUDIO_FORMATS.has(format)) fail();
    const args = [
      "process",
      `authorized-input.${format}`,
      "-t",
      request.contentType,
      "--title",
      safeTitle(source, request.filePath, request.requestedTitle),
    ];
    if (request.language) args.push("--language", request.language);
    args.push(
      "--authorized-input-fd",
      "3",
      "--authorized-input-bytes",
      String(initialInfo.size),
      "--authorized-input-format",
      format
    );

    const extraEnv = { ...(request.extraEnv ?? {}) };
    delete extraEnv.MINUTES_MCP_OUTER_PROCESS_GROUP;
    const child = spawn(request.cliBinary, args, {
      detached: false,
      stdio: ["ignore", "inherit", "inherit", sourceFd],
      env: {
        ...process.env,
        ...extraEnv,
        RUST_LOG: "info",
        MINUTES_CLI_RESTRICTED_POLICY: "deny",
      },
    });
    childStarted = true;
    child.once("error", fail);
    child.once("close", (code) => {
      closeSync(sourceFd);
      process.exit(typeof code === "number" ? code : 70);
    });
  } catch {
    if (!childStarted) closeSync(sourceFd);
    fail();
  }
}

void main().catch(fail);
