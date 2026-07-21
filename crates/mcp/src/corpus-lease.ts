import { createHash, randomBytes } from "node:crypto";
import { constants, watch, type Dirent, type FSWatcher } from "node:fs";
import { lstat, mkdir, open, opendir, realpath, stat } from "node:fs/promises";
import { basename, extname, isAbsolute, join, relative } from "node:path";

import {
  decodePolicyUtf8,
  fingerprintTextFileFromBoundParent,
  readTextFileWithRevisionFromBoundParent,
  type BoundFileRevision,
} from "./secure-read.js";

const MAX_AUTHORIZATION_ATTEMPTS = 2;
const DEFAULT_FENCE_TIMEOUT_MS = 5_000;
const DEFAULT_AUTHORIZATION_TIMEOUT_MS = 15_000;
const MAX_ACTIVE_WATCHERS = 64;
// Snapshot content is retained as JavaScript strings, whose backing storage
// may require two bytes per source byte. Reserve that worst case for the full
// lease so concurrent agent requests cannot each retain an 80 MiB corpus.
const MAX_RETAINED_CORPUS_MEMORY_BYTES = 256 * 1024 * 1024;
const RETAINED_FILE_OBJECT_OVERHEAD_BYTES = 2 * 1024;
const RETAINED_DIRECTORY_ENTRY_OVERHEAD_BYTES = 2 * 1024;
const RETAINED_DIRECTORY_OVERHEAD_BYTES = 4 * 1024;
const MAX_RETAINED_SENTINELS = MAX_ACTIVE_WATCHERS * 2;
const MAX_SENTINEL_NAMESPACE_ENTRIES = 2;
const SENTINEL_NAMESPACE = ".minutes-corpus-lease-v1";
const SENTINEL_BASENAME = /^lease-shared-[01]\.fence$/;
const SENTINEL_TOKEN_BYTES = 32;
const INACTIVE_CORPUS_DIRS = new Set([
  "archive",
  "processed",
  "failed",
  "failed-captures",
]);

export type CorpusReadBudgets = {
  maxFileBytes: number;
  maxCorpusBytes: number;
  maxRetainedPathBytes: number;
  maxFileCount: number;
  maxDirectoryCount: number;
  maxDirectoryEntries: number;
  maxWatcherCount: number;
  maxReaderCount: number;
};

export const DEFAULT_CORPUS_READ_BUDGETS: Readonly<CorpusReadBudgets> =
  Object.freeze({
    maxFileBytes: 16 * 1024 * 1024,
    maxCorpusBytes: 80 * 1024 * 1024,
    maxRetainedPathBytes: 8 * 1024 * 1024,
    maxFileCount: 4_096,
    maxDirectoryCount: 512,
    maxDirectoryEntries: 8_192,
    maxWatcherCount: 512,
    maxReaderCount: 64,
  });

export type CorpusVerificationStats = Readonly<{
  fileCount: number;
  retainedContentBytes: number;
  totalBytes: number;
}>;

export type StableCorpusFile = Readonly<{
  readonly path: string;
  readonly relativePath: string;
  readonly content: string;
}>;

export type StableCorpusSnapshot = Readonly<{
  readonly canonicalRoot: string;
  files: readonly StableCorpusFile[];
}>;

export type CorpusLeaseControls = {
  failWatcher: (reason?: string) => void;
  suppressNextFence: () => void;
  requireRepulseForNextFence: () => void;
  failNextFencePulse: () => void;
  failNextSentinelOpen: () => void;
  pauseNextSentinelOpen: (
    until: Promise<void>,
    onReserved?: () => void
  ) => void;
  pauseNextFenceAfterPending: (
    until: Promise<void>,
    onPending?: () => void
  ) => void;
};

/**
 * Test/diagnostic hooks for deterministic authorization-race coverage.
 * Every awaited hook runs before the final sentinel fence; the successful
 * final fence remains the operation's linearization point.
 */
export type CorpusLeaseHooks = {
  /** Explicit corpus resource limits; omitted fields use the safe defaults. */
  budgets?: Partial<CorpusReadBudgets>;
  timeoutMs?: number;
  beforeSentinelCreate?: (
    context: {
      attempt: number;
      slot: number;
      capacity: Readonly<{
        globalReserved: number;
        globalRetained: number;
        rootReserved: number;
      }>;
    }
  ) => void | Promise<void>;
  onWatcherReady?: (
    context: { attempt: number; controls: CorpusLeaseControls }
  ) => void | Promise<void>;
  afterBaseline?: (
    context: { attempt: number; controls: CorpusLeaseControls }
  ) => void | Promise<void>;
  beforeFinalManifest?: (
    context: { attempt: number; controls: CorpusLeaseControls }
  ) => void | Promise<void>;
  afterFinalManifest?: (
    context: {
      attempt: number;
      controls: CorpusLeaseControls;
      verification: CorpusVerificationStats;
    }
  ) => void | Promise<void>;
  beforeFinalFence?: (
    context: { attempt: number; controls: CorpusLeaseControls }
  ) => void | Promise<void>;
};

type RootIdentity = {
  canonicalRoot: string;
  fingerprint: string;
};

type Manifest = {
  fingerprint: string;
  snapshot?: StableCorpusSnapshot;
  verification: CorpusVerificationStats;
};

type PendingFence = {
  reject: (error: Error) => void;
  resolve: () => void;
  sentinel: LiveSentinel;
  suppressEntireFence: boolean;
  token: Buffer;
};

type LiveSentinel = {
  directory: string;
  handle: Awaited<ReturnType<typeof open>>;
  inUse: boolean;
  lastUsed: number;
  name: string;
  path: string;
};

type SentinelOpenControl = {
  fail: boolean;
  onReserved?: () => void;
  pauseUntil?: Promise<void>;
};

type FencePendingControl = {
  onPending?: () => void;
  pauseUntil?: Promise<void>;
};

let activeWatcherCount = 0;
let reservedCorpusMemoryBytes = 0;
let retainedSentinelUseSequence = 0;
const retainedSentinels = new Map<string, LiveSentinel>();
let reservedSentinelCreations = 0;
const reservedSentinelCreationsByDirectory = new Map<string, number>();

type SentinelCapacityReservation = {
  directory: string;
  released: boolean;
};

class CorpusLeaseChangedError extends Error {}
class CorpusLeaseBudgetError extends Error {}

type CorpusMemoryReservation = {
  bytes: number;
  released: boolean;
};

function reserveCorpusMemory(
  budgets: Readonly<CorpusReadBudgets>
): CorpusMemoryReservation {
  // Retained UTF-8 may widen to two-byte JS strings. Path/name metadata has
  // the same widening risk, and each retained file has a fixed conservative
  // object/array/hash overhead. One max-sized source Buffer may coexist with
  // the already-retained strings while it is decoded.
  const bytes =
    budgets.maxCorpusBytes * 2 +
    budgets.maxFileBytes +
    budgets.maxRetainedPathBytes * 2 +
    budgets.maxFileCount * RETAINED_FILE_OBJECT_OVERHEAD_BYTES +
    budgets.maxDirectoryEntries * RETAINED_DIRECTORY_ENTRY_OVERHEAD_BYTES +
    budgets.maxDirectoryCount * RETAINED_DIRECTORY_OVERHEAD_BYTES;
  if (
    !Number.isSafeInteger(bytes) ||
    bytes < 0 ||
    reservedCorpusMemoryBytes > MAX_RETAINED_CORPUS_MEMORY_BYTES - bytes
  ) {
    throw new CorpusLeaseBudgetError(
      "meeting corpus retained snapshots exceeded their process budget"
    );
  }
  // Synchronous admission: no peer can interleave between the check and
  // charge, even when several MCP handlers begin in the same event-loop turn.
  reservedCorpusMemoryBytes += bytes;
  return { bytes, released: false };
}

function releaseCorpusMemory(reservation: CorpusMemoryReservation): void {
  if (reservation.released) return;
  reservation.released = true;
  reservedCorpusMemoryBytes = Math.max(
    0,
    reservedCorpusMemoryBytes - reservation.bytes
  );
}

function resolveCorpusReadBudgets(
  requested: Partial<CorpusReadBudgets> | undefined
): Readonly<CorpusReadBudgets> {
  const candidate = { ...DEFAULT_CORPUS_READ_BUDGETS, ...requested };
  if (
    !Number.isSafeInteger(candidate.maxFileBytes) ||
    candidate.maxFileBytes < 0 ||
    !Number.isSafeInteger(candidate.maxCorpusBytes) ||
    candidate.maxCorpusBytes < 0 ||
    !Number.isSafeInteger(candidate.maxRetainedPathBytes) ||
    candidate.maxRetainedPathBytes < 0 ||
    !Number.isSafeInteger(candidate.maxFileCount) ||
    candidate.maxFileCount < 0 ||
    !Number.isSafeInteger(candidate.maxDirectoryCount) ||
    candidate.maxDirectoryCount < 1 ||
    !Number.isSafeInteger(candidate.maxDirectoryEntries) ||
    candidate.maxDirectoryEntries < 0 ||
    !Number.isSafeInteger(candidate.maxWatcherCount) ||
    candidate.maxWatcherCount < 1 ||
    !Number.isSafeInteger(candidate.maxReaderCount) ||
    candidate.maxReaderCount < 1
  ) {
    throw new Error("Access denied: invalid meeting corpus read budget");
  }
  const budgets: CorpusReadBudgets = {
    maxFileBytes: Math.min(candidate.maxFileBytes, DEFAULT_CORPUS_READ_BUDGETS.maxFileBytes),
    maxCorpusBytes: Math.min(candidate.maxCorpusBytes, DEFAULT_CORPUS_READ_BUDGETS.maxCorpusBytes),
    maxRetainedPathBytes: Math.min(
      candidate.maxRetainedPathBytes,
      DEFAULT_CORPUS_READ_BUDGETS.maxRetainedPathBytes
    ),
    maxFileCount: Math.min(candidate.maxFileCount, DEFAULT_CORPUS_READ_BUDGETS.maxFileCount),
    maxDirectoryCount: Math.min(candidate.maxDirectoryCount, DEFAULT_CORPUS_READ_BUDGETS.maxDirectoryCount),
    maxDirectoryEntries: Math.min(candidate.maxDirectoryEntries, DEFAULT_CORPUS_READ_BUDGETS.maxDirectoryEntries),
    maxWatcherCount: Math.min(candidate.maxWatcherCount, DEFAULT_CORPUS_READ_BUDGETS.maxWatcherCount),
    maxReaderCount: Math.min(candidate.maxReaderCount, DEFAULT_CORPUS_READ_BUDGETS.maxReaderCount),
  };
  return Object.freeze(budgets);
}

function resolveFenceTimeout(timeoutMs: number | undefined): number {
  const requested = timeoutMs ?? DEFAULT_FENCE_TIMEOUT_MS;
  if (!Number.isSafeInteger(requested) || requested < 1) {
    throw new Error("Access denied: invalid meeting corpus fence timeout");
  }
  return Math.min(requested, DEFAULT_FENCE_TIMEOUT_MS);
}

function authorizationDeadline(timeoutMs: number | undefined): bigint {
  const requested = timeoutMs ?? DEFAULT_AUTHORIZATION_TIMEOUT_MS;
  if (!Number.isSafeInteger(requested) || requested < 1) {
    throw new Error("Access denied: invalid meeting corpus authorization timeout");
  }
  return process.hrtime.bigint() + BigInt(Math.min(requested, DEFAULT_AUTHORIZATION_TIMEOUT_MS)) * 1_000_000n;
}

function remainingAuthorizationMs(deadline: bigint): number {
  const remainingNs = deadline - process.hrtime.bigint();
  if (remainingNs <= 0n) {
    throw new CorpusLeaseChangedError("meeting corpus authorization deadline elapsed");
  }
  return Math.max(1, Number((remainingNs + 999_999n) / 1_000_000n));
}

function normalizedRelativePath(path: string): string {
  return path.replaceAll("\\", "/");
}

function activeRelativePath(path: string): boolean {
  if (!path || isAbsolute(path)) return false;
  return normalizedRelativePath(path)
    .split("/")
    .every(
      (component) =>
        component.length > 0 &&
        component !== ".." &&
        !component.startsWith(".") &&
        !INACTIVE_CORPUS_DIRS.has(component.toLowerCase())
    );
}

function metadataFingerprint(info: any): string {
  return [
    info.dev,
    info.ino,
    info.size,
    info.mtimeNs ?? info.mtimeMs,
    info.ctimeNs ?? info.ctimeMs,
    info.birthtimeNs ?? info.birthtimeMs,
    info.mode,
    info.nlink,
  ]
    .map(String)
    .join(":");
}

function sentinelIdentityMetadataAccepted(info: any): boolean {
  if (!info.isFile() || info.isSymbolicLink() || BigInt(info.nlink) !== 1n) {
    return false;
  }
  // Windows' mode bits do not describe its ACL. The empty sentinel is not an
  // authorization capability there: its event is only an ordering hint, and
  // the post-fence full root/manifest reread is the authorization boundary.
  // Path/handle identity still prevents accidental reuse. Never claim POSIX
  // owner/mode proof on Windows.
  if (process.platform === "win32") return true;
  const currentUid = process.getuid?.();
  return (
    currentUid === undefined ||
    (BigInt(info.uid) === BigInt(currentUid) &&
      (BigInt(info.mode) & 0o077n) === 0n)
  );
}

async function sentinelIsIdle(sentinel: LiveSentinel): Promise<boolean> {
  try {
    const pathBefore = await lstat(sentinel.path, { bigint: true });
    const exact = await sentinel.handle.stat({ bigint: true });
    const pathAfter = await lstat(sentinel.path, { bigint: true });
    return (
      sentinelIdentityMetadataAccepted(pathBefore) &&
      sentinelIdentityMetadataAccepted(exact) &&
      sentinelIdentityMetadataAccepted(pathAfter) &&
      BigInt(exact.size) === 0n &&
      metadataFingerprint(pathBefore) === metadataFingerprint(exact) &&
      metadataFingerprint(pathAfter) === metadataFingerprint(exact)
    );
  } catch {
    return false;
  }
}

async function sentinelIdentityStillBound(sentinel: LiveSentinel): Promise<boolean> {
  try {
    const pathBefore = await lstat(sentinel.path, { bigint: true });
    const exact = await sentinel.handle.stat({ bigint: true });
    const pathAfter = await lstat(sentinel.path, { bigint: true });
    return (
      sentinelIdentityMetadataAccepted(pathBefore) &&
      sentinelIdentityMetadataAccepted(exact) &&
      sentinelIdentityMetadataAccepted(pathAfter) &&
      metadataFingerprint(pathBefore) === metadataFingerprint(exact) &&
      metadataFingerprint(pathAfter) === metadataFingerprint(exact)
    );
  } catch {
    return false;
  }
}

async function restoreBoundSentinelToIdle(sentinel: LiveSentinel): Promise<boolean> {
  if (!(await sentinelIdentityStillBound(sentinel))) return false;
  await sentinel.handle.truncate(0);
  await sentinel.handle.sync();
  return sentinelIsIdle(sentinel);
}

async function sentinelCarriesToken(
  sentinel: LiveSentinel,
  token: Buffer
): Promise<boolean> {
  try {
    const pathBefore = await lstat(sentinel.path, { bigint: true });
    const exactBefore = await sentinel.handle.stat({ bigint: true });
    if (
      !sentinelIdentityMetadataAccepted(pathBefore) ||
      !sentinelIdentityMetadataAccepted(exactBefore) ||
      metadataFingerprint(pathBefore) !== metadataFingerprint(exactBefore) ||
      BigInt(exactBefore.size) !== BigInt(token.length)
    ) {
      return false;
    }
    const observed = Buffer.alloc(token.length);
    const { bytesRead } = await sentinel.handle.read(
      observed,
      0,
      observed.length,
      0
    );
    const exactAfter = await sentinel.handle.stat({ bigint: true });
    const pathAfter = await lstat(sentinel.path, { bigint: true });
    return (
      bytesRead === token.length &&
      observed.equals(token) &&
      sentinelIdentityMetadataAccepted(exactAfter) &&
      sentinelIdentityMetadataAccepted(pathAfter) &&
      metadataFingerprint(exactBefore) === metadataFingerprint(exactAfter) &&
      metadataFingerprint(pathAfter) === metadataFingerprint(exactAfter)
    );
  } catch {
    return false;
  }
}

async function sentinelNamespace(
  canonicalRoot: string
): Promise<{ directory: string; entryCount: number }> {
  const namespace = join(canonicalRoot, SENTINEL_NAMESPACE);
  try {
    await mkdir(namespace, { mode: 0o700 });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
  }
  const info = await lstat(namespace, { bigint: true });
  const canonical = await realpath(namespace);
  if (
    !info.isDirectory() ||
    info.isSymbolicLink() ||
    canonical !== namespace ||
    relative(canonicalRoot, canonical) !== SENTINEL_NAMESPACE
  ) {
    throw new CorpusLeaseChangedError("meeting corpus sentinel namespace changed");
  }
  if (process.platform !== "win32") {
    const currentUid = process.getuid?.();
    if (
      (currentUid !== undefined && BigInt(info.uid) !== BigInt(currentUid)) ||
      (BigInt(info.mode) & 0o077n) !== 0n
    ) {
      throw new CorpusLeaseChangedError("meeting corpus sentinel namespace is not private");
    }
  }
  const handle = await opendir(namespace);
  let entries = 0;
  try {
    for (;;) {
      const entry = await handle.read();
      if (!entry) break;
      entries += 1;
      if (
        entries > MAX_SENTINEL_NAMESPACE_ENTRIES ||
        !entry.isFile() ||
        !SENTINEL_BASENAME.test(entry.name)
      ) {
        throw new CorpusLeaseBudgetError(
          "meeting corpus sentinel namespace exceeded its retained budget"
        );
      }
    }
  } finally {
    await handle.close().catch(() => {});
  }
  return { directory: namespace, entryCount: entries };
}

function reserveSentinelCreation(
  directory: string
): SentinelCapacityReservation {
  // This function intentionally contains no await. Every async acquisition
  // owns its global and per-root capacity before it can yield to a peer.
  if (
    retainedSentinels.size + reservedSentinelCreations >=
    MAX_RETAINED_SENTINELS
  ) {
    throw new CorpusLeaseBudgetError(
      "meeting corpus retained sentinels exceeded their process budget"
    );
  }
  reservedSentinelCreations += 1;
  reservedSentinelCreationsByDirectory.set(
    directory,
    (reservedSentinelCreationsByDirectory.get(directory) ?? 0) + 1
  );
  return { directory, released: false };
}

function releaseSentinelCreation(
  reservation: SentinelCapacityReservation
): void {
  if (reservation.released) return;
  reservation.released = true;
  reservedSentinelCreations -= 1;
  const rootReserved =
    (reservedSentinelCreationsByDirectory.get(reservation.directory) ?? 0) - 1;
  if (rootReserved === 0) {
    reservedSentinelCreationsByDirectory.delete(reservation.directory);
  } else {
    reservedSentinelCreationsByDirectory.set(
      reservation.directory,
      rootReserved
    );
  }
}

function sentinelCapacitySnapshot(directory: string) {
  return Object.freeze({
    globalReserved: reservedSentinelCreations,
    globalRetained: retainedSentinels.size,
    rootReserved: reservedSentinelCreationsByDirectory.get(directory) ?? 0,
  });
}

async function evictIdleSentinelForCapacity(): Promise<boolean> {
  let oldest: LiveSentinel | undefined;
  for (const sentinel of retainedSentinels.values()) {
    if (
      !sentinel.inUse &&
      (!oldest || sentinel.lastUsed < oldest.lastUsed)
    ) {
      oldest = sentinel;
    }
  }
  if (!oldest) return false;
  oldest.inUse = true;
  try {
    // Eviction only releases this process's descriptor accounting; it never
    // mutates the ambient pathname. A displaced or already-removed leaf is
    // safe to close and forget; authorization still fails if it is reopened.
    await oldest.handle.close();
    retainedSentinels.delete(oldest.path);
    return true;
  } catch (error) {
    oldest.inUse = false;
    oldest.lastUsed = ++retainedSentinelUseSequence;
    throw error;
  }
}

async function acquireSentinel(
  canonicalRoot: string,
  slot: number,
  openControl?: SentinelOpenControl,
  afterReserved?: (
    capacity: Readonly<{
      globalReserved: number;
      globalRetained: number;
      rootReserved: number;
    }>
  ) => void | Promise<void>
): Promise<LiveSentinel> {
  const directory = join(canonicalRoot, SENTINEL_NAMESPACE);
  const name = `lease-shared-${slot}.fence`;
  if (!SENTINEL_BASENAME.test(name)) {
    throw new CorpusLeaseBudgetError("meeting corpus sentinel slot was invalid");
  }
  const path = join(directory, name);
  const retained = retainedSentinels.get(path);
  if (retained) {
    if (retained.inUse) {
      throw new CorpusLeaseBudgetError("meeting corpus sentinel slot is already active");
    }
    retained.inUse = true;
    if (await sentinelIsIdle(retained)) return retained;
    if (await restoreBoundSentinelToIdle(retained)) return retained;
    try {
      await retained.handle.close();
      retainedSentinels.delete(retained.path);
    } catch (error) {
      retained.inUse = true;
      throw error;
    }
    throw new CorpusLeaseChangedError("meeting corpus sentinel identity changed");
  }
  while (
    retainedSentinels.size + reservedSentinelCreations >=
    MAX_RETAINED_SENTINELS
  ) {
    if (!(await evictIdleSentinelForCapacity())) {
      throw new CorpusLeaseBudgetError(
        "meeting corpus retained sentinels exceeded their process budget"
      );
    }
  }
  const reservation = reserveSentinelCreation(directory);
  try {
    await afterReserved?.(sentinelCapacitySnapshot(directory));
    await sentinelNamespace(canonicalRoot);
    openControl?.onReserved?.();
    await openControl?.pauseUntil;
    if (openControl?.fail) {
      throw new Error("injected meeting corpus sentinel open failure");
    }
    const handle = await open(
      path,
      constants.O_RDWR |
        constants.O_CREAT |
        (constants.O_NOFOLLOW ?? 0),
      0o600
    );
    const sentinel = {
      directory,
      handle,
      inUse: true,
      lastUsed: 0,
      name,
      path,
    };
    // Convert the reserved descriptor slot to retained capacity synchronously
    // before the next await. The namespace itself has only two fixed names.
    releaseSentinelCreation(reservation);
    retainedSentinels.set(path, sentinel);
    if (
      !(await sentinelIsIdle(sentinel)) &&
      !(await restoreBoundSentinelToIdle(sentinel))
    ) {
      try {
        await handle.close();
        retainedSentinels.delete(path);
      } catch {
        sentinel.inUse = true;
      }
      throw new CorpusLeaseChangedError(
        "meeting corpus sentinel identity changed"
      );
    }
    return sentinel;
  } catch (error) {
    releaseSentinelCreation(reservation);
    throw error;
  }
}

async function releaseSentinel(sentinel: LiveSentinel): Promise<void> {
  try {
    await sentinel.handle.sync();
    let identityStillBound = await sentinelIsIdle(sentinel);
    if (!identityStillBound) {
      identityStillBound = await restoreBoundSentinelToIdle(sentinel);
    }
    if (!identityStillBound) {
      throw new CorpusLeaseChangedError("meeting corpus sentinel changed during cleanup");
    }
    sentinel.inUse = false;
    sentinel.lastUsed = ++retainedSentinelUseSequence;
  } catch (error) {
    try {
      await sentinel.handle.close();
      retainedSentinels.delete(sentinel.path);
    } catch {
      // Failed-close handles remain charged and unavailable.
      sentinel.inUse = true;
    }
    throw error;
  }
}

async function resolveRootIdentity(root: string): Promise<RootIdentity> {
  const canonicalRoot = await realpath(root);
  const info = await stat(canonicalRoot, { bigint: true });
  if (!info.isDirectory()) {
    throw new Error("Access denied: meeting corpus root is not a directory");
  }
  return {
    canonicalRoot,
    fingerprint: `${canonicalRoot}\0${metadataFingerprint(info)}`,
  };
}

type TraversalResources = {
  directoryCount: number;
  entryCount: number;
  pathBytes: number;
};

function chargePathBytes(
  resources: TraversalResources,
  budgets: Readonly<CorpusReadBudgets>,
  ...values: string[]
): void {
  for (const value of values) {
    resources.pathBytes += Buffer.byteLength(value, "utf8");
    if (resources.pathBytes > budgets.maxRetainedPathBytes) {
      throw new CorpusLeaseBudgetError(
        "meeting corpus path metadata exceeded its budget"
      );
    }
  }
}

async function boundedDirectoryEntries(
  directory: string,
  resources: TraversalResources,
  budgets: Readonly<CorpusReadBudgets>,
  deadline: bigint
): Promise<Dirent[]> {
  remainingAuthorizationMs(deadline);
  const handle = await opendir(directory);
  const entries: Dirent[] = [];
  try {
    for (;;) {
      remainingAuthorizationMs(deadline);
      const entry = await handle.read();
      if (!entry) break;
      resources.entryCount += 1;
      if (resources.entryCount > budgets.maxDirectoryEntries) {
        throw new CorpusLeaseBudgetError("meeting corpus directory entries exceeded their budget");
      }
      chargePathBytes(resources, budgets, entry.name);
      entries.push(entry);
    }
  } finally {
    await handle.close().catch(() => {});
  }
  entries.sort((left, right) => left.name.localeCompare(right.name));
  return entries;
}

function chargeDirectory(
  resources: TraversalResources,
  budgets: Readonly<CorpusReadBudgets>
): void {
  resources.directoryCount += 1;
  if (resources.directoryCount > budgets.maxDirectoryCount) {
    throw new CorpusLeaseBudgetError("meeting corpus directory count exceeded its budget");
  }
}

async function collectManifest(
  canonicalRoot: string,
  budgets: Readonly<CorpusReadBudgets>,
  retainContent: boolean,
  deadline: bigint
): Promise<Manifest> {
  remainingAuthorizationMs(deadline);
  const files: StableCorpusFile[] | undefined = retainContent ? [] : undefined;
  const manifestHash = createHash("sha256");
  let fileCount = 0;
  let totalBytes = 0;
  const resources: TraversalResources = {
    directoryCount: 0,
    entryCount: 0,
    pathBytes: 0,
  };
  chargePathBytes(resources, budgets, canonicalRoot);

  const visit = async (directory: string): Promise<void> => {
    remainingAuthorizationMs(deadline);
    chargeDirectory(resources, budgets);
    const entries = await boundedDirectoryEntries(directory, resources, budgets, deadline);
    for (const entry of entries) {
      remainingAuthorizationMs(deadline);
      if (entry.name.startsWith(".")) continue;
      const lexicalPath = join(directory, entry.name);
      // Parent entry arrays remain live across recursive descent. Charge each
      // constructed full path, including non-Markdown and directory entries,
      // instead of accounting only for retained meeting files.
      chargePathBytes(resources, budgets, lexicalPath);
      if (entry.isDirectory()) {
        if (!INACTIVE_CORPUS_DIRS.has(entry.name.toLowerCase())) {
          await visit(lexicalPath);
        }
        continue;
      }
      if (!entry.isFile() || extname(entry.name).toLowerCase() !== ".md") {
        continue;
      }

      fileCount += 1;
      if (fileCount > budgets.maxFileCount) {
        throw new CorpusLeaseBudgetError("meeting corpus file count exceeded its budget");
      }

      const canonicalPath = await realpath(lexicalPath);
      const scoped = relative(canonicalRoot, canonicalPath);
      if (!activeRelativePath(scoped)) {
        throw new CorpusLeaseChangedError("meeting corpus membership escaped its root");
      }
      const before = await lstat(canonicalPath, { bigint: true });
      if (
        !before.isFile() ||
        before.isSymbolicLink() ||
        BigInt(before.nlink) !== 1n
      ) {
        throw new CorpusLeaseChangedError("meeting corpus member was not a regular file");
      }
      const remainingCorpusBytes = budgets.maxCorpusBytes - totalBytes;
      const maxBytes = Math.min(budgets.maxFileBytes, remainingCorpusBytes);
      let content: Buffer | undefined;
      let revision: BoundFileRevision;
      if (retainContent) {
        const read = await readTextFileWithRevisionFromBoundParent(canonicalPath, {
          maxBytes,
          maxReaders: budgets.maxReaderCount,
          timeoutMs: remainingAuthorizationMs(deadline),
        });
        content = read.content;
        revision = read.revision;
      } else {
        revision = await fingerprintTextFileFromBoundParent(canonicalPath, {
          maxBytes,
          maxReaders: budgets.maxReaderCount,
          timeoutMs: remainingAuthorizationMs(deadline),
        });
      }
      const after = await lstat(canonicalPath, { bigint: true });
      const beforeFingerprint = metadataFingerprint(before);
      const afterFingerprint = metadataFingerprint(after);
      if (
        !after.isFile() ||
        BigInt(after.nlink) !== 1n ||
        beforeFingerprint !== afterFingerprint ||
        beforeFingerprint !== revision.leafFingerprint
      ) {
        throw new CorpusLeaseChangedError("meeting corpus member changed during manifest read");
      }

      totalBytes += revision.byteLength;
      if (totalBytes > budgets.maxCorpusBytes) {
        throw new CorpusLeaseBudgetError("meeting corpus bytes exceeded their budget");
      }

      const relativePath = normalizedRelativePath(scoped);
      chargePathBytes(resources, budgets, canonicalPath, relativePath);
      if (files && content) {
        files.push({
          path: canonicalPath,
          relativePath,
          content: decodePolicyUtf8(content),
        });
        content = undefined;
      }
      manifestHash.update(
        `${JSON.stringify(relativePath)}:${revision.leafFingerprint}:${revision.byteLength}:${revision.sha256}\n`
      );
    }
  };

  await visit(canonicalRoot);
  files?.sort((left, right) => left.relativePath.localeCompare(right.relativePath));
  const verification = Object.freeze({
    fileCount,
    retainedContentBytes: retainContent ? totalBytes : 0,
    totalBytes,
  });
  return {
    fingerprint: manifestHash.digest("hex"),
    verification,
    ...(files
      ? {
          snapshot: Object.freeze({
            canonicalRoot,
            files: Object.freeze(files.map((file) => Object.freeze(file))),
          }),
        }
      : {}),
  };
}

class WatchedCorpusAttempt {
  private watcher: FSWatcher | undefined;
  private readonly pendingFences = new Map<string, PendingFence>();
  private readonly sentinels: LiveSentinel[] = [];
  private nextFenceSentinel = 0;
  private watcherReserved = false;
  private failure: Error | null = null;
  private suppressNext = false;
  private failNextPulse = false;
  private nextSentinelOpen: SentinelOpenControl | undefined;
  private nextFencePending: FencePendingControl | undefined;
  generation = 0;

  readonly controls: CorpusLeaseControls = Object.freeze({
    failWatcher: (reason = "injected watcher failure") => {
      this.fail(new Error(`Access denied: ${reason}`));
    },
    suppressNextFence: () => {
      this.suppressNext = true;
    },
    // Kept as a compatibility-only diagnostic hook. A fence now has exactly
    // one pulse and one acknowledgement; there are no outstanding repulses.
    requireRepulseForNextFence: () => {},
    failNextFencePulse: () => {
      this.failNextPulse = true;
    },
    failNextSentinelOpen: () => {
      this.nextSentinelOpen = {
        ...this.nextSentinelOpen,
        fail: true,
      };
    },
    pauseNextSentinelOpen: (pauseUntil, onReserved) => {
      this.nextSentinelOpen = {
        fail: this.nextSentinelOpen?.fail ?? false,
        onReserved,
        pauseUntil,
      };
    },
    pauseNextFenceAfterPending: (pauseUntil, onPending) => {
      this.nextFencePending = { onPending, pauseUntil };
    },
  });

  private constructor(
    private readonly canonicalRoot: string,
    private readonly deadline: bigint,
    private readonly fenceTimeoutMs: number
  ) {}

  static async create(
    canonicalRoot: string,
    deadline: bigint,
    fenceTimeoutMs: number,
    budgets: Readonly<CorpusReadBudgets>,
    attempt: number,
    beforeSentinelCreate: CorpusLeaseHooks["beforeSentinelCreate"]
  ): Promise<WatchedCorpusAttempt> {
    const lease = new WatchedCorpusAttempt(canonicalRoot, deadline, fenceTimeoutMs);
    try {
      // Open both fixed shared slots before watcher registration. Each fence
      // carries a fresh random token, so a peer or delayed callback cannot
      // acknowledge a different operation.
      const processLimit = Math.min(MAX_ACTIVE_WATCHERS, budgets.maxWatcherCount);
      if (activeWatcherCount >= processLimit) {
        throw new CorpusLeaseBudgetError(
          "meeting corpus watcher attempts exceeded their process budget"
        );
      }
      activeWatcherCount += 1;
      lease.watcherReserved = true;
      // Exactly two authorization fences are used per attempt. Give each one
      // a distinct sentinel created before watcher registration, preventing a
      // delayed callback from fence N from acknowledging fence N+1.
      lease.sentinels.push(
        await acquireSentinel(canonicalRoot, 0, undefined, (capacity) =>
          beforeSentinelCreate?.(
            Object.freeze({ attempt, slot: 0, capacity })
          )
        ),
        await acquireSentinel(canonicalRoot, 1, undefined, (capacity) =>
          beforeSentinelCreate?.(
            Object.freeze({ attempt, slot: 1, capacity })
          )
        )
      );
      // Node 20+ supports recursive fs.watch on the supported desktop
      // platforms. If a runtime/backend cannot provide it, construction throws
      // and authorization fails closed instead of composing unordered handles.
      lease.watcher = watch(
        canonicalRoot,
        { encoding: "utf8", persistent: false, recursive: true },
        (_eventType, filename) => lease.onEvent(filename)
      );
      lease.watcher.on("error", () => {
        lease.fail(new Error("Access denied: meeting corpus watcher failed"));
      });
      lease.assertHealthy();
      return lease;
    } catch (error) {
      await lease.close().catch(() => {});
      throw error;
    }
  }

  assertHealthy(): void {
    if (this.failure) throw this.failure;
  }

  async fence(): Promise<void> {
    this.assertHealthy();
    await this.fenceSentinel();
    this.assertHealthy();
  }

  async close(): Promise<void> {
    this.watcher?.close();
    this.watcher = undefined;
    if (this.watcherReserved) {
      activeWatcherCount -= 1;
      this.watcherReserved = false;
    }
    for (const pending of this.pendingFences.values()) {
      pending.reject(new Error("Access denied: meeting corpus lease closed"));
    }
    this.pendingFences.clear();
    let cleanupFailed = false;
    for (const sentinel of this.sentinels.splice(0)) {
      try {
        await releaseSentinel(sentinel);
      } catch {
        cleanupFailed = true;
      }
    }
    if (cleanupFailed) {
      throw new Error("Access denied: meeting corpus sentinel cleanup failed");
    }
  }

  private onEvent(filename: string | Buffer | null): void {
    if (filename === null) {
      this.fail(new Error("Access denied: meeting corpus watcher omitted a filename"));
      return;
    }
    const normalized = normalizedRelativePath(filename.toString());
    const name = basename(normalized);
    const pending = this.pendingFences.get(normalized);
    if (pending) {
      if (!pending.suppressEntireFence) {
        void sentinelCarriesToken(pending.sentinel, pending.token).then(
          (matches) => {
            if (matches && this.pendingFences.get(normalized) === pending) {
              pending.resolve();
            }
          },
          () => {}
        );
      }
      return;
    }
    // Shared-slot peer events are internal noise. Token verification above is
    // what binds an acknowledgement to this exact operation.
    if (
      normalized.startsWith(`${SENTINEL_NAMESPACE}/`) &&
      SENTINEL_BASENAME.test(name)
    ) return;
    this.generation += 1;
  }

  private async fenceSentinel(): Promise<void> {
    this.assertHealthy();
    const suppressEntireFence = this.suppressNext;
    this.suppressNext = false;
    let finished = false;
    const sentinel = this.sentinels[this.nextFenceSentinel++];
    if (!sentinel) {
      throw new CorpusLeaseChangedError("meeting corpus sentinel was unavailable");
    }
    const openControl = this.nextSentinelOpen;
    this.nextSentinelOpen = undefined;
    openControl?.onReserved?.();
    await openControl?.pauseUntil;
    if (openControl?.fail) throw new Error("injected meeting corpus sentinel open failure");
    const { handle, name } = sentinel;
    if (!(await sentinelIsIdle(sentinel))) {
      throw new CorpusLeaseChangedError("meeting corpus sentinel was displaced");
    }
    const directory = join(this.canonicalRoot, SENTINEL_NAMESPACE);
    const directoryBefore = await lstat(directory, { bigint: true });
    if (!directoryBefore.isDirectory() || directoryBefore.isSymbolicLink()) {
      throw new CorpusLeaseChangedError("meeting corpus fence directory changed");
    }
    let resolveFence!: () => void;
    let rejectFence!: (error: Error) => void;
    const observed = new Promise<void>((resolve, reject) => {
      resolveFence = () => {
        if (finished) return;
        finished = true;
        resolve();
      };
      rejectFence = (error) => {
        if (finished) return;
        finished = true;
        reject(error);
      };
    });
    const pending: PendingFence = {
      resolve: resolveFence,
      reject: rejectFence,
      sentinel,
      suppressEntireFence,
      token: randomBytes(SENTINEL_TOKEN_BYTES),
    };
    const pendingKey = `${SENTINEL_NAMESPACE}/${name}`;
    this.pendingFences.set(pendingKey, pending);
    const timeout = setTimeout(() => {
      rejectFence(new Error("Access denied: meeting corpus sentinel fence timed out"));
    }, Math.min(this.fenceTimeoutMs, remainingAuthorizationMs(this.deadline)));
    timeout.unref();

    try {
      const pendingControl = this.nextFencePending;
      this.nextFencePending = undefined;
      pendingControl?.onPending?.();
      await pendingControl?.pauseUntil;
      if (this.failNextPulse) {
        this.failNextPulse = false;
        throw new Error("Access denied: meeting corpus sentinel pulse failed");
      }
      if (!(await sentinelIsIdle(sentinel))) {
        throw new CorpusLeaseChangedError(
          "meeting corpus sentinel changed before acknowledgement"
        );
      }
      await handle.truncate(0);
      await handle.write(pending.token, 0, pending.token.length, 0);
      await handle.sync();
      await observed;
      if (!(await sentinelCarriesToken(sentinel, pending.token))) {
        throw new CorpusLeaseChangedError(
          "meeting corpus sentinel token changed during acknowledgement"
        );
      }
      await handle.truncate(0);
      await handle.sync();
      const directoryAfter = await lstat(directory, { bigint: true });
      if (
        !directoryAfter.isDirectory() ||
        directoryAfter.isSymbolicLink() ||
        metadataFingerprint(directoryAfter) !== metadataFingerprint(directoryBefore) ||
        !(await sentinelIsIdle(sentinel))
      ) {
        throw new CorpusLeaseChangedError(
          "meeting corpus sentinel changed during acknowledgement"
        );
      }
      this.assertHealthy();
    } finally {
      finished = true;
      clearTimeout(timeout);
      if (this.pendingFences.get(pendingKey) === pending) {
        this.pendingFences.delete(pendingKey);
      }
    }
  }

  private fail(error: Error): void {
    if (this.failure) return;
    this.failure = error;
    for (const pending of this.pendingFences.values()) {
      pending.reject(error);
    }
  }
}

/**
 * Run a multi-source read against one bounded watcher-fenced corpus snapshot.
 * A supported watcher must observe each sentinel fence; root identity and the
 * complete in-budget manifest must also agree before return. No claim is made
 * that uncontrolled writers cannot mutate in the JS check-to-return gap.
 */
export async function withStableCorpusLease<T>(
  root: string,
  operation: (
    snapshot: StableCorpusSnapshot,
    attempt: number,
    signal: AbortSignal
  ) => T | Promise<T>,
  hooks: CorpusLeaseHooks = {}
): Promise<T> {
  const budgets = resolveCorpusReadBudgets(hooks.budgets);
  const deadline = authorizationDeadline(hooks.timeoutMs);
  const fenceTimeoutMs = resolveFenceTimeout(hooks.timeoutMs);
  const memoryReservation = reserveCorpusMemory(budgets);

  try {
    for (let attempt = 1; attempt <= MAX_AUTHORIZATION_ATTEMPTS; attempt += 1) {
      let lease: WatchedCorpusAttempt | undefined;
      try {
      remainingAuthorizationMs(deadline);
      const initialRoot = await resolveRootIdentity(root);
      lease = await WatchedCorpusAttempt.create(
        initialRoot.canonicalRoot,
        deadline,
        fenceTimeoutMs,
        budgets,
        attempt,
        hooks.beforeSentinelCreate
      );
      const diagnosticContext = Object.freeze({
        attempt,
        controls: lease.controls,
      });
      await hooks.onWatcherReady?.(diagnosticContext);
      remainingAuthorizationMs(deadline);
      await lease.fence();
      // The coverage probe creates hidden sentinels, which changes directory
      // metadata. Establish the root baseline only after that probe.
      const authorizedRoot = await resolveRootIdentity(root);
      if (authorizedRoot.canonicalRoot !== initialRoot.canonicalRoot) {
        throw new CorpusLeaseChangedError("meeting corpus root changed during initial fence");
      }
      const baselineGeneration = lease.generation;
      const baseline = await collectManifest(
        authorizedRoot.canonicalRoot,
        budgets,
        true,
        deadline
      );
      if (!baseline.snapshot) {
        throw new CorpusLeaseChangedError("meeting corpus snapshot was unavailable");
      }
      if (lease.generation !== baselineGeneration) {
        throw new CorpusLeaseChangedError("meeting corpus changed during baseline");
      }
      await hooks.afterBaseline?.(diagnosticContext);
      remainingAuthorizationMs(deadline);

      const operationAbort = new AbortController();
      const operationTimeout = setTimeout(() => {
        operationAbort.abort(
          new CorpusLeaseChangedError("meeting corpus operation deadline elapsed")
        );
      }, remainingAuthorizationMs(deadline));
      operationTimeout.unref();
      let result: T;
      try {
        result = await Promise.race([
          Promise.resolve(
            operation(baseline.snapshot, attempt, operationAbort.signal)
          ),
          new Promise<never>((_resolve, reject) => {
            operationAbort.signal.addEventListener(
              "abort",
              () => reject(operationAbort.signal.reason),
              { once: true }
            );
          }),
        ]);
      } finally {
        clearTimeout(operationTimeout);
      }
      remainingAuthorizationMs(deadline);
      await hooks.beforeFinalManifest?.(diagnosticContext);
      const finalManifest = await collectManifest(
        authorizedRoot.canonicalRoot,
        budgets,
        false,
        deadline
      );
      await hooks.afterFinalManifest?.(Object.freeze({
        attempt,
        controls: lease.controls,
        verification: finalManifest.verification,
      }));
      const finalRoot = await resolveRootIdentity(root);
      if (
        lease.generation !== baselineGeneration ||
        finalManifest.fingerprint !== baseline.fingerprint ||
        finalRoot.fingerprint !== authorizedRoot.fingerprint
      ) {
        throw new CorpusLeaseChangedError("meeting corpus changed before final fence");
      }
      await hooks.beforeFinalFence?.(diagnosticContext);

      // This is deliberately the last awaited authorization action. Generation
      // is checked synchronously after the sentinel event before returning.
      await lease.fence();
      if (lease.generation !== baselineGeneration) {
        throw new CorpusLeaseChangedError("meeting corpus changed at final fence");
      }
      // The sentinel acknowledgement is never an authorization capability.
      // In particular, a Windows principal that can enumerate this corpus may
      // inject a sentinel event, but that only advances execution to this full
      // reread. Authorization still requires the exact root, single-link file
      // identities, bytes, and hashes to match the baseline after the event.
      // A genuine fence additionally orders recursive-root callbacks before
      // this snapshot. Thus an outside hard-link alias, restricted overwrite,
      // or root swap cannot inherit the stale result merely by forging an ack.
      const authorizedManifest = await collectManifest(
        authorizedRoot.canonicalRoot,
        budgets,
        false,
        deadline
      );
      const authorizationRoot = await resolveRootIdentity(root);
      if (
        lease.generation !== baselineGeneration ||
        authorizedManifest.fingerprint !== baseline.fingerprint ||
        authorizationRoot.fingerprint !== authorizedRoot.fingerprint
      ) {
        throw new CorpusLeaseChangedError(
          "meeting corpus changed at authorization point"
        );
      }
      await lease.close();
      lease = undefined;
      return result;
      } catch {
        // Retry without retaining path-bearing filesystem errors. The public
        // failure below is deliberately privacy-safe.
      } finally {
        // Body failures are retried behind the one path-free public error. A
        // cleanup failure on the successful path is handled above and therefore
        // also denies; here it must not replace that privacy-safe error.
        await lease?.close().catch(() => {});
      }
    }

    throw new Error("Access denied: stable meeting corpus authorization failed");
  } finally {
    releaseCorpusMemory(memoryReservation);
  }
}
