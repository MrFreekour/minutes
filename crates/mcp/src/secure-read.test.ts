import {
  linkSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { readTextFileFromBoundParent } from "./secure-read.js";

describe("MCP OS-bound parent reads", () => {
  it("enforces the byte budget inside the bound reader", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-budget-"));
    try {
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
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("rejects an in-root hard link to an out-of-root file", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-hardlink-root-"));
    const outside = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-hardlink-outside-"));
    try {
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
    } finally {
      rmSync(root, { recursive: true, force: true });
      rmSync(outside, { recursive: true, force: true });
    }
  });

  it("rejects held-read admission overflow and recovers every reservation", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-admission-"));
    try {
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
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("keeps a failed-kill reader charged until termination is confirmed", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-reap-"));
    const meetingPath = join(root, "meeting.md");
    writeFileSync(meetingPath, "SAFE_REAP_BYTES");
    const canonicalPath = realpathSync(meetingPath);
    let release: (() => void) | undefined;
    let failedKillCalls = 0;
    let allowKill = false;
    try {
      expect(
        (await readTextFileFromBoundParent(canonicalPath)).toString("utf8")
      ).toBe("SAFE_REAP_BYTES");

      let announcePaused!: () => void;
      const paused = new Promise<void>((resolve) => (announcePaused = resolve));
      const held = new Promise<void>((resolve) => (release = resolve));
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

      await paused;
      expect(await outcome).toBeInstanceOf(Error);
      await expect(
        readTextFileFromBoundParent(canonicalPath, {
          maxBytes: 32,
          maxReaders: 1,
        })
      ).rejects.toThrow("Access denied: bound reader capacity exceeded");
      expect(failedKillCalls).toBeGreaterThan(0);
      allowKill = true;
      release();
      release = undefined;

      expect(
        (
          await readTextFileFromBoundParent(canonicalPath, {
            maxBytes: 32,
            maxReaders: 1,
          })
        ).toString("utf8")
      ).toBe("SAFE_REAP_BYTES");
    } finally {
      allowKill = true;
      release?.();
      rmSync(root, { recursive: true, force: true });
    }
  }, 10_000);

  it("keeps the full failed-child memory charge until termination", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-reap-memory-"));
    let release: (() => void) | undefined;
    let allowKill = false;
    try {
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
      const held = new Promise<void>((resolve) => (release = resolve));
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

      await paused;
      expect(await outcome).toBeInstanceOf(Error);
      await expect(
        readTextFileFromBoundParent(second, {
          maxBytes,
          maxReaders: 2,
          maxReservedBytes,
        })
      ).rejects.toThrow("Access denied: bound reader capacity exceeded");

      allowKill = true;
      release?.();
      release = undefined;
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
    } finally {
      allowKill = true;
      release?.();
      rmSync(root, { recursive: true, force: true });
    }
  }, 10_000);

  it("aborts an in-flight bound read and retires its child before reuse", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-abort-"));
    let release: (() => void) | undefined;
    try {
      const meetingPath = join(root, "meeting.md");
      writeFileSync(meetingPath, "SAFE_ABORT_BYTES");
      const canonicalPath = realpathSync(meetingPath);
      const controller = new AbortController();
      let announcePaused!: () => void;
      const paused = new Promise<void>((resolve) => (announcePaused = resolve));
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
      release = undefined;

      expect(
        (await readTextFileFromBoundParent(canonicalPath)).toString("utf8")
      ).toBe("SAFE_ABORT_BYTES");
    } finally {
      release?.();
      rmSync(root, { recursive: true, force: true });
    }
  }, 10_000);

  it("flushes a backpressured maximum response before idle exit", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-backpressure-"));
    try {
      const meetingPath = join(root, "meeting.md");
      const bytes = Buffer.alloc(8 * 1024 * 1024, 0x61);
      writeFileSync(meetingPath, bytes);

      const observed = await readTextFileFromBoundParent(
        realpathSync(meetingPath),
        {
          maxBytes: bytes.byteLength,
          timeoutMs: 10_000,
          afterFirstRead: () => {
            setImmediate(() => {
              Atomics.wait(
                new Int32Array(new SharedArrayBuffer(4)),
                0,
                0,
                2_000
              );
            });
          },
        }
      );
      expect(observed.equals(bytes)).toBe(true);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }, 15_000);
});
