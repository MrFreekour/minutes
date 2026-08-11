import { describe, expect, it } from "vitest";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { withStableCorpusLease } from "./corpus-lease.js";
import { retireBoundReadersForProcessShutdown } from "./secure-read.js";

// This file deliberately strands a child whose death is never confirmed and
// leaves the process refusing every later lease, so it must stay a file of
// exactly one test.
//
// An earlier version kept this beside the recovery test with a comment saying
// it must run last, plus an opening assertion that the process was still
// clean. That only catches refusal arriving from earlier tests; a test added
// after it would inherit the refusal and could pass for the wrong reason,
// since most assertions here are of the shape "the lease is refused". Vitest
// runs each file in its own process, so a file boundary enforces what a
// comment could only request.
//
// The recovering half of the same contract lives in
// corpus-lease-recovery.test.ts, which cannot share a process with this one.
function withCorpus(run: (root: string) => Promise<void>): Promise<void> {
  const root = mkdtempSync(join(tmpdir(), "minutes-corpus-poisoning-"));
  return run(root).finally(async () => {
    await retireBoundReadersForProcessShutdown();
    rmSync(root, { recursive: true, force: true });
  });
}

describe("stable corpus lease poisoning", () => {
  it("keeps failing closed while a disclosed worker's death stays unconfirmed", async () => {
    await withCorpus(async (root) => {
      writeFileSync(join(root, "meeting.md"), "disclosed canary");
      // The counterpart to the never-fed recovery case in
      // corpus-lease-refusal.test.ts. A generous budget lets `begin` reach the
      // child, so the worker knows which directory to read; stalling it forces
      // the kill.
      //
      // While the corpus root has been disclosed and the child is unreaped,
      // every later lease has to be refused: the child may still be reading.
      // The never-settled hold below is what makes that "while" testable, by
      // standing in for a reap that has not happened yet. Without it the real
      // SIGKILLed child is reaped in milliseconds and the assertion would race
      // the recovery it is supposed to exclude.
      await expect(
        withStableCorpusLease(root, () => "unreachable", {
          timeoutMs: 2_000,
          workerStallPhaseForTest: "before-baseline",
          forceUnconfirmedTerminationForTest: true,
          confirmTerminationForTest: new Promise<void>(() => {}),
        })
      ).rejects.toThrow("stable meeting corpus authorization failed");

      await expect(
        withStableCorpusLease(root, () => "must not run")
      ).rejects.toThrow("killed without confirming it died");
    });
  });
});
