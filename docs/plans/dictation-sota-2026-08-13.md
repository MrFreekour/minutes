# Dictation SOTA Plan — 2026-08-13

## Product thesis

Minutes dictation should feel like a small, excellent local writing instrument:

> Hold, speak naturally, release, and the right text is already at the cursor.
> Silence is safe. Corrections are reversible. Nothing is lost.

The product does not win by exposing more speech-engine controls. It wins by
being instant, physically legible, context-aware, honest about what happened,
and uniquely trustworthy because raw capture, cleanup provenance, insertion
outcomes, and recovery remain local.

This plan extends the existing dictation platform contracts and preserves the
proven rule that uncertain rewriting is worse than no rewrite.

## Current evidence

The implementation already has valuable foundations:

- local Whisper partials with optional Apple Speech or Parakeet finalization
- deterministic cleanup with conservative filler removal
- unified hold-to-talk and tap-to-lock shortcut handling
- cursor insertion with `typed`, `pasted`, `copied`, and `blocked` outcomes
- clipboard restoration discipline
- durable local dictation history with raw and cleaned text
- a compact overlay, honest permission fallback, model recovery, and reduced motion

Local dogfood telemetry collected before this plan shows:

- listening emitted in 72 ms median and 147 ms p95 across 34 measured starts
- 33 of 34 measured runs used an already-warm model cache
- release-to-insertion at 1.29 seconds median and 2.37 seconds p95 across 19 measured releases
- 24 recent dictations: 7 verified `typed`, 17 unverified `pasted`
- the majority of recent targets were Ghostty, making terminal and agent-prompt
  dictation a first-class product case rather than an edge case

The first visible overlay frame currently says `Loading model...`. That state is
misleading on warm starts and can outlive the real warmup because the overlay
WebView is rebuilt for every dictation while backend events begin immediately.
If the page listener attaches after `loading` or `listening` was emitted, the
static first frame remains visible until a later event arrives.

## Interaction contract

### Invocation and capture

| Moment | Product behavior | Target |
| --- | --- | --- |
| Shortcut press | HUD appears without stealing focus; capture mode is known | press-to-HUD under 50 ms p95 |
| Ready | A dry cue and truthful waveform confirm that microphone capture is active | press-to-listening under 200 ms p95 |
| Speech | Stable partial text appears only when an engine clears the partial-quality gate | first useful partial under 700 ms p95 |
| Pause | Quiet is neutral; the UI never reports a dead microphone merely because the user is thinking | no false error during ordinary pauses |
| Release or explicit stop | Final text is produced once and committed once | release-to-visible-text under 750 ms p95 |
| Completion | Inserted text is the confirmation; the HUD acknowledges quietly and disappears | routine success visible for roughly 300 ms |
| Failure | Captured words remain recoverable and one relevant next action is offered | zero silent loss |

### Gesture semantics

The shortcut keeps its efficient dual behavior, but the mode must be visible:

- Holding begins hold-to-talk. The HUD says `Dictating` and release finishes.
- A quick tap visibly latches the session. The HUD says `Locked · tap fn to finish`.
- A second tap finishes a locked session.
- Escape cancels the active session without inserting text.
- Captures shorter than the accidental-input threshold disappear without
  producing history noise.
- Hold-to-talk never ends merely because the user pauses before releasing.
- Locked mode uses manual stop by default. A long safety timeout may remain,
  but the current two-second post-speech timeout is not a locked-mode stop rule.

### Overlay state model

The overlay renders an authoritative replayable snapshot rather than depending
on best-effort event timing:

1. `starting` — a neutral, sub-perceptual transition; no model jargon and no
   claim that the microphone is already active.
2. `listening` — microphone is active and the user may speak.
3. `dictating_hold` or `dictating_locked` — speech is being captured and the
   finishing gesture is unambiguous.
4. `processing` — shown only when finalization exceeds the immediate-response
   threshold.
5. `inserting` — shown only when insertion exceeds the immediate-response threshold.
6. `typed`, `pasted`, or `copied` — the exact outcome, never a generic success.
7. `blocked`, `failed`, or `recoverable` — plain explanation plus one action.

Every state change updates backend-owned snapshot state before emitting a UI
event. When the WebView becomes ready, it requests and renders the current
snapshot, closing the existing missed-event race.

## Writing quality contract

### Conservative local cleanup

The default stays deterministic and meaning-preserving:

- normalize whitespace and punctuation spacing
- capitalize sentences and standalone `I`
- remove only unambiguous vocalized pauses
- retain the raw transcript whenever cleaned output differs
- never silently apply a low-confidence proper-name or vocabulary rewrite

### Context modes

Minutes may infer a sparse local text-mode hint from the focused target:

- `terminal_code`: literal identifiers, minimal punctuation intervention, no
  sentence-style capitalization that damages commands or code
- `agent_prompt`: readable prose while preserving paths, flags, identifiers,
  and line structure
- `chat`: light punctuation and filler cleanup
- `email_document`: paragraphs, lists, and natural punctuation
- `unknown`: conservative general prose

Surrounding-text access is separately permissioned, local, minimal, and never a
requirement for core dictation.

### Explicit corrections first

Phase one supports unambiguous commands such as `scratch that`, `new line`,
`new paragraph`, `bullet`, and confirmed spelling or snippet commands.

The bare word `actually` is always treated as ordinary dictated content. It is
not a command and does not trigger deletion or replacement.

Semantic backtracking such as `meet at two — actually, three` is a later,
evaluation-gated capability. It may ship only when all of the following hold:

- the system identifies a compatible correction target with high confidence
- the rewrite is shown or otherwise trivially reversible
- raw audio and raw text remain available for restore
- a one-command undo restores the pre-correction result
- an adversarial corpus demonstrates that intentional uses of `actually` stay literal

Until that bar is met, explicit `scratch that` or `no, make that ...` commands
are preferable to clever ambiguity.

## Workstreams and dependency order

### Wave 1 — Make the core loop feel instant and unambiguous

**Instant-start truth.** Replace `Loading model...` with the replayable overlay
state contract. Measure WebView readiness and warm/cold engine paths. Delay
technical preparation copy until a real wait exceeds 250 ms; never expose
backend names in the routine HUD.

**Calm capture semantics.** Routine dictation and meeting recording use a
dedicated cool capture signal rather than error red. Persistent labels, elapsed
time, waveform activity, tray state, and the stop affordance carry recording
truth. Red remains reserved for errors, destructive controls, and exceptional
attention. Avoid redundant pulsing or breathing when the waveform already
proves that audio is active.

**Gesture and silence correctness.** Carry capture style into the dictation
runtime and overlay. Separate hold, locked, and automatic-stop semantics. Treat
ordinary silence as neutral and reserve microphone errors for actual device or
permission evidence.

**Insertion latency.** Make native macOS Accessibility insertion the preferred
path when supported. Keep clipboard paste and copy as explicit fallbacks.
Insertion should become visible before nonessential clipboard restoration or
verification bookkeeping completes, without weakening clipboard safety.

**Latency observability.** Record press-to-HUD, press-to-listening,
speech-to-first-partial, release-to-final, final-to-insert, and insertion outcome
with engine, target class, and warm/cold provenance. Do not store dictated text
in telemetry.

### Wave 2 — Earn live feedback and excellent writing

**Partial-quality gate.** Benchmark a genuinely incremental local path. Do not
put two-second batch-style partials back on screen. Stable-prefix behavior, CPU
cost, and long-utterance degradation are release criteria alongside first-partial latency.

**Context-aware deterministic formatting.** Introduce the text-mode contract,
starting with terminal, agent prompt, chat, document, and unknown. Keep target
detection sparse and local.

**Explicit voice editing.** Add conservative editing and formatting commands,
snippets, and confirmed vocabulary teaching. Semantic `actually` backtracking
remains behind its own evaluation and reversibility gate.

### Wave 3 — Make trust and recovery effortless

**Recovery and reprocessing.** Preserve audio ephemerally until successful
insertion, allow retry without re-speaking, expose raw-versus-cleaned restore,
and add copy, re-paste, and reprocess-last-dictation actions outside Settings.

**Settings distillation.** The everyday surface contains shortcut, destination,
writing style, recent-history policy, and microphone. Engines, model sizes,
silence milliseconds, cleanup internals, and daily-note duplication move to Advanced.

**HUD ergonomics and accessibility.** Make the HUD movable and remember its
edge position; avoid covering send buttons and terminal prompts. Keep motion
restrained, sounds independently configurable, state changes screen-reader
legible, partial revisions non-chatty, and high contrast/reduced motion complete.

### Wave 4 — Platform parity and release proof

**Platform capability honesty.** macOS, Windows, X11, Wayland, and headless
paths report only proven insertion capabilities. `Typed`, `Pasted`, and `Copied`
remain distinct everywhere.

**Dogfood and promotion gate.** Run a private corpus and app matrix covering
short prompts, long prose, technical names, paths, self-corrections, quiet
pauses, AirPods, built-in microphone, sleep/wake, target switching, permission
loss, and process interruption. Ship defaults only after the metric gates below pass.

## Quality gates

- press-to-listening under 200 ms p95 on a warm supported Mac
- first useful partial under 700 ms p95 when partials are enabled
- release-to-visible-text under 750 ms p95
- successful insertion above 99.5% across the ten most-used supported target apps
- unverified paste fallback below 5% on supported macOS controls
- no duplicate insertion, stale-target insertion, clipboard loss, or silent dictation loss
- no false microphone error during representative thinking pauses
- no default engine switch without dictation-specific WER, latency, required-term,
  forbidden-term, punctuation, and hallucination evidence
- no ambiguous correction default without literal-`actually` adversarial coverage
- a full week of dogfood without needing Settings or wondering whether Minutes heard the user

## Release strategy

Each wave lands behind existing conservative defaults where needed. Behavioral
changes receive focused unit and invariant coverage, the dictation benchmark,
and an installed `Minutes Dev.app` click/feel test using the stable signed
development identity. UI work is not complete based on HTML inspection or Rust
tests alone.

The first shipped slice is instant-start truth because it removes a misleading
state from every use while establishing the replayable state foundation needed
by gesture, partial, recovery, and latency work.
