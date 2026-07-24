# Sidekick Engine Replay Checkpoint — 2026-07-24

This checkpoint adds the first one-command, no-human Sidekick orchestration
gate. It is a milestone inside `minutes-k1qp.2`, not completion of that bead
and not a general SOTA claim.

## Run it

From the repository root:

```bash
bash scripts/sidekick_engine_eval.sh
```

The command runs without a microphone, screen, network, click, or typed
response. The deterministic lane has no external prerequisites. The
prerecorded-media lane requires the local `ggml-tiny.bin` model under
`~/.minutes/models/` and fails clearly rather than silently skipping if it is
missing. It writes bounded JSON artifacts to:

```text
target/sidekick-eval/live-sidekick-engine-eval.json
target/sidekick-eval/live-sidekick-media-eval.json
target/sidekick-eval/live-sidekick-vm-ui-eval.json
```

The artifacts contain no meeting text, user text, image bytes, local paths, or
provider payloads. They record only scenario IDs, pass/fail assertions,
bounded counts, hashes, timing, an explicit coverage boundary, and—only for
the deterministic lane—a reproducible SHA-256 digest.

## What passed

The checkpoint executes each scenario twice through the public production
`LiveSidekickEngine` and requires byte-equivalent normalized results:

| Scenario | Production behavior exercised |
| --- | --- |
| Exact screen publication | Exact selected PNG bytes, evidence receipt, independent verifier, publication gate |
| Exact-session screen store | Production context database/session links select one session's PNG, exclude another session, and deliver the selected bytes to Sidekick |
| Correction during verification | Moving transcript window, refreshed verification, contradiction rejection, fresh regeneration |
| Provider failure and recovery | Retryable network-class failure, capture isolation, provider epoch replacement, successful retry |
| Screen unavailable | Missing image bytes rejected, fabricated visual provenance blocked, transcript-only recovery |
| Foreground preemption | Typed user turn interrupts background work; late background completion is ignored |
| Provider steering | Active persistent turn is steered into foreground work without a second generation |
| Evidence bounds | Only the newest configured transcript items enter a request and the serialized envelope stays under budget |
| Transcript JSONL adapter | Production cursor/parser ingress, torn-line tolerance, idempotence, and speaker-track anonymization |
| Context mutation and recovery | Real prepared-brief/project assembly, restricted/ambient exclusion, stale-source rejection, and refresh |
| Participant archive grounding | Exact-person retrieval from a populated prior-meeting archive, restricted/unrelated exclusion, source receipts, and live-plus-history publication |
| Teardown | Active work and provider sessions close; a late completion has no visible effect |

Result at this checkpoint:

```text
12/12 scenarios
47/47 assertions
reproducible=true
digest=e48a0d1c27ef8d898ac0b185ac1a138ca50cded0a01820b05faca1a2cc8a8e48
```

The second lane then transcribes the committed 10.6-second spoken-meeting WAV
through Minutes' production Whisper meeting path, serializes its real timed
segments into the production live-transcript JSONL contract, reduces them
through the real Sidekick engine, and requires independently verified
publication. On the 2026-07-24 VM run:

```text
7/7 media assertions
28 recognized words
3 timed segments
WER=0.192 (tiny model, required bound <=0.50)
ASR elapsed=0.8-1.2s across checkpoint runs (non-gating)
corrupt WAV rejected=true
source-aware attribution=2 speakers / 4 alternating segments
```

ASR timing and transcript hashes are machine-dependent and are intentionally
not described as deterministic. The fixture, model, and transcript hashes are
recorded as receipts; raw transcript content is not written into the artifact.
The source-aware check generates two separate, alternating synthetic audio
stems at runtime and passes them through Minutes' production energy-based
attributor. It proves local/remote source separation, not two-human room-mic
voice clustering or identity.

The third VM lane binds the current production Sidekick/main-window markup,
frontend handlers, and acceptance evaluators to one source digest, then runs
the startup, event-order, paint, deduplication, reload-recovery, and
false-green tests headlessly:

```text
69/69 VM UI contract tests
0 failed, skipped, cancelled, or todo
source_sha256=6120f8384244668d19e2a96918212ae642db9750ae2d173abac3dd854f6e72e6
```

That lane catches the prior missing-listener `ReferenceError` class and
ordering failures without a person or Mac. It is not a signed WebView run and
does not prove native visibility, focus, permissions, or accessibility.

The deterministic backend lives only under the example/integration-test
boundary. It implements the same persistent, streaming, steerable,
provider-neutral contract as Codex app-server or a future Claude/local
backend; it is not compiled into the production Minutes engine.

## Honest coverage boundary

This checkpoint uses the real Minutes batch meeting ASR, source-aware stem
attributor, live-transcript JSONL cursor/parser adapter, exact-session context
store retrieval, historical/project `ContextCard` assembler, reducer,
evidence-window assembler, provider contract, verification gate,
suppression/publication decision, recovery, and teardown paths.

It does **not** yet exercise:

- native microphone or system-audio capture;
- live/streaming speech recognition from a native recording;
- mixed room-mic voice clustering, named-speaker identity, or live diarization;
- the native macOS Screen Recording permission adapter;
- real Codex, Claude-compatible, or local-model network behavior; or
- signed-app/WebView UI event order and accessibility.

Those remain required by `minutes-k1qp.2`. The existing signed-Mac checkpoint
and real Codex SOTA suite complement these VM lanes, but their evidence must
not be collapsed into one inflated release claim.

## Adversarial review outcome

The initial implementation placed scripted eval machinery in the production
core module. Review rejected that structure even though the scenarios passed.
The final shape keeps all deterministic provider state under
`crates/core/tests/support/`, shares it with a tiny example entry point, and
leaves the shipped provider-neutral engine untouched.

The deterministic transcript-adapter scenario begins with already-produced
synthetic JSONL. The separate media lane proves prerecorded speech recognition
and real ASR-segment ingress, but it deliberately supplies no speaker labels
from its spoken fixture. Its separate generated-stem check proves the production
local/remote attributor only; it does not turn synthetic tones into a
two-person room-mic claim. Historical-context replay uses a populated meeting
archive and an explicit participant, while excluding unrelated and restricted
files; it still does not prove calendar identity resolution or live
diarization-to-person matching.

The artifact also labels every provider duration as simulated and sets
`release_ready_from_this_report_alone=false`. A future change cannot turn this
gate green into a claim about native microphone capture, live ASR,
diarization, permissions, cloud latency, or signed-app UX.
