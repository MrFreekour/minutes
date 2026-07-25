# Privacy-B lane handoff (refreshed 2026-07-25)

You are taking over the **conversation-trust privacy epic, Slice B integration**
lane. The previous lane ran to ~723k context and was retired deliberately; this
document plus bead **minutes-ew09** is how you pick up without loss.

- Worktree: `~/Sites/minutes-conversation-trust-privacy-b-v4`
- Branch: `integrate/conversation-trust-privacy-b-v4`
- **Never push, merge, tag, sign, or release.** Steady-background only.
- Durable state and full verdict history: `bd show minutes-ew09`. Read it. The
  notes there are the authority; this file is the orientation.

## Read this first: how work gets accepted here

A block is only a checkpoint after **three independent blind reviews ACCEPT with
zero P0 and zero P1**. Reviews are run by spawning three subagents with distinct
lenses against an exact SHA, told not to trust the commit message. Nine reviews
were run on track 1 across three rounds and six on track 2; every one returned
REJECT. That is not unusual for this lane and is not a reason to lower the bar.

**The single most important lesson, learned expensively:** across every round,
the recurring finding was not a class of bug, it was claims written before they
were verified. Commit messages asserted behaviour the code did not have, and
tests were written in the shape of the claim so they could not fail. Three
separate reviewers independently proved that reverting a "fix" left the suite
green.

The only procedure that reliably catches this, and the one this lane now
follows: **break the code first, watch the test fail, then restore it.** Not
"write a test" — prove the test fails. Every fix in the two most recent commits
was mutation-verified that way, and the reviewers who re-checked them agreed.
Corollary: never cite a command weaker than the claim implies (`cargo check` is
not coverage; `cargo test -p minutes-app` had never been run when it was cited).

## Where the code is

Recent commits, newest first:

| SHA | What |
|---|---|
| `c6badc34` | track-1 round-3 remediation, gate-clean, **not yet reviewed** |
| `d5945d1c` | diarization panic fix (pre-existing epic bug, see below) |
| `f28182ad` | track-1 round-2 remediation — REJECTED 3/3 |
| `ca2a659c` | track-2 MCP work — **REJECTED 3/3, do not build on it** |
| `1ed3a82c`/`99b89af1` | earlier track-1 attempts, both REJECTED 3/3 |

Rejected candidates are preserved at `rejected/block8-track1-51f040ba`,
`rejected/block8-track2-73d94270`, `wip/block8-suspension-revival-rejected`.
Last accepted checkpoint remains block 7; everything after it is unaccepted.

## Immediate next step

**Run the three-blind-review gate on `c6badc34`.** It is the first track-1
candidate where every claimed fix is mutation-verified, and it has not been
reviewed. Suggested lenses, mirroring what has been productive: (1) scope and
routing across every compressed-audio surface, (2) platform and containment
across the full cfg matrix, (3) test integrity and claim accuracy with
instructions to reproduce the mutations independently.

## State of each track

### Track 1 — compressed-import parity. Gate-clean, unreviewed.

Restores decoding of m4a/mp3/ogg/etc. when ffmpeg is absent, via a bounded
child running Symphonia. `origin/main` did this in-process; this branch had
deleted it and made ffmpeg mandatory, which broke `minutes watch` over iPhone
voice memos — the headline regression this track exists to close. That case is
now verified working end to end by a reviewer.

Known and unfixed, deliberately carried rather than dropped:

- The diarize fallback is asymmetric: a launchable-but-failing ffmpeg, or output
  past the diarization sample cap, still loses speaker labels without reaching
  the worker.
- `bounded_decode_fallback_available()` copies the whole executable to answer a
  boolean, and is called several times per import (admission, routing, probe,
  decode). On macOS/Windows that is real temp-dir I/O per file.
- Two tests pollute `MINUTES_FFMPEG` across each other, so the documented dev
  command in CLAUDE.md is red without `--test-threads=1`. CI passes only because
  it uses that flag.
- `verify()` re-hashes the retained descriptor while `execve` resolves the
  pathname, so a `rename()` over the snapshot defeats it on macOS. Linux is
  immune (sealed memfd); Windows incidentally so (share mode).
- On Windows `close_extra_descriptors()` is a no-op. `graph_worker` already
  solves this with `close_inherited_windows_handles_before_authority()` plus a
  canary test; the decode child does neither.

### Track 2 — MCP derived-record tools. Rejected, blocked on a decision.

`get_agent_annotations` is retired to an unavailable stub, and that was
validated as correct by reviewers: an annotation's source pointer and body are
both author-supplied, so revalidating the pointer bounds nothing.

`get_meeting_insights` is kept but **rejected 3/3**. It currently returns zero
records on Mat's own machine. **Do not patch this before the identity decision
is made** — see the dedicated bead note. Three options are written up there
(stable frontmatter id, canonical relative path, content hash) with tradeoffs,
plus two attached sub-decisions (archived meetings, and a rebuild path) and a
recommendation of B-now/A-durable/C-rejected.

Also outstanding on track 2, all downstream of that decision: the withheld tally
is still a per-record oracle via `limit`-differencing; `limit` silently changed
meaning from "max results" to "size of the pre-policy window"; and two of the
tests are tautological and must be rewritten.

## Things that are true and easy to get wrong

- **`zeroize` on a `Vec` clears it to length 0.** This caused a silent,
  epic-wide loss of speaker labels for every recording not an exact multiple of
  ten seconds, from the epic's first commit until `d5945d1c`. If you touch
  buffer reuse, remember the wipe truncates.
- **macOS is measurable.** Mat has a MacBook on the tailnet: `ssh
  mss-macbook-pro` (macOS 26.6, arm64, no Rust toolchain, no ffmpeg, both
  Minutes apps installed). Measured there, contradicting earlier assumptions:
  descriptor exec of an unlinked inode fails **EACCES**, not ETXTBSY; Darwin
  **permits** exec on a linked file with a live writer, unlike Linux;
  `_NSGetExecutablePath` returns the invoking symlink so `O_NOFOLLOW` gives
  ELOOP on a Homebrew install; `RLIMIT_AS` **is** enforced; and an absolute
  3 GiB ceiling is rejected EINVAL because the baseline is ~425 GiB. Use it
  rather than reasoning about Darwin from Linux.
- **This lane cannot compile macOS.** Cross-compilation fails at the C
  toolchain. Say so plainly rather than implying coverage.
- The suite has order- and load-sensitive tests. Re-run any failure in isolation
  before reporting it, and prefer unloaded single-threaded runs for evidence.

## Verification commands

```bash
cargo test -p minutes-core --no-default-features --lib -- --test-threads=1   # 1625 pass / 1 ignored
cargo test -p minutes-core --no-default-features --features diarize --lib -- --test-threads=1  # 1640 / 3 ignored
cargo test -p minutes-app --no-default-features                              # 272 pass
cargo clippy -p minutes-core -p minutes-cli --no-default-features -- -D warnings
cargo clippy -p minutes-core --no-default-features --features diarize -- -D warnings
cargo fmt --all -- --check
cd crates/mcp && npx vitest run && npx tsc --noEmit && node test/mcp_tools_test.mjs
npm --prefix site run check:llms
```

Rebase on `origin/main` before new work; it has moved several times mid-lane.

## Still deferred, do not revive

- Parakeet/Apple secure byte transport: **BLOCKED** on a P0 design question for
  Mat — can an engine that must exec a third-party binary ever satisfy block
  7's App-Sandbox-only property? Recorded in the bead. Do not code around it.
- Recall: deferred to **#514**. Do not un-defer.
- Suspension/SIGCONT worker boundary: rejected six times. Dead. Do not revive.
