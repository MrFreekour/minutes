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
| Correction during verification | Moving transcript window, refreshed verification, contradiction rejection, fresh regeneration |
| Provider failure and recovery | Retryable network-class failure, capture isolation, provider epoch replacement, successful retry |
| Screen unavailable | Missing image bytes rejected, fabricated visual provenance blocked, transcript-only recovery |
| Foreground preemption | Typed user turn interrupts background work; late background completion is ignored |
| Provider steering | Active persistent turn is steered into foreground work without a second generation |
| Evidence bounds | Only the newest configured transcript items enter a request and the serialized envelope stays under budget |
| Transcript JSONL adapter | Production cursor/parser ingress, torn-line tolerance, idempotence, and speaker-track anonymization |
| Context mutation and recovery | Real prepared-brief/project assembly, restricted/ambient exclusion, stale-source rejection, and refresh |
| Teardown | Active work and provider sessions close; a late completion has no visible effect |

Result at this checkpoint:

```text
10/10 scenarios
37/37 assertions
reproducible=true
digest=0689cac48091143fa7edcf74fa65cc1ce44520d3adfe9ce5e23b5e8d932e5a56
```

The second lane then transcribes the committed 10.6-second spoken-meeting WAV
through Minutes' production Whisper meeting path, serializes its real timed
segments into the production live-transcript JSONL contract, reduces them
through the real Sidekick engine, and requires independently verified
publication. On the 2026-07-24 VM run:

```text
6/6 media assertions
28 recognized words
3 timed segments
WER=0.192 (tiny model, required bound <=0.50)
ASR elapsed=776ms
corrupt WAV rejected=true
```

ASR timing and transcript hashes are machine-dependent and are intentionally
not described as deterministic. The fixture, model, and transcript hashes are
recorded as receipts; raw transcript content is not written into the artifact.

The deterministic backend lives only under the example/integration-test
boundary. It implements the same persistent, streaming, steerable,
provider-neutral contract as Codex app-server or a future Claude/local
backend; it is not compiled into the production Minutes engine.

## Honest coverage boundary

This checkpoint uses the real Minutes batch meeting ASR, live-transcript JSONL
cursor/parser adapter, historical/project `ContextCard` assembler, reducer,
evidence-window assembler, provider contract, verification gate,
suppression/publication decision, recovery, and teardown paths.

It does **not** yet exercise:

- native microphone or system-audio capture;
- live/streaming speech recognition from a native recording;
- two-speaker diarization;
- the native macOS Screen Recording permission adapter;
- participant-scoped historical search against a populated meeting archive;
- real Codex, Claude-compatible, or local-model network behavior; or
- signed-app UI event order.

Those remain required by `minutes-k1qp.2`. The existing signed-Mac checkpoint
and real Codex SOTA suite complement this deterministic lane, but the three
artifacts must not be collapsed into one inflated release claim.

## Adversarial review outcome

The initial implementation placed scripted eval machinery in the production
core module. Review rejected that structure even though the scenarios passed.
The final shape keeps all deterministic provider state under
`crates/core/tests/support/`, shares it with a tiny example entry point, and
leaves the shipped provider-neutral engine untouched.

The deterministic transcript-adapter scenario begins with already-produced
synthetic JSONL. The separate media lane now proves prerecorded speech
recognition and real ASR-segment ingress, but it deliberately supplies no
speaker labels and therefore proves nothing about diarization. The
project-context scenario proves allowlisted repository root files plus a
prepared brief; participant-scoped archive search with populated meeting
history remains separate.

The artifact also labels every provider duration as simulated and sets
`release_ready_from_this_report_alone=false`. A future change cannot turn this
gate green into a claim about native microphone capture, live ASR,
diarization, permissions, cloud latency, or signed-app UX.
