import { describe, expect, it } from "vitest";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { withStableCorpusLease } from "./corpus-lease.js";
import { retireBoundReadersForProcessShutdown } from "./secure-read.js";

// This file deliberately poisons the worker path and leaves the process
// unusable, so it must stay a file of exactly one test.
//
// An earlier version kept this beside the recovery test with a comment saying
// it must run last, plus an opening assertion that the process was still
// clean. That only catches poison arriving from earlier tests; a test added
// after it would inherit the poison and could pass for the wrong reason, since
// most assertions here are of the shape "the lease is refused". Vitest runs
// each file in its own process, so a file boundary enforces what a comment
// could only request.
function withCorpus(run: (root: string) => Promise<void>): Promise<void> {
  const root = mkdtempSync(join(tmpdir(), "minutes-corpus-poisoning-"));
  return run(root).finally(async () => {
    await retireBoundReadersForProcessShutdown();
    rmSync(root, { recursive: true, force: true });
  });
}

describe("stable corpus lease poisoning", () => {
  it("still fails closed after an unconfirmed kill of a worker that knew the corpus", async () => {
    await withCorpus(async (root) => {
      writeFileSync(join(root, "meeting.md"), "disclosed canary");
      // The counterpart to the never-fed recovery case in
      // corpus-lease-refusal.test.ts, and the regression that fix must not
      // introduce. A generous budget lets `begin` reach the child, so the
      // worker knows which directory to read; stalling it forces the kill.
      //
      // Once the corpus root has been disclosed, an unconfirmed kill has to
      // keep failing closed: the child may still be reading, and refusing
      // every later lease is the intended answer to that.
      await expect(
        withStableCorpusLease(root, () => "unreachable", {
          timeoutMs: 2_000,
          workerStallPhaseForTest: "before-baseline",
          forceUnconfirmedTerminationForTest: true,
        })
      ).rejects.toThrow("stable meeting corpus authorization failed");

      await expect(
        withStableCorpusLease(root, () => "must not run")
      ).rejects.toThrow("requires a process restart");
    });
  });
});
