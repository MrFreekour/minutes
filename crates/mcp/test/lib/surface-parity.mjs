/**
 * Cross-process surface parity helpers.
 *
 * Comparing a full `tools/list` between two separately spawned server
 * processes is not a deterministic assertion. Each process runs
 * `probeCapabilitiesSync` at module load with a 2 second `execFileSync`
 * budget, and a cold or contended CLI binary blows that budget. The probe
 * then returns `unsupported-cli`, `hasFeature` answers false for everything,
 * and that process registers eight fewer tools and two fewer resources than
 * its sibling. The surfaces differ for a reason that has nothing to do with
 * the transport under test.
 *
 * The fix is not to loosen the comparison. It is to compare each process
 * against *its own* declared capability state, and to compare the two
 * processes only on the part of the surface that no capability gate can move:
 *
 *   core     = advertised names minus every capability-gated name
 *   gated    = exactly the names this process's own probe outcome predicts
 *
 * Core parity stays an exact set comparison between the two processes. The
 * gated part is asserted per process against what that process itself decided,
 * so a probe timeout produces a *correct passing* assertion (zero gated tools,
 * which is what `unsupported-cli` means) instead of a spurious parity failure.
 *
 * ## Where the declared capability state comes from
 *
 * `index.ts` records the probe outcome through `crashTrace`, which appends
 * JSONL to `~/.minutes/logs/mcp-crash.log` carrying the writing process's own
 * pid and ppid. That is the only channel that reports what a process *decided*
 * rather than what it *advertised*. The distinction is the whole point: read
 * from the surface alone, "probe timed out" and "the factory dropped the gated
 * tools" look identical, and the second is exactly the defect this test exists
 * to catch.
 *
 * It is a debug artifact, so every reader here fails loudly when the row is
 * missing rather than degrading to a weaker assertion.
 */

import { execFileSync } from "child_process";
import { existsSync, readFileSync } from "fs";
import { homedir, tmpdir } from "os";
import { join } from "path";

const MCP_DIR = join(import.meta.dirname, "..", "..");
const REPO_ROOT = join(MCP_DIR, "..", "..");

/**
 * Tools registered behind a capability gate, mapped to the feature key that
 * gates them. Anything not listed here is core and must be present in every
 * process regardless of what the probe returned.
 *
 * Four are gated inline by `if (hasFeature(CLI_CAPABILITIES, "..."))`; the
 * four copilot tools share one `if (COPILOT_SUPPORTED)` block.
 * `assertGateMapMatchesSource` fails if a gate is added to `index.ts` without
 * being added here, because an unlisted gated tool would silently land in the
 * core set and reintroduce the flake this module exists to remove.
 */
export const CAPABILITY_GATED_TOOLS = Object.freeze({
  activity_summary: "activity_summary",
  search_context: "search_context",
  get_moment: "get_moment",
  get_screen_context: "screen_context",
  start_copilot: "copilot_realtime",
  stop_copilot: "copilot_realtime",
  copilot_status: "copilot_realtime",
  read_copilot_nudges: "copilot_realtime",
});

/**
 * Resources behind the same gates. `resources/list` carries static URIs only,
 * so the `minutes://events/live{?since_seq,limit}` template is deliberately
 * absent: it is registered with `list: undefined` and never appears here. The
 * template is covered by the in-process `serverFactory.test.ts` check, which
 * reads `_registeredResourceTemplates` directly.
 */
export const CAPABILITY_GATED_RESOURCES = Object.freeze({
  "minutes://events/live": "events_since_seq",
  "minutes://live/copilot": "copilot_realtime",
});

/** Feature keys reachable through either map, for the source-drift guard. */
const GATED_FEATURE_KEYS = new Set([
  ...Object.values(CAPABILITY_GATED_TOOLS),
  ...Object.values(CAPABILITY_GATED_RESOURCES),
]);

/** Number of inline `if (hasFeature(...))` tool gates the maps above account for. */
const INLINE_GATED_TOOL_COUNT = 4;

/**
 * Resolve the binary the server will pick, replicating `findMinutesBinary`'s
 * candidate order. Release comes *before* debug: warming `target/debug/minutes`
 * unconditionally, the way this suite used to, warms the wrong file the moment
 * a release build exists.
 */
export function resolveMinutesBinary() {
  const ext = process.platform === "win32" ? ".exe" : "";
  const candidates = [
    join(REPO_ROOT, "target", "release", `minutes${ext}`),
    join(REPO_ROOT, "target", "debug", `minutes${ext}`),
    ...(process.platform === "win32"
      ? [join(homedir(), ".minutes", "bin", `minutes${ext}`)]
      : []),
    join(homedir(), ".cargo", "bin", `minutes${ext}`),
    ...(process.platform === "win32"
      ? []
      : [
          join(homedir(), ".local", "bin", "minutes"),
          "/opt/homebrew/bin/minutes",
          "/usr/local/bin/minutes",
        ]),
  ];
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  return "minutes";
}

/**
 * Probe the capability report the way the server does, but with a budget wide
 * enough that this call cannot be the flaky one. Runs twice: the first call
 * pays any cold-start cost, the second is the measurement, which also leaves
 * the binary warm for the children spawned next.
 */
export function probeCapabilityReport(binPath, timeoutMs = 30000) {
  const run = () =>
    execFileSync(binPath, ["capabilities", "--json"], {
      timeout: timeoutMs,
      encoding: "utf-8",
      stdio: ["ignore", "pipe", "ignore"],
    });
  try {
    run();
    const parsed = JSON.parse(run().trim());
    if (
      typeof parsed?.version !== "string" ||
      typeof parsed?.api_version !== "number" ||
      !parsed?.features ||
      typeof parsed.features !== "object"
    ) {
      return null;
    }
    return parsed;
  } catch {
    // An older CLI has no `capabilities` subcommand. Callers treat null as
    // "no report available" and must not silently assume a gated set.
    return null;
  }
}

/** Both locations `crashTracer.ts` may choose for its log. */
function crashLogPaths() {
  return [
    join(homedir(), ".minutes", "logs", "mcp-crash.log"),
    join(tmpdir(), "minutes-mcp-crash.log"),
  ];
}

function readCrashRows() {
  const rows = [];
  for (const path of crashLogPaths()) {
    if (!existsSync(path)) continue;
    let raw;
    try {
      raw = readFileSync(path, "utf-8");
    } catch {
      continue;
    }
    for (const line of raw.split("\n")) {
      if (!line.trim()) continue;
      try {
        rows.push(JSON.parse(line));
      } catch {
        // A torn final line while a server is mid-write. Skip it.
      }
    }
  }
  return rows;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Read one process's own declared capability state.
 *
 * Filtered on pid *and* ppid *and* a timestamp floor, because the log is a
 * shared append-only file that outlives sessions and macOS recycles pids.
 *
 * Ordering is not a hazard — the probe runs at module load, long before the
 * server reports a port or answers `initialize` — but the write is a separate
 * syscall from the one the caller waited on, so this retries briefly rather
 * than reading once.
 *
 * Throws when no row is found. Degrading to a weaker assertion here would
 * leave the suite green while covering less, which is the failure mode this
 * whole module is a response to.
 */
export async function readDeclaredCapabilityState({
  pid,
  ppid = process.pid,
  sinceIso,
  attempts = 30,
  intervalMs = 100,
  label = `pid ${pid}`,
}) {
  if (typeof pid !== "number" || Number.isNaN(pid)) {
    throw new Error(`${label}: no pid available to identify the server process`);
  }
  for (let attempt = 0; attempt < attempts; attempt++) {
    const match = readCrashRows()
      .filter(
        (row) =>
          row.pid === pid &&
          row.ppid === ppid &&
          typeof row.event === "string" &&
          row.event.startsWith("cli-capabilities") &&
          (!sinceIso || String(row.ts) >= sinceIso)
      )
      .pop();
    if (match) {
      const kind =
        match.event === "cli-capabilities-probed"
          ? "report"
          : match.event === "cli-capabilities-cli-missing"
            ? "missing-cli"
            : "unsupported-cli";
      return {
        kind,
        cliVersion: match.detail?.cliVersion ?? null,
        apiVersion: match.detail?.apiVersion ?? null,
        featureCount: match.detail?.featureCount ?? null,
        pid,
      };
    }
    await sleep(intervalMs);
  }
  throw new Error(
    `${label}: no cli-capabilities row for pid ${pid} (ppid ${ppid}) after ${attempts} attempts. ` +
      `The capability witness is ${crashLogPaths().join(" or ")}; without it this test cannot ` +
      `distinguish a probe timeout from a dropped registration.`
  );
}

/**
 * The gated names a process in this state must advertise.
 *
 * Mirrors `hasFeature`: `missing-cli` keeps every gated name visible so a
 * first-run auto-install session does not lose them, `unsupported-cli` is
 * fail-closed, and `report` follows the feature map.
 *
 * For `report` the caller supplies the feature map probed from the same
 * binary. `assertSameCli` is what makes that substitution legitimate, so call
 * it first.
 */
export function expectedGatedNames(gatedMap, state, report) {
  const all = Object.keys(gatedMap);
  if (state.kind === "missing-cli") return new Set(all);
  if (state.kind === "unsupported-cli") return new Set();
  if (!report) {
    throw new Error(
      `state is "report" but no capability report was supplied to compare against`
    );
  }
  return new Set(all.filter((name) => report.features[gatedMap[name]] === true));
}

/**
 * Fail unless a child's probe outcome came from the binary this test probed.
 * Without it, a child resolving a different `minutes` binary is
 * indistinguishable from a child that agrees.
 */
export function assertSameCli(label, state, report) {
  if (state.kind !== "report") return;
  if (!report) {
    throw new Error(
      `${label}: reported a capability payload but the test could not probe one to compare`
    );
  }
  const expectedFeatureCount = Object.keys(report.features).length;
  if (
    state.cliVersion !== report.version ||
    state.apiVersion !== report.api_version ||
    state.featureCount !== expectedFeatureCount
  ) {
    throw new Error(
      `${label}: probed a different CLI than this test did — process reported ` +
        `version=${state.cliVersion} api=${state.apiVersion} features=${state.featureCount}, ` +
        `test probed version=${report.version} api=${report.api_version} features=${expectedFeatureCount}`
    );
  }
}

/** Split an advertised surface into the gate-independent core and the gated part. */
export function splitSurface(names, gatedMap) {
  const core = [];
  const gated = [];
  for (const name of names) {
    if (Object.prototype.hasOwnProperty.call(gatedMap, name)) gated.push(name);
    else core.push(name);
  }
  return { core: core.sort(), gated: gated.sort() };
}

function formatSetDiff(label, a, b) {
  const onlyA = a.filter((n) => !b.includes(n));
  const onlyB = b.filter((n) => !a.includes(n));
  return `${label}\n  missing: ${onlyB.join(", ") || "(none)"}\n  unexpected: ${onlyA.join(", ") || "(none)"}`;
}

/**
 * Assert a single process advertises exactly the gated names its own probe
 * outcome predicts.
 *
 * This is the assertion that keeps the gated half covered rather than merely
 * excluded. A tool the factory replay dropped is missing from a process whose
 * own state says it should be there, and that fails here — even though it
 * would never disturb core parity.
 */
export function assertOwnStateGatedSurface({ label, names, gatedMap, state, report }) {
  const { gated } = splitSurface(names, gatedMap);
  const expected = [...expectedGatedNames(gatedMap, state, report)].sort();
  if (gated.join(",") !== expected.join(",")) {
    throw new Error(
      formatSetDiff(
        `${label}: advertised gated surface does not match its own declared capability state ` +
          `(${state.kind}${state.kind === "report" ? `, ${state.featureCount} features` : ""})`,
        gated,
        expected
      )
    );
  }
  return gated;
}

/**
 * Assert two processes advertise the identical core surface.
 *
 * No capability gate can move a core name, so this is exact and timing cannot
 * perturb it. `requiredAnchors` guards the degenerate case where both sides
 * are empty and match trivially.
 */
export function assertCoreParity({ label, aLabel, aNames, bLabel, bNames, gatedMap, requiredAnchors = [] }) {
  const a = splitSurface(aNames, gatedMap).core;
  const b = splitSurface(bNames, gatedMap).core;
  if (a.length === 0 || b.length === 0) {
    throw new Error(`${label}: a core surface was empty (${aLabel}=${a.length}, ${bLabel}=${b.length})`);
  }
  for (const anchor of requiredAnchors) {
    if (!a.includes(anchor)) throw new Error(`${label}: ${aLabel} is missing core entry ${anchor}`);
    if (!b.includes(anchor)) throw new Error(`${label}: ${bLabel} is missing core entry ${anchor}`);
  }
  if (a.join(",") !== b.join(",")) {
    throw new Error(
      formatSetDiff(
        `${label}: core surface drift between ${aLabel} and ${bLabel}`,
        a,
        b
      )
    );
  }
  return a;
}

/** Do two processes' probe outcomes agree closely enough to compare gated sets directly? */
export function capabilityStatesAgree(a, b) {
  return (
    a.kind === b.kind &&
    a.cliVersion === b.cliVersion &&
    a.apiVersion === b.apiVersion &&
    a.featureCount === b.featureCount
  );
}

/** One line describing a divergence, for tests to print instead of failing on it. */
export function describeStateDivergence(aLabel, a, bLabel, b) {
  return (
    `NOTE: capability probes diverged — ${aLabel}=${a.kind}` +
    `${a.kind === "report" ? `(${a.featureCount} features)` : ""}, ` +
    `${bLabel}=${b.kind}${b.kind === "report" ? `(${b.featureCount} features)` : ""}. ` +
    `Gated tools were checked against each process's own state; core parity is still exact.`
  );
}

/**
 * Fail when `index.ts` grows a capability gate the maps above do not model.
 *
 * Two guards, because they catch different mistakes: an unknown feature key
 * catches a brand-new capability, and the inline-gate count catches a new
 * gated tool that reuses an existing key.
 */
export function assertGateMapMatchesSource(source = readFileSync(join(MCP_DIR, "src", "index.ts"), "utf-8")) {
  const keys = new Set();
  let inlineGates = 0;
  const pattern = /hasFeature\(\s*CLI_CAPABILITIES\s*,\s*"([a-z_]+)"\s*\)/g;
  for (const match of source.matchAll(pattern)) {
    keys.add(match[1]);
  }
  const inlinePattern = /if \(hasFeature\(\s*CLI_CAPABILITIES\s*,\s*"([a-z_]+)"\s*\)\)/g;
  for (const _ of source.matchAll(inlinePattern)) inlineGates++;

  const unmodelled = [...keys].filter((key) => !GATED_FEATURE_KEYS.has(key));
  if (unmodelled.length > 0) {
    throw new Error(
      `index.ts gates on capability keys this test does not model: ${unmodelled.join(", ")}. ` +
        `Add them to CAPABILITY_GATED_TOOLS or CAPABILITY_GATED_RESOURCES — an unmodelled gate lands ` +
        `in the core set and makes cross-process parity flaky again.`
    );
  }
  const missingFromSource = [...GATED_FEATURE_KEYS].filter((key) => !keys.has(key));
  if (missingFromSource.length > 0) {
    throw new Error(
      `this test models capability keys index.ts no longer gates on: ${missingFromSource.join(", ")}`
    );
  }
  if (inlineGates !== INLINE_GATED_TOOL_COUNT) {
    throw new Error(
      `index.ts has ${inlineGates} inline hasFeature tool gates, this test models ${INLINE_GATED_TOOL_COUNT}. ` +
        `A gate added with an already-known feature key still needs its tool name in CAPABILITY_GATED_TOOLS.`
    );
  }
  return { keys: [...keys].sort(), inlineGates };
}
