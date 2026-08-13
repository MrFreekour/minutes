#!/usr/bin/env node

/**
 * Self-tests for the cross-process surface parity helper.
 *
 * These spawn nothing. They feed synthetic surfaces and synthetic capability
 * states to the same functions the HTTP suites use, because the properties
 * that matter cannot be demonstrated by running the real test repeatedly:
 *
 *  - a core mismatch must FAIL, and name the tool (the assertion still bites)
 *  - a probe divergence must PASS (the flake is gone, not the coverage)
 *  - a gated tool missing from a process whose own state says it should be
 *    there must FAIL (the own-state check is real, not decorative)
 *
 * The second case is the actual evidence that the timing flake is fixed.
 * Repeat runs of the live test only show that it did not misbehave on a quiet
 * machine; the flake was never reproduced on demand.
 *
 * Run: node crates/mcp/test/surface_parity_test.mjs
 */

import {
  CAPABILITY_GATED_RESOURCES,
  CAPABILITY_GATED_TOOLS,
  assertCoreParity,
  assertGateMapMatchesSource,
  assertOwnStateGatedSurface,
  assertSameCli,
  capabilityStatesAgree,
  expectedGatedNames,
  splitSurface,
} from "./lib/surface-parity.mjs";

let passed = 0;
let failed = 0;

function test(name, fn) {
  try {
    fn();
    console.log(`  PASS: ${name}`);
    passed++;
  } catch (e) {
    console.error(`  FAIL: ${name} — ${e.message}`);
    failed++;
  }
}

function assert(condition, msg) {
  if (!condition) throw new Error(msg || "assertion failed");
}

/** Run fn, return the thrown Error. Fails if it does not throw. */
function throws(fn, what) {
  try {
    fn();
  } catch (e) {
    return e;
  }
  throw new Error(`expected ${what} to throw, it returned normally`);
}

const GATED = Object.keys(CAPABILITY_GATED_TOOLS);
const CORE = [
  "get_status",
  "list_meetings",
  "search_meetings",
  "start_recording",
  "resummarize_meeting",
];

const REPORT = {
  version: "0.24.0",
  api_version: 1,
  features: Object.fromEntries(
    [...new Set(Object.values(CAPABILITY_GATED_TOOLS))].map((key) => [key, true])
  ),
};
const STATE_REPORT = {
  kind: "report",
  cliVersion: "0.24.0",
  apiVersion: 1,
  featureCount: Object.keys(REPORT.features).length,
};
const STATE_UNSUPPORTED = {
  kind: "unsupported-cli",
  cliVersion: null,
  apiVersion: null,
  featureCount: null,
};
const STATE_MISSING = {
  kind: "missing-cli",
  cliVersion: null,
  apiVersion: null,
  featureCount: null,
};

console.log("Surface Parity Helper Self-Tests\n");

// ── The assertion still bites ───────────────────────────────

test("a missing core tool fails loudly and names it", () => {
  const full = [...CORE, ...GATED];
  const doctored = full.filter((n) => n !== "search_meetings");
  const error = throws(
    () =>
      assertCoreParity({
        label: "tool surface",
        aLabel: "http",
        aNames: doctored,
        bLabel: "stdio",
        bNames: full,
        gatedMap: CAPABILITY_GATED_TOOLS,
      }),
    "a core tool missing over HTTP"
  );
  assert(
    error.message.includes("search_meetings"),
    `the failure must name the tool, got: ${error.message}`
  );
  assert(
    error.message.includes("missing"),
    `the failure must say what is missing, got: ${error.message}`
  );
});

test("an extra core tool on one side fails too", () => {
  const full = [...CORE, ...GATED];
  const error = throws(
    () =>
      assertCoreParity({
        label: "tool surface",
        aLabel: "http",
        aNames: [...full, "phantom_tool"],
        bLabel: "stdio",
        bNames: full,
        gatedMap: CAPABILITY_GATED_TOOLS,
      }),
    "an unexpected core tool"
  );
  assert(error.message.includes("phantom_tool"), error.message);
});

test("two empty surfaces do not match trivially", () => {
  throws(
    () =>
      assertCoreParity({
        label: "tool surface",
        aLabel: "http",
        aNames: [],
        bLabel: "stdio",
        bNames: [],
        gatedMap: CAPABILITY_GATED_TOOLS,
      }),
    "an empty-vs-empty comparison"
  );
});

test("a required anchor missing from one side fails", () => {
  const full = [...CORE, ...GATED];
  const error = throws(
    () =>
      assertCoreParity({
        label: "tool surface",
        aLabel: "http",
        aNames: full.filter((n) => n !== "get_status"),
        bLabel: "stdio",
        bNames: full.filter((n) => n !== "get_status"),
        gatedMap: CAPABILITY_GATED_TOOLS,
        requiredAnchors: ["get_status"],
      }),
    "both sides missing an anchor"
  );
  assert(error.message.includes("get_status"), error.message);
});

// ── The flake is gone ───────────────────────────────────────

test("a probe divergence passes: core parity holds, gated sets differ legitimately", () => {
  // Exactly the historical flake: one process probed the CLI in time, the
  // other's 2s budget expired and it fell back to unsupported-cli.
  const healthy = [...CORE, ...GATED];
  const timedOut = [...CORE];

  // Core parity is unaffected.
  const core = assertCoreParity({
    label: "tool surface",
    aLabel: "http",
    aNames: healthy,
    bLabel: "stdio",
    bNames: timedOut,
    gatedMap: CAPABILITY_GATED_TOOLS,
    requiredAnchors: ["get_status", "list_meetings"],
  });
  assert(core.length === CORE.length, `core should be ${CORE.length}, got ${core.length}`);

  // And each process matches its own declared state.
  assertOwnStateGatedSurface({
    label: "http",
    names: healthy,
    gatedMap: CAPABILITY_GATED_TOOLS,
    state: STATE_REPORT,
    report: REPORT,
  });
  assertOwnStateGatedSurface({
    label: "stdio",
    names: timedOut,
    gatedMap: CAPABILITY_GATED_TOOLS,
    state: STATE_UNSUPPORTED,
    report: REPORT,
  });

  assert(
    !capabilityStatesAgree(STATE_REPORT, STATE_UNSUPPORTED),
    "the divergence should be detectable for reporting"
  );
});

test("the old full-list comparison would have failed on that same input", () => {
  // Guards the premise. If a naive comparison of the same two surfaces passed,
  // there would have been no flake to fix and this module would be pointless.
  const healthy = [...CORE, ...GATED].sort();
  const timedOut = [...CORE].sort();
  assert(
    healthy.join(",") !== timedOut.join(","),
    "the divergent surfaces must actually differ"
  );
});

// ── The own-state check is real ─────────────────────────────

test("a gated tool missing while its own state says present fails", () => {
  const short = [...CORE, ...GATED.filter((n) => n !== "start_copilot")];
  const error = throws(
    () =>
      assertOwnStateGatedSurface({
        label: "http",
        names: short,
        gatedMap: CAPABILITY_GATED_TOOLS,
        state: STATE_REPORT,
        report: REPORT,
      }),
    "a gated tool dropped by a process whose probe succeeded"
  );
  assert(error.message.includes("start_copilot"), error.message);
});

test("a gated tool present while its own state says unsupported fails", () => {
  const error = throws(
    () =>
      assertOwnStateGatedSurface({
        label: "stdio",
        names: [...CORE, "get_moment"],
        gatedMap: CAPABILITY_GATED_TOOLS,
        state: STATE_UNSUPPORTED,
        report: REPORT,
      }),
    "a gated tool advertised by a fail-closed process"
  );
  assert(error.message.includes("get_moment"), error.message);
});

test("missing-cli expects every gated name, matching hasFeature", () => {
  const expected = expectedGatedNames(CAPABILITY_GATED_TOOLS, STATE_MISSING, null);
  assert(expected.size === GATED.length, `expected all ${GATED.length}, got ${expected.size}`);
  assertOwnStateGatedSurface({
    label: "first-run",
    names: [...CORE, ...GATED],
    gatedMap: CAPABILITY_GATED_TOOLS,
    state: STATE_MISSING,
    report: null,
  });
});

test("a feature reported false hides exactly its tools", () => {
  const partial = {
    ...REPORT,
    features: { ...REPORT.features, copilot_realtime: false },
  };
  const state = { ...STATE_REPORT, featureCount: Object.keys(partial.features).length };
  const expected = expectedGatedNames(CAPABILITY_GATED_TOOLS, state, partial);
  assert(!expected.has("start_copilot"), "copilot tools should be hidden");
  assert(expected.has("get_moment"), "unrelated gated tools should stay");
  assertOwnStateGatedSurface({
    label: "partial",
    names: [...CORE, "activity_summary", "search_context", "get_moment", "get_screen_context"],
    gatedMap: CAPABILITY_GATED_TOOLS,
    state,
    report: partial,
  });
});

test("a process that probed a different CLI fails rather than being compared", () => {
  const error = throws(
    () =>
      assertSameCli(
        "http",
        { ...STATE_REPORT, cliVersion: "0.23.0" },
        REPORT
      ),
    "a version mismatch between the child's probe and the test's"
  );
  assert(error.message.includes("0.23.0"), error.message);
});

test("a feature-count mismatch fails even when versions agree", () => {
  throws(
    () => assertSameCli("http", { ...STATE_REPORT, featureCount: 99 }, REPORT),
    "a feature-count mismatch"
  );
});

// ── Resources get the same treatment ────────────────────────

test("gated resources split the same way", () => {
  const uris = [
    "minutes://status",
    "minutes://meetings/recent",
    "minutes://events/live",
    "minutes://live/copilot",
  ];
  const { core, gated } = splitSurface(uris, CAPABILITY_GATED_RESOURCES);
  assert(core.length === 2, `core should be 2, got ${core.join(",")}`);
  assert(gated.length === 2, `gated should be 2, got ${gated.join(",")}`);
  assertOwnStateGatedSurface({
    label: "timed-out process",
    names: ["minutes://status", "minutes://meetings/recent"],
    gatedMap: CAPABILITY_GATED_RESOURCES,
    state: STATE_UNSUPPORTED,
    report: REPORT,
  });
});

// ── Drift guard against index.ts ────────────────────────────

test("the gate map matches the gates in index.ts", () => {
  const { keys, inlineGates } = assertGateMapMatchesSource();
  assert(keys.length > 0, "expected to find capability gates in index.ts");
  assert(inlineGates > 0, "expected inline hasFeature tool gates");
});

test("a new unmodelled capability gate fails the drift guard", () => {
  const error = throws(
    () =>
      assertGateMapMatchesSource(
        `if (hasFeature(CLI_CAPABILITIES, "activity_summary"))\n` +
          `if (hasFeature(CLI_CAPABILITIES, "search_context"))\n` +
          `if (hasFeature(CLI_CAPABILITIES, "get_moment"))\n` +
          `if (hasFeature(CLI_CAPABILITIES, "screen_context"))\n` +
          `const A = hasFeature(CLI_CAPABILITIES, "events_since_seq");\n` +
          `const B = hasFeature(CLI_CAPABILITIES, "copilot_realtime");\n` +
          `const C = hasFeature(CLI_CAPABILITIES, "brand_new_thing");\n`
      ),
    "an unmodelled capability key"
  );
  assert(error.message.includes("brand_new_thing"), error.message);
});

test("a new inline gate reusing a known key still fails the drift guard", () => {
  const error = throws(
    () =>
      assertGateMapMatchesSource(
        `if (hasFeature(CLI_CAPABILITIES, "activity_summary"))\n` +
          `if (hasFeature(CLI_CAPABILITIES, "search_context"))\n` +
          `if (hasFeature(CLI_CAPABILITIES, "get_moment"))\n` +
          `if (hasFeature(CLI_CAPABILITIES, "screen_context"))\n` +
          `if (hasFeature(CLI_CAPABILITIES, "get_moment"))\n` +
          `const A = hasFeature(CLI_CAPABILITIES, "events_since_seq");\n` +
          `const B = hasFeature(CLI_CAPABILITIES, "copilot_realtime");\n`
      ),
    "a fifth inline gate"
  );
  assert(error.message.includes("inline"), error.message);
});

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed > 0 ? 1 : 0);
