# Sidekick Real-Provider Checkpoint — 2026-07-24

This checkpoint exercises the production Sidekick prompt, persistent-session
orchestration, bounded evidence contract, independent evidence verifier, and
semantic judge against real Codex app-server sessions. It is quality and
reliability evidence for `minutes-k1qp.2`; it is not a release-ready or general
SOTA claim.

## Run it

From the repository root:

```bash
node scripts/sidekick_sota_eval.mjs \
  --output target/sidekick-eval/real-provider-sota.json
```

The default corpus has seven executable synthetic scenarios across agenda
silence, incident response, pricing contradiction, repository release
boundaries, restricted-context injection, quantitative runway strategy, and
speaker/history misidentification.

The strategist uses the configured realtime Codex Fast backend. Each visible
answer must also pass a separate evidence verifier and a distinct-model
semantic judge. The runner attests the provider executable before and after
the corpus and fails closed if its path, version, or bytes change.

## Results

The first full run exposed three provider-capacity errors and two completed
turns above the eight-second total-response budget. Every scenario that
reached grading passed its mechanical and semantic gates:

```text
2/7 complete end-to-end passes
4/4 graded scenarios passed quality
14/14 required insights found
first-token p95=3.170s
total-response p95=12.589s
3 provider-capacity errors
2 graded latency failures
```

Each of the five failed scenarios was then rerun once, independently. All five
passed quality and latency:

```text
5/5 complete end-to-end passes
19/19 required insights found
first token=2.224-4.338s
total response=4.805-6.224s
```

A second full-corpus run reproduced the distinction between coaching quality
and provider reliability:

```text
5/7 complete end-to-end passes
6/6 graded scenarios passed quality
21/21 required insights found
first-token p95=3.037s
total-response p95=10.834s
1 provider session-start timeout at 60s
1 graded latency failure
```

No completed scenario failed the quality rubric across these runs. That is
strong evidence that the current reasoning behavior generalizes beyond
Meridian. It is not reliability evidence sufficient for release: a meeting
assistant cannot make the user retry manually after a provider-capacity event
or wait 60 seconds to learn that a session did not start.

## False-Green Hardening

The report schema is now version 2. It separately records:

- how many attempted scenarios reached quality grading;
- quality pass rate over the graded subset;
- whether quality coverage was complete;
- scenario-execution errors and classified provider failures;
- exact quality- and latency-failure scenario IDs;
- provider-capacity error count;
- insight coverage and latency percentiles; and
- strict behavioral-path and full-corpus pass states.

A run with perfect quality on completed scenarios still fails when another
scenario never reaches grading or misses its latency budget. Regression tests
cover the mixed case of a fast quality pass, a slow quality pass, a provider
capacity failure, a provider timeout, and a non-provider harness failure.

## Honest Coverage Boundary

This lane proves real provider sessions, production prompts, persistent turns,
bounded evidence, independent verification, semantic grading, provider binary
attestation, and observed provider latency.

It does not prove:

- native microphone or system-audio capture;
- live ASR, mixed room-mic diarization, or person identity;
- the native Screen Recording permission adapter;
- signed-WebView paint, focus, accessibility, or recovery;
- Claude-compatible or local-model quality;
- 99% start reliability or the required latency tail over a statistically
  meaningful repeated corpus; or
- blind preference against the production terminal-Codex baseline.

The foreground generation recovery identified by this checkpoint is now
implemented in Minutes' provider-neutral engine: one retry preserves the exact
user turn and evidence window, both attempts in latency and telemetry, and a
Minutes-owned attempt identity that rejects late events from a replaced
provider session. Remaining reliability blockers are fast failure and bounded
recovery during initial attach, equivalent provider-failure handling in the
independent verifier lane, and an inline recovery action if the bounded turn
retry fails.
