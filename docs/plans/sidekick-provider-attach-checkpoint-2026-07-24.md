# Sidekick Provider Attach Recovery Checkpoint — 2026-07-24

This checkpoint closes one startup-reliability gap in `minutes-k1qp.2`.
Minutes now gives initial Sidekick provider attachment one bounded automatic
recovery when the provider reports overload, timeout, or unavailability.

This is provider-neutral engine behavior. Codex app-server is the current
implementation under the interface; the retry policy, attempt budget, ready
state, latency receipt, and failure decision remain owned by Minutes.

## User-visible behavior

- A routine attach starts once and becomes ready as before.
- One temporary provider transport failure is retried without asking the user
  to stop Sidekick, click again, or retype anything.
- Authentication and malformed-provider failures are not disguised as
  transient outages.
- A second temporary failure stops safely; there is no unbounded startup loop.
- Capture state is reduced once. Recovery replaces only the reasoning
  attachment and does not create a second meeting or capture session.

The ready receipt exposes `reasoning_ready_attempts` as either one or two.
`reasoning_ready_ms` includes the entire attach path, including the failed
attempt. The native diagnostic and signed-app acceptance receipt distinguish a
provider session replaced before ready from one replaced during real user
turns. Persistent-turn proof now requires the recovered ready-session identity
and session count to remain stable across the exercised turns.

## Automated evidence

The focused engine suite covers:

- one unavailable startup failure followed by a successful attach;
- authentication failure with no retry;
- a protocol failure marked retryable with no retry;
- two transport failures with exactly two attempts and no third attempt; and
- the ordinary one-attempt attach path.

The native diagnostic parser accepts a two-attempt recovered attach only when
the final provider session stays stable for every fixture turn.

VM gates at this checkpoint:

```text
minutes-core no-default: 1193 passed, 1 ignored
pipeline integration: 8/8 passed
deterministic Sidekick engine eval: 1/1 passed
Sidekick reducer eval: 1/1 passed
focused Sidekick engine tests: 43/43 passed
native diagnostic CLI tests: 10/10 passed
strict no-default minutes-core clippy: passed
minutes-app no-default check: passed with pre-existing Linux warnings
fmt and diff check: passed
```

## Adversarial review

The retry classifier is intentionally narrower than the provider's generic
`retryable` flag. Only overload, timeout, and unavailable transport classes
qualify. A provider cannot cause repeated startup work merely by marking an
authentication, invalid-request, or protocol failure retryable.

The acceptance definition was also changed from the brittle assertion that
exactly one provider session had ever existed. That assertion would reject a
legitimate pre-ready recovery. The harness now records the provider session at
ready and fails if its identity or successful-session count changes during the
real turns.

## Honest boundary

This checkpoint proves deterministic orchestration and fail-closed receipts on
the VM. It does not establish the required 99% successful-start rate, a
statistically meaningful startup-latency tail, native macOS behavior, or signed
recovery UX. It also does not recover a provider transport failure inside the
independent semantic-verifier lane; that is the next reliability slice.

