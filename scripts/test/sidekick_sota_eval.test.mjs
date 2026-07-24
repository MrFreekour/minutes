import assert from "node:assert/strict";
import test from "node:test";
import {
  assertDistinctSotaJudgeModels,
  buildSidekickSotaEvalPlan,
  scoreSidekickSotaLatency,
  sidekickSotaExitCode,
  summarizeSidekickSotaResults,
} from "../sidekick_sota_eval.mjs";
import {
  loadSidekickSotaFixtures,
} from "../lib/sidekick_sota_fixture.mjs";

const FIXTURE_DIRECTORY = new URL(
  "../../tests/fixtures/sidekick_sota/v1/",
  import.meta.url,
);

test("the autonomous SOTA runner executes only current production-path scenarios by default", async () => {
  const fixtures = await loadSidekickSotaFixtures(FIXTURE_DIRECTORY);
  const plan = buildSidekickSotaEvalPlan(fixtures);
  assert.deepEqual(plan.counts, {
    total: 7,
    matched: 7,
    runnable: 7,
    skipped: 0,
  });
  assert.ok(
    plan.runnable.every(
      ({ fixture }) => fixture.execution.status === "executable",
    ),
  );
  assert.deepEqual(plan.skipped, []);
});

test("repository context scenario runs through the current production evidence contract", async () => {
  const fixtures = await loadSidekickSotaFixtures(FIXTURE_DIRECTORY);
  const scenario = "synthetic-repository-release-boundary";
  assert.deepEqual(
    buildSidekickSotaEvalPlan(fixtures, { scenario }).counts,
    { total: 7, matched: 1, runnable: 1, skipped: 0 },
  );
});

test("unknown scenario names fail before provider startup", async () => {
  const fixtures = await loadSidekickSotaFixtures(FIXTURE_DIRECTORY);
  assert.throws(
    () => buildSidekickSotaEvalPlan(fixtures, { scenario: "missing" }),
    /unknown Sidekick SOTA scenario/,
  );
});

test("candidate and semantic judge model identities must be distinct", () => {
  assert.throws(
    () =>
      assertDistinctSotaJudgeModels({
        strategistModel: "model-a",
        judgeModel: "model-a",
      }),
    /must be distinct/,
  );
  assert.doesNotThrow(() =>
    assertDistinctSotaJudgeModels({
      strategistModel: "model-a",
      judgeModel: "model-b",
    }),
  );
});

test("partial corpus success requires an explicit non-release opt-in", () => {
  const aggregate = {
    behavioral_path_all_passed: true,
    full_corpus_passed: false,
  };
  assert.equal(sidekickSotaExitCode(aggregate), 1);
  assert.equal(sidekickSotaExitCode(aggregate, { allowPartial: true }), 0);
  assert.equal(
    sidekickSotaExitCode({
      behavioral_path_all_passed: true,
      full_corpus_passed: true,
    }),
    0,
  );
});

test("foreground latency is a fail-closed part of the SOTA result", async () => {
  const fixtures = await loadSidekickSotaFixtures(FIXTURE_DIRECTORY);
  const fixture = fixtures.find(
    ({ fixture: candidate }) =>
      candidate.id === "synthetic-runway-hiring-tradeoff",
  ).fixture;
  const fast = scoreSidekickSotaLatency({
    fixture,
    latencies: {
      runway_decision: { first_token_ms: 1_200, total_ms: 4_800 },
    },
  });
  assert.equal(fast.passed, true);

  const slow = scoreSidekickSotaLatency({
    fixture,
    latencies: {
      runway_decision: { first_token_ms: 1_200, total_ms: 17_000 },
    },
  });
  assert.equal(slow.passed, false);
  assert.equal(slow.total_p95_ms, 17_000);

  const missing = scoreSidekickSotaLatency({ fixture, latencies: {} });
  assert.equal(missing.passed, false);
});

test("aggregate reporting distinguishes quality coverage, provider capacity, and latency tails", () => {
  const aggregate = summarizeSidekickSotaResults({
    results: [
      {
        fixture_id: "fast-quality-pass",
        passed: true,
        quality_passed: true,
        latency: {
          passed: true,
          checks: [{ first_token_ms: 2_000, total_ms: 4_000 }],
        },
        semantic: { insights: { passed: 3, total: 3 } },
      },
      {
        fixture_id: "slow-quality-pass",
        passed: false,
        quality_passed: true,
        latency: {
          passed: false,
          checks: [{ first_token_ms: 2_500, total_ms: 12_000 }],
        },
        semantic: { insights: { passed: 2, total: 2 } },
      },
      {
        fixture_id: "missing-measurements",
        passed: false,
        quality_passed: true,
      },
      {
        fixture_id: "capacity-error",
        passed: false,
        error: "Selected model is at capacity. Please try a different model.",
      },
      {
        fixture_id: "timeout-error",
        passed: false,
        error: "thread/start timed out after 60000 ms",
      },
      {
        fixture_id: "harness-error",
        passed: false,
        error: "fixture parser exploded",
      },
      {
        fixture_id: "state-error",
        passed: false,
        error:
          "failed to initialize sqlite state runtime: failed to initialize state runtime",
      },
    ],
    planCounts: { matched: 7, total: 7 },
  });

  assert.equal(aggregate.graded, 3);
  assert.equal(aggregate.graded_quality_passed, 3);
  assert.equal(aggregate.graded_quality_pass_rate, 1);
  assert.equal(aggregate.quality_coverage_complete, false);
  assert.equal(aggregate.insight_coverage_complete, false);
  assert.equal(aggregate.quality_passed, false);
  assert.equal(aggregate.latency_passed, false);
  assert.deepEqual(aggregate.latency_failure_scenarios, [
    "slow-quality-pass",
    "missing-measurements",
  ]);
  assert.equal(aggregate.scenario_execution_error_count, 4);
  assert.deepEqual(aggregate.scenario_execution_errors, [
    { fixture_id: "capacity-error", category: "provider_capacity" },
    { fixture_id: "timeout-error", category: "provider_timeout" },
    { fixture_id: "harness-error", category: "scenario_execution" },
    { fixture_id: "state-error", category: "provider_state" },
  ]);
  assert.equal(aggregate.provider_error_count, 3);
  assert.equal(aggregate.provider_capacity_error_count, 1);
  assert.deepEqual(aggregate.provider_error_scenarios, [
    { fixture_id: "capacity-error", category: "provider_capacity" },
    { fixture_id: "timeout-error", category: "provider_timeout" },
    { fixture_id: "state-error", category: "provider_state" },
  ]);
  assert.equal(aggregate.required_insight_rate, 1);
  assert.equal(aggregate.behavioral_path_all_passed, false);
  assert.equal(aggregate.full_corpus_passed, false);
  assert.match(aggregate.release_blockers.join("\n"), /scenario execution lane/);
  assert.match(aggregate.release_blockers.join("\n"), /latency budget/);
  assert.match(aggregate.release_blockers.join("\n"), /did not cover every/);
  assert.match(aggregate.release_blockers.join("\n"), /missing or malformed/);
});

test("aggregate reporting passes only a fully graded, full-corpus behavioral path", () => {
  const aggregate = summarizeSidekickSotaResults({
    results: [
      {
        fixture_id: "quality-pass",
        passed: true,
        quality_passed: true,
        latency: {
          passed: true,
          checks: [{ first_token_ms: 2_000, total_ms: 4_000 }],
        },
        semantic: { insights: { passed: 3, total: 3 } },
      },
    ],
    planCounts: { matched: 1, total: 1 },
  });

  assert.equal(aggregate.quality_coverage_complete, true);
  assert.equal(aggregate.insight_coverage_complete, true);
  assert.equal(aggregate.quality_passed, true);
  assert.equal(aggregate.latency_passed, true);
  assert.equal(aggregate.behavioral_path_all_passed, true);
  assert.equal(aggregate.full_corpus_passed, true);
  assert.deepEqual(aggregate.release_blockers, [
    "capture and diarization are not exercised by this behavioral replay",
  ]);
});
