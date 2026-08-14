# Dictation partial-transcription gate — 2026-08-13

## Decision

Keep live transcript text hidden in the shipping HUD. None of the tested true
streaming candidates clears the complete latency and quality gate yet. The
waveform and capture state remain the immediate feedback; final transcription
and insertion behavior are unchanged.

This is deliberately fail-closed. A fast but unstable hypothesis is more
distracting than no transcript, and a low-quality first-pass model must not be
presented as state-of-the-art merely because it is technically incremental.

## Gate

- first useful partial: under 700 ms p95
- useful visible update cadence: 100–250 ms when the hypothesis changes
- decode real-time factor: at most 0.5
- punctuation-insensitive WER: at most 30% on the checked-in fixture
- required terms present and forbidden hallucination terms absent
- stable text is never rewritten; only the explicitly provisional suffix may change
- only the configured final backend may produce the single inserted result

## Measured candidates

| Candidate | First useful partial | Other evidence | Result |
| --- | ---: | --- | --- |
| Apple `DictationTranscriber`, progressive short dictation | 789 ms on the punctuated preset | Coherent final; 38 progressive events on the 10.63 s demo; `Minutes` and `Parakeet` were not recovered exactly | Miss |
| Apple `DictationTranscriber`, volatile + no punctuation | 751 ms p95 over 10 paced runs with 120 ms delivery | 713 ms best full-fixture run; same required-term misses; punctuation intentionally deferred to the final backend | Near miss |
| Sherpa online Zipformer 20M | 2,200 ms p95 | 603 ms p95 cadence, 0.009 decode RTF, 38.46% WER, missed `Minutes` | Miss |
| FluidAudio Parakeet EOU 120M, 160 ms tier | 2,914 ms | 46 updates, 604 ms maximum cadence; misheard `Wesley`, `benchmark`, and `Parakeet` | Miss |
| Whisper tiny, sub-second rolling probes | 479–879 ms including decode | Returned unstable hallucinations such as `Imagine what you do`, `Maddened West`, and `Maddened Wesley of the World` | Miss |

The Apple result is the strongest candidate and is close enough to re-evaluate
after runtime or hardware updates. It is not rounded down to satisfy the plan.
The checked-in Swift benchmark feeds real-time-paced audio instead of handing
the analyzer an entire file at once; the latter makes first-result timing look
artificially fast and does not represent microphone capture.

## Reproduction

```bash
./scripts/run_apple_speech_streaming_benchmark.sh crates/assets/demo.wav 10 120 volatile-no-punctuation

MINUTES_SHERPA_STREAMING_MODEL_DIR=/path/to/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17 \
  ./scripts/run_dictation_partial_benchmark.sh tests/eval/fixtures/dictation-benchmark-corpus.json 10
```

The Sherpa benchmark exits non-zero when the gate fails. Both reports contain
transcript text because they are explicit local evaluation tools; product
latency telemetry remains content-free.

## Re-evaluation triggers

- Apple changes `DictationTranscriber` progressive emission latency
- a cache-aware streaming model clears both the Minutes fixture and a broader private corpus
- FluidAudio materially improves first-token latency and required-term recovery
- a new local decoder supports stable prefixes without repeated whole-buffer transcription
