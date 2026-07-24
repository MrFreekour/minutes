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

The foreground generation and initial-attach recovery identified by this
checkpoint are now implemented in Minutes' provider-neutral engine. Foreground
recovery preserves the exact user turn and evidence window, both attempts in
latency and telemetry, and a Minutes-owned attempt identity that rejects late
events from a replaced provider session. Initial attach gets one bounded retry
for overload, timeout, or unavailable failures and records the full ready time
and attempt count. The independent verifier also gets one foreground transport
retry on a fresh verifier session, replaying the exact candidate and evidence
seal without rerunning the strategist. Remaining reliability blockers are
repeated real-provider tail evidence and an inline recovery action if bounded
recovery is exhausted.

## Post-recovery repeated corpus

Four additional full-corpus runs exercised the three recovery layers and the
real provider lifecycle. The first passed cleanly. The second exposed a
harness-owned teardown defect: a scenario could start while a verifier, judge,
or strategist process from the prior scenario was still exiting, causing three
Codex state-runtime initialization failures. It also captured one genuine
25.525-second foreground latency tail.

The client shutdown contract now resolves only after the provider child exits,
escalates a wedged child from `SIGTERM` to `SIGKILL`, and is idempotent across
competing owners. Sidekick session, judge, provider, and verifier teardown all
propagate that completion. The verifier tracks consumed and speculative
backends until each has exited, while retaining a bounded shutdown for a
non-cooperative preparation promise. A regression deliberately withholds
backend exit and proves verifier shutdown cannot report completion early.

After that correction, two baseline runs had no provider or state errors and no
Sidekick-owned provider process survived the corpus:

| Run | End-to-end | Quality | Insights | First-token p95 | Total p95 | Provider errors |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| post-recovery 3 | 5/7 | 7/7 | 25/25 | 3.323s | 14.849s | 0 |
| post-recovery 4 | 7/7 | 7/7 | 25/25 | 3.415s | 6.222s | 0 |

Run 3 failed only the unchanged latency bar. One first draft correctly failed
independent verification and the repaired answer arrived in 14.849 seconds;
one otherwise clean answer arrived in 9.043 seconds. Run 4 passed every
behavioral and latency gate. This is evidence that teardown-induced state
contention is fixed, not statistical proof that the total-response tail is
ready.

An adversarial A/B lowered verifier reasoning from `low` to `none` without
changing any rubric or threshold. It regressed to 4/7 end-to-end, 6/7 quality,
and a 16.378-second total p95. The product default therefore remains `low`.
Faster verification must earn promotion through repeated safety and quality
evidence; the latency budget will not be weakened to accommodate it.

The complete Sidekick JavaScript bank at this checkpoint is 162/162, including
normal exit, forced exit, idempotent close, state-error classification, and
verifier lifecycle ownership.

## Provenance and decision-frame tail checkpoint

A follow-on adversarial loop targeted two repeatable first-draft defects that
caused expensive verifier retries:

- a response could mention or dismiss a live rationale while omitting its
  evidence receipt; and
- a quantitative response could correctly compute the governing exposure but
  then invert the reframe and call that same governing constraint
  non-decisive.

The evidence-ID obligation now lives in the structured output schema as well as
the instructions: referenced or rejected proposals and rationales named or
dismissed as non-decisive count as evidence use. The decision contract also
forbids dismissing the consequence it just identified as governing. The
negotiation fixture's mechanical minimum now requires only the prior boundary
and the live buyer proposal; if an answer uses the separate internal "logo"
rationale, its semantic forbidden-behavior rule still requires that exact
receipt. This removes an unconditional benchmark false negative without
weakening conditional provenance.

Targeted real-provider results:

```text
margin/provenance: 3/3 quality+latency, no retries, 5.392-6.846s total
runway/reframe:     3/3 quality+latency, no retries, 5.827-7.315s total
```

The final full corpus at the pause seam reached 7/7 graded quality, 25/25
required insights, and zero provider/state errors. It passed 6/7 end to end:
the independent verifier falsely classified a complete price-concession
candidate as `incomplete_material_consequence`, triggering a repair and a
12.833-second total. The semantic judge passed the published result. No further
real-provider runs were started to preserve the user's Codex budget.

The remaining next defect is therefore narrow and explicit: calibrate or
deterministically adjudicate false verifier completeness rejections without
bypassing unsupported-fact, contradiction, privacy, or provenance checks and
without weakening the 5s/8s latency bar. See
[`sidekick-quality-tail-handoff-2026-07-24.md`](sidekick-quality-tail-handoff-2026-07-24.md).
