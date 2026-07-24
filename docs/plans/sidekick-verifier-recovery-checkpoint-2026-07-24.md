# Sidekick Independent-Verifier Recovery Checkpoint — 2026-07-24

This checkpoint closes the foreground verifier transport gap in
`minutes-k1qp.2`. If the independent semantic evidence verifier loses its
provider connection during a typed user turn, Minutes now opens one fresh
verifier session and replays the exact candidate and immutable evidence seal.

The strategist is not rerun and its persistent session is not replaced.
Codex app-server remains one implementation of the provider-neutral verifier
contract; Minutes owns the attempt budget, evidence seal, stale-event defense,
latency accounting, and publish decision.

## Recovery contract

- Only foreground verifier overload, timeout, and unavailable failures qualify.
- Authentication, protocol, invalid-request, background, and second failures
  fail closed.
- The retry uses a fresh independent verifier session.
- The candidate, typed user message, invocation, and bounded transcript,
  context, and exact-screen evidence bytes are byte-equivalent across attempts.
- A Minutes-owned verifier-attempt identity rejects delayed events even if the
  old and new provider sessions reuse the same provider-local turn ID.
- Failed-attempt and recovery wall time remain in total publication latency.
- The evidence-verification receipt exposes one or two provider attempts.

If transcript or relevant screen evidence changes during the outage, Minutes
does not relabel the replayed old window as current. It lets that exact retry
settle, then requires a fresh verification against the newer evidence before
publication.

## Harness coverage

Focused engine tests cover:

- asynchronous unavailable failure and exact-seal replay;
- synchronous verifier-turn start timeout;
- same-provider-turn-ID late completion from the failed session;
- transcript mutation during the transport outage;
- two transport failures with no third attempt;
- background failure with no retry; and
- a protocol error marked retryable with no retry.

The deterministic no-human Sidekick harness now contains a separate
`verifier_failure_and_recovery` scenario. It proves that the verifier session
is replaced while the strategist session remains stable, the candidate and
evidence request are exactly replayed, and the visible publication carries a
two-attempt verifier receipt.

Checkpoint result:

```text
minutes-core no-default=1198 passed, 1 ignored
pipeline integration=8/8
13/13 deterministic scenarios
52/52 deterministic assertions
reproducible=true
digest=05cf6cac524b63b12f88cce9b8f953dbd1fa9904622a4e1910d69001a653b45a
focused Sidekick engine tests=48/48
native diagnostic CLI tests=10/10
media replay=7/7, WER .192, 2 source speakers / 4 segments
VM UI contracts=69/69
```

## Honest boundary

This is deterministic VM evidence. It does not prove provider-tail reliability
at meaningful sample size, signed macOS recovery UX, native capture, live ASR,
mixed room-mic diarization, accessibility, or general release readiness.
Repeated real-provider corpus runs and the signed product path remain separate
gates.
