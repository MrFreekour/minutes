import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { normalizeNpmPackPermissions } from "./normalize_npm_pack_permissions.mjs";

test("normalizes npm payload files while preserving bin executability", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "minutes-pack-permissions-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, "dist"), { recursive: true });
  await writeFile(
    path.join(root, "package.json"),
    `${JSON.stringify({ files: ["dist/"], main: "dist/index.js", bin: { demo: "dist/cli.js" } })}\n`,
  );
  await writeFile(path.join(root, "README.md"), "read me\n");
  await writeFile(path.join(root, "dist", "index.js"), "export {};\n");
  await writeFile(path.join(root, "dist", "cli.js"), "#!/usr/bin/env node\n");
  await chmod(path.join(root, "package.json"), 0o600);
  await chmod(path.join(root, "README.md"), 0o600);
  await chmod(path.join(root, "dist", "index.js"), 0o600);
  await chmod(path.join(root, "dist", "cli.js"), 0o600);

  await normalizeNpmPackPermissions(root);

  assert.equal((await stat(path.join(root, "package.json"))).mode & 0o777, 0o644);
  assert.equal((await stat(path.join(root, "README.md"))).mode & 0o777, 0o644);
  assert.equal((await stat(path.join(root, "dist", "index.js"))).mode & 0o777, 0o644);
  assert.equal((await stat(path.join(root, "dist", "cli.js"))).mode & 0o777, 0o755);
  assert.match(await readFile(path.join(root, "dist", "cli.js"), "utf8"), /^#!/);
});

test("rejects package entries that escape the package directory", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "minutes-pack-permissions-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "package.json"), `${JSON.stringify({ files: ["../secret"] })}\n`);
  await assert.rejects(normalizeNpmPackPermissions(root), /escapes package directory/);
});
