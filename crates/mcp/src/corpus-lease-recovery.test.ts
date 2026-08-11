import { describe, expect, it } from "vitest";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { withStableCorpusLease } from "./corpus-lease.js";
import { retireBoundReadersForProcessShutdown } from "./secure-read.js";

// The recovering half of the contract whose failing-closed half lives in
// corpus-lease-poisoning.test.ts. That file strands a child forever on
// purpose, so these cases cannot share a process with it.
function withCorpus(run: (root: string) => Promise<void>): Promise<void> {
  const root = mkdtempSync(join(tmpdir(), "minutes-corpus-recovery-"));
  return run(root).finally(async () => {
    await retireBoundReadersForProcessShutdown();
    rmSync(root, { recursive: true, force: true });
  });
}

const REFUSAL = "killed without confirming it died";

/**
 * Recovery waits on the real reap as well as any test confirmation, so it
 * lands after a real interval rather than on the next microtask. Polling for
 * the condition is what keeps that from being a timing guess.
 */
async function recoverWithin(root: string, budgetMs: number): Promise<unknown> {
  const deadline = Date.now() + budgetMs;
  for (;;) {
    try {
      return await withStableCorpusLease(root, () => "recovered");
    } catch (error) {
      if (
        !(error instanceof Error) ||
        !error.message.includes(REFUSAL) ||
        Date.now() > deadline
      ) {
        throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
}

/**
 * Long enough for a reap and its continuation to have been applied, so that a
 * following "still refused" assertion reflects a release that really happened
 * rather than one that has not run yet. Erring long only ever weakens
 * detection on a slow machine; it cannot turn a correct implementation red.
 */
function settle(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 250));
}

// Small enough that two leases fit under the process memory cap at once. The
// default budgets reserve enough that a second concurrent lease is refused for
// memory before it can spawn a child, which would leave the multi-child case
// below quietly exercising one child.
const STRAND_BUDGETS = {
  maxFileBytes: 1024 * 1024,
  maxCorpusBytes: 8 * 1024 * 1024,
  maxRetainedPathBytes: 256 * 1024,
  maxFileCount: 8,
  maxDirectoryCount: 4,
  maxDirectoryEntries: 16,
  maxWatcherCount: 4,
  maxReaderCount: 2,
};

/** One lease whose child is killed with its death deliberately unconfirmed. */
function strandOneWorker(
  root: string,
  confirmTerminationForTest: Promise<void>
): Promise<unknown> {
  return withStableCorpusLease(root, () => "unreachable", {
    timeoutMs: 2_000,
    budgets: STRAND_BUDGETS,
    workerStallPhaseForTest: "before-baseline",
    forceUnconfirmedTerminationForTest: true,
    confirmTerminationForTest,
  });
}

describe("stable corpus lease recovery", () => {
  it("reopens the process once the stranded child is confirmed reaped", async () => {
    await withCorpus(async (root) => {
      writeFileSync(join(root, "meeting.md"), "recovery canary");
      let confirmReap!: () => void;
      const reaped = new Promise<void>((resolve) => {
        confirmReap = resolve;
      });

      await expect(strandOneWorker(root, reaped)).rejects.toThrow(
        "stable meeting corpus authorization failed"
      );

      // Unconfirmed: the child may still be reading, so nothing may run.
      await expect(
        withStableCorpusLease(root, () => "must not run")
      ).rejects.toThrow(REFUSAL);

      confirmReap();
      // A reaped child reads nothing, so the reason for the refusal is gone.
      await expect(recoverWithin(root, 10_000)).resolves.toBe("recovered");
    });
  });

  it("keeps refusing until the last of several stranded children is reaped", async () => {
    await withCorpus(async (root) => {
      writeFileSync(join(root, "meeting.md"), "two canaries");
      let confirmFirst!: () => void;
      const firstReaped = new Promise<void>((resolve) => {
        confirmFirst = resolve;
      });
      let confirmSecond!: () => void;
      const secondReaped = new Promise<void>((resolve) => {
        confirmSecond = resolve;
      });

      // Concurrently, because the first strand refuses every later lease: a
      // second child can only be stranded by one already in flight.
      const [first, second] = await Promise.allSettled([
        strandOneWorker(root, firstReaped),
        strandOneWorker(root, secondReaped),
      ]);
      // Both must have been stranded by the kill, not turned away by a
      // budget before spawning, or the reap-one-of-two assertion below is
      // measuring a single child.
      for (const settled of [first, second]) {
        expect(settled.status).toBe("rejected");
        expect(
          settled.status === "rejected" ? (settled.reason as Error).message : ""
        ).toContain("stable meeting corpus authorization failed");
      }

      // Reaping one of two must not reopen the process. A flag instead of a
      // count passes the single-child case above and fails here.
      confirmFirst();
      await settle();
      await expect(
        withStableCorpusLease(root, () => "must not run")
      ).rejects.toThrow(REFUSAL);

      confirmSecond();
      await expect(recoverWithin(root, 10_000)).resolves.toBe("recovered");
    });
  });

  it("recovers without help when the real child is reaped, and frees its memory", async () => {
    await withCorpus(async (root) => {
      // The corpus stays empty: the budget pair at the end of this test allows
      // zero files, matching the admission case in corpus-lease.test.ts that it
      // mirrors. Disclosure does not depend on the contents, since the root
      // reaches the child in `begin` either way.
      // No confirmation hook: this is the production path, and the CI flake it
      // guards. SIGKILL cannot be caught, so the child is reaped a moment
      // after the grace expires and the process must reopen by itself.
      await expect(
        withStableCorpusLease(root, () => "unreachable", {
          timeoutMs: 2_000,
          budgets: STRAND_BUDGETS,
          workerStallPhaseForTest: "before-baseline",
          forceUnconfirmedTerminationForTest: true,
        })
      ).rejects.toThrow("stable meeting corpus authorization failed");

      await expect(recoverWithin(root, 10_000)).resolves.toBe("recovered");

      // The stranded lease also held a 23,724,032 byte reservation. Proving
      // that was released needs a pair sized to straddle the 256 MiB process
      // cap: two of these fit together (266,895,360) only while the stranded
      // bytes are gone, and the second is refused once they are added back.
      //
      // An earlier version of this assertion used two copies of the budget
      // from the admission case in corpus-lease.test.ts. Those exceed the cap
      // on their own, so the second lease was refused whether or not the
      // stranded bytes leaked, and the test proved nothing.
      const halfCapBudgets = {
        maxFileBytes: 0,
        maxCorpusBytes: 64_000_000,
        // Room for the lease's own on-disk fence entry, and a watcher budget
        // that admits a second concurrent worker: this pair has to be refused
        // for memory or not at all, never for an unrelated cap.
        maxRetainedPathBytes: 65_536,
        maxFileCount: 0,
        maxDirectoryCount: 1,
        maxDirectoryEntries: 2,
        maxWatcherCount: 4,
        maxReaderCount: 1,
      };
      let release!: () => void;
      const hold = new Promise<void>((resolve) => {
        release = resolve;
      });
      let ready!: () => void;
      const retained = new Promise<void>((resolve) => {
        ready = resolve;
      });
      const held = withStableCorpusLease(root, () => "held", {
        budgets: halfCapBudgets,
        afterBaseline: async () => {
          ready();
          await hold;
        },
      });
      // Racing surfaces an admission failure as its own error. A bare await on
      // `retained` would hang to the suite timeout instead, because the
      // baseline hook that resolves it never runs.
      await Promise.race([retained, held]);
      await expect(
        withStableCorpusLease(root, () => "second", { budgets: halfCapBudgets })
      ).resolves.toBe("second");
      release();
      await expect(held).resolves.toBe("held");
    });
  });
});
