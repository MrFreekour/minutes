import { createHash } from "node:crypto";
import {
  appendFileSync,
  chmodSync,
  existsSync,
  fstatSync,
  linkSync,
  mkdtempSync,
  mkdirSync,
  openSync,
  readSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { ResourceUpdatedNotificationSchema } from "@modelcontextprotocol/sdk/types.js";
import { describe, expect, it } from "vitest";
import { z } from "zod";

import {
  afterRequiredCli,
  afterActiveCopilotReadiness,
  afterContentBearingToolReadiness,
  afterContentResourceReadiness,
  afterAgentTrustReadiness,
  assistantSafeContextLinks,
  buildMcpProcessAudioArgs,
  buildLiveCopilotResourcePayload,
  buildLiveEventsResourcePayload,
  buildPrivacySafeProcessingJobsResult,
  buildPrivacySafeStatusResource,
  buildPrivacySafeStatusText,
  canonicalPathWireEquals,
  collectPolicyVerifiedMeetingSnapshots,
  collectPolicyToolSearchSnapshots,
  contentBearingAgentToolNames,
  contentBearingAgentResourceNames,
  enforceRestrictedContentPolicy,
  enrichWithFrontmatter,
  extractMarkdownSection,
  getEffectiveMeetingsDir,
  handleMcpProcessAudioRequest,
  isActiveCorpusMeetingPath,
  isPathWithinCanonicalRoot,
  LIVE_COPILOT_RESOURCE_URI,
  LIVE_EVENTS_RESOURCE_URI,
  LIVE_EVENTS_SUBSCRIPTIONS_ENABLED,
  liveMeetingSnippet,
  meetingDetailPayload,
  verifiedCliSpeakerOverlay,
  verifiedStopRecordingSummary,
  meetingListItem,
  meetingSearchItem,
  mcpCliChildEnv,
  MCP_ADD_NOTE_INPUT_SCHEMA,
  MCP_ACTION_RESULT_MAX,
  MCP_AGENT_ANNOTATIONS_UNAVAILABLE_DESCRIPTION,
  MCP_INTENT_RESULT_MAX,
  MCP_MEETING_RESULT_MAX,
  MCP_MEETING_INSIGHTS_UNAVAILABLE_DESCRIPTION,
  MCP_PERSON_PROFILE_MEETING_MAX,
  MCP_PERSON_PROFILE_OPEN_ACTION_MAX,
  MCP_PERSON_PROFILE_TOPIC_MAX,
  MCP_POLICY_MEETING_RESULT_MAX,
  MCP_PROCESSING_JOB_RESULT_MAX,
  MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS,
  MCP_RELATIONSHIP_RESULT_MAX,
  MCP_RESEARCH_DECISION_RESULT_MAX,
  MCP_RESEARCH_MEETING_RESULT_MAX,
  MCP_RESEARCH_TOPIC_RESULT_MAX,
  MEETING_INSIGHT_KINDS,
  normalizeCanonicalPathWire,
  normalizeMcpMeetingResultLimit,
  mcpProcessAudioPlatformPolicy,
  openActionsFromMeetings,
  personProfileFromMeetings,
  policyIntentResults,
  parseCopilotNudgeLog,
  parseCopilotStatusOutput,
  parseKnowledgeConfig,
  parseDictationModelMissingError,
  parseMeetingsRootSnapshot,
  parseLiveEventsResourceUri,
  parsePolicyVerifiedMeeting,
  policyVerifiedExactMeetingSnapshot,
  policyListMeetings,
  policySearchMeetings,
  registerDocsAppToolWithRestrictedPolicy,
  registerLiveEventsSubscriptionHandlers,
  registerToolWithRestrictedPolicy,
  registerUnavailableCompatibilityTools,
  readAgentTrustReadiness,
  readCopilotStatusFromCli,
  readKnowledgeStatusSnapshot,
  readLiveEventsResource,
  readVerifiedScreenImage,
  relationshipMapFromMeetings,
  researchTopicProjection,
  runAgentToolPolicies,
  stopCopilotBeforeStatusRead,
  restrictedMeetingStubResult,
  restrictedContentPolicyFromEnv,
  requireAgentTrustReadiness,
  terminalControlBeforeContentReadiness,
  selectCopilotNudges,
  shouldRunMainEntry,
  runMcpProcessAudioCli,
  runIsolatedMcpProcessAudio,
  validateMcpProcessAudioInput,
  withAuthorizedMcpProcessAudioInput,
  withPolicyBoundContextPath,
  type CopilotNudgeObservation,
  type AuthorizedMcpProcessAudioInput,
} from "./index.js";

describe("dictation model preflight errors", () => {
  it("extracts the model, expected path, and interrupted-download repair command", () => {
    const error = [
      "Error: Dictation model not installed: small",
      "Expected: /Users/test/.minutes/models/ggml-small.bin",
      "Fix: rm \"/Users/test/.minutes/models/ggml-small.bin\" && minutes setup --model small",
    ].join("\n");

    expect(parseDictationModelMissingError(error)).toEqual({
      model: "small",
      expectedPath: "/Users/test/.minutes/models/ggml-small.bin",
      setupCommand:
        "rm \"/Users/test/.minutes/models/ggml-small.bin\" && minutes setup --model small",
    });
  });

  it("ignores unrelated startup errors", () => {
    expect(parseDictationModelMissingError("microphone permission denied")).toBeNull();
  });
});

function rustCanonicalPathWire(path: string): string {
  if (process.platform !== "win32") return path;
  return path.startsWith("\\\\")
    ? `\\\\?\\UNC\\${path.slice(2)}`
    : `\\\\?\\${path}`;
}

describe("verified screen image reads", () => {
  const png = (suffix = "") =>
    Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      Buffer.from(suffix),
    ]);

  it("reads a stable PNG from an explicitly bound screen root", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-screen-root-"));
    try {
      const session = join(root, "session-a");
      mkdirSync(session);
      const image = join(session, "capture.png");
      const bytes = png("SCREEN_CANARY");
      writeFileSync(image, bytes);

      await expect(
        readVerifiedScreenImage(
          image,
          bytes.length,
          createHash("sha256").update(bytes).digest("hex"),
          root
        )
      ).resolves.toEqual(bytes);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("rejects a leaf replacement after the bound reader's first read", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-screen-leaf-swap-"));
    try {
      const session = join(root, "session-a");
      mkdirSync(session);
      const image = join(session, "capture.png");
      const original = png("ORIGINAL_SCREEN_CANARY");
      writeFileSync(image, original);

      await expect(
        readVerifiedScreenImage(
          image,
          original.length,
          createHash("sha256").update(original).digest("hex"),
          root,
          {
            afterFirstRead: () => {
              rmSync(image);
              writeFileSync(image, png("REPLACEMENT_SCREEN_CANARY"));
            },
          }
        )
      ).rejects.toThrow(/Access denied/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("rejects a parent replacement after validation", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-screen-parent-swap-"));
    try {
      const session = join(root, "session-a");
      const displaced = join(root, "session-displaced");
      mkdirSync(session);
      const image = join(session, "capture.png");
      const original = png("ORIGINAL_PARENT_CANARY");
      writeFileSync(image, original);

      await expect(
        readVerifiedScreenImage(
          image,
          original.length,
          createHash("sha256").update(original).digest("hex"),
          root,
          {
            afterFirstRead: () => {
              renameSync(session, displaced);
              mkdirSync(session);
              writeFileSync(image, png("REPLACEMENT_PARENT_CANARY"));
            },
          }
        )
      ).rejects.toThrow(/Access denied/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("rejects oversized and signature-invalid PNG paths", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-screen-invalid-"));
    try {
      const session = join(root, "session-a");
      mkdirSync(session);
      const fake = join(session, "fake.png");
      const fakeBytes = Buffer.from("NOT_A_PNG");
      writeFileSync(fake, fakeBytes);
      await expect(
        readVerifiedScreenImage(
          fake,
          fakeBytes.length,
          createHash("sha256").update(fakeBytes).digest("hex"),
          root
        )
      ).rejects.toThrow("not a verified PNG");

      const oversized = join(session, "oversized.png");
      const bytes = Buffer.alloc(10 * 1024 * 1024 + 1);
      png().copy(bytes);
      writeFileSync(oversized, bytes);
      await expect(
        readVerifiedScreenImage(
          oversized,
          bytes.length,
          createHash("sha256").update(bytes).digest("hex"),
          root
        )
      ).rejects.toThrow("invalid capture-time byte bound");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("rejects same-size bytes that no longer match the capture-time digest", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-screen-digest-"));
    try {
      const session = join(root, "session-a");
      mkdirSync(session);
      const image = join(session, "capture.png");
      const original = png("ORIGINAL_BYTES");
      const replacement = png("REPLACEMENT_BY");
      expect(replacement.length).toBe(original.length);
      writeFileSync(image, replacement);

      await expect(
        readVerifiedScreenImage(
          image,
          original.length,
          createHash("sha256").update(original).digest("hex"),
          root
        )
      ).rejects.toThrow("capture-time digest");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe("privacy-safe operational status", () => {
  const forbiddenCanaries = [
    "TITLE_PRIVATE_CANARY",
    "AUDIO_PATH_PRIVATE_CANARY",
    "OUTPUT_PATH_PRIVATE_CANARY",
    "USER_NOTES_PRIVATE_CANARY",
    "PRE_CONTEXT_PRIVATE_CANARY",
    "CONSENT_PRIVATE_CANARY",
    "CALENDAR_PRIVATE_CANARY",
    "TEMPLATE_PRIVATE_CANARY",
    "ERROR_PRIVATE_CANARY",
    "RAW_STAGE_PRIVATE_CANARY",
  ];

  it("projects job records into a closed path-free text and structured schema", () => {
    const result = buildPrivacySafeProcessingJobsResult([
      {
        id: "job-20260715123456789-4321-0",
        state: "Summarizing",
        stage: "RAW_STAGE_PRIVATE_CANARY",
        title: "TITLE_PRIVATE_CANARY",
        audio_path: "/private/AUDIO_PATH_PRIVATE_CANARY.wav",
        output_path: "/private/OUTPUT_PATH_PRIVATE_CANARY.md",
        user_notes: "USER_NOTES_PRIVATE_CANARY",
        pre_context: "PRE_CONTEXT_PRIVATE_CANARY",
        consent_notice: "CONSENT_PRIVATE_CANARY",
        calendar_event: { title: "CALENDAR_PRIVATE_CANARY" },
        template_slug: "TEMPLATE_PRIVATE_CANARY",
        error: "ERROR_PRIVATE_CANARY",
      },
    ]);

    expect(result).toEqual({
      content: [
        {
          type: "text",
          text: "Processing jobs:\n\n- job-20260715123456789-4321-0: summarizing — Generating summary",
        },
      ],
      structuredContent: {
        jobs: [
          {
            id: "job-20260715123456789-4321-0",
            state: "summarizing",
            stage: "Generating summary",
          },
        ],
      },
    });
    const serialized = JSON.stringify(result);
    for (const canary of forbiddenCanaries) {
      expect(serialized).not.toContain(canary);
    }
  });

  it("stops projecting jobs at the documented processing-result cap", () => {
    const jobs = Array.from(
      { length: MCP_PROCESSING_JOB_RESULT_MAX + 25 },
      (_, index) => ({
        id: `job-20260715123456789-4321-${index}`,
        state: "queued",
      })
    );
    const result = buildPrivacySafeProcessingJobsResult(jobs);
    expect(result.structuredContent.jobs).toHaveLength(
      MCP_PROCESSING_JOB_RESULT_MAX
    );
    expect(result.content[0].text).not.toContain(
      `job-20260715123456789-4321-${MCP_PROCESSING_JOB_RESULT_MAX}`
    );
  });

  it("drops source-derived fields from both status text and the status resource", () => {
    const rawStatus = {
      recording: false,
      processing: true,
      processing_stage: "Generating summary",
      recording_mode: "meeting",
      processing_job_count: 2,
      processing_title: "TITLE_PRIVATE_CANARY",
      processing_job_id: "OUTPUT_PATH_PRIVATE_CANARY",
      pid: 4321,
      duration_secs: 42,
      wav_path: "/private/AUDIO_PATH_PRIVATE_CANARY.wav",
      error: "ERROR_PRIVATE_CANARY",
    };

    const text = buildPrivacySafeStatusText(rawStatus);
    const resource = buildPrivacySafeStatusResource(rawStatus);
    expect(text).toBe("Processing: Generating summary (2 jobs queued)");
    expect(JSON.parse(resource.contents[0].text)).toEqual({
      schema_version: 1,
      status_available: true,
      recording: false,
      processing: true,
      recording_mode: "meeting",
      processing_stage: "Generating summary",
      processing_job_count: 2,
    });
    const serialized = JSON.stringify({ text, resource });
    for (const canary of forbiddenCanaries) {
      expect(serialized).not.toContain(canary);
    }
  });

  it("fails closed without echoing malformed CLI payloads", () => {
    expect(
      buildPrivacySafeProcessingJobsResult([
        {
          id: "TITLE_PRIVATE_CANARY",
          state: "RAW_STAGE_PRIVATE_CANARY",
          stage: "PRE_CONTEXT_PRIVATE_CANARY",
        },
      ])
    ).toEqual({
      content: [
        {
          type: "text",
          text: "Processing jobs:\n\n- job-1: unknown — Status unavailable",
        },
      ],
      structuredContent: {
        jobs: [{ id: "job-1", state: "unknown", stage: "Status unavailable" }],
      },
    });
    const unavailable = buildPrivacySafeStatusResource("ERROR_PRIVATE_CANARY");
    expect(unavailable.contents[0].text).not.toContain("ERROR_PRIVATE_CANARY");
    expect(JSON.parse(unavailable.contents[0].text)).toMatchObject({
      status_available: false,
      recording: false,
      processing: false,
    });
  });
});

describe("assistant child and derived-input policy", () => {
  it("compares only Rust and Node Windows canonical path wire spellings", () => {
    const drive = "C:\\Users\\test\\meetings\\normal.md";
    const unc = "\\\\server\\share\\meetings\\normal.md";

    expect(normalizeCanonicalPathWire(`\\\\?\\${drive}`)).toBe(drive);
    expect(
      normalizeCanonicalPathWire("\\\\?\\UNC\\server\\share\\meetings\\normal.md")
    ).toBe(unc);
    expect(canonicalPathWireEquals(`\\\\?\\${drive}`, drive)).toBe(true);
    expect(
      canonicalPathWireEquals(
        "\\\\?\\UNC\\server\\share\\meetings\\normal.md",
        unc
      )
    ).toBe(true);

    // Keep every non-namespace distinction exact. This is not general Windows
    // path normalization and cannot authorize case, separator, dot-segment,
    // trailing-separator, device-namespace, or relative-path differences.
    expect(
      canonicalPathWireEquals(`\\\\?\\${drive}`, drive.toLowerCase())
    ).toBe(false);
    expect(
      canonicalPathWireEquals(`\\\\?\\${drive}`, drive.replaceAll("\\", "/"))
    ).toBe(false);
    expect(canonicalPathWireEquals(`${drive}\\`, drive)).toBe(false);
    expect(
      canonicalPathWireEquals(
        "\\\\?\\GLOBALROOT\\Device\\x",
        "GLOBALROOT\\Device\\x"
      )
    ).toBe(false);
    expect(canonicalPathWireEquals("normal.md", drive)).toBe(false);
  });

  it("forces the CLI deny policy after ambient and call-site overrides", () => {
    const previous = process.env.MINUTES_CLI_RESTRICTED_POLICY;
    try {
      process.env.MINUTES_CLI_RESTRICTED_POLICY = "logged-override";
      expect(mcpCliChildEnv().MINUTES_CLI_RESTRICTED_POLICY).toBe("deny");
      expect(
        mcpCliChildEnv({ MINUTES_CLI_RESTRICTED_POLICY: "allow" })
          .MINUTES_CLI_RESTRICTED_POLICY
      ).toBe("deny");
      delete process.env.MINUTES_CLI_RESTRICTED_POLICY;
      expect(mcpCliChildEnv().MINUTES_CLI_RESTRICTED_POLICY).toBe("deny");
    } finally {
      if (previous === undefined) {
        delete process.env.MINUTES_CLI_RESTRICTED_POLICY;
      } else {
        process.env.MINUTES_CLI_RESTRICTED_POLICY = previous;
      }
    }
  });

  it("fails Windows process_audio closed before CLI, validation, reads, or fd retention", async () => {
    const pathCanary = "C:\\Synthetic\\Downloads\\PRIVATE-AUDIO-PATH-CANARY.wav";
    let cliChecks = 0;
    let executions = 0;
    const result = await handleMcpProcessAudioRequest(
      { file_path: pathCanary, type: "memo" },
      {
        isCliAvailable: async () => {
          cliChecks += 1;
          throw new Error("CLI availability must not be inspected");
        },
        execute: async () => {
          executions += 1;
          throw new Error("validation/read/fd retention must not execute");
        },
      },
      "win32"
    );

    expect(cliChecks).toBe(0);
    expect(executions).toBe(0);
    expect(result.isError).toBe(true);
    expect(result.structuredContent).toEqual({
      available: false,
      error: "windows-agent-audio-fd-unavailable",
    });
    expect(JSON.stringify(result)).not.toContain(pathCanary);
    expect(JSON.stringify(result)).toMatch(/No audio was read or passed/i);
    expect(mcpProcessAudioPlatformPolicy("darwin")).toEqual({ available: true });
    expect(mcpProcessAudioPlatformPolicy("linux")).toEqual({ available: true });
    expect(mcpProcessAudioPlatformPolicy("freebsd")).toMatchObject({
      available: false,
      error: expect.stringMatching(/only on macOS and Linux/i),
    });
  });

  it("returns an honest structured error when the CLI is unavailable", async () => {
    const pathCanary = "/synthetic/PRIVATE-CLI-UNAVAILABLE-PATH.wav";
    let executions = 0;
    const result = await handleMcpProcessAudioRequest(
      { file_path: pathCanary, type: "memo" },
      {
        isCliAvailable: async () => false,
        execute: async () => {
          executions += 1;
          throw new Error("must not execute");
        },
      },
      "linux"
    );

    expect(executions).toBe(0);
    expect(result.isError).toBe(true);
    expect(result.structuredContent).toEqual({
      available: false,
      error: "cli-unavailable",
    });
    expect(JSON.stringify(result)).not.toContain(pathCanary);
    expect(JSON.stringify(result)).toMatch(/No audio was read or passed/i);
  });

  it("accepts only a complete process_audio success contract", async () => {
    const result = await handleMcpProcessAudioRequest(
      { file_path: "/synthetic/input.wav", type: "meeting" },
      {
        isCliAvailable: async () => true,
        execute: async () => ({
          stdout: JSON.stringify({
            status: "done",
            file: " /synthetic/meeting.md ",
            title: " Synthetic review ",
            words: 42,
          }),
        }),
      },
      "linux"
    );

    expect(result.isError).not.toBe(true);
    expect(result.structuredContent).toEqual({
      available: true,
      status: "done",
      file: "/synthetic/meeting.md",
      title: "Synthetic review",
      words: 42,
    });
    expect(result.content).toEqual([{
      type: "text",
      text: "Processed: /synthetic/meeting.md\nTitle: Synthetic review\nWords: 42",
    }]);
  });

  it("bounds held CLI readiness work before any excess availability check", async () => {
    let availabilityChecks = 0;
    let executions = 0;
    let announceFull!: () => void;
    const full = new Promise<void>((resolve) => (announceFull = resolve));
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const dependencies = {
      isCliAvailable: async () => {
        availabilityChecks += 1;
        if (availabilityChecks === MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS) {
          announceFull();
        }
        await held;
        return true;
      },
      execute: async () => {
        executions += 1;
        return {
          stdout: JSON.stringify({
            status: "done",
            file: "/synthetic/readiness.md",
            title: "Readiness bounded",
            words: 1,
          }),
        };
      },
    };
    const active = Array.from(
      { length: MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS },
      (_, index) =>
        handleMcpProcessAudioRequest(
          { file_path: `/synthetic/readiness-${index}.wav`, type: "memo" },
          dependencies,
          "linux"
        )
    );

    try {
      await full;
      const overflow = await handleMcpProcessAudioRequest(
        { file_path: "/synthetic/readiness-overflow.wav", type: "memo" },
        dependencies,
        "linux"
      );
      expect(overflow.isError).toBe(true);
      expect(overflow.structuredContent).toEqual({
        available: false,
        error: "processing-failed",
      });
      expect(availabilityChecks).toBe(MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS);
      expect(executions).toBe(0);
    } finally {
      release();
    }

    const settled = await Promise.all(active);
    expect(settled.every((result) => result.isError !== true)).toBe(true);
    expect(executions).toBe(MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS);
    const recovery = await handleMcpProcessAudioRequest(
      { file_path: "/synthetic/readiness-recovery.wav", type: "memo" },
      {
        isCliAvailable: async () => true,
        execute: async () => ({
          stdout: JSON.stringify({
            status: "done",
            file: "/synthetic/readiness-recovered.md",
            title: "Readiness recovered",
            words: 2,
          }),
        }),
      },
      "linux"
    );
    expect(recovery.isError).not.toBe(true);
  });

  it("fails excess active process_audio jobs immediately and recovers admission", async () => {
    let executions = 0;
    let announceFull!: () => void;
    const full = new Promise<void>((resolve) => (announceFull = resolve));
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const dependencies = {
      isCliAvailable: async () => true,
      execute: async () => {
        executions += 1;
        if (executions === MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS) announceFull();
        await held;
        return {
          stdout: JSON.stringify({
            status: "done",
            file: "/synthetic/meeting.md",
            title: "Bounded job",
            words: 1,
          }),
        };
      },
    };
    const active = Array.from(
      { length: MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS },
      (_, index) =>
        handleMcpProcessAudioRequest(
          { file_path: `/synthetic/held-${index}.wav`, type: "memo" },
          dependencies,
          "linux"
        )
    );

    try {
      await full;
      const overflow = await handleMcpProcessAudioRequest(
        { file_path: "/synthetic/overflow.wav", type: "memo" },
        dependencies,
        "linux"
      );
      expect(overflow.isError).toBe(true);
      expect(overflow.structuredContent).toEqual({
        available: false,
        error: "processing-failed",
      });
      expect(executions).toBe(MCP_PROCESS_AUDIO_MAX_ACTIVE_JOBS);
    } finally {
      release();
    }

    const settled = await Promise.all(active);
    expect(settled.every((result) => result.isError !== true)).toBe(true);
    const recovery = await handleMcpProcessAudioRequest(
      { file_path: "/synthetic/recovery.wav", type: "memo" },
      {
        isCliAvailable: async () => true,
        execute: async () => ({
          stdout: JSON.stringify({
            status: "done",
            file: "/synthetic/recovered.md",
            title: "Recovered job",
            words: 2,
          }),
        }),
      },
      "linux"
    );
    expect(recovery.isError).not.toBe(true);
  });

  it("rejects malformed and contract-invalid CLI output without echoing it", async () => {
    const stdoutCanary = "PRIVATE-MALFORMED-STDOUT-CANARY";
    const invalidPayloads: string[] = [
      `not-json-${stdoutCanary}`,
      "null",
      "[]",
      JSON.stringify({ status: "pending", file: stdoutCanary, title: "Title", words: 1 }),
      JSON.stringify({ status: "done", file: "", title: "Title", words: 1, canary: stdoutCanary }),
      JSON.stringify({ status: "done", file: "/synthetic/out.md", title: "", words: 1, canary: stdoutCanary }),
      JSON.stringify({ status: "done", file: "/synthetic/out.md", title: "Title", words: -1, canary: stdoutCanary }),
      JSON.stringify({ status: "done", file: "/synthetic/out.md", title: "Title", words: 1.5, canary: stdoutCanary }),
      JSON.stringify({ status: "done", file: "/synthetic/out.md", title: "Title", words: Number.MAX_SAFE_INTEGER + 1, canary: stdoutCanary }),
      JSON.stringify({ status: "done", file: "/synthetic/out.md", title: "Title", words: "1", canary: stdoutCanary }),
    ];

    for (const stdout of invalidPayloads) {
      const result = await handleMcpProcessAudioRequest(
        { file_path: "/synthetic/input.wav", type: "memo" },
        {
          isCliAvailable: async () => true,
          execute: async () => ({ stdout }),
        },
        "linux"
      );
      expect(result.isError).toBe(true);
      expect(result.structuredContent).toEqual({
        available: false,
        error: "invalid-cli-response",
      });
      const serialized = JSON.stringify(result);
      expect(serialized).not.toContain(stdoutCanary);
      expect(serialized).not.toContain(stdout);
    }
  });

  it("redacts availability, authorization, and execution exceptions", async () => {
    const exceptionCanary = "/synthetic/PRIVATE-EXCEPTION-PATH-CANARY.wav";
    const cases = [
      {
        isCliAvailable: async () => {
          throw new Error(`availability failed at ${exceptionCanary}`);
        },
        execute: async () => ({ stdout: "must-not-run" }),
      },
      {
        isCliAvailable: async () => true,
        execute: async () => {
          throw new Error(`authorization/execution failed at ${exceptionCanary}`);
        },
      },
    ];

    for (const dependencies of cases) {
      const result = await handleMcpProcessAudioRequest(
        { file_path: exceptionCanary, type: "memo" },
        dependencies,
        "linux"
      );
      expect(result.isError).toBe(true);
      expect(result.structuredContent).toEqual({
        available: false,
        error: "processing-failed",
      });
      expect(JSON.stringify(result)).not.toContain(exceptionCanary);
    }
  });

  it("rejects every retained output-root descendant even when roots overlap", () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-process-audio-policy-"));
    try {
      const inbox = join(root, "inbox");
      const downloads = join(root, "downloads");
      const meetings = join(downloads, "meetings");
      mkdirSync(inbox);
      mkdirSync(downloads);
      mkdirSync(meetings);
      const inboxAudio = join(inbox, "new.wav");
      const retained = join(meetings, "private.voice.wav");
      writeFileSync(inboxAudio, "audio");
      writeFileSync(retained, "restricted audio");
      const extensions = [".wav"];

      expect(
        validateMcpProcessAudioInput(
          inboxAudio,
          [inbox, downloads],
          meetings,
          extensions
        )
      ).toBe(realpathSync(inboxAudio));
      expect(isPathWithinCanonicalRoot(retained, meetings)).toBe(true);
      expect(() =>
        validateMcpProcessAudioInput(
          retained,
          [inbox, downloads],
          meetings,
          extensions
        )
      ).toThrow(/retained meeting audio/i);
      expect(() =>
        validateMcpProcessAudioInput(
          retained,
          [inbox, downloads],
          downloads,
          extensions
        )
      ).toThrow(/retained meeting audio/i);

      if (process.platform !== "win32") {
        const alias = join(inbox, "alias.wav");
        symlinkSync(retained, alias);
        expect(() =>
          validateMcpProcessAudioInput(
            alias,
            [inbox, downloads],
            meetings,
            extensions
          )
        ).toThrow(/access denied|retained meeting audio/i);
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  function processAudioFixture(content = "synthetic-audio-bytes") {
    const root = mkdtempSync(join(tmpdir(), "minutes-authorized-fd-"));
    const inbox = join(root, "inbox");
    const meetings = join(root, "meetings");
    mkdirSync(inbox);
    mkdirSync(meetings);
    const source = join(inbox, "synthetic-input.wav");
    writeFileSync(source, content);
    return { root, inbox, meetings, source, content };
  }

  function writeProcessAudioFdChild(root: string): string {
    const childPath = join(root, "synthetic-fd-child.cjs");
    writeFileSync(
      childPath,
      [
        "#!/usr/bin/env node",
        "const fs = require('node:fs');",
        "const crypto = require('node:crypto');",
        "const childProcess = require('node:child_process');",
        "const mode = process.env.MINUTES_FD_CHILD_MODE || 'success';",
        "if (mode === 'timeout') { setInterval(() => {}, 1000); }",
        "else if (mode === 'descendant') {",
        "  const descendant = childProcess.spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });",
        "  fs.writeFileSync(process.env.MINUTES_DESCENDANT_PID_FILE, String(descendant.pid));",
        "  setInterval(() => {}, 1000);",
        "}",
        "else if (mode === 'stdout') { process.stdout.write('S'.repeat(4096)); setInterval(() => {}, 1000); }",
        "else if (mode === 'stderr') { process.stderr.write('E'.repeat(4096)); setInterval(() => {}, 1000); }",
        "else {",
        "  const bytes = fs.readFileSync(3);",
        "  process.stdout.write(JSON.stringify({",
        "    argv: process.argv.slice(2),",
        "    bytes: bytes.length,",
        "    sha256: crypto.createHash('sha256').update(bytes).digest('hex'),",
        "    restrictedPolicy: process.env.MINUTES_CLI_RESTRICTED_POLICY,",
        "    outerProcessGroup: process.env.MINUTES_MCP_OUTER_PROCESS_GROUP",
        "  }));",
        "}",
      ].join("\n"),
      { mode: 0o700 }
    );
    chmodSync(childPath, 0o700);
    return childPath;
  }

  it("retains one exact source fd at offset zero without named staging or registry state", async () => {
    const fixture = processAudioFixture("synthetic-offset-proof");
    let retainedFd = -1;
    try {
      const beforeInbox = readdirSync(fixture.inbox);
      const result = await withAuthorizedMcpProcessAudioInput(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        fixture.source,
        async (authorized) => {
          retainedFd = authorized.fd;
          const invalidInputs: AuthorizedMcpProcessAudioInput[] = [
            { ...authorized, fd: 1.5 },
            { ...authorized, digest: { byteLength: -1 } },
            {
              ...authorized,
              digest: {
                ...authorized.digest,
                byteLength: authorized.digest.byteLength + 1,
              },
            },
            { ...authorized, format: "wav/path" },
            { ...authorized, format: "m4a" },
            { ...authorized, safeTitle: "synthetic/path" },
          ];
          for (const invalid of invalidInputs) {
            expect(() => buildMcpProcessAudioArgs(invalid, "memo")).toThrow(
              /capability is invalid/i
            );
          }
          expect(() =>
            buildMcpProcessAudioArgs(
              authorized,
              "other" as "memo",
              "../private"
            )
          ).toThrow(/arguments are invalid/i);
          const first = Buffer.alloc(1);
          // A non-positional read here proves authorization metadata checks
          // left the shared file description at offset zero.
          expect(readSync(authorized.fd, first, 0, 1, null)).toBe(1);
          const args = buildMcpProcessAudioArgs(authorized, "memo", "en");
          return { first: first.toString("utf8"), authorized, args };
        }
      );

      expect(result.first).toBe(fixture.content[0]);
      expect(result.authorized.digest).toEqual({
        byteLength: Buffer.byteLength(fixture.content),
      });
      expect(result.authorized.format).toBe("wav");
      expect(result.authorized.safeTitle).toBe("synthetic-input");
      expect(result.args[1]).toBe("authorized-input.wav");
      expect(result.args).toContain("--authorized-input-fd");
      expect(result.args[result.args.indexOf("--authorized-input-fd") + 1]).toBe(
        "3"
      );
      expect(result.args.join(" ")).not.toContain(fixture.source);
      expect(readdirSync(fixture.inbox)).toEqual(beforeInbox);
      expect(() => fstatSync(retainedFd)).toThrow();

      let leakFd = -1;
      const leakFailure = await withAuthorizedMcpProcessAudioInput(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        undefined,
        async (authorized) => {
          leakFd = authorized.fd;
          return { stdout: fixture.source, stderr: "" };
        }
      ).then(
        () => "unexpected-success",
        (error) => String(error)
      );
      expect(leakFailure).toMatch(/result exposed its source/i);
      expect(leakFailure).not.toContain(fixture.source);
      expect(() => fstatSync(leakFd)).toThrow();

      const implementation = readFileSync(
        new URL("./index.ts", import.meta.url),
        "utf8"
      );
      expect(implementation).not.toMatch(
        /\.minutes-mcp-process-inputs|mcp-process-audio-reservations-v1|stageMcpProcessAudioInput/
      );
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("rejects compressed agent audio before the operation receives a capability", async () => {
    const fixture = processAudioFixture("synthetic-compressed-container");
    const compressed = join(fixture.inbox, "synthetic-input.m4a");
    renameSync(fixture.source, compressed);
    let operations = 0;
    try {
      await expect(
        withAuthorizedMcpProcessAudioInput(
          compressed,
          [fixture.inbox],
          [".m4a"],
          async () => fixture.meetings,
          undefined,
          async () => {
            operations += 1;
            return { unexpected: true };
          }
        )
      ).rejects.toThrow(/bounded WAV input only/i);
      expect(operations).toBe(0);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("inherits only the authorized input as fd 3 with synthetic argv, exact proof, and deny-last env", async () => {
    const fixture = processAudioFixture("synthetic-child-proof");
    const childPath = writeProcessAudioFdChild(fixture.root);
    let retainedFd = -1;
    try {
      const result = await withAuthorizedMcpProcessAudioInput(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        "Synthetic review",
        async (authorized) => {
          retainedFd = authorized.fd;
          return runMcpProcessAudioCli(authorized, "meeting", "en", {
            binary: childPath,
            extraEnv: {
              MINUTES_CLI_RESTRICTED_POLICY: "allow",
              MINUTES_MCP_OUTER_PROCESS_GROUP: "0",
              MINUTES_FD_CHILD_MODE: "success",
            },
          });
        }
      );
      const payload = JSON.parse(result.stdout);
      const args = payload.argv as string[];
      expect(args.slice(0, 2)).toEqual(["process", "authorized-input.wav"]);
      expect(args[args.indexOf("--authorized-input-fd") + 1]).toBe("3");
      expect(args[args.indexOf("--authorized-input-format") + 1]).toBe("wav");
      expect(args[args.indexOf("--authorized-input-bytes") + 1]).toBe(
        String(Buffer.byteLength(fixture.content))
      );
      expect(payload.sha256).toBe(
        createHash("sha256").update(fixture.content).digest("hex")
      );
      expect(payload.bytes).toBe(Buffer.byteLength(fixture.content));
      expect(payload.restrictedPolicy).toBe("deny");
      expect(payload.outerProcessGroup).toBeUndefined();
      expect(JSON.stringify(payload)).not.toContain(fixture.source);
      expect(result.stderr).toBe("");
      expect(() => fstatSync(retainedFd)).toThrow();
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("isolates authorization and inherits only exact fd 3 with path-free argv", async () => {
    if (process.platform !== "linux" && process.platform !== "darwin") return;
    const fixture = processAudioFixture("synthetic-isolated-helper");
    const childPath = writeProcessAudioFdChild(fixture.root);
    try {
      const result = await runIsolatedMcpProcessAudio(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        "Synthetic isolated review",
        "meeting",
        "en",
        {
          binary: childPath,
          extraEnv: {
            MINUTES_CLI_RESTRICTED_POLICY: "allow",
            MINUTES_MCP_OUTER_PROCESS_GROUP: "1",
            MINUTES_FD_CHILD_MODE: "success",
          },
        }
      );
      const payload = JSON.parse(result.stdout);
      expect(payload.argv.slice(0, 2)).toEqual([
        "process",
        "authorized-input.wav",
      ]);
      expect(payload.argv.join(" ")).not.toContain(fixture.source);
      expect(payload.bytes).toBe(Buffer.byteLength(fixture.content));
      expect(payload.sha256).toBe(
        createHash("sha256").update(fixture.content).digest("hex")
      );
      expect(payload.restrictedPolicy).toBe("deny");
      expect(payload.outerProcessGroup).toBeUndefined();
      expect(JSON.stringify(result)).not.toContain(fixture.source);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("isolated authorization rejects a post-open source replacement", async () => {
    if (process.platform !== "linux" && process.platform !== "darwin") return;
    const fixture = processAudioFixture("synthetic-isolated-race");
    const childPath = writeProcessAudioFdChild(fixture.root);
    try {
      let replacementRan = false;
      const failure = await runIsolatedMcpProcessAudio(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        undefined,
        "memo",
        undefined,
        { binary: childPath },
        {
          afterValidation: () => {
            replacementRan = true;
            renameSync(fixture.source, join(fixture.inbox, "displaced.wav"));
            writeFileSync(fixture.source, "synthetic-replacement");
          },
        }
      ).then(
        () => "unexpected-success",
        (error) => String(error)
      );
      expect(replacementRan).toBe(true);
      expect(failure).toMatch(/failed safely/i);
      expect(failure).not.toContain(fixture.source);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("fails source replacement, hard-linking, and mutation closed and closes every retained fd", async () => {
    for (const race of ["replace", "hardlink", "mutate"] as const) {
      const fixture = processAudioFixture("synthetic-race-proof");
      let retainedFd = -1;
      let operations = 0;
      try {
        const failure = await withAuthorizedMcpProcessAudioInput(
          fixture.source,
          [fixture.inbox],
          [".wav"],
          async () => fixture.meetings,
          undefined,
          async () => {
            operations += 1;
            return "must-not-run";
          },
          {
            onRetainedFd: (fd) => {
              retainedFd = fd;
            },
            afterHash: () => {
              if (race === "replace") {
                renameSync(fixture.source, join(fixture.inbox, "displaced.wav"));
                writeFileSync(fixture.source, "synthetic-replacement");
              } else if (race === "hardlink") {
                linkSync(fixture.source, join(fixture.inbox, "alias.wav"));
              } else {
                appendFileSync(fixture.source, "-changed");
              }
            },
          }
        ).then(
          () => "unexpected-success",
          (error) => String(error)
        );
        expect(failure).toMatch(/access denied/i);
        expect(failure).not.toContain(fixture.source);
        expect(operations).toBe(0);
        expect(() => fstatSync(retainedFd)).toThrow();
      } finally {
        rmSync(fixture.root, { recursive: true, force: true });
      }
    }
  });

  it("bounds each input and aggregate retained capability admission, then recovers", async () => {
    const fixture = processAudioFixture("12345678");
    const second = join(fixture.inbox, "second.wav");
    writeFileSync(second, "abcdefgh");
    let releaseHeld!: () => void;
    let announceHeld!: () => void;
    const held = new Promise<void>((resolve) => (releaseHeld = resolve));
    const announced = new Promise<void>((resolve) => (announceHeld = resolve));
    const hooks = { maxBytes: 8, maxAggregateBytes: 8 };
    try {
      const active = withAuthorizedMcpProcessAudioInput(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        undefined,
        async () => {
          announceHeld();
          await held;
          return "done";
        },
        hooks
      );
      await announced;
      await expect(
        withAuthorizedMcpProcessAudioInput(
          second,
          [fixture.inbox],
          [".wav"],
          async () => fixture.meetings,
          undefined,
          async () => "must-not-run",
          hooks
        )
      ).rejects.toThrow(/resource budget/i);
      releaseHeld();
      await expect(active).resolves.toBe("done");

      await expect(
        withAuthorizedMcpProcessAudioInput(
          second,
          [fixture.inbox],
          [".wav"],
          async () => fixture.meetings,
          undefined,
          async () => "recovered",
          hooks
        )
      ).resolves.toBe("recovered");
      appendFileSync(second, "x");
      await expect(
        withAuthorizedMcpProcessAudioInput(
          second,
          [fixture.inbox],
          [".wav"],
          async () => fixture.meetings,
          undefined,
          async () => "must-not-run",
          hooks
        )
      ).rejects.toThrow(/resource budget/i);
    } finally {
      releaseHeld?.();
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("re-attests the live meeting root and its identity immediately before dispatch", async () => {
    const fixture = processAudioFixture();
    const alternate = join(fixture.root, "alternate-meetings");
    mkdirSync(alternate);
    try {
      let calls = 0;
      let operations = 0;
      await expect(
        withAuthorizedMcpProcessAudioInput(
          fixture.source,
          [fixture.inbox],
          [".wav"],
          async () => (++calls === 1 ? fixture.meetings : alternate),
          undefined,
          async () => {
            operations += 1;
          }
        )
      ).rejects.toThrow(/meeting root changed/i);
      expect(operations).toBe(0);

      calls = 0;
      const displaced = join(fixture.root, "displaced-meetings");
      await expect(
        withAuthorizedMcpProcessAudioInput(
          fixture.source,
          [fixture.inbox],
          [".wav"],
          async () => {
            calls += 1;
            return fixture.meetings;
          },
          undefined,
          async () => {
            operations += 1;
          },
          {
            beforeFinalAttestation: () => {
              renameSync(fixture.meetings, displaced);
              mkdirSync(fixture.meetings);
            },
          }
        )
      ).rejects.toThrow(/meeting root changed/i);
      expect(calls).toBe(2);
      expect(operations).toBe(0);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("kills timeout and oversized-output children with path-free errors and closes the parent fd", async () => {
    const fixture = processAudioFixture("synthetic-bounded-child");
    const childPath = writeProcessAudioFdChild(fixture.root);
    try {
      for (const mode of ["timeout", "stdout", "stderr"] as const) {
        let retainedFd = -1;
        const failure = await withAuthorizedMcpProcessAudioInput(
          fixture.source,
          [fixture.inbox],
          [".wav"],
          async () => fixture.meetings,
          undefined,
          async (authorized) => {
            retainedFd = authorized.fd;
            return runMcpProcessAudioCli(authorized, "memo", undefined, {
              binary: childPath,
              timeoutMs: mode === "timeout" ? 50 : 2_000,
              maxStdoutBytes: 64,
              maxStderrBytes: 64,
              extraEnv: { MINUTES_FD_CHILD_MODE: mode },
            });
          }
        ).then(
          () => "unexpected-success",
          (error) => String(error)
        );
        expect(failure).toMatch(
          mode === "timeout" ? /time budget/i : /byte budget/i
        );
        expect(failure).not.toContain(fixture.source);
        expect(() => fstatSync(retainedFd)).toThrow();
      }

      let retainedFd = -1;
      const spawnFailure = await withAuthorizedMcpProcessAudioInput(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        undefined,
        async (authorized) => {
          retainedFd = authorized.fd;
          return runMcpProcessAudioCli(authorized, "memo", undefined, {
            binary: join(fixture.root, "missing-binary"),
          });
        }
      ).then(
        () => "unexpected-success",
        (error) => String(error)
      );
      expect(spawnFailure).toMatch(/could not be started safely/i);
      expect(spawnFailure).not.toContain(fixture.source);
      expect(() => fstatSync(retainedFd)).toThrow();

      const descendantPidFile = join(fixture.root, "synthetic-descendant.pid");
      const descendantFailure = await withAuthorizedMcpProcessAudioInput(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        undefined,
        (authorized) =>
          runMcpProcessAudioCli(authorized, "memo", undefined, {
            binary: childPath,
            timeoutMs: 200,
            extraEnv: {
              MINUTES_FD_CHILD_MODE: "descendant",
              MINUTES_DESCENDANT_PID_FILE: descendantPidFile,
            },
          })
      ).then(
        () => "unexpected-success",
        (error) => String(error)
      );
      expect(descendantFailure).toMatch(/time budget/i);
      expect(existsSync(descendantPidFile)).toBe(true);
      const descendantPid = Number.parseInt(
        readFileSync(descendantPidFile, "utf8"),
        10
      );
      let descendantAlive = true;
      for (let attempt = 0; attempt < 20 && descendantAlive; attempt += 1) {
        try {
          process.kill(descendantPid, 0);
          await new Promise((resolve) => setTimeout(resolve, 10));
        } catch {
          descendantAlive = false;
        }
      }
      expect(descendantAlive).toBe(false);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  it("settles without a helper close event, bounds the tree, and poisons further audio after a forced kill", async () => {
    if (process.platform !== "linux" && process.platform !== "darwin") return;
    const fixture = processAudioFixture("synthetic-isolated-timeout");
    const childPath = writeProcessAudioFdChild(fixture.root);
    const descendantPidFile = join(fixture.root, "isolated-descendant.pid");
    try {
      let eventLoopTicked = false;
      const tick = setTimeout(() => {
        eventLoopTicked = true;
      }, 10);
      const startedAt = Date.now();
      const failure = await runIsolatedMcpProcessAudio(
        fixture.source,
        [fixture.inbox],
        [".wav"],
        async () => fixture.meetings,
        undefined,
        "memo",
        undefined,
        {
          binary: childPath,
          timeoutMs: 100,
          extraEnv: {
            MINUTES_FD_CHILD_MODE: "descendant",
            MINUTES_DESCENDANT_PID_FILE: descendantPidFile,
          },
        },
        { ignoreHelperCloseForTest: true }
      ).then(
        () => "unexpected-success",
        (error) => String(error)
      );
      clearTimeout(tick);
      expect(eventLoopTicked).toBe(true);
      expect(Date.now() - startedAt).toBeLessThan(2_000);
      expect(failure).toMatch(/time budget/i);
      expect(failure).not.toContain(fixture.source);

      if (existsSync(descendantPidFile)) {
        const descendantPid = Number.parseInt(
          readFileSync(descendantPidFile, "utf8"),
          10
        );
        let alive = true;
        for (let attempt = 0; attempt < 30 && alive; attempt += 1) {
          try {
            process.kill(descendantPid, 0);
            await new Promise((resolve) => setTimeout(resolve, 10));
          } catch {
            alive = false;
          }
        }
        expect(alive).toBe(false);
      }

      await expect(
        runIsolatedMcpProcessAudio(
          fixture.source,
          [fixture.inbox],
          [".wav"],
          async () => fixture.meetings,
          undefined,
          "memo",
          undefined,
          { binary: childPath }
        )
      ).rejects.toThrow(/requires an MCP restart/i);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });
  it("holds a normal context source through the final lease fence", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-context-policy-"));
    const source = join(root, "normal.md");
    const content = [
      "---",
      "title: Normal context",
      "type: meeting",
      "date: 2026-07-15T10:00:00Z",
      "duration: 1m",
      "sensitivity: normal",
      "---",
      "",
      "CONTEXT_SOURCE_CANARY",
    ].join("\n");
    writeFileSync(source, content);
    try {
      const result = await withPolicyBoundContextPath(
        source,
        root,
        async (canonicalPath) => ({
          source_authorization: {
            session_id: "session-normal",
            // On Windows this is the exact JSON spelling emitted from Rust's
            // std::fs::canonicalize, checked against Node's realpath spelling.
            path: rustCanonicalPathWire(canonicalPath),
            sha256: createHash("sha256").update(content).digest("hex"),
          },
          value: "safe",
        }),
        (value, sessionId) => ({ value: value.value, sessionId })
      );
      expect(result).toEqual({ value: "safe", sessionId: "session-normal" });
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("omits every private context capability while retaining the exact authorized artifact", () => {
    const source = "/synthetic/meetings/authorized.md";
    const links = assistantSafeContextLinks(
      [
        { session_id: "synthetic-session", kind: "job", target: "job-safe" },
        {
          session_id: "synthetic-session",
          kind: "markdown-artifact",
          target: source,
        },
        {
          session_id: "synthetic-session",
          kind: "audio-capture",
          target: "/private/PRIVATE_AUDIO_CANARY.wav",
        },
        {
          session_id: "synthetic-session",
          kind: "screenshot-directory",
          target: "/private/PRIVATE_SCREEN_CANARY",
        },
        {
          session_id: "synthetic-session",
          kind: "markdown-artifact",
          target: "/synthetic/meetings/sibling.md",
        },
      ],
      source
    );
    expect(links).toHaveLength(1);
    const rendered = JSON.stringify({ links, view: "context" });
    expect(rendered).not.toContain("job-safe");
    expect(rendered).toContain(source);
    expect(rendered).not.toContain("PRIVATE_AUDIO_CANARY");
    expect(rendered).not.toContain("PRIVATE_SCREEN_CANARY");
    expect(rendered).not.toContain("sibling.md");
  });

  it("rejects a stale capture revision after same-path replacement", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-context-policy-revision-"));
    const source = join(root, "meeting.md");
    const restrictedAtLink = [
      "---",
      "title: Restricted at link time",
      "type: meeting",
      "date: 2026-07-15T10:00:00Z",
      "duration: 1m",
      "sensitivity: restricted",
      "---",
      "",
      "RESTRICTED_LINK_REVISION",
    ].join("\n");
    const normalReplacement = restrictedAtLink
      .replace("Restricted at link time", "Normal replacement")
      .replace("sensitivity: restricted", "sensitivity: normal")
      .replace("RESTRICTED_LINK_REVISION", "NORMAL_REPLACEMENT_REVISION");
    writeFileSync(source, normalReplacement);
    try {
      await expect(
        withPolicyBoundContextPath(
          source,
          root,
          async (canonicalPath) => ({
            source_authorization: {
              session_id: "session-stale-revision",
              path: canonicalPath,
              sha256: createHash("sha256").update(restrictedAtLink).digest("hex"),
            },
          }),
          async () => "must-not-return"
        )
      ).rejects.toThrow(/stable meeting corpus authorization failed|authorization no longer matches/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("rejects a normal-to-restricted context transition at the final fence", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-context-policy-race-"));
    const source = join(root, "meeting.md");
    const normal = [
      "---",
      "title: Context race",
      "type: meeting",
      "date: 2026-07-15T10:00:00Z",
      "duration: 1m",
      "sensitivity: normal",
      "---",
      "",
      "NORMAL_CONTEXT_CANARY",
    ].join("\n");
    const restricted = normal
      .replace("sensitivity: normal", "sensitivity: restricted")
      .replace("NORMAL_CONTEXT_CANARY", "RESTRICTED_CONTEXT_CANARY");
    writeFileSync(source, normal);
    try {
      await expect(
        withPolicyBoundContextPath(
          source,
          root,
          async (canonicalPath) => ({
            source_authorization: {
              session_id: "session-race",
              path: canonicalPath,
              sha256: createHash("sha256").update(normal).digest("hex"),
            },
          }),
          async () => "must-not-return",
          {
            beforeFinalManifest: () => {
              writeFileSync(source, restricted);
            },
          }
        )
      ).rejects.toThrow(/access denied/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe("unavailable compatibility tools", () => {
  it("advertises and invokes both names as path-free machine-readable errors", async () => {
    const mcpServer = new McpServer({
      name: "minutes-unavailable-compatibility-test",
      version: "0.0.0",
    });
    registerUnavailableCompatibilityTools(mcpServer);
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client(
      { name: "unavailable-compatibility-client", version: "0.0.0" },
      { capabilities: {} }
    );
    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      const listed = await client.listTools();
      const descriptions = new Map(
        listed.tools.map((tool) => [tool.name, tool.description || ""])
      );
      expect(descriptions.get("get_agent_annotations")).toContain(
        MCP_AGENT_ANNOTATIONS_UNAVAILABLE_DESCRIPTION
      );
      expect(descriptions.get("get_meeting_insights")).toContain(
        MCP_MEETING_INSIGHTS_UNAVAILABLE_DESCRIPTION
      );

      const pathCanary = "/synthetic/PRIVATE-ANNOTATION-PATH-CANARY.md";
      const participantCanary = "PRIVATE-PARTICIPANT-CANARY";
      const annotations = await client.callTool({
        name: "get_agent_annotations",
        arguments: {
          limit: 7,
          agent_id: "PRIVATE-AGENT-CANARY",
          meeting_id: "PRIVATE-MEETING-CANARY",
          meeting_path: pathCanary,
        },
      });
      const insights = await client.callTool({
        name: "get_meeting_insights",
        arguments: {
          kind: "decision",
          participant: participantCanary,
          since: "2026-01-01",
          limit: 9,
        },
      });

      for (const result of [annotations, insights]) {
        expect(result.isError).toBe(true);
        expect(Object.keys(result.structuredContent || {}).sort()).toEqual([
          "available",
          "error",
        ]);
        expect(result.structuredContent).toMatchObject({
          available: false,
          error: { code: "source-policy-provenance-required" },
        });
        const serialized = JSON.stringify(result);
        expect(serialized).not.toMatch(/"annotations"|"insights"|"count"|"requested"/);
        expect(serialized).not.toContain(pathCanary);
        expect(serialized).not.toContain(participantCanary);
        expect(serialized).toMatch(/unavailable/i);
      }
    } finally {
      await client.close();
      await mcpServer.close();
    }
  });
});

describe("restricted content policy", () => {
  it("keeps restricted exact-read stubs path-free across every MCP result field", () => {
    const parentCanary = "RESTRICTED-MCP-PARENT-CANARY";
    const fileCanary = "RESTRICTED-MCP-FILENAME-CANARY";
    const path = `/synthetic/${parentCanary}/${fileCanary}.md`;
    const meeting = parsePolicyVerifiedMeeting(
      [
        "---",
        "title: Synthetic restricted review",
        "type: meeting",
        "date: 2026-07-15T10:00:00Z",
        "sensitivity: restricted",
        "---",
        "",
        "RESTRICTED-MCP-BODY-CANARY",
      ].join("\n"),
      path
    );
    expect(meeting).not.toBeNull();
    const result = restrictedMeetingStubResult(meeting!);
    expect(Object.keys(result.structuredContent).sort()).toEqual([
      "date",
      "restricted_stub",
      "sensitivity",
      "title",
      "type",
      "view",
    ]);
    expect(Object.keys(result._meta).sort()).toEqual(["ui", "view"]);
    const serialized = JSON.stringify(result);
    expect(serialized).not.toContain(parentCanary);
    expect(serialized).not.toContain(fileCanary);
    expect(serialized).not.toContain("RESTRICTED-MCP-BODY-CANARY");
    expect(serialized).not.toContain("/synthetic/");
  });

  it("keeps the standalone logged override but recognizes native deny mode", () => {
    const records: string[] = [];
    expect(restrictedContentPolicyFromEnv(undefined)).toBe("deny");
    expect(restrictedContentPolicyFromEnv(" DENY ")).toBe("deny");
    expect(restrictedContentPolicyFromEnv("typo")).toBe("deny");
    expect(restrictedContentPolicyFromEnv("logged-override")).toBe(
      "logged-override"
    );
    expect(restrictedContentPolicyFromEnv("logged-override", "win32")).toBe(
      "logged-override"
    );
    expect(() =>
      enforceRestrictedContentPolicy(
        { include_restricted: true, query: "PRIVATE_QUERY_CANARY" },
        "search_meetings",
        "deny"
      )
    ).toThrow(/unavailable/i);
    expect(() =>
      enforceRestrictedContentPolicy(
        { include_restricted: false },
        "search_meetings",
        "deny"
      )
    ).not.toThrow();
    expect(() =>
      enforceRestrictedContentPolicy(
        { include_restricted: true, query: "PRIVATE_QUERY_CANARY" },
        "search_meetings",
        "logged-override",
        "/ignored/by/capability-bridge",
        (_path, line) => records.push(line)
      )
    ).not.toThrow();
    const audit = records.join("")
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line));
    expect(audit).toHaveLength(1);
    expect(audit[0]).toMatchObject({
      event: "sensitivity.override",
      surface: "search_meetings",
      authorization: "operator-launch-policy+tool-argument",
      scope_fields: ["query"],
    });
    expect(audit[0].scope_sha256).toMatch(/^[a-f0-9]{64}$/);
    expect(records.join("")).not.toContain("PRIVATE_QUERY_CANARY");
  });

  it("blocks both runtime registration surfaces before handlers execute", async () => {
    const previousPolicy = process.env.MINUTES_MCP_RESTRICTED_POLICY;
    process.env.MINUTES_MCP_RESTRICTED_POLICY = "deny";

    const exercise = async (kind: "tool" | "app") => {
      const mcpServer = new McpServer({
        name: `minutes-policy-${kind}`,
        version: "0.0.0",
      });
      let handlerExecutions = 0;
      const name = `policy_${kind}`;
      const handler = async () => {
        handlerExecutions += 1;
        return { content: [{ type: "text" as const, text: "handler ran" }] };
      };
      const inputSchema = {
        include_restricted: z.boolean().optional().default(false),
      };

      if (kind === "tool") {
        registerToolWithRestrictedPolicy(
          mcpServer,
          name,
          "Policy test tool",
          inputSchema,
          { readOnlyHint: true },
          handler
        );
      } else {
        registerDocsAppToolWithRestrictedPolicy(
          mcpServer,
          name,
          {
            description: "Policy test app tool",
            inputSchema,
            annotations: { readOnlyHint: true },
            _meta: { ui: { resourceUri: "ui://minutes/policy-test.html" } },
          },
          handler
        );
      }

      const [clientTransport, serverTransport] =
        InMemoryTransport.createLinkedPair();
      const client = new Client(
        { name: `policy-${kind}-client`, version: "0.0.0" },
        { capabilities: {} }
      );
      try {
        await Promise.all([
          mcpServer.connect(serverTransport),
          client.connect(clientTransport),
        ]);
        const denied = await client.callTool({
          name,
          arguments: { include_restricted: true },
        });
        expect(denied.isError).toBe(true);
        expect(JSON.stringify(denied.content)).toMatch(/unavailable/i);
        expect(handlerExecutions).toBe(0);

        const allowed = await client.callTool({
          name,
          arguments: { include_restricted: false },
        });
        expect(allowed.isError).not.toBe(true);
        expect(handlerExecutions).toBe(1);
      } finally {
        await client.close();
        await mcpServer.close();
      }
    };

    try {
      await exercise("tool");
      await exercise("app");
    } finally {
      if (previousPolicy === undefined) {
        delete process.env.MINUTES_MCP_RESTRICTED_POLICY;
      } else {
        process.env.MINUTES_MCP_RESTRICTED_POLICY = previousPolicy;
      }
    }
  });

  it("denies an override when its exact audit writer does not complete", () => {
    for (const failure of ["open", "write", "sync"] as const) {
      const auditDir = mkdtempSync(join(tmpdir(), "minutes-override-audit-io-"));
      const auditPath = join(auditDir, "audit.jsonl");
      let caught: unknown;
      try {
        enforceRestrictedContentPolicy(
          { include_restricted: true, query: "PRIVATE_AUDIT_IO_CANARY" },
          "search_meetings",
          "logged-override",
          auditPath,
          () => {
            throw new Error(`injected ${failure} error`);
          }
        );
      } catch (error) {
        caught = error;
      }
      expect(caught).toBeInstanceOf(Error);
      expect((caught as Error).message).toBe(
        "MCP error -32603: Restricted override denied because its audit record could not be written safely."
      );
      expect((caught as Error).message).not.toContain(auditPath);
      expect((caught as Error).message).not.toContain("PRIVATE_AUDIT_IO_CANARY");
      rmSync(auditDir, { recursive: true, force: true });
    }
  });

  it("bounds each audit record before invoking the native capability bridge", async () => {
    const records: string[] = [];
    const boundedWriter = (_path: string, line: string) => {
      if (Buffer.byteLength(line, "utf8") > 16 * 1024) {
        throw new Error("bounded native bridge refusal");
      }
      records.push(line);
    };
    await Promise.all(
      Array.from({ length: 16 }, (_, index) =>
        Promise.resolve().then(() =>
          enforceRestrictedContentPolicy(
            { include_restricted: true, index },
            "list_meetings",
            "logged-override",
            "/ignored/by/capability-bridge",
            boundedWriter
          )
        )
      )
    );
    const lines = records.join("").trim().split("\n");
    expect(lines).toHaveLength(16);
    expect(lines.every((line) => JSON.parse(line).event === "sensitivity.override"))
      .toBe(true);

    const oversizedField = `field_${"x".repeat(20 * 1024)}`;
    expect(() =>
      enforceRestrictedContentPolicy(
        { include_restricted: true, [oversizedField]: true },
        "list_meetings",
        "logged-override",
        "/ignored/by/capability-bridge",
        boundedWriter
      )
    ).toThrow("Restricted override denied");
  });

  it("enforces positive bounded meeting limits and an independent action cap", async () => {
    expect(normalizeMcpMeetingResultLimit(1)).toBe(1);
    expect(normalizeMcpMeetingResultLimit(MCP_MEETING_RESULT_MAX)).toBe(
      MCP_MEETING_RESULT_MAX
    );
    for (const invalid of [0, -1, 1.5, Number.NaN, MCP_MEETING_RESULT_MAX + 1]) {
      expect(() => normalizeMcpMeetingResultLimit(invalid)).toThrow(/limit must be/i);
    }

    const meetings = [
      {
        path: "/bounded/meeting.md",
        frontmatter: {
          action_items: Array.from(
            { length: MCP_ACTION_RESULT_MAX + 25 },
            (_, index) => ({
              task: `action-${index}`,
              assignee: "owner",
              status: "open",
            })
          ),
        },
      },
    ] as any;
    expect(openActionsFromMeetings(meetings)).toHaveLength(
      MCP_ACTION_RESULT_MAX
    );
    for (const invalid of [0, -1, 1.5, Number.NaN, MCP_ACTION_RESULT_MAX + 1]) {
      expect(() => openActionsFromMeetings(meetings, invalid)).toThrow(
        /open action limit must be/i
      );
    }

    for (const invalid of [
      0,
      -1,
      1.5,
      Number.NaN,
      MCP_POLICY_MEETING_RESULT_MAX + 1,
    ]) {
      await expect(
        policyListMeetings("/does-not-matter", invalid, false)
      ).rejects.toThrow(/policy meeting limit must be/i);
      await expect(
        policySearchMeetings("/does-not-matter", "query", invalid, false)
      ).rejects.toThrow(/policy search limit must be/i);
    }
  });

  it("selects the newest bounded policy corpus before downstream slicing", () => {
    const files = Array.from(
      { length: MCP_POLICY_MEETING_RESULT_MAX + 1 },
      (_, index) => {
        const day = String((index % 28) + 1).padStart(2, "0");
        const year = 2000 + Math.floor(index / 28);
        const path = `/bounded/${String(index).padStart(5, "0")}.md`;
        return {
          path,
          relativePath: `${String(index).padStart(5, "0")}.md`,
          content: `---\ntitle: Meeting ${index}\ntype: meeting\ndate: ${year}-01-${day}T00:00:00Z\nduration: 1m\n---\nbody\n`,
        };
      }
    );
    const snapshots = collectPolicyVerifiedMeetingSnapshots(
      { canonicalRoot: "/bounded", files } as any,
      false
    );

    expect(snapshots).toHaveLength(MCP_POLICY_MEETING_RESULT_MAX);
    expect(snapshots.some((entry) => entry.path.endsWith("05000.md"))).toBe(true);
    expect(snapshots.some((entry) => entry.path.endsWith("00000.md"))).toBe(false);
  });

  it("matches text and structured intents across the full bounded scan before retention", () => {
    const oldTextToken = "SYNTHETIC-OLD-ONLY-TEXT";
    const oldIntentToken = "SYNTHETIC-OLD-ONLY-DECISION";
    const commonToken = "SYNTHETIC-COMMON-SEARCH";
    const meetingFile = (
      index: number,
      options: { restricted?: boolean; oldMatch?: boolean } = {}
    ) => {
      const date = new Date(Date.UTC(2020, 0, index + 1)).toISOString();
      const path = `/bounded-search/${String(index).padStart(5, "0")}.md`;
      return {
        path,
        relativePath: `${String(index).padStart(5, "0")}.md`,
        content: [
          "---",
          `title: Synthetic meeting ${index}`,
          "type: meeting",
          `date: ${date}`,
          `sensitivity: ${options.restricted ? "restricted" : "normal"}`,
          "tags: []",
          "attendees: []",
          "people: []",
          "action_items: []",
          "decisions:",
          `  - text: ${options.oldMatch ? oldIntentToken : `unrelated-${index}`}`,
          "intents: []",
          "---",
          "",
          `${commonToken} ${options.oldMatch ? oldTextToken : "unrelated body"}`,
        ].join("\n"),
      };
    };
    const files = Array.from(
      { length: MCP_POLICY_MEETING_RESULT_MAX + 1 },
      (_, index) => meetingFile(index, { oldMatch: index === 0 })
    );
    files.push({
      ...meetingFile(MCP_POLICY_MEETING_RESULT_MAX + 1, {
        restricted: true,
        oldMatch: true,
      }),
      path: "/bounded-search/restricted-newest.md",
      relativePath: "restricted-newest.md",
    });
    const snapshot = { canonicalRoot: "/bounded-search", files } as any;

    const oldText = collectPolicyToolSearchSnapshots(snapshot, false, {
      query: oldTextToken,
      contentType: "meeting",
      since: "2019-01-01",
    });
    expect(oldText.map((entry) => entry.path)).toEqual([
      "/bounded-search/00000.md",
    ]);

    const oldIntent = collectPolicyToolSearchSnapshots(snapshot, false, {
      query: oldIntentToken,
      intentKind: "decision",
      intentsOnly: true,
      since: "2019-01-01",
    });
    expect(oldIntent.map((entry) => entry.path)).toEqual([
      "/bounded-search/00000.md",
    ]);
    expect(
      policyIntentResults(
        oldIntent.map((entry) => entry.meeting),
        oldIntentToken,
        "decision",
        undefined,
        1
      )
    ).toHaveLength(1);

    const oldOnlySnapshot = {
      canonicalRoot: "/bounded-search",
      files: [files[0]],
    } as any;
    expect(collectPolicyToolSearchSnapshots(oldOnlySnapshot, false, {
      query: oldTextToken,
      contentType: "memo",
    })).toEqual([]);
    expect(collectPolicyToolSearchSnapshots(oldOnlySnapshot, false, {
      query: oldTextToken,
      since: "2021-01-01",
    })).toEqual([]);

    const common = collectPolicyToolSearchSnapshots(snapshot, false, {
      query: commonToken,
    });
    expect(common).toHaveLength(MCP_POLICY_MEETING_RESULT_MAX);
    expect(common[0].path).toBe("/bounded-search/05000.md");
    expect(common.some((entry) => entry.path.endsWith("00000.md"))).toBe(false);
    expect(common.some((entry) => entry.path.includes("restricted"))).toBe(false);
  });

  it("bounds derived profile, intent, research, and relationship collections before output", () => {
    const long = "x".repeat(10_000);
    const baseMeeting = {
      path: `/bounded/${long}.md`,
      body: `Alex ${long}`,
      frontmatter: {
        title: long,
        type: "meeting",
        date: "2026-07-16T12:00:00Z",
        duration: "1m",
        tags: Array.from({ length: 75 }, (_, index) => `topic-${index}-${long}`),
        attendees: ["Alex"],
        attendees_raw: "",
        people: [],
        action_items: Array.from({ length: 75 }, (_, index) => ({
          assignee: "Alex",
          task: `task-${index}-${long}`,
          status: "open",
        })),
        decisions: Array.from({ length: 75 }, (_, index) => ({
          text: `decision-${index}-${long}`,
        })),
        intents: Array.from({ length: 75 }, (_, index) => ({
          kind: "commitment",
          what: `intent-${index}-${long}`,
          who: "Alex",
          status: "open",
        })),
      },
    } as any;
    const meetings = Array.from(
      { length: MCP_PERSON_PROFILE_MEETING_MAX + 25 },
      (_, index) => ({
        ...baseMeeting,
        path: `/bounded/meeting-${String(index).padStart(3, "0")}.md`,
      })
    );

    const profile = personProfileFromMeetings(meetings, "Alex");
    expect(profile.meetings).toHaveLength(MCP_PERSON_PROFILE_MEETING_MAX);
    expect(profile.openActions).toHaveLength(
      MCP_PERSON_PROFILE_OPEN_ACTION_MAX
    );
    expect(profile.topics).toHaveLength(MCP_PERSON_PROFILE_TOPIC_MAX);
    expect(profile.meetings.every((meeting) => meeting.title.length <= 2_048)).toBe(
      true
    );
    expect(profile.openActions.every((action) => action.task.length <= 2_048)).toBe(
      true
    );
    for (const [field, max] of [
      ["meetingLimit", MCP_PERSON_PROFILE_MEETING_MAX],
      ["openActionLimit", MCP_PERSON_PROFILE_OPEN_ACTION_MAX],
      ["topicLimit", MCP_PERSON_PROFILE_TOPIC_MAX],
    ] as const) {
      expect(() =>
        personProfileFromMeetings(meetings, "Alex", { [field]: max + 1 })
      ).toThrow(/person profile .* limit must be/i);
    }

    const intents = policyIntentResults(
      meetings,
      "",
      undefined,
      undefined,
      MCP_INTENT_RESULT_MAX,
      new Set(["open"])
    );
    expect(intents).toHaveLength(MCP_INTENT_RESULT_MAX);
    expect(intents.every((intent) => intent.what.length <= 2_048)).toBe(true);

    const research = researchTopicProjection(meetings, long);
    expect(research.meetings).toHaveLength(MCP_RESEARCH_MEETING_RESULT_MAX);
    expect(research.decisions).toHaveLength(MCP_RESEARCH_DECISION_RESULT_MAX);
    expect(research.openIntents).toHaveLength(MCP_INTENT_RESULT_MAX);
    expect(research.topics).toHaveLength(MCP_RESEARCH_TOPIC_RESULT_MAX);
    expect(research.text.length).toBeLessThanOrEqual(256 * 1024);
    expect(research.decisions.every((decision) => decision.length <= 2_048)).toBe(
      true
    );

    const relationshipMeeting = {
      ...baseMeeting,
      frontmatter: {
        ...baseMeeting.frontmatter,
        attendees: Array.from(
          { length: MCP_RELATIONSHIP_RESULT_MAX + 25 },
          (_, index) => `person-${index}-${long}`
        ),
        action_items: [],
      },
    } as any;
    const relationships = relationshipMapFromMeetings(
      [relationshipMeeting],
      10,
      20
    );
    expect(relationships).toHaveLength(10);
    expect(relationships.every((person) => person.name.length <= 2_048)).toBe(
      true
    );
    for (const invalid of [0, -1, 1.5, MCP_RELATIONSHIP_RESULT_MAX + 1]) {
      expect(() => relationshipMapFromMeetings([relationshipMeeting], invalid)).toThrow(
        /relationship limit must be/i
      );
    }
  });

  it("fails closed on malformed or unknown sensitivity frontmatter", () => {
    const base = [
      "---",
      "title: Policy probe",
      "type: meeting",
      "date: 2026-07-15T10:00:00Z",
      "duration: 1m",
      "SENSITIVITY",
      "---",
      "",
      "canary",
    ].join("\n");
    expect(
      parsePolicyVerifiedMeeting(base.replace("SENSITIVITY", "sensitivity: normal"), "normal.md")
    ).not.toBeNull();
    expect(
      parsePolicyVerifiedMeeting(
        base.replace("SENSITIVITY", "sensitivity: restricted"),
        "restricted.md"
      )?.frontmatter.sensitivity
    ).toBe("restricted");
    expect(
      parsePolicyVerifiedMeeting(
        base.replace("SENSITIVITY", "sensitivity: confidential"),
        "unknown.md"
      )
    ).toBeNull();
    expect(parsePolicyVerifiedMeeting("no frontmatter canary", "bad.md")).toBeNull();
    for (const invalidRequiredField of [
      "title: Policy probe",
      "type: meeting",
      "date: 2026-07-15T10:00:00Z",
    ]) {
      expect(
        parsePolicyVerifiedMeeting(
          base.replace(`${invalidRequiredField}\n`, ""),
          "missing-required.md"
        )
      ).toBeNull();
    }
  });

  it("denies invalid UTF-8 policy bytes across exact, stable, research, and tool reads", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-invalid-utf8-policy-"));
    const invalidPath = join(meetingsDir, "invalid-utf8.md");
    const privateCanary = "INVALID-UTF8-MCP-PRIVATE-CANARY";
    const normalCanary = "INVALID-UTF8-MCP-NORMAL-CANARY";
    const invalidBytes = Buffer.from(
      [
        "---",
        "title: Invalid UTF-8 policy probe",
        "type: meeting",
        "date: 2026-07-15T10:00:00Z",
        "sensitivity: restricted",
        "---",
        "",
        privateCanary,
      ].join("\n")
    );
    const keyOffset = invalidBytes.indexOf(Buffer.from("sensitivity"));
    expect(keyOffset).toBeGreaterThanOrEqual(0);
    invalidBytes[keyOffset + 5] = 0xff;
    writeFileSync(invalidPath, invalidBytes);
    writeFileSync(
      join(meetingsDir, "normal.md"),
      [
        "---",
        "title: Normal policy probe",
        "type: meeting",
        "date: 2026-07-16T10:00:00Z",
        "sensitivity: normal",
        "---",
        "",
        normalCanary,
      ].join("\n")
    );

    const mcpServer = new McpServer({
      name: "minutes-invalid-utf8-policy",
      version: "0.0.0",
    });
    let researchProjectionExecutions = 0;
    registerToolWithRestrictedPolicy(
      mcpServer,
      "invalid_utf8_research",
      "Synthetic research boundary for invalid UTF-8 policy bytes",
      { query: z.string() },
      { readOnlyHint: true },
      async ({ query }) => {
        const meetings = await policyListMeetings(
          meetingsDir,
          MCP_POLICY_MEETING_RESULT_MAX,
          false
        );
        researchProjectionExecutions += 1;
        return {
          content: [
            {
              type: "text" as const,
              text: researchTopicProjection(meetings, query).text,
            },
          ],
        };
      }
    );

    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client(
      { name: "invalid-utf8-policy-client", version: "0.0.0" },
      { capabilities: {} }
    );
    try {
      expect(
        await policyVerifiedExactMeetingSnapshot(invalidPath, meetingsDir, true)
      ).toBeNull();

      const aggregateOutcomes = await Promise.allSettled([
        policyListMeetings(meetingsDir, 10, false),
        policySearchMeetings(meetingsDir, privateCanary, 10, false),
        policyListMeetings(meetingsDir, 10, true),
      ]);
      expect(
        aggregateOutcomes.every((outcome) => outcome.status === "rejected")
      ).toBe(true);
      const aggregateSerialized = JSON.stringify(aggregateOutcomes);
      expect(aggregateSerialized).not.toContain(privateCanary);
      expect(aggregateSerialized).not.toContain(normalCanary);
      expect(aggregateSerialized).not.toContain(invalidPath);

      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      const researchResult = await client.callTool({
        name: "invalid_utf8_research",
        arguments: { query: privateCanary },
      });
      expect(researchResult.isError).toBe(true);
      expect(researchProjectionExecutions).toBe(0);
      const toolSerialized = JSON.stringify(researchResult);
      expect(toolSerialized).not.toContain(privateCanary);
      expect(toolSerialized).not.toContain(normalCanary);
      expect(toolSerialized).not.toContain(invalidPath);
    } finally {
      await client.close().catch(() => {});
      await mcpServer.close().catch(() => {});
      rmSync(meetingsDir, { recursive: true, force: true });
    }
  });

  it("re-verifies installed-SDK list and search candidates from live files", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-installed-sdk-policy-"));
    const meeting = (title: string, sensitivity: string, body: string) =>
      [
        "---",
        `title: ${title}`,
        "type: meeting",
        "date: 2026-07-15T10:00:00Z",
        "duration: 1m",
        ...(sensitivity ? [`sensitivity: ${sensitivity}`] : []),
        "tags: []",
        "attendees: []",
        "people: []",
        "action_items: []",
        "decisions: []",
        "intents: []",
        "---",
        "",
        body,
      ].join("\n");
    writeFileSync(join(meetingsDir, "normal.md"), meeting("Normal", "", "shared canary"));
    writeFileSync(
      join(meetingsDir, "restricted.md"),
      meeting("Restricted", "restricted", "restricted shared canary")
    );
    writeFileSync(
      join(meetingsDir, "unknown.md"),
      meeting("Unknown", "confidential", "UNKNOWN_POLICY_CANARY shared canary")
    );

    try {
      expect(
        (await policyListMeetings(meetingsDir, 10, false)).map(
          (item) => item.frontmatter.title
        )
      ).toEqual(["Normal"]);
      expect(
        (await policySearchMeetings(meetingsDir, "shared canary", 10, false)).map(
          (item) => item.frontmatter.title
        )
      ).toEqual(["Normal"]);
      expect(
        (await policyListMeetings(meetingsDir, 10, true))
          .map((item) => item.frontmatter.title)
          .sort()
      ).toEqual(["Normal", "Restricted"]);
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
    }
  });

  it("orders list, search, type-filter, and research projections by normalized date descending", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-policy-recency-"));
    const meeting = (
      title: string,
      type: "meeting" | "memo",
      date: string,
      body: string
    ) =>
      [
        "---",
        `title: ${title}`,
        `type: ${type}`,
        `date: ${date}`,
        "sensitivity: normal",
        "tags: []",
        "attendees: []",
        "people: []",
        "action_items: []",
        "decisions: []",
        "intents: []",
        "---",
        "",
        body,
      ].join("\n");
    writeFileSync(
      join(meetingsDir, "a-old.md"),
      meeting("Old meeting", "meeting", "2024-01-01T09:00:00-08:00", "shared research topic")
    );
    writeFileSync(
      join(meetingsDir, "b-newest-memo.md"),
      meeting("Newest memo", "memo", "2026-05-01T18:00:00Z", "shared research topic")
    );
    writeFileSync(
      join(meetingsDir, "z-new-meeting.md"),
      meeting("New meeting", "meeting", "2026-04-30T20:00:00-07:00", "shared research topic")
    );
    writeFileSync(
      join(meetingsDir, "c-tie-a.md"),
      meeting("Tie A", "memo", "2025-06-01T12:00:00Z", "shared research topic")
    );
    writeFileSync(
      join(meetingsDir, "d-tie-b.md"),
      meeting("Tie B", "memo", "2025-06-01T12:00:00Z", "shared research topic")
    );

    try {
      expect(
        (await policyListMeetings(meetingsDir, 2, false)).map(
          (item) => item.frontmatter.title
        )
      ).toEqual(["Newest memo", "New meeting"]);
      expect(
        (await policySearchMeetings(meetingsDir, "shared research", 2, false)).map(
          (item) => item.frontmatter.title
        )
      ).toEqual(["Newest memo", "New meeting"]);

      const ordered = await policyListMeetings(meetingsDir, 100, false);
      expect(ordered.slice(2, 4).map((item) => item.frontmatter.title)).toEqual([
        "Tie A",
        "Tie B",
      ]);
      expect(
        ordered
          .filter((item) => item.frontmatter.type === "meeting")
          .slice(0, 1)
          .map((item) => item.frontmatter.title)
      ).toEqual(["New meeting"]);
      expect(
        ordered
          .filter((item) => item.body.includes("shared research topic"))
          .slice(0, 2)
          .map((item) => item.frontmatter.title)
      ).toEqual(["Newest memo", "New meeting"]);
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
    }
  });

  it("rejects inactive corpus components again at the MCP boundary", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-inactive-policy-"));
    const inactivePaths = [
      join(meetingsDir, "Archive", "private.md"),
      join(meetingsDir, ".git", "private.md"),
      join(meetingsDir, "nested", ".private", "private.md"),
    ];
    for (const [index, privatePath] of inactivePaths.entries()) {
      mkdirSync(join(privatePath, ".."), { recursive: true });
      writeFileSync(
        privatePath,
        [
          "---",
          `title: INACTIVE-MCP-CANARY-${index}`,
          "type: meeting",
          "date: 2026-07-15T10:00:00Z",
          "---",
          "",
          `INACTIVE-MCP-CANARY-${index}`,
        ].join("\n")
      );
    }
    try {
      for (const privatePath of inactivePaths) {
        expect(isActiveCorpusMeetingPath(privatePath, meetingsDir)).toBe(false);
      }
      expect(await policyListMeetings(meetingsDir, 100, true)).toEqual([]);
      for (const privatePath of inactivePaths) {
        expect(
          await enrichWithFrontmatter(
            [{ source_path: privatePath, snippet: "INACTIVE-MCP-CANARY" }],
            true,
            meetingsDir
          )
        ).toEqual([]);
      }
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
    }
  });

  it("binds exact meeting reads to the active corpus root", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-exact-policy-"));
    const outsideDir = mkdtempSync(join(tmpdir(), "minutes-exact-outside-"));
    const meeting = (title: string, sensitivity = "") =>
      [
        "---",
        `title: ${title}`,
        "type: meeting",
        "date: 2026-07-15T10:00:00Z",
        ...(sensitivity ? [`sensitivity: ${sensitivity}`] : []),
        "---",
        "",
        `${title} body`,
      ].join("\n");
    const normalPath = join(meetingsDir, "normal.md");
    const restrictedPath = join(meetingsDir, "restricted.md");
    const inactivePath = join(meetingsDir, "Archive", "inactive.md");
    const outsidePath = join(outsideDir, "outside.md");
    writeFileSync(normalPath, meeting("Normal exact"));
    writeFileSync(restrictedPath, meeting("Restricted exact", "restricted"));
    mkdirSync(join(meetingsDir, "Archive"));
    writeFileSync(inactivePath, meeting("Inactive exact"));
    writeFileSync(outsidePath, meeting("Outside exact"));

    try {
      expect(
        (
          await policyVerifiedExactMeetingSnapshot(
            normalPath,
            meetingsDir,
            false
          )
        )?.meeting.frontmatter.title
      ).toBe("Normal exact");
      expect(
        await policyVerifiedExactMeetingSnapshot(
          restrictedPath,
          meetingsDir,
          false
        )
      ).toBeNull();
      expect(
        await policyVerifiedExactMeetingSnapshot(
          inactivePath,
          meetingsDir,
          true
        )
      ).toBeNull();
      expect(
        await policyVerifiedExactMeetingSnapshot(
          outsidePath,
          meetingsDir,
          true
        )
      ).toBeNull();
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
      rmSync(outsideDir, { recursive: true, force: true });
    }
  });

  it("retries a persistent A-to-restricted flip without dropping stable B", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-post-snapshot-policy-"));
    const path = join(meetingsDir, "a.md");
    const meeting = (title: string, sensitivity: string, body: string) =>
      [
        "---",
        `title: ${title}`,
        "type: meeting",
        "date: 2026-07-15T10:00:00Z",
        ...(sensitivity ? [`sensitivity: ${sensitivity}`] : []),
        "---",
        "",
        body,
      ].join("\n");
    writeFileSync(path, meeting("A private", "", "POST-SNAPSHOT-PRIVATE-CANARY"));
    writeFileSync(
      join(meetingsDir, "b.md"),
      meeting("B stable", "", "POST-SNAPSHOT-STABLE-CANARY")
    );
    try {
      let flipped = false;
      const result = await policyListMeetings(meetingsDir, 10, false, () => {
        if (flipped) return;
        flipped = true;
        writeFileSync(
          path,
          meeting("A private", "restricted", "POST-SNAPSHOT-PRIVATE-CANARY")
        );
      });
      expect(result.map((item) => item.frontmatter.title)).toEqual(["B stable"]);
      expect(JSON.stringify(result)).not.toContain("POST-SNAPSHOT-PRIVATE-CANARY");
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
    }
  });

  it("rejects an exact-byte ABA transition instead of trusting restored current state", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-policy-aba-"));
    const path = join(meetingsDir, "mutable.md");
    const normal = [
      "---",
      "title: ABA private",
      "type: meeting",
      "date: 2026-07-15T10:00:00Z",
      "---",
      "",
      "EXACT-ABA-PRIVATE-CANARY",
    ].join("\n");
    const restricted = normal.replace("date: 2026", "sensitivity: restricted\ndate: 2026");
    writeFileSync(path, normal);
    const initial = statSync(path);

    try {
      await expect(
        policySearchMeetings(meetingsDir, "EXACT-ABA", 10, false, () => {}, {
          beforeFinalManifest: () => {
            writeFileSync(path, restricted);
            writeFileSync(path, normal);
            utimesSync(path, initial.atime, initial.mtime);
          },
        })
      ).rejects.toThrow("stable meeting corpus authorization failed");
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
    }
  });

  it("fails closed on watcher failure and sentinel timeout", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-policy-watcher-"));
    writeFileSync(
      join(meetingsDir, "normal.md"),
      "---\ntitle: Watcher\ntype: meeting\ndate: 2026-07-15T10:00:00Z\n---\nwatcher canary"
    );

    try {
      await expect(
        policyListMeetings(meetingsDir, 10, false, () => {}, {
          onWatcherReady: ({ controls }) => controls.failWatcher("test failure"),
        })
      ).rejects.toThrow("stable meeting corpus authorization failed");
      await expect(
        policyListMeetings(meetingsDir, 10, false, () => {}, {
          timeoutMs: 25,
          onWatcherReady: ({ controls }) => controls.suppressNextFence(),
        })
      ).rejects.toThrow("stable meeting corpus authorization failed");
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
    }
  });
});

describe("QMD sensitivity verification", () => {
  it("drops restricted, malformed, unreadable, and out-of-root index hits", async () => {
    const meetingsDir = mkdtempSync(join(tmpdir(), "minutes-qmd-meetings-"));
    const outsideDir = mkdtempSync(join(tmpdir(), "minutes-qmd-outside-"));
    const normalPath = join(meetingsDir, "normal.md");
    const restrictedPath = join(meetingsDir, "restricted.md");
    const unknownPath = join(meetingsDir, "unknown.md");
    const malformedPath = join(meetingsDir, "malformed-yaml.md");
    const outsidePath = join(outsideDir, "outside.md");
    const symlinkPath = join(meetingsDir, "outside-link.md");
    const meeting = (title: string, sensitivity?: string) =>
      [
        "---",
        `title: ${title}`,
        "type: meeting",
        "date: 2026-07-15T10:00:00Z",
        "duration: 10m",
        ...(sensitivity ? [`sensitivity: ${sensitivity}`] : []),
        "tags: []",
        "attendees: []",
        "people: []",
        "action_items: []",
        "decisions: []",
        "intents: []",
        "---",
        "",
        `## Transcript\\n\\n${title} canary`,
      ].join("\n");

    writeFileSync(normalPath, meeting("Normal"));
    writeFileSync(restrictedPath, meeting("Restricted", "restricted"));
    writeFileSync(unknownPath, meeting("Unknown", "confidential"));
    writeFileSync(
      malformedPath,
      "---\ntitle: Broken\nsensitivity: [unterminated\n---\nMALFORMED_YAML_CANARY"
    );
    writeFileSync(outsidePath, meeting("Outside"));
    symlinkSync(outsidePath, symlinkPath);

    try {
      const hits = [
        { source_path: normalPath, snippet: "poisoned stale index canary" },
        { source_path: restrictedPath, snippet: "restricted canary" },
        { source_path: unknownPath, snippet: "unknown canary" },
        { source_path: malformedPath, snippet: "malformed canary" },
        { source_path: outsidePath, snippet: "outside canary" },
        { source_path: symlinkPath, snippet: "symlink canary" },
        { source_path: join(meetingsDir, "missing.md"), snippet: "missing canary" },
      ];
      const filtered = await enrichWithFrontmatter(hits, false, meetingsDir);
      expect(filtered).toHaveLength(1);
      expect(filtered[0]).toMatchObject({
        title: "Normal",
        path: realpathSync(normalPath),
      });
      expect(filtered[0].snippet).toContain("Normal canary");
      expect(filtered[0].snippet).not.toContain("poisoned stale index canary");
      expect(JSON.stringify(filtered)).not.toMatch(
        /restricted|unknown|malformed|outside|symlink|missing canary/i
      );

      const standaloneOverride = await enrichWithFrontmatter(
        hits,
        true,
        meetingsDir
      );
      expect(standaloneOverride.map((hit) => hit.title).sort()).toEqual([
        "Normal",
        "Restricted",
      ]);
    } finally {
      rmSync(meetingsDir, { recursive: true, force: true });
      rmSync(outsideDir, { recursive: true, force: true });
    }
  });

  it("derives snippets only from the verified live body", () => {
    expect(
      liveMeetingSnippet("prefix words unique target and safe suffix", "unique target")
    ).toContain("unique target");
    expect(liveMeetingSnippet("  live\n body  ")).toBe("live body");
  });
});

describe("meeting insight contract", () => {
  it("exports only the insight kinds the pipeline emits today", () => {
    expect(MEETING_INSIGHT_KINDS).toEqual(["decision", "commitment", "question"]);
  });
});

describe("meeting shape contract", () => {
  const meeting = {
    path: "/tmp/meeting.md",
    frontmatter: {
      date: "2026-05-05T10:00:00-07:00",
      title: "Capture Health Review",
      type: "meeting",
      duration: "12m",
      recording_health: {
        capture_warnings: [
          {
            kind: "silent",
            source: "system",
            message: "System audio was silent.",
            diagnostic_confidence: "inferred",
          },
        ],
        diarization_path: "ml-bleed-degraded",
      },
    },
  };

  it("omits recording_health from list and search results", () => {
    expect(meetingListItem(meeting)).toEqual({
      date: "2026-05-05T10:00:00-07:00",
      title: "Capture Health Review",
      content_type: "meeting",
      path: "/tmp/meeting.md",
      duration: "12m",
    });
    expect(meetingSearchItem(meeting)).toEqual({
      date: "2026-05-05T10:00:00-07:00",
      title: "Capture Health Review",
      content_type: "meeting",
      path: "/tmp/meeting.md",
    });
  });

  it("bounds every list/search field before structured output", () => {
    const oversized = "x".repeat(10_000);
    const boundedList = meetingListItem({
      path: oversized,
      frontmatter: {
        date: oversized,
        title: oversized,
        type: oversized,
        duration: oversized,
      },
    });
    const boundedSearch = meetingSearchItem({
      path: oversized,
      frontmatter: { date: oversized, title: oversized, type: oversized },
    });
    for (const value of [
      ...Object.values(boundedList),
      ...Object.values(boundedSearch),
    ]) {
      expect(value?.length).toBeLessThanOrEqual(2_048);
    }
  });

  it("surfaces recording_health in detail payloads", () => {
    expect(
      meetingDetailPayload({
        path: meeting.path,
        speaker_map: [],
        recording_health: meeting.frontmatter.recording_health,
        overlay_applied: false,
      })
    ).toEqual({
      path: "/tmp/meeting.md",
      view: "detail",
      speaker_map: [],
      recording_health: meeting.frontmatter.recording_health,
      overlay_applied: false,
    });
  });

  it("surfaces the transcript body and synthesis fields in detail payloads (issue #255)", () => {
    const actionItems = [{ assignee: "Mat", task: "Ship fix", status: "open" }];
    const decisions = [{ text: "Enrich structuredContent" }];
    const intents = [{ kind: "commitment", what: "Reply to contributor", status: "open" }];

    const payload = meetingDetailPayload({
      path: meeting.path,
      speaker_map: [],
      overlay_applied: false,
      title: "Native Call",
      summary: "We agreed to fix get_meeting.",
      action_items: actionItems,
      decisions,
      intents,
      body: "## Summary\n\nWe agreed to fix get_meeting.\n\n## Transcript\n\n[00:00] Hello.",
    });

    expect(payload).toMatchObject({
      path: "/tmp/meeting.md",
      view: "detail",
      title: "Native Call",
      summary: "We agreed to fix get_meeting.",
      action_items: actionItems,
      decisions,
      intents,
    });
    expect(payload.body).toContain("## Transcript");
  });

  it("omits synthesis fields entirely when not provided", () => {
    expect(meetingDetailPayload({ path: meeting.path })).toEqual({
      path: "/tmp/meeting.md",
      view: "detail",
    });
  });

  it("accepts CLI overlays only with an exact source-bound proof", () => {
    const source = "---\ntitle: Safe\n---\nSPEAKER_0: hello\n";
    const exact = verifiedCliSpeakerOverlay(
      {
        overlay_applied: true,
        overlay_source_sha256: createHash("sha256").update(source).digest("hex"),
        raw_markdown: source,
        frontmatter: {
          speaker_map: [
            {
              speaker_label: "SPEAKER_0",
              name: "Alex",
              confidence: "high",
              source: "manual",
            },
          ],
        },
      },
      source
    );
    expect(exact?.overlay_applied).toBe(true);
    expect(exact?.speaker_map).toHaveLength(1);

    for (const stale of [
      { overlay_source_sha256: "0".repeat(64) },
      { raw_markdown: source.replace("hello", "replacement") },
      { overlay_applied: false },
    ]) {
      expect(
        verifiedCliSpeakerOverlay(
          {
            overlay_applied: true,
            overlay_source_sha256: createHash("sha256").update(source).digest("hex"),
            raw_markdown: source,
            frontmatter: { speaker_map: [{ name: "STALE-PRIVATE-CANARY" }] },
            ...stale,
          },
          source
        )
      ).toBeNull();
    }
  });
});

describe("extractMarkdownSection", () => {
  const body = [
    "## Summary",
    "",
    "First synthesized line.",
    "Second synthesized line.",
    "",
    "## Decisions",
    "",
    "- Ship the fix.",
    "",
    "## Transcript",
    "",
    "[00:00] Hello.",
  ].join("\n");

  it("returns a section's text up to the next heading", () => {
    expect(extractMarkdownSection(body, "Summary")).toBe(
      "First synthesized line.\nSecond synthesized line."
    );
  });

  it("returns undefined for an absent section", () => {
    expect(extractMarkdownSection(body, "Commitments")).toBeUndefined();
  });

  it("returns undefined for empty or missing input", () => {
    expect(extractMarkdownSection(undefined, "Summary")).toBeUndefined();
    expect(extractMarkdownSection("## Summary\n\n", "Summary")).toBeUndefined();
  });
});

describe("verified stop recording responses", () => {
  it("materializes rich output only from the authorized meeting snapshot", () => {
    const summary = verifiedStopRecordingSummary({
      path: "/safe/meetings/authorized.md",
      meeting: {
        body: "## Transcript\n\nAuthorized words only.",
        frontmatter: {
          title: "Authorized title",
          duration: "12m",
          people: ["Alex"],
          action_items: [
            { task: "Ship safely", assignee: "Avery", status: "open" },
          ],
          decisions: [{ text: "Keep the boundary fail-closed" }],
        },
      },
    });

    expect(summary).toContain("Authorized title");
    expect(summary).toContain("/safe/meetings/authorized.md");
    expect(summary).toContain("Ship safely");
    expect(summary).not.toContain("CLI-PRIVATE-CANARY");
    expect(summary).not.toContain("job-private-canary");
  });
});

describe("parseKnowledgeConfig", () => {
  it("only treats enabled=true inside the knowledge section as enabling the knowledge base", () => {
    const parsed = parseKnowledgeConfig(`
[recording]
enabled = true

[knowledge]
enabled = false
path = "~/kb"
`);

    expect(parsed).toEqual({
      enabled: false,
      path: "~/kb",
      adapter: "wiki",
      engine: "none",
    });
  });

  it("reads knowledge settings from the knowledge section", () => {
    const parsed = parseKnowledgeConfig(`
[knowledge]
enabled = true
path = "~/kb"
adapter = "para"
engine = "agent"
`);

    expect(parsed).toEqual({
      enabled: true,
      path: "~/kb",
      adapter: "para",
      engine: "agent",
    });
  });
});

describe("atomic Rust knowledge status bridge", () => {
  it("returns only the Rust-owned snapshot from one command", async () => {
    const calls: string[][] = [];
    await expect(
      readKnowledgeStatusSnapshot(async (args) => {
        calls.push(args);
        return {
          stdout: '{"enabled":true,"configured":true,"adapter":"wiki","engine":"none","people_count":2,"log_entries":3}',
          stderr: "",
        };
      })
    ).resolves.toMatchObject({ people_count: 2, log_entries: 3 });
    expect(calls).toEqual([["knowledge-status", "--json"]]);
  });

  it("fails closed on malformed, negative, or failed bridge responses", async () => {
    for (const stdout of [
      "",
      "not-json",
      '{"enabled":true}',
      '{"enabled":true,"configured":true,"adapter":"wiki","engine":"none","people_count":-1,"log_entries":0}',
      "null",
    ]) {
      await expect(
        readKnowledgeStatusSnapshot(async () => ({
          stdout,
          stderr: "PRIVATE-DERIVATIVE-CANARY",
        }))
      ).rejects.toThrow(/could not be safely read/i);
    }

    await expect(
      readKnowledgeStatusSnapshot(async () => {
        throw new Error("bridge failed");
      })
    ).rejects.toThrow("bridge failed");
  });
});

describe("agent trust readiness bridge", () => {
  it("authorizes and audits restricted input before any mutating readiness", async () => {
    const deniedOrder: string[] = [];
    expect(() =>
      runAgentToolPolicies(
        "search_meetings",
        { include_restricted: true },
        () => deniedOrder.push("handler"),
        async () => deniedOrder.push("readiness"),
        "deny",
        () => deniedOrder.push("audit")
      )
    ).toThrow("Restricted meeting content is unavailable");
    expect(deniedOrder).toEqual([]);

    const overrideOrder: string[] = [];
    await expect(
      runAgentToolPolicies(
        "search_meetings",
        { include_restricted: true, query: "PRIVATE_ORDER_CANARY" },
        () => {
          overrideOrder.push("handler");
          return "authorized";
        },
        async () => {
          overrideOrder.push("readiness");
        },
        "logged-override",
        (_path, line) => {
          expect(line).not.toContain("PRIVATE_ORDER_CANARY");
          overrideOrder.push("audit");
        }
      )
    ).resolves.toBe("authorized");
    expect(overrideOrder).toEqual(["audit", "readiness", "handler"]);
  });

  it("probes the required CLI before connecting without globally gating controls on QMD", async () => {
    const order: string[] = [];
    const result = await afterRequiredCli(
      async () => {
        order.push("connect");
        return "connected";
      },
      async () => {
        order.push("cli");
        return true;
      }
    );

    expect(result).toBe("connected");
    expect(order).toEqual(["cli", "connect"]);
  });

  it("fails path-free before connect when the required CLI is unavailable", async () => {
    let connected = false;
    const error = await afterRequiredCli(
      async () => {
        connected = true;
      },
      async () => false
    ).catch((failure: unknown) => failure);

    expect(connected).toBe(false);
    expect(error).toBeInstanceOf(Error);
    expect((error as Error).message).toBe(
      "Minutes CLI is required to establish the agent trust boundary."
    );
  });

  it("rechecks readiness for every content read after a runtime registry change", async () => {
    let ready = true;
    let readinessChecks = 0;
    let reads = 0;
    const readiness = async () => {
      readinessChecks += 1;
      if (!ready) throw new Error("registry changed after startup");
    };

    await expect(
      afterContentBearingToolReadiness(
        "search_meetings",
        async () => {
          reads += 1;
          return "first read";
        },
        readiness
      )
    ).resolves.toBe("first read");

    ready = false;
    await expect(
      afterContentBearingToolReadiness(
        "search_meetings",
        async () => {
          reads += 1;
          return "stale authorization read";
        },
        readiness
      )
    ).rejects.toThrow("registry changed after startup");

    expect(readinessChecks).toBe(2);
    expect(reads).toBe(1);
  });

  it("rechecks readiness before every content-bearing resource read", async () => {
    let ready = true;
    let readinessChecks = 0;
    let reads = 0;
    const readiness = async () => {
      readinessChecks += 1;
      if (!ready) throw new Error("resource registry changed after connection");
    };

    await expect(
      afterContentResourceReadiness("recent_meetings", async () => {
        reads += 1;
        return "first resource";
      }, readiness)
    ).resolves.toBe("first resource");

    ready = false;
    await expect(
      afterContentResourceReadiness("recent_meetings", async () => {
        reads += 1;
        return "PRIVATE-RESOURCE-CANARY";
      }, readiness)
    ).rejects.toThrow("resource registry changed after connection");
    expect(readinessChecks).toBe(2);
    expect(reads).toBe(1);
  });

  it("enumerates every registered content-bearing resource behind the per-read gate", () => {
    expect(contentBearingAgentResourceNames()).toEqual(
      [
        "live_copilot",
        "live_events",
        "live_events_since_seq",
        "meeting",
        "open_actions",
        "recent-ideas",
        "recent_meetings",
      ].sort()
    );
  });

  it("does not gate non-content mutation tools on agent-read readiness", async () => {
    let readinessChecks = 0;
    await expect(
      afterContentBearingToolReadiness(
        "add_note",
        async () => "mutated",
        async () => {
          readinessChecks += 1;
          throw new Error("must not run");
        }
      )
    ).resolves.toBe("mutated");
    expect(readinessChecks).toBe(0);
  });

  it("exposes add_note only for the active recording", () => {
    expect(Object.keys(MCP_ADD_NOTE_INPUT_SCHEMA)).toEqual(["text"]);
    expect(MCP_ADD_NOTE_INPUT_SCHEMA).not.toHaveProperty("meeting_path");
  });

  it("enumerates every registered content-bearing tool behind the per-call gate", () => {
    expect(contentBearingAgentToolNames()).toEqual(
      [
        "activity_summary",
        "confirm_speaker",
        "consistency_report",
        "get_meeting",
        "get_moment",
        "get_person_profile",
        "get_screen_context",
        "ingest_meeting",
        "list_meetings",
        "list_processing_jobs",
        "list_voices",
        "process_audio",
        "read_live_transcript",
        "relationship_map",
        "research_topic",
        "search_context",
        "search_meetings",
        "start_copilot",
        "track_commitments",
      ].sort()
    );
  });

  it("allows content-free inactive copilot status without QMD readiness", async () => {
    let readinessChecks = 0;
    let reads = 0;
    await expect(
      afterActiveCopilotReadiness(
        { active: false },
        async () => {
          reads += 1;
          return "Copilot is not active (Off).";
        },
        async () => {
          readinessChecks += 1;
          throw new Error("blocked QMD retirement");
        }
      )
    ).resolves.toBe("Copilot is not active (Off).");
    expect(readinessChecks).toBe(0);
    expect(reads).toBe(1);
  });

  it("blocks active copilot content before reading the observation stream", async () => {
    let reads = 0;
    await expect(
      afterActiveCopilotReadiness(
        { active: true },
        async () => {
          reads += 1;
          return "PRIVATE-COPILOT-NUDGE-CANARY";
        },
        async () => {
          throw new Error("blocked QMD retirement");
        }
      )
    ).rejects.toThrow("blocked QMD retirement");
    expect(reads).toBe(0);
  });

  it("runs terminal controls before blocked readiness and withholds their result", async () => {
    const order: string[] = [];
    const stopped = await terminalControlBeforeContentReadiness(
      async () => {
        order.push("stop");
        return "PRIVATE-STOP-RESULT-CANARY";
      },
      async () => {
        order.push("readiness");
        throw new Error("blocked");
      }
    );
    expect(order).toEqual(["stop", "readiness"]);
    expect(stopped.mayRevealContent).toBe(false);
    const response = stopped.mayRevealContent
      ? stopped.result
      : "stopped, result withheld";
    expect(response).toBe("stopped, result withheld");
    expect(response).not.toContain("PRIVATE-STOP-RESULT-CANARY");
  });

  it("admits only a clean external registry before MCP connection", async () => {
      const qmdRetirement = "ready-clean" as const;
      const calls: string[][] = [];
      await expect(
        requireAgentTrustReadiness(async (args) => {
          calls.push(args);
          return {
            stdout: JSON.stringify({
              schema: 1,
              ready: true,
              qmd_retirement: qmdRetirement,
            }),
            stderr: "",
          };
        })
      ).resolves.toMatchObject({ qmd_retirement: qmdRetirement });
      expect(calls).toEqual([["agent-readiness", "--json"]]);
  });

  it("blocks MCP readiness with the path-free remediation returned by Rust", async () => {
    await expect(
      requireAgentTrustReadiness(async () => ({
        stdout: JSON.stringify({
          schema: 1,
          ready: false,
          qmd_retirement: "blocked",
          remediation:
            "Run minutes qmd cleanup, then restart Minutes before using Recall or agent features.",
        }),
        stderr: "PRIVATE-DERIVATIVE-CANARY",
      }))
    ).rejects.toThrow(/run minutes qmd cleanup/i);
  });

  it("fails closed on malformed or inconsistent readiness responses", async () => {
    for (const stdout of [
      "",
      "not-json",
      '{"schema":2,"ready":true,"qmd_retirement":"ready-clean","remediation":null}',
      '{"schema":1,"ready":false,"qmd_retirement":"ready-clean","remediation":null}',
      '{"schema":1,"ready":true,"qmd_retirement":"blocked","remediation":"retry"}',
      '{"schema":1,"ready":false,"qmd_retirement":"blocked","remediation":null}',
      '{"schema":1,"ready":true,"qmd_retirement":"ready-clean","remediation":"unexpected"}',
      '{"schema":1,"ready":true,"qmd_retirement":"ready-deferred-no-execution"}',
    ]) {
      await expect(
        readAgentTrustReadiness(async () => ({
          stdout,
          stderr: "PRIVATE-DERIVATIVE-CANARY",
        }))
      ).rejects.toThrow(/could not be verified safely/i);
    }
  });

  it("redacts rejected CLI failures and never connects the MCP transport", async () => {
    let connected = false;
    const error = await afterAgentTrustReadiness(
      async () => {
        connected = true;
      },
      async () => {
        throw new Error(
          "exec /PRIVATE/HOME/minutes failed: PRIVATE-CONFIG-CANARY"
        );
      }
    ).catch((failure: unknown) => failure);

    expect(connected).toBe(false);
    expect(error).toBeInstanceOf(Error);
    expect((error as Error).message).toBe(
      "Minutes agent readiness could not be verified safely."
    );
    expect((error as Error).message).not.toContain("PRIVATE");
  });
});

describe("strict live meeting root bridge", () => {
  it("uses an explicit environment override without invoking the CLI", async () => {
    const root = mkdtempSync(join(tmpdir(), "minutes-explicit-root-"));
    let invoked = false;
    try {
      await expect(
        getEffectiveMeetingsDir(
          async () => {
            invoked = true;
            throw new Error("must not run");
          },
          async () => {
            invoked = true;
            return true;
          },
          root
        )
      ).resolves.toBe(realpathSync(root));
      expect(invoked).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("strictly parses one exact schema and fails closed on bridge errors", async () => {
    expect(
      parseMeetingsRootSnapshot(
        JSON.stringify({ schema_version: 1, output_dir: "/tmp/meetings" })
      )
    ).toBe("/tmp/meetings");
    for (const stdout of [
      "not-json PRIVATE-ROOT-CANARY",
      JSON.stringify({ output_dir: "/tmp/meetings" }),
      JSON.stringify({ schema_version: 1, output_dir: "" }),
      JSON.stringify({ schema_version: 1, output_dir: "/tmp/meetings", extra: true }),
    ]) {
      expect(() => parseMeetingsRootSnapshot(stdout)).toThrow(
        "The live meeting root could not be safely resolved."
      );
    }
    await expect(
      getEffectiveMeetingsDir(
        async () => {
          throw new Error("PRIVATE-ROOT-CANARY");
        },
        async () => true,
        undefined
      )
    ).rejects.toThrow("The live meeting root could not be safely resolved.");
  });

  it("resolves every operation anew after a runtime config-root flip", async () => {
    const roots = ["/tmp/meetings-one", "/tmp/meetings-two"];
    let call = 0;
    const runner = async () => ({
      stdout: JSON.stringify({ schema_version: 1, output_dir: roots[call++] }),
      stderr: "",
    });
    await expect(getEffectiveMeetingsDir(runner, async () => true, undefined)).resolves.toBe(
      roots[0]
    );
    await expect(getEffectiveMeetingsDir(runner, async () => true, undefined)).resolves.toBe(
      roots[1]
    );
    expect(call).toBe(2);
  });
});

describe("shouldRunMainEntry", () => {
  it("accepts npm .bin shims that realpath to the module file", () => {
    const tempRoot = mkdtempSync(join(tmpdir(), "minutes-mcp-entry-"));
    const packageDir = join(tempRoot, "node_modules", "minutes-mcp", "dist");
    const binDir = join(tempRoot, "node_modules", ".bin");
    const modulePath = join(packageDir, "index.js");
    const shimPath = join(binDir, "minutes-mcp");

    mkdirSync(packageDir, { recursive: true });
    mkdirSync(binDir, { recursive: true });
    writeFileSync(modulePath, "export {};\n");
    symlinkSync(modulePath, shimPath);

    try {
      expect(shouldRunMainEntry(shimPath, modulePath)).toBe(true);
    } finally {
      rmSync(tempRoot, { recursive: true, force: true });
    }
  });

  it("accepts equivalent paths once symlinks are resolved", () => {
    expect(shouldRunMainEntry(import.meta.filename, import.meta.filename)).toBe(true);
  });

  it("rejects unrelated worker entrypoints", () => {
    expect(
      shouldRunMainEntry(
        "/Users/dev/project/node_modules/vitest/dist/workers/forks.js",
        "/Users/dev/project/crates/mcp/src/index.ts"
      )
    ).toBe(false);
  });
});

describe("copilot MCP observation contract", () => {
  const createdMs = Date.parse("2026-07-14T12:00:00.000Z");
  const firstNudge = {
    v: 1,
    id: "nudge-41-1",
    kind: "Ask",
    text: "Ask who owns the rollout date.",
    source_chip: "rollout date",
    evidence_revision: 41,
    created_ts: "2026-07-14T12:00:00.000Z",
    ttl_ms: 12000,
  };
  const secondNudge = {
    ...firstNudge,
    id: "nudge-42-2",
    kind: "Clarify",
    text: "Clarify whether Friday means launch or handoff.",
    evidence_revision: 42,
    created_ts: "2026-07-14T12:00:05.000Z",
    supersedes: "nudge-41-1",
  };

  it("parses the exact versioned CLI status without retaining raw or content fields", () => {
    expect(parseCopilotStatusOutput(JSON.stringify({
      schema_version: 1,
      active: false,
      state: "Off",
      pid: null,
      surface: null,
      evidence_cursor: 0,
      input_mode: "final_only",
      setup_needed: false,
    }))).toEqual({
      schema_version: 1,
      available: true,
      active: false,
      state: "Off",
      pid: null,
      surface: null,
      evidence_cursor: 0,
      input_mode: "final_only",
      setup_needed: false,
    });

    const active = parseCopilotStatusOutput(JSON.stringify({
      schema_version: 1,
      active: true,
      state: "Listening",
      pid: 4321,
      surface: "stdout",
      evidence_cursor: 42,
      input_mode: "realtime",
      setup_needed: false,
    }));
    expect(active).toEqual({
      schema_version: 1,
      available: true,
      active: true,
      state: "Listening",
      pid: 4321,
      surface: "stdout",
      evidence_cursor: 42,
      input_mode: "realtime",
      setup_needed: false,
    });
    expect(active).not.toHaveProperty("raw");
    expect(active).not.toHaveProperty("goal");
    expect(active).not.toHaveProperty("last_error");
  });

  it("rejects status extensions instead of leaking their content", () => {
    const canary = "PRIVATE-STATUS-CONTENT-CANARY";
    const parse = () => parseCopilotStatusOutput(JSON.stringify({
      schema_version: 1,
      active: false,
      state: "Off",
      pid: null,
      surface: null,
      evidence_cursor: 0,
      input_mode: "final_only",
      setup_needed: false,
      goal: canary,
    }));
    expect(parse).toThrow("Copilot status response was invalid.");
    try {
      parse();
    } catch (error) {
      expect(String(error)).not.toContain(canary);
    }
  });

  it("requests only the strict JSON status bridge and contains CLI failures", async () => {
    const calls: string[][] = [];
    const status = await readCopilotStatusFromCli(
      async (args) => {
        calls.push(args);
        return {
          stdout: JSON.stringify({
            schema_version: 1,
            active: false,
            state: "Off",
            pid: null,
            surface: null,
            evidence_cursor: 0,
            input_mode: "final_only",
            setup_needed: false,
          }),
          stderr: "",
        };
      },
      async () => true
    );
    expect(calls).toEqual([["copilot", "status", "--json"]]);
    expect(status).toMatchObject({ available: true, active: false });

    const canary = "/private/status/PATH-CONTENT-CANARY";
    const failed = await readCopilotStatusFromCli(
      async () => {
        throw new Error(canary);
      },
      async () => true
    );
    expect(failed).toMatchObject({
      available: false,
      error: "Unable to read copilot status safely.",
    });
    expect(JSON.stringify(failed)).not.toContain(canary);
  });

  it("returns content-free inactive status after issuing stop", async () => {
    const order: string[] = [];
    let engineActive = true;
    const stopped = await stopCopilotBeforeStatusRead(
      async () => {
        order.push("stop");
        expect(engineActive).toBe(true);
        engineActive = false;
      },
      async () => {
        order.push("status");
        expect(engineActive).toBe(false);
        return parseCopilotStatusOutput(JSON.stringify({
          schema_version: 1,
          active: false,
          state: "Off",
          pid: null,
          surface: null,
          evidence_cursor: 42,
          input_mode: "realtime",
          setup_needed: false,
        }));
      },
      async () => {
        order.push("readiness");
      }
    );

    expect(order).toEqual(["stop", "status"]);
    expect(stopped).toMatchObject({ mayRevealContent: true, status: { active: false } });
  });

  it("still stops but withholds a remaining active session when readiness is blocked", async () => {
    const order: string[] = [];
    const stopped = await stopCopilotBeforeStatusRead(
      async () => {
        order.push("stop");
      },
      async () => {
        order.push("status");
        return parseCopilotStatusOutput(JSON.stringify({
          schema_version: 1,
          active: true,
          state: "Listening",
          pid: 42,
          surface: "stdout",
          evidence_cursor: 7,
          input_mode: "realtime",
          setup_needed: false,
        }));
      },
      async () => {
        order.push("readiness");
        throw new Error("blocked");
      }
    );

    expect(order).toEqual(["stop", "status", "readiness"]);
    expect(stopped).toEqual({ mayRevealContent: false });
  });

  it("parses JSON nudges with cursor and TTL metadata", () => {
    const nudges = parseCopilotNudgeLog(
      `${JSON.stringify(firstNudge)}\n${JSON.stringify(secondNudge)}\n`,
      createdMs + 6000
    );

    expect(nudges).toHaveLength(2);
    expect(nudges[0]).toMatchObject({ cursor: 1, format: "json", expired: false });
    expect(nudges[1]).toMatchObject({
      cursor: 2,
      format: "json",
      expired: false,
      nudge: { id: "nudge-42-2", supersedes: "nudge-41-1" },
    });
  });

  it("returns lossless cursor pages and resets a cursor from a prior session", () => {
    const nudges = parseCopilotNudgeLog(
      `${JSON.stringify(firstNudge)}\n${JSON.stringify(secondNudge)}\n`,
      createdMs + 6000
    );
    const observation: CopilotNudgeObservation = {
      attached: true,
      cursor: 2,
      session: null,
      nudges,
      note: "attached",
    };

    expect(selectCopilotNudges(observation, { cursor: 0, limit: 1 })).toMatchObject({
      cursor: 2,
      next_cursor: 1,
      cursor_reset: false,
      has_more: true,
      nudges: [{ cursor: 1 }],
    });
    expect(selectCopilotNudges(observation, { cursor: 99 })).toMatchObject({
      cursor: 2,
      next_cursor: 2,
      cursor_reset: true,
      has_more: false,
      nudges: [{ cursor: 1 }, { cursor: 2 }],
    });
    expect(
      selectCopilotNudges(observation, { since: "2s" }, createdMs + 6000).nudges
    ).toMatchObject([{ cursor: 2 }]);
  });

  it("exposes latest but never current advice after TTL expiry", () => {
    const status = parseCopilotStatusOutput(JSON.stringify({
      schema_version: 1,
      active: true,
      state: "Nudge",
      pid: 4321,
      surface: "stdout",
      evidence_cursor: 42,
      input_mode: "realtime",
      setup_needed: false,
    }));
    const nudges = parseCopilotNudgeLog(JSON.stringify(firstNudge), createdMs + 13000);
    const payload = buildLiveCopilotResourcePayload(status, {
      attached: true,
      cursor: 1,
      session: null,
      nudges,
      note: "attached",
    });

    expect(payload.latest_nudge).toMatchObject({ cursor: 1, expired: true });
    expect(payload.current_nudge).toBeNull();
  });
});

describe("live event MCP resource", () => {
  it("keeps production reads and subscriptions constant when hidden events change", async () => {
    expect(LIVE_EVENTS_SUBSCRIPTIONS_ENABLED).toBe(false);
    const first = await readLiveEventsResource(new URL(LIVE_EVENTS_RESOURCE_URI));
    const second = await readLiveEventsResource(new URL(LIVE_EVENTS_RESOURCE_URI));
    expect(second).toEqual(first);
    const payload = JSON.parse(first.contents[0].text);
    expect(payload).toMatchObject({
      latest_seq: 0,
      events: [],
      reconnect: {
        cursor: 0,
        read_uri: `${LIVE_EVENTS_RESOURCE_URI}?since_seq=0`,
      },
    });
    expect(payload.unavailable).toContain("non-sensitive cursor");

    const requested = await readLiveEventsResource(
      new URL(`${LIVE_EVENTS_RESOURCE_URI}?since_seq=42&limit=7`)
    );
    expect(JSON.parse(requested.contents[0].text)).toMatchObject({
      latest_seq: 42,
      events: [],
      reconnect: {
        cursor: 42,
        read_uri: `${LIVE_EVENTS_RESOURCE_URI}?since_seq=42`,
      },
    });
  });

  it("keeps exported subscription handlers fail-closed by default", async () => {
    const mcpServer = new McpServer({ name: "minutes-safe-default-test", version: "0.0.0" });
    let sourceReads = 0;
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      latestEventSeq: async () => {
        sourceReads += 1;
        return 1;
      },
      readEventsSinceSeq: async () => {
        sourceReads += 1;
        return [{ seq: 1 }];
      },
      resourceReadiness: async () => {},
    });
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client(
      { name: "safe-default-client", version: "0.0.0" },
      { capabilities: {} }
    );

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await expect(
        client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI })
      ).rejects.toThrow();
      await new Promise((resolve) => setTimeout(resolve, 25));
      expect(sourceReads).toBe(0);
      expect(controller.subscriptionCount()).toBe(0);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("parses the base resource and cursor read URIs", () => {
    expect(parseLiveEventsResourceUri("minutes://events/live")).toMatchObject({
      uri: "minutes://events/live",
      sinceSeq: null,
      limit: 20,
    });
    expect(parseLiveEventsResourceUri("minutes://events/live?since_seq=42&limit=7")).toMatchObject({
      uri: "minutes://events/live?since_seq=42&limit=7",
      sinceSeq: 42,
      limit: 7,
    });
    expect(parseLiveEventsResourceUri("minutes://events/recent")).toBeNull();
  });

  it("builds a reconnect cursor from the highest delivered sequence", () => {
    const payload = buildLiveEventsResourcePayload(
      { uri: "minutes://events/live?since_seq=10", sinceSeq: 10, limit: 100 },
      [{ seq: 11 }, { seq: 14 }],
      12
    );

    expect(payload.latest_seq).toBe(14);
    expect(payload.reconnect).toEqual({
      cursor: 14,
      read_uri: "minutes://events/live?since_seq=14",
    });
  });

  it("keeps the reconnect cursor on the delivered page boundary", () => {
    const payload = buildLiveEventsResourcePayload(
      { uri: "minutes://events/live?since_seq=10&limit=1", sinceSeq: 10, limit: 1 },
      [{ seq: 11 }],
      14
    );

    expect(payload.latest_seq).toBe(14);
    expect(payload.reconnect).toEqual({
      cursor: 11,
      read_uri: "minutes://events/live?since_seq=11",
    });
  });

  it("does not move a future reconnect cursor backward", () => {
    const payload = buildLiveEventsResourcePayload(
      { uri: "minutes://events/live?since_seq=99", sinceSeq: 99, limit: 100 },
      [],
      14
    );

    expect(payload.latest_seq).toBe(14);
    expect(payload.reconnect).toEqual({
      cursor: 99,
      read_uri: "minutes://events/live?since_seq=99",
    });
  });

  it("sends resource updated notifications over an MCP client subscription", async () => {
    const mcpServer = new McpServer({ name: "minutes-test", version: "0.0.0" });
    const updates: string[] = [];
    let readCursor = 4;
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: true,
      latestEventSeq: async () => 4,
      readEventsSinceSeq: async (sinceSeq) => {
        if (sinceSeq >= readCursor) {
          readCursor = 9;
          return [{ seq: 9, event_type: "live.utterance.final" }];
        }
        return [];
      },
      resourceReadiness: async () => {},
    });

    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "test-client", version: "0.0.0" }, { capabilities: {} });
    client.setNotificationHandler(ResourceUpdatedNotificationSchema, (notification) => {
      updates.push(notification.params.uri);
    });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });

      await waitFor(() => updates.length > 0);
      expect(updates).toEqual([LIVE_EVENTS_RESOURCE_URI]);

      await client.unsubscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      expect(controller.subscriptionCount()).toBe(0);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("routes copilot updates through the same subscription handler", async () => {
    const mcpServer = new McpServer({ name: "minutes-copilot-test", version: "0.0.0" });
    const updates: string[] = [];
    let fingerprint = "off:0";
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: false,
      enableCopilot: true,
      copilotFingerprint: async () => fingerprint,
      resourceReadiness: async () => {},
    });

    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "copilot-test-client", version: "0.0.0" }, { capabilities: {} });
    client.setNotificationHandler(ResourceUpdatedNotificationSchema, (notification) => {
      updates.push(notification.params.uri);
    });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      fingerprint = "listening:1";

      await waitFor(() => updates.length > 0);
      expect(updates).toEqual([LIVE_COPILOT_RESOURCE_URI]);

      await client.unsubscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      expect(controller.subscriptionCount()).toBe(0);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("stops live-event subscription source reads when readiness is revoked", async () => {
    const mcpServer = new McpServer({ name: "minutes-event-revocation-test", version: "0.0.0" });
    const updates: string[] = [];
    let readinessAllowed = true;
    let sourceReads = 0;
    let nextSeq = 4;
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: true,
      latestEventSeq: async () => 4,
      readEventsSinceSeq: async (sinceSeq) => {
        sourceReads += 1;
        return nextSeq > sinceSeq ? [{ seq: nextSeq }] : [];
      },
      resourceReadiness: async () => {
        if (!readinessAllowed) throw new Error("synthetic readiness revocation");
      },
      sendResourceUpdated: async (uri) => {
        updates.push(uri);
      },
      onError: () => {},
    });

    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "event-revocation-client", version: "0.0.0" }, { capabilities: {} });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      readinessAllowed = false;
      nextSeq = 9;
      const readsAtRevocation = sourceReads;

      await new Promise((resolve) => setTimeout(resolve, 40));
      expect(sourceReads).toBe(readsAtRevocation);
      expect(updates).toEqual([]);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("stops Copilot subscription source reads when readiness is revoked", async () => {
    const mcpServer = new McpServer({ name: "minutes-copilot-revocation-test", version: "0.0.0" });
    const updates: string[] = [];
    let readinessAllowed = true;
    let sourceReads = 0;
    let fingerprint = "quiet:0";
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: false,
      enableCopilot: true,
      copilotFingerprint: async () => {
        sourceReads += 1;
        return fingerprint;
      },
      resourceReadiness: async () => {
        if (!readinessAllowed) throw new Error("synthetic readiness revocation");
      },
      sendResourceUpdated: async (uri) => {
        updates.push(uri);
      },
      onError: () => {},
    });

    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "copilot-revocation-client", version: "0.0.0" }, { capabilities: {} });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      readinessAllowed = false;
      fingerprint = "changed:1";
      const readsAtRevocation = sourceReads;

      await new Promise((resolve) => setTimeout(resolve, 40));
      expect(sourceReads).toBe(readsAtRevocation);
      expect(updates).toEqual([]);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("does not advance or notify when live-event readiness is revoked during a read", async () => {
    const mcpServer = new McpServer({ name: "minutes-event-mid-read-test", version: "0.0.0" });
    const updates: string[] = [];
    const seenCursors: number[] = [];
    let readinessAllowed = true;
    let signalReadStarted!: () => void;
    let releaseRead!: () => void;
    const readStarted = new Promise<void>((resolve) => { signalReadStarted = resolve; });
    const readRelease = new Promise<void>((resolve) => { releaseRead = resolve; });
    let suspendNextRead = true;
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: true,
      latestEventSeq: async () => 4,
      readEventsSinceSeq: async (sinceSeq) => {
        seenCursors.push(sinceSeq);
        if (suspendNextRead) {
          suspendNextRead = false;
          signalReadStarted();
          await readRelease;
        }
        return [{ seq: 9 }];
      },
      resourceReadiness: async () => {
        if (!readinessAllowed) throw new Error("synthetic mid-read revocation");
      },
      sendResourceUpdated: async (uri) => {
        updates.push(uri);
      },
      onError: () => {},
    });
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "event-mid-read-client", version: "0.0.0" }, { capabilities: {} });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      await readStarted;
      readinessAllowed = false;
      releaseRead();

      await new Promise((resolve) => setTimeout(resolve, 30));
      expect(updates).toEqual([]);
      expect(seenCursors).toEqual([4]);

      readinessAllowed = true;
      await waitFor(() => updates.length > 0);
      expect(seenCursors.slice(0, 2)).toEqual([4, 4]);
      expect(updates).toEqual([LIVE_EVENTS_RESOURCE_URI]);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("does not advance or notify when Copilot readiness is revoked during a read", async () => {
    const mcpServer = new McpServer({ name: "minutes-copilot-mid-read-test", version: "0.0.0" });
    const updates: string[] = [];
    let readinessAllowed = true;
    let fingerprintReads = 0;
    let signalReadStarted!: () => void;
    let releaseRead!: () => void;
    const readStarted = new Promise<void>((resolve) => { signalReadStarted = resolve; });
    const readRelease = new Promise<void>((resolve) => { releaseRead = resolve; });
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: false,
      enableCopilot: true,
      copilotFingerprint: async () => {
        fingerprintReads += 1;
        if (fingerprintReads === 1) return "quiet:0";
        if (fingerprintReads === 2) {
          signalReadStarted();
          await readRelease;
        }
        return "changed:1";
      },
      resourceReadiness: async () => {
        if (!readinessAllowed) throw new Error("synthetic mid-read revocation");
      },
      sendResourceUpdated: async (uri) => {
        updates.push(uri);
      },
      onError: () => {},
    });
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "copilot-mid-read-client", version: "0.0.0" }, { capabilities: {} });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      await readStarted;
      readinessAllowed = false;
      releaseRead();

      await new Promise((resolve) => setTimeout(resolve, 30));
      expect(updates).toEqual([]);
      expect(fingerprintReads).toBe(2);

      readinessAllowed = true;
      await waitFor(() => updates.length > 0);
      expect(fingerprintReads).toBeGreaterThanOrEqual(3);
      expect(updates).toEqual([LIVE_COPILOT_RESOURCE_URI]);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("does not deliver an in-flight event read to a replacement subscription", async () => {
    const mcpServer = new McpServer({ name: "minutes-event-epoch-test", version: "0.0.0" });
    const updates: string[] = [];
    const seenCursors: number[] = [];
    let signalReadStarted!: () => void;
    let releaseRead!: () => void;
    const readStarted = new Promise<void>((resolve) => { signalReadStarted = resolve; });
    const readRelease = new Promise<void>((resolve) => { releaseRead = resolve; });
    let reads = 0;
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: true,
      latestEventSeq: async () => 4,
      readEventsSinceSeq: async (sinceSeq) => {
        seenCursors.push(sinceSeq);
        reads += 1;
        if (reads === 1) {
          signalReadStarted();
          await readRelease;
          return [{ seq: 9 }];
        }
        return [];
      },
      resourceReadiness: async () => {},
      sendResourceUpdated: async (uri) => { updates.push(uri); },
      onError: () => {},
    });
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "event-epoch-client", version: "0.0.0" }, { capabilities: {} });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      await readStarted;
      await client.unsubscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      await client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      releaseRead();

      await new Promise((resolve) => setTimeout(resolve, 40));
      expect(updates).toEqual([]);
      expect(seenCursors.length).toBeGreaterThanOrEqual(2);
      expect(seenCursors.every((cursor) => cursor === 4)).toBe(true);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("does not let an old Copilot poll seed a replacement subscription", async () => {
    const mcpServer = new McpServer({ name: "minutes-copilot-epoch-test", version: "0.0.0" });
    const updates: string[] = [];
    let signalReadStarted!: () => void;
    let releaseRead!: () => void;
    const readStarted = new Promise<void>((resolve) => { signalReadStarted = resolve; });
    const readRelease = new Promise<void>((resolve) => { releaseRead = resolve; });
    let fingerprintReads = 0;
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 5,
      enableLiveEvents: false,
      enableCopilot: true,
      copilotFingerprint: async () => {
        fingerprintReads += 1;
        if (fingerprintReads === 1) return "initial:0";
        if (fingerprintReads === 2) {
          signalReadStarted();
          await readRelease;
          return "obsolete:1";
        }
        return "replacement:0";
      },
      resourceReadiness: async () => {},
      sendResourceUpdated: async (uri) => { updates.push(uri); },
      onError: () => {},
    });
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "copilot-epoch-client", version: "0.0.0" }, { capabilities: {} });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      await readStarted;
      await client.unsubscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      await client.subscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      releaseRead();

      await new Promise((resolve) => setTimeout(resolve, 40));
      expect(fingerprintReads).toBeGreaterThanOrEqual(4);
      expect(updates).toEqual([]);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });

  it("reinitializes one resource after unsubscribe while the other poller remains live", async () => {
    const mcpServer = new McpServer({ name: "minutes-resource-reset-test", version: "0.0.0" });
    let latestSeqReads = 0;
    const controller = registerLiveEventsSubscriptionHandlers(mcpServer, {
      pollIntervalMs: 20,
      enableLiveEvents: true,
      enableCopilot: true,
      latestEventSeq: async () => {
        latestSeqReads += 1;
        return latestSeqReads === 1 ? 4 : 10;
      },
      readEventsSinceSeq: async () => [],
      copilotFingerprint: async () => "steady:0",
      resourceReadiness: async () => {},
      sendResourceUpdated: async () => {},
      onError: () => {},
    });
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "resource-reset-client", version: "0.0.0" }, { capabilities: {} });

    try {
      await Promise.all([
        mcpServer.connect(serverTransport),
        client.connect(clientTransport),
      ]);
      await client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      await client.subscribeResource({ uri: LIVE_COPILOT_RESOURCE_URI });
      await client.unsubscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      expect(controller.subscriptionCount()).toBe(1);
      await client.subscribeResource({ uri: LIVE_EVENTS_RESOURCE_URI });
      expect(latestSeqReads).toBe(2);
    } finally {
      controller.stop();
      await client.close();
      await mcpServer.close();
    }
  });
});

async function waitFor(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 1000;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error("timed out waiting for condition");
}
