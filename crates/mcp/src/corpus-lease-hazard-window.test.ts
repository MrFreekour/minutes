import { describe, expect, it } from "vitest";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { withStableCorpusLease } from "./corpus-lease.js";
import { retireBoundReadersForProcessShutdown } from "./secure-read.js";

// This case ends with a projection that ignores cancellation and therefore
// never confirms, so the process is left refusing every later lease. Like
// corpus-lease-poisoning.test.ts it must stay a file of exactly one test.
function withCorpus(run: (root: string) => Promise<void>): Promise<void> {
  const root = mkdtempSync(join(tmpdir(), "minutes-corpus-hazard-"));
  return run(root).finally(async () => {
    await retireBoundReadersForProcessShutdown();
    rmSync(root, { recursive: true, force: true });
  });
}

describe("stable corpus lease hazard window", () => {
  it("never reopens between the worker's reap and the projection's hold", async () => {
    await withCorpus(async (root) => {
      writeFileSync(join(root, "meeting.md"), "window canary");

      let markStarted!: () => void;
      const started = new Promise<void>((resolve) => {
        markStarted = resolve;
      });
      let forceOperationDeadline!: () => void;
      const operationDeadline = new Promise<void>((resolve) => {
        forceOperationDeadline = resolve;
      });

      // Two hazards from one lease: a worker whose death is not confirmed
      // inside its grace, and a projection that ignores cancellation. They are
      // decided in separate branches with an await between them, which is the
      // gap this test exists to close.
      const lease = withStableCorpusLease(
        root,
        () => {
          markStarted();
          return new Promise<never>(() => {});
        },
        {
          timeoutMs: 15_000,
          operationDeadlineForTest: operationDeadline,
          forceUnconfirmedTerminationForTest: true,
          // Settles the worker hazard at the earliest moment it can exist, so
          // the interval this test samples is the whole projection grace
          // rather than whatever is left after a real reap lands. It is ANDed
          // with the real termination, so it cannot open a window that
          // production would keep shut.
          confirmTerminationForTest: Promise.resolve(),
        }
      );
      await started;

      // Runs continuously from before the failure until well after both
      // branches have been decided. While the lease is healthy this is refused
      // for memory, since two default reservations exceed the process cap;
      // once the lease fails it must be refused because the hazards hold the
      // process closed. An accounting that releases per hazard instead of per
      // lease drops to zero when the worker is reaped inside the projection's
      // grace, and this poller is admitted in that gap.
      let admitted = false;
      let polling = true;
      const poller = (async () => {
        while (polling) {
          try {
            await withStableCorpusLease(root, () => "admitted");
            admitted = true;
            return;
          } catch {
            // Refusal is the expected answer at every instant.
          }
          await new Promise((resolve) => setTimeout(resolve, 2));
        }
      })();

      forceOperationDeadline();
      await expect(lease).rejects.toThrow(
        "stable meeting corpus authorization failed"
      );
      // Comfortably longer than the projection confirmation grace, so the
      // poller spans the whole interval between the two branches.
      await new Promise((resolve) => setTimeout(resolve, 400));
      polling = false;
      await poller;

      expect(admitted).toBe(false);
    });
  }, 20_000);
});
