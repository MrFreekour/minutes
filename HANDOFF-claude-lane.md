# Privacy-B lane handoff (refreshed 2026-07-26)

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

**The single most important lesson, learned expensively over four rounds:** the
recurring finding is not a class of bug, it is claims written before they were
verified. Commit messages asserted behaviour the code did not have, and tests
were written in the shape of the claim so they could not fail.

The procedure that catches it: **break the code first, watch the test fail, then
restore it.** Not "write a test" — prove the test fails.

Three hard-won corollaries, each from a specific failure:
1. **Do it per fix, and scope the claim to what you did.** Round 4 performed the
   procedure for four of seven bullets and then wrote one blanket sentence
   covering all seven. Reviewers reverted the other three with a green suite.
   If a bullet has no test, say so in the same breath.
2. **Assert the production path, not a helper you call the same way.** A ceiling
   test called `build_decode_command` directly and never asserted the caller
   used it, so restoring an unbounded inline builder passed.
3. **Verify with a different method than you edited with.** A README fix used a
   plain-literal replace and confirmed with a grep for the same literal; the
   line contained a markdown link, so both missed it identically.

Also: never cite a command weaker than the claim implies (`cargo check` is not
coverage), and assert a test's preconditions rather than assuming them — two
tests passed *with the bug present* when `target/debug/minutes` was absent.

## Where the code is

Recent commits, newest first:

| SHA | What |
|---|---|
| `b341876a` | track-2 Option B remediation — **not yet re-gated** |
| `49fdf4a5` | track-2 Option B — REJECTED (P0 fixed by `b341876a`) |
| `c6badc34` | track-1 round-3 remediation — **REJECTED 3/3**, remediation list in bead |
| `d5945d1c` | diarization panic fix (pre-existing epic bug, see below) |
| `f28182ad` | track-1 round-2 remediation — REJECTED 3/3 |
| `ca2a659c` | track-2 MCP work — **REJECTED 3/3, do not build on it** |
| `1ed3a82c`/`99b89af1` | earlier track-1 attempts, both REJECTED 3/3 |

Rejected candidates are preserved at `rejected/block8-track1-51f040ba`,
`rejected/block8-track2-73d94270`, `wip/block8-suspension-revival-rejected`.
Last accepted checkpoint remains block 7; everything after it is unaccepted.

## Immediate next step

`c6badc34` was gated and **REJECTED 3/3** (2026-07-26). Full findings and an
ordered remediation list are in the bead. The short version: the production code
was judged substantively correct, but the commit claimed "every fix
mutation-verified" when the procedure had been done for four of seven bullets,
and reviewers reverted the other three with a fully green suite. Three
user-facing defects also survived: a WebM/MKV duration probe that reads ~44x
short (millisecond `n_frames` in Matroska), a guidance message that fires in a
state its own doc calls impossible (symphonia has no Opus decoder, so
Opus-in-WebM/OGG and ALAC hit it), and README:453.

**Do the numbered remediation list in the bead, then re-gate.** Track 2 has moved
on since: Option B is built at `49fdf4a5` + `b341876a` and needs its own re-gate,
see below.

Add one item to the track-1 list, found 2026-07-27: the documented dev command
is not deterministically green even at `--test-threads=1`.
`audio_decode_worker::tests::the_probe_command_carries_the_same_ceiling_as_the_decode_command`
failed once in two full runs with "no Minutes binary was found next to this
process" while `target/debug/minutes` existed, and passes in isolation. Order-
or load-sensitive, same assumed-precondition family as item 3.

## State of each track

### Track 1 — compressed-import parity. REJECTED 3/3, remediation list in bead.

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

### Track 2 — MCP derived-record tools. Option B built, awaiting a re-gate.

`get_agent_annotations` is retired to an unavailable stub, and that was
validated as correct by reviewers: an annotation's source pointer and body are
both author-supplied, so revalidating the pointer bounds nothing.

`get_meeting_insights` returned zero records because the pipeline writes
`source_meeting` as an absolute path and release required that exact path to sit
in the live corpus. Measured on the real log: 1537 records, 355 distinct
sources, 0 released, all reported as a policy denial that was untrue.

**The identity decision is SETTLED (2026-07-26): Option B (canonical relative
path) now; Option A (stable frontmatter id + index) later as its own reviewable
block. Do not build A now.** The full options write-up is **lost** — the handoff
used to point at the bead for it, it is not there, and no bead has it. Most
likely the bd concurrent-write clobber. The decision survived; the reasoning did
not, so if A is picked up its tradeoffs need rederiving.

B-now is **built across two commits and not yet re-gated**:

| SHA | What |
|---|---|
| `49fdf4a5` | Option B identity, oracle closure, limit semantics, test rewrites. **REJECTED** (1 of 3 lenses; P0 below) |
| `b341876a` | remediation of that P0 plus two false claims in `49fdf4a5`'s message |

All four B-now items shipped: `source_meeting` resolves relative to the live
corpus root (452 of 1537 released on the real log), the `limit`-differencing
oracle is closed along with the same defect on the `since` axis, `limit` means
max results again, and both tautological tests were deleted and rewritten.

The P0 that got `49fdf4a5` rejected is worth remembering: the normaliser joined
everything after the anchor onto the live root and never inspected what came
before, so an `archive/` component to the left was silently dropped and a
restored corpus could release a restricted meeting's insight under an
unrestricted namesake. Three guards now cover it (recorded path still exists,
exactly one anchor, no inactive or hidden segment anywhere).

**Next step: re-gate `b341876a` with three fresh blind reviews.** The residual
heuristic limit is documented at `resolveCorpusRelativeSourcePath` and is
Option A's to remove.

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
