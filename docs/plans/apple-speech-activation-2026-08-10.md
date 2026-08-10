# Apple Speech Activation Plan

**Date:** 2026-08-10
**Depends on:** secure Apple Speech byte transport (bead minutes-hueo, merged in PR #610)
**Goal:** turn Apple Speech from a retained-but-inert preference into a working, selectable transcription engine on capable macOS 26 hardware, without ever degrading a recording when the device cannot run it.

## Status of the decisions

Mat delegated the four Phase 0 calls. They are recorded here as decided, not open:

1. Rollout is shadow-first, mirroring the proactive-gate shadow pattern already in the repo.
2. Surface order is live transcript first, then batch, then dictation.
3. The default module is `speech`, which hardware testing showed is materially better than `dictation` (correct proper nouns, punctuation, timed segments, faster).
4. Capability posture is attempt-and-fallback for all macOS 26, with no Apple Intelligence eligibility gate up front, because the process isolation makes it safe and it covers more devices.

Flipping any user-facing default still requires Mat's explicit word at the moment it happens; these decisions set direction, not a standing authorization to activate.

## Non-goals

Parakeet activation stays out of scope; it remains dormant behind its own gate. Whisper stays the default engine; this epic does not change that. Nothing here targets non-macOS or pre-26 macOS.

## What is already proven

The secure byte transport is on main and passed signed acceptance: exact authenticated bytes cross the parent-to-XPC-worker path with a matching checksum, no named plaintext file, and the product gate stays closed. On real Apple Silicon the Swift bridge produces correct transcripts at roughly 30x realtime, and the `speech` module beats `dictation`. The worker fails closed when its trust verdict is indeterminate. The gate is one boolean at `crates/core/src/pipeline.rs:1334`, flowing through `resolved_apple_speech_backend`.

One honesty caveat carried into Phase 6: the hardware transcription proof so far used a C-driver harness linking the Swift bridge, not the real Rust-linked `minutes-apple-speech-worker` binary. Closing that confound is an explicit acceptance item, not an assumption.

## The core problem this epic must solve

Runtime capability cannot be reliably predicted before attempting. The eight-probe investigation (recorded in the lane evidence FINDINGS.md) established that symbols-present plus framework-loaded does not equal "works": a hosted VM without Speech assets aborts the worker inside `swift_getTypeByMangledName`, while the same code succeeds on real hardware. Some real user Macs (macOS 26 but not Apple-Intelligence-eligible, or with assets not installed) will land in the failing state. So activation cannot be a static "on for macOS 26" switch.

The resolution is architectural and already in place: the worker is a separate XPC process, so its crash is failure-isolated. When the analyzer aborts, the parent receives XPC_ERROR_CONNECTION_INTERRUPTED and falls back to Whisper for that utterance. This is exactly the RFC 0004 failure-isolation boundary and the standing "recording must never be degraded by an optional consumer" decision, applied to engine selection. Activation therefore does not need perfect prediction; it needs attempt-and-recover plus a cached verdict so it never crashes twice for the same reason.

## Phases

### Phase 1: safe runtime fallback

Make the gate conditional rather than a static false, and make every surface degrade to Whisper transparently when Apple Speech cannot transcribe. Per utterance: attempt the worker; on XPC interruption (worker crash), a Speech error, or `runtimeSupported: false`, fall back to Whisper with no lost utterance and WAV preserved. Cache the verdict per session so a first failure switches the session to Whisper instead of re-crashing. Acceptance for this phase proves the fallback itself: with Speech forced unavailable, the utterance still transcribes via Whisper. This is the load-bearing safety work; every later phase depends on it.

### Phase 2: asset provisioning

Detect missing assets (the "not subscribed to transcription.en" case), trigger `AssetInventory.assetInstallationRequest` for the locale, and drive the `.downloading` lifecycle (the status string already exists). While assets download, fall back to Whisper; switch to Apple Speech once ready. Asset download must never block recording.

### Phase 3: capability detection and caching

A durable per-machine verdict (usable, downloading, or unavailable) computed by attempt-once-and-cache, surfaced through the health check honestly. This replaces the dlsym probe, which the investigation showed cannot distinguish the failing case because the symbols are present even where the worker aborts.

### Phase 4: surface integration and the attribution question

Wire live transcript, then batch, then dictation to the safe-fallback path, in that order. Open investigation item to resolve inside this phase, not assume: how Apple Speech segments interact with pyannote diarization and the `speaker_map` attribution. Apple Speech emits its own timed segments; the plan must verify they feed the existing diarize-plus-attribution pipeline correctly, and the standing "a wrong rewrite is worse than none" attribution invariant must still hold.

### Phase 5: config, health, docs honesty

Flip config comments, health output, and docs from "resolves to Whisper" to conditional real availability. Health should read like "Apple Speech: available", "downloading assets, using Whisper", or "unavailable on this device, using Whisper".

### Phase 6: shadow, acceptance, and adversarial review

Ship shadow mode first: Apple Speech runs alongside Whisper, results logged and compared, nothing user-facing, to measure the real-world capability distribution and the quality delta before any exposure. Extend acceptance to prove the fallback path and to prove the real Rust-linked worker binary transcribes on hardware (closing the C-driver confound), using the tailnet Apple Silicon device. Codex adversarial review gates the merge before it lands, per the review-discipline standard. Only after shadow data supports it does opt-in ship (`engine = "apple-speech"`, Whisper still default), and a default change is a later, separate decision.

## Risks

Incapable user Macs, mitigated by process-isolation fallback plus session caching. Per-utterance crash cost on incapable machines, mitigated by caching the first failure. Diarization and attribution interaction, unresolved and owned by Phase 4. Asset-download UX blocking recording, addressed by the explicit non-blocking requirement. Audience limited to macOS 26, which is inherent.

## Acceptance criteria for the epic

Apple Speech transcribes real audio on capable hardware through the full product path; an incapable device transparently falls back to Whisper with no lost utterance and no crash reaching the user; assets install on demand without blocking recording; health and docs report true state; Whisper remains default; shadow data exists before any exposure; and codex review passed before merge.
