# Dictation semantic correction gate — 2026-08-13

## Decision

Do not ship implicit `actually` backtracking. Bare `actually` remains literal.
The shipped correction path is the explicit, whole-utterance command grammar
(`scratch that` and confirmed spelling), with raw and pre-command restoration.

## Promotion bar

An implicit semantic rewrite is allowed only with zero false rewrites in the
curated adversarial corpus, at least 90% recall on genuine corrections, and at
least 1,000 reviewed cases spanning accents, engines, target modes, and sentence
structures. Every promoted rewrite must also retain raw text and one-action
recovery. A small synthetic corpus can reject a candidate, but cannot prove the
required false-rewrite rate.

## Candidate evaluated

The offline-only candidate recognizes `actually,` after an em dash or hyphen
when followed by one to five words. It is intentionally not linked into the
dictation runtime.

Run:

```bash
cargo run -p minutes-core --example dictation_semantic_correction_eval \
  --no-default-features --features whisper,streaming
```

Observed on the checked-in 32-case corpus:

- genuine corrections: 12
- non-corrections: 20
- correction recall: 100%
- false rewrites: 9 of 20 (45%)
- promotion passed: false

The failures include discourse markers, quoted text, technical qualification,
and multiple possible antecedents. Punctuation plus word count cannot determine
what the speaker intended to replace. The result is far below the zero-false-
rewrite safety bar, and the corpus is also far smaller than the minimum evidence
set. The candidate therefore stays evaluation-only.

Fixture: `docs/evals/fixtures/dictation-semantic-corrections.json`.
