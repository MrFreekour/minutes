import { existsSync, realpathSync } from "fs";
import { homedir } from "os";
import { extname, isAbsolute, join, resolve, sep } from "path";
import {
  decodePolicyUtf8,
  readTextFileFromBoundParent,
} from "./secure-read.js";

export function expandHomeLikePath(input: string): string {
  const home = homedir();

  if (input === "~") {
    return home;
  }

  if (input.startsWith("~/") || input.startsWith("~\\")) {
    return join(home, input.slice(2));
  }

  if (input === "$HOME") {
    return home;
  }

  if (input.startsWith("$HOME/") || input.startsWith("$HOME\\")) {
    return join(home, input.slice("$HOME".length + 1));
  }

  if (input === "${HOME}") {
    return home;
  }

  if (input.startsWith("${HOME}/") || input.startsWith("${HOME}\\")) {
    return join(home, input.slice("${HOME}".length + 1));
  }

  // Minutes core resolves a relative MINUTES_HOME against the user's home,
  // never the caller's process cwd. Keep MCP correction and audit stores on
  // that same deterministic root so different hosts cannot split state.
  return isAbsolute(input) ? input : resolve(home, input);
}

export function canonicalizeFilePath(path: string): string {
  if (!existsSync(path)) {
    throw new Error(`Path does not exist: ${path}`);
  }
  return realpathSync(path);
}

export function canonicalizeRoot(root: string): string {
  const expandedRoot = expandHomeLikePath(root);

  // Roots may not exist yet (e.g. ~/.minutes/inbox on first run).
  // Use realpath if it exists, otherwise lexical resolve.
  return existsSync(expandedRoot) ? realpathSync(expandedRoot) : resolve(expandedRoot);
}

export function isWithinDirectory(candidate: string, root: string): boolean {
  // Ensure root ends with separator to prevent prefix attacks (e.g. ~/meetings-evil)
  // Use path.sep for cross-platform correctness — realpathSync returns OS-native
  // separators, so on Windows the root will use backslashes.
  const rootWithSep = root.endsWith(sep) ? root : root + sep;
  return candidate === root || candidate.startsWith(rootWithSep);
}

export function validatePathInDirectory(path: string, root: string, allowedExts: string[]): string {
  const canonicalPath = canonicalizeFilePath(path);
  const canonicalRoot = canonicalizeRoot(root);

  if (!allowedExts.includes(extname(canonicalPath).toLowerCase())) {
    throw new Error(
      `Access denied: path must be within ${canonicalRoot} and end with ${allowedExts.join(", ")}`
    );
  }

  if (!isWithinDirectory(canonicalPath, canonicalRoot)) {
    throw new Error(`Access denied: path must be within ${canonicalRoot}`);
  }

  return canonicalPath;
}

export function validatePathInDirectories(
  path: string,
  roots: string[],
  allowedExts: string[]
): string {
  const canonicalPath = canonicalizeFilePath(path);

  if (!allowedExts.includes(extname(canonicalPath).toLowerCase())) {
    throw new Error(
      `Access denied: path must end with one of ${allowedExts.join(", ")}`
    );
  }

  const canonicalRoots = roots.map((root) => canonicalizeRoot(root));
  if (!canonicalRoots.some((root) => isWithinDirectory(canonicalPath, root))) {
    throw new Error(
      `Access denied: file must be inside one of ${canonicalRoots.join(", ")}`
    );
  }

  return canonicalPath;
}

export type VerifiedTextFile = {
  path: string;
  content: string;
};

/**
 * Open and read a text file through an OS-bound parent directory.
 *
 * The path is canonicalized and root/extension checked first. A dedicated
 * helper process pins the canonical parent as its cwd before opening only the
 * basename. Intermediate directory swaps therefore cannot redirect the open;
 * final symlinks, identity changes, byte changes, and displaced parents fail
 * closed.
 */
export async function readTextFileInDirectory(
  path: string,
  root: string,
  allowedExts: string[],
  afterValidation: (canonicalPath: string) => void | Promise<void> = () => {},
  afterFirstRead: (canonicalPath: string) => void | Promise<void> = () => {}
): Promise<VerifiedTextFile> {
  const canonicalPath = validatePathInDirectory(path, root, allowedExts);
  await afterValidation(canonicalPath);
  const bytes = await readTextFileFromBoundParent(canonicalPath, {
    afterFirstRead: () => afterFirstRead(canonicalPath),
  });
  return { path: canonicalPath, content: decodePolicyUtf8(bytes) };
}
