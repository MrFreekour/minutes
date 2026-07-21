import {
  existsSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  renameSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, describe, expect, it } from "vitest";

import { readTextFileFromBoundParent } from "./secure-read.js";

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("OS-bound parent reads", () => {
  it("enforces the byte budget inside the bound reader", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-budget-"));
    roots.push(root);
    const meetingPath = join(root, "meeting.md");
    writeFileSync(meetingPath, "FIVE!");
    const canonicalPath = realpathSync(meetingPath);

    await expect(
      readTextFileFromBoundParent(canonicalPath, { maxBytes: 4 })
    ).rejects.toThrow(/^Access denied:/);
    expect(
      (
        await readTextFileFromBoundParent(canonicalPath, { maxBytes: 5 })
      ).toString("utf8")
    ).toBe("FIVE!");
  });

  it("rejects an in-root hard link to an out-of-root file", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-hardlink-root-"));
    const outside = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-hardlink-outside-"));
    roots.push(root, outside);
    const outsidePath = join(outside, "private.md");
    const meetingPath = join(root, "meeting.md");
    writeFileSync(outsidePath, "OUTSIDE_HARDLINK_CANARY");
    linkSync(outsidePath, meetingPath);

    let failure: unknown;
    try {
      await readTextFileFromBoundParent(realpathSync(meetingPath));
    } catch (error) {
      failure = error;
    }
    expect(failure).toBeInstanceOf(Error);
    expect((failure as Error).message).toMatch(/^Access denied:/);
    expect((failure as Error).message).not.toContain("OUTSIDE_HARDLINK_CANARY");
    expect((failure as Error).message).not.toContain(outside);
  });

  it("keeps a paused read alive while a bounded concurrent read completes", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-concurrent-"));
    roots.push(root);
    const firstPath = join(root, "first.md");
    const secondPath = join(root, "second.md");
    writeFileSync(firstPath, "FIRST_SAFE_BYTES");
    writeFileSync(secondPath, "SECOND_SAFE_BYTES");

    let announcePaused!: () => void;
    const paused = new Promise<void>((resolve) => (announcePaused = resolve));
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const first = readTextFileFromBoundParent(realpathSync(firstPath), {
      maxBytes: 64,
      afterFirstRead: async () => {
        announcePaused();
        await held;
      },
    });
    await paused;
    try {
      const second = await readTextFileFromBoundParent(realpathSync(secondPath), {
        maxBytes: 64,
      });
      expect(second.toString("utf8")).toBe("SECOND_SAFE_BYTES");
      await new Promise((resolve) => setTimeout(resolve, 850));
    } finally {
      release();
      expect((await first).toString("utf8")).toBe("FIRST_SAFE_BYTES");
    }
  });

  it("rejects held-read admission overflow and recovers every reservation", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-admission-"));
    roots.push(root);
    const parentA = join(root, "a");
    const parentB = join(root, "b");
    mkdirSync(parentA);
    mkdirSync(parentB);
    const firstPath = join(parentA, "first.md");
    const sameParentPath = join(parentA, "second.md");
    const otherParentPath = join(parentB, "third.md");
    writeFileSync(firstPath, "FIRST");
    writeFileSync(sameParentPath, "SECOND");
    writeFileSync(otherParentPath, "THIRD");
    const first = realpathSync(firstPath);
    const sameParent = realpathSync(sameParentPath);
    const otherParent = realpathSync(otherParentPath);

    const hold = async (hooks: {
      maxBytes: number;
      maxInFlightPerReader: number;
      maxInFlightGlobal: number;
      maxReservedBytes: number;
    }) => {
      let announcePaused!: () => void;
      const paused = new Promise<void>((resolve) => (announcePaused = resolve));
      let release!: () => void;
      const held = new Promise<void>((resolve) => (release = resolve));
      const read = readTextFileFromBoundParent(first, {
        ...hooks,
        afterFirstRead: async () => {
          announcePaused();
          await held;
        },
      });
      await paused;
      return { read, release };
    };

    const perReader = {
      maxBytes: 8,
      maxInFlightPerReader: 1,
      maxInFlightGlobal: 4,
      maxReservedBytes: 384 * 1024 * 1024,
    };
    let active = await hold(perReader);
    await expect(
      readTextFileFromBoundParent(sameParent, perReader)
    ).rejects.toThrow("Access denied: bound reader capacity exceeded");
    active.release();
    expect((await active.read).toString("utf8")).toBe("FIRST");
    expect(
      (await readTextFileFromBoundParent(sameParent, perReader)).toString("utf8")
    ).toBe("SECOND");

    const global = {
      maxBytes: 8,
      maxInFlightPerReader: 4,
      maxInFlightGlobal: 1,
      maxReservedBytes: 384 * 1024 * 1024,
    };
    active = await hold(global);
    await expect(
      readTextFileFromBoundParent(otherParent, global)
    ).rejects.toThrow("Access denied: bound reader capacity exceeded");
    active.release();
    await active.read;
    expect(
      (await readTextFileFromBoundParent(otherParent, global)).toString("utf8")
    ).toBe("THIRD");

    const reserved = {
      maxBytes: 8,
      maxInFlightPerReader: 1,
      maxInFlightGlobal: 1,
      maxReservedBytes: 32 * 1024 * 1024,
    };
    await expect(
      readTextFileFromBoundParent(otherParent, reserved)
    ).rejects.toThrow("Access denied: bound reader capacity exceeded");
    expect(
      (
        await readTextFileFromBoundParent(otherParent, {
          ...reserved,
          maxBytes: 8,
          maxReservedBytes: 384 * 1024 * 1024,
        })
      ).toString("utf8")
    ).toBe("THIRD");
  });

  it("retires a timed-out child before a late hook resumes", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-timeout-"));
    roots.push(root);
    const meetingPath = join(root, "meeting.md");
    writeFileSync(meetingPath, "SAFE_AFTER_TIMEOUT");

    // Warm the parent-bound reader before arming the deliberately short test
    // timeout. The behavior under test is retirement after the first-read
    // hook pauses, not process startup latency on a pressured CI host.
    expect(
      (await readTextFileFromBoundParent(realpathSync(meetingPath))).toString("utf8")
    ).toBe("SAFE_AFTER_TIMEOUT");

    let announcePaused!: () => void;
    const paused = new Promise<void>((resolve) => (announcePaused = resolve));
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const timedOut = readTextFileFromBoundParent(realpathSync(meetingPath), {
      afterFirstRead: async () => {
        announcePaused();
        await held;
      },
      timeoutMs: 2_000,
      maxBytes: 32,
      maxReservedBytes: 384 * 1024 * 1024,
    });
    // Attach the rejection handler immediately. Waiting for the hook first can
    // otherwise turn a slow child start into both an unhandled rejection and a
    // permanently unresolved `paused` promise.
    const timeoutOutcome = timedOut.then(
      () => ({ ok: true as const }),
      (error: unknown) => ({ ok: false as const, error })
    );

    try {
      const startup = await Promise.race([
        paused.then(() => "paused" as const),
        timeoutOutcome.then(() => "settled" as const),
      ]);
      expect(startup).toBe("paused");
      const outcome = await timeoutOutcome;
      if (outcome.ok) throw new Error("expected the paused bound read to time out");
      expect(outcome.error).toBeInstanceOf(Error);
      expect((outcome.error as Error).message).toMatch(/timed out/i);
    } finally {
      // Always let the late hook resume. It must observe the retired reader and
      // cannot revive or publish the timed-out request.
      release();
      await timeoutOutcome;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(
      (
        await readTextFileFromBoundParent(realpathSync(meetingPath), {
          maxBytes: 32,
          maxReservedBytes: 384 * 1024 * 1024,
        })
      ).toString("utf8")
    ).toBe("SAFE_AFTER_TIMEOUT");
  }, 10_000);

  it("keeps a failed-kill reader charged until termination is confirmed", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-reap-"));
    roots.push(root);
    const meetingPath = join(root, "meeting.md");
    writeFileSync(meetingPath, "SAFE_REAP_BYTES");
    const canonicalPath = realpathSync(meetingPath);

    expect(
      (await readTextFileFromBoundParent(canonicalPath)).toString("utf8")
    ).toBe("SAFE_REAP_BYTES");

    let announcePaused!: () => void;
    const paused = new Promise<void>((resolve) => (announcePaused = resolve));
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    let failedKillCalls = 0;
    let allowKill = false;
    const timedOut = readTextFileFromBoundParent(canonicalPath, {
      afterFirstRead: async () => {
        announcePaused();
        await held;
      },
      timeoutMs: 25,
      maxBytes: 32,
      maxReaders: 1,
      retireChildForTest: (terminate) => {
        failedKillCalls += 1;
        return allowKill ? terminate() : false;
      },
    });
    const outcome = timedOut.then(
      () => null,
      (error: unknown) => error
    );

    try {
      await paused;
      expect(await outcome).toBeInstanceOf(Error);
      await expect(
        readTextFileFromBoundParent(canonicalPath, {
          maxBytes: 32,
          maxReaders: 1,
        })
      ).rejects.toThrow("Access denied: bound reader capacity exceeded");
      expect(failedKillCalls).toBeGreaterThan(0);
    } finally {
      allowKill = true;
      release();
    }

    expect(
      (
        await readTextFileFromBoundParent(canonicalPath, {
          maxBytes: 32,
          maxReaders: 1,
        })
      ).toString("utf8")
    ).toBe("SAFE_REAP_BYTES");
  }, 10_000);

  it("keeps the full failed-child memory charge until termination", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-reap-memory-"));
    roots.push(root);
    const firstParent = join(root, "first");
    const secondParent = join(root, "second");
    mkdirSync(firstParent);
    mkdirSync(secondParent);
    const firstPath = join(firstParent, "meeting.md");
    const secondPath = join(secondParent, "meeting.md");
    writeFileSync(firstPath, "FIRST_SAFE_BYTES");
    writeFileSync(secondPath, "SECOND_SAFE_BYTES");
    const first = realpathSync(firstPath);
    const second = realpathSync(secondPath);
    const maxBytes = 4 * 1024 * 1024;
    const maxReservedBytes = 160 * 1024 * 1024;

    expect((await readTextFileFromBoundParent(first)).toString("utf8")).toBe(
      "FIRST_SAFE_BYTES"
    );
    let announcePaused!: () => void;
    const paused = new Promise<void>((resolve) => (announcePaused = resolve));
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    let allowKill = false;
    const timedOut = readTextFileFromBoundParent(first, {
      afterFirstRead: async () => {
        announcePaused();
        await held;
      },
      timeoutMs: 25,
      maxBytes,
      maxReaders: 2,
      maxReservedBytes,
      retireChildForTest: (terminate) => (allowKill ? terminate() : false),
    });
    const outcome = timedOut.then(
      () => null,
      (error: unknown) => error
    );

    try {
      await paused;
      expect(await outcome).toBeInstanceOf(Error);
      await expect(
        readTextFileFromBoundParent(second, {
          maxBytes,
          maxReaders: 2,
          maxReservedBytes,
        })
      ).rejects.toThrow("Access denied: bound reader capacity exceeded");
    } finally {
      allowKill = true;
      release();
    }

    expect(
      (
        await readTextFileFromBoundParent(first, {
          maxBytes: 32,
          maxReaders: 2,
          maxReservedBytes,
        })
      ).toString("utf8")
    ).toBe("FIRST_SAFE_BYTES");
    expect(
      (
        await readTextFileFromBoundParent(second, {
          maxBytes,
          maxReaders: 2,
          maxReservedBytes,
        })
      ).toString("utf8")
    ).toBe("SECOND_SAFE_BYTES");
  }, 10_000);

  it("aborts an in-flight bound read and retires its child before reuse", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-abort-"));
    roots.push(root);
    const meetingPath = join(root, "meeting.md");
    writeFileSync(meetingPath, "SAFE_ABORT_BYTES");
    const canonicalPath = realpathSync(meetingPath);
    const controller = new AbortController();
    let announcePaused!: () => void;
    const paused = new Promise<void>((resolve) => (announcePaused = resolve));
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const read = readTextFileFromBoundParent(canonicalPath, {
      signal: controller.signal,
      afterFirstRead: async () => {
        announcePaused();
        await held;
      },
    });

    await paused;
    controller.abort();
    await expect(read).rejects.toThrow("Access denied: bound read aborted");
    release();

    expect(
      (await readTextFileFromBoundParent(canonicalPath)).toString("utf8")
    ).toBe("SAFE_ABORT_BYTES");
  }, 10_000);

  it("flushes a backpressured maximum response before idle exit", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-backpressure-"));
    roots.push(root);
    const meetingPath = join(root, "meeting.md");
    const bytes = Buffer.alloc(8 * 1024 * 1024, 0x61);
    writeFileSync(meetingPath, bytes);

    const observed = await readTextFileFromBoundParent(realpathSync(meetingPath), {
      maxBytes: bytes.byteLength,
      timeoutMs: 10_000,
      afterFirstRead: () => {
        setImmediate(() => {
          Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 2_000);
        });
      },
    });
    expect(observed.equals(bytes)).toBe(true);
  }, 15_000);

  it.runIf(process.platform !== "win32")(
    "keeps outside bytes inert across a parent swap and restore",
    async () => {
      const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-root-"));
      const outside = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-outside-"));
      roots.push(root, outside);
      const parent = join(root, "nested");
      const displaced = join(root, "displaced-parent");
      mkdirSync(parent);
      const meetingPath = join(parent, "meeting.md");
      writeFileSync(meetingPath, "SAFE_BYTES");
      writeFileSync(join(outside, "meeting.md"), "OUTSIDE_CANARY");

      const bytes = await readTextFileFromBoundParent(realpathSync(meetingPath), {
        afterFirstRead: () => {
          renameSync(parent, displaced);
          symlinkSync(outside, parent);
          rmSync(parent, { force: true });
          renameSync(displaced, parent);
        },
      });

      expect(bytes.toString("utf8")).toBe("SAFE_BYTES");
      expect(bytes.toString("utf8")).not.toContain("OUTSIDE_CANARY");
    }
  );

  it.runIf(process.platform === "win32")(
    "never returns outside bytes across a Windows parent junction swap",
    async () => {
      const root = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-win-root-"));
      const outside = mkdtempSync(join(tmpdir(), "minutes-sdk-bound-win-outside-"));
      roots.push(root, outside);
      const parent = join(root, "nested");
      const displaced = join(root, "displaced-parent");
      mkdirSync(parent);
      const meetingPath = join(parent, "meeting.md");
      writeFileSync(meetingPath, "SAFE_BYTES");
      writeFileSync(join(outside, "meeting.md"), "WINDOWS_OUTSIDE_CANARY");

      expect(
        (await readTextFileFromBoundParent(realpathSync(meetingPath))).toString("utf8")
      ).toBe("SAFE_BYTES");
      const preflight = join(root, "junction-preflight");
      symlinkSync(outside, preflight, "junction");
      expect(realpathSync(preflight)).toBe(realpathSync(outside));
      rmSync(preflight, { recursive: true, force: true });

      let renameWasBlocked = false;
      let junctionWasInstalled = false;
      let setupFailure: unknown = null;
      let observed: string | null = null;
      try {
        observed = (
          await readTextFileFromBoundParent(realpathSync(meetingPath), {
            afterFirstRead: () => {
              try {
                renameSync(parent, displaced);
              } catch {
                renameWasBlocked = true;
                return;
              }
              try {
                symlinkSync(outside, parent, "junction");
                junctionWasInstalled = true;
                rmSync(parent, { recursive: true, force: true });
                renameSync(displaced, parent);
              } catch (error) {
                setupFailure = error;
                if (existsSync(parent)) rmSync(parent, { recursive: true, force: true });
                if (existsSync(displaced)) renameSync(displaced, parent);
                throw error;
              }
            },
          })
        ).toString("utf8");
      } catch {
        // A successfully installed junction may cause a fail-closed read.
      }

      expect(setupFailure).toBeNull();
      expect(renameWasBlocked || junctionWasInstalled).toBe(true);
      if (renameWasBlocked) expect(observed).toBe("SAFE_BYTES");
      expect(observed ?? "").not.toContain("WINDOWS_OUTSIDE_CANARY");
    }
  );
});
