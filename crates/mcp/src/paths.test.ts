import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  utimesSync,
  writeFileSync,
} from "fs";
import { homedir, tmpdir } from "os";
import { join } from "path";
import { afterEach, describe, expect, it } from "vitest";

import {
  canonicalizeRoot,
  expandHomeLikePath,
  isWithinDirectory,
  readTextFileInDirectory,
  validatePathInDirectory,
} from "./paths.js";
import { readTextFileFromBoundParent } from "./secure-read.js";

const tempRoots: string[] = [];

afterEach(() => {
  for (const root of tempRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("path normalization", () => {
  it("expands shell-style home roots", () => {
    expect(expandHomeLikePath("~/meetings")).toBe(join(homedir(), "meetings"));
    expect(expandHomeLikePath("$HOME/meetings")).toBe(join(homedir(), "meetings"));
    expect(expandHomeLikePath("${HOME}/meetings")).toBe(join(homedir(), "meetings"));
  });

  it("anchors relative roots to home instead of the process cwd", () => {
    expect(expandHomeLikePath(".minutes/corrections")).toBe(
      join(homedir(), ".minutes", "corrections")
    );
    expect(canonicalizeRoot("relative-minutes-home")).toBe(
      join(homedir(), "relative-minutes-home")
    );
  });

  it("accepts a meeting file when the configured root uses ${HOME}", () => {
    const tempRoot = mkdtempSync(join(tmpdir(), "minutes-mcp-paths-"));
    tempRoots.push(tempRoot);
    const originalHome = process.env.HOME;
    const originalUserProfile = process.env.USERPROFILE;
    process.env.HOME = tempRoot;
    process.env.USERPROFILE = tempRoot;

    try {
      const meetingsDir = join(tempRoot, "meetings");
      mkdirSync(meetingsDir, { recursive: true });

      const meetingPath = join(meetingsDir, "2026-03-28-home-expansion.md");
      writeFileSync(meetingPath, "# test meeting\n");

      expect(validatePathInDirectory(meetingPath, "${HOME}/meetings", [".md"])).toBe(
        realpathSync(meetingPath)
      );
    } finally {
      if (originalHome === undefined) {
        delete process.env.HOME;
      } else {
        process.env.HOME = originalHome;
      }
      if (originalUserProfile === undefined) {
        delete process.env.USERPROFILE;
      } else {
        process.env.USERPROFILE = originalUserProfile;
      }
    }
  });
});

describe("isWithinDirectory", () => {
  it("rejects paths that share a prefix but are not children", () => {
    // ~/meetings-evil should NOT be within ~/meetings
    expect(isWithinDirectory("/home/user/meetings-evil", "/home/user/meetings")).toBe(false);
    expect(isWithinDirectory("/home/user/meetings-evil/file.md", "/home/user/meetings")).toBe(false);
  });

  it("accepts exact root match and direct children", () => {
    expect(isWithinDirectory("/home/user/meetings", "/home/user/meetings")).toBe(true);
    expect(isWithinDirectory("/home/user/meetings/file.md", "/home/user/meetings")).toBe(true);
    expect(isWithinDirectory("/home/user/meetings/sub/file.md", "/home/user/meetings")).toBe(true);
  });

  it("uses native Windows separators when running on Windows", () => {
    if (process.platform !== "win32") {
      return;
    }

    expect(isWithinDirectory("C:\\Users\\alice\\meetings", "C:\\Users\\alice\\meetings")).toBe(true);
    expect(
      isWithinDirectory("C:\\Users\\alice\\meetings\\daily\\note.md", "C:\\Users\\alice\\meetings")
    ).toBe(true);
    expect(
      isWithinDirectory("C:\\Users\\alice\\meetings-evil\\note.md", "C:\\Users\\alice\\meetings")
    ).toBe(false);
  });
});

describe("descriptor-bound reads", () => {
  it("keeps a paused read alive while a concurrent read completes", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-concurrent-"));
    tempRoots.push(root);
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

  it("retires a timed-out child before a late hook resumes", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-bound-timeout-"));
    tempRoots.push(root);
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
      (await readTextFileFromBoundParent(realpathSync(meetingPath))).toString("utf8")
    ).toBe("SAFE_AFTER_TIMEOUT");
  }, 10_000);

  it("rejects a same-inode same-size rewrite between complete reads", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-mcp-read-mutation-"));
    tempRoots.push(root);
    const meetingPath = join(root, "meeting.md");
    writeFileSync(meetingPath, "SAFE_BYTES");
    const original = statSync(meetingPath);

    await expect(
      readTextFileInDirectory(
        meetingPath,
        root,
        [".md"],
        () => {},
        () => {
          writeFileSync(meetingPath, "EVIL_BYTES");
          // Restore mtime to prove authorization does not depend on a coarse
          // or caller-controlled modification timestamp.
          utimesSync(meetingPath, original.atime, original.mtime);
        }
      )
    ).rejects.toThrow(/changed/i);
  });

  it.runIf(process.platform !== "win32")(
    "rejects synchronized final-component and parent-directory symlink swaps",
    async () => {
      for (const swapParent of [false, true]) {
        const root = mkdtempSync(join(tmpdir(), "minutes-mcp-read-root-"));
        const outside = mkdtempSync(join(tmpdir(), "minutes-mcp-read-outside-"));
        tempRoots.push(root, outside);
        const parent = join(root, "nested");
        mkdirSync(parent);
        const meetingPath = join(parent, "meeting.md");
        writeFileSync(meetingPath, "SAFE_BYTES");
        writeFileSync(join(outside, "meeting.md"), "OUTSIDE_CANARY");

        await expect(
          readTextFileInDirectory(meetingPath, root, [".md"], () => {
            if (swapParent) {
              renameSync(parent, join(root, "original-parent"));
              symlinkSync(outside, parent);
            } else {
              renameSync(meetingPath, join(parent, "original.md"));
              symlinkSync(join(outside, "meeting.md"), meetingPath);
            }
          })
        ).rejects.toThrow(/denied|changed|ELOOP/i);
      }
    }
  );

  it.runIf(process.platform !== "win32")(
    "never returns outside bytes when a parent is swapped and restored mid-read",
    async () => {
      const root = mkdtempSync(join(tmpdir(), "minutes-mcp-read-restore-root-"));
      const outside = mkdtempSync(join(tmpdir(), "minutes-mcp-read-restore-outside-"));
      tempRoots.push(root, outside);
      const parent = join(root, "nested");
      const displaced = join(root, "displaced-parent");
      mkdirSync(parent);
      const meetingPath = join(parent, "meeting.md");
      writeFileSync(meetingPath, "SAFE_BYTES");
      writeFileSync(join(outside, "meeting.md"), "OUTSIDE_CANARY");

      const verified = await readTextFileInDirectory(
        meetingPath,
        root,
        [".md"],
        () => {},
        () => {
          renameSync(parent, displaced);
          symlinkSync(outside, parent);
          rmSync(parent, { force: true });
          renameSync(displaced, parent);
        }
      );

      expect(verified.content).toBe("SAFE_BYTES");
      expect(verified.content).not.toContain("OUTSIDE_CANARY");
    }
  );

  it.runIf(process.platform === "win32")(
    "never returns outside bytes across a Windows parent junction swap",
    async () => {
      const root = mkdtempSync(join(tmpdir(), "minutes-mcp-read-win-root-"));
      const outside = mkdtempSync(join(tmpdir(), "minutes-mcp-read-win-outside-"));
      tempRoots.push(root, outside);
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
          await readTextFileInDirectory(
            meetingPath,
            root,
            [".md"],
            () => {},
            () => {
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
            }
          )
        ).content;
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
