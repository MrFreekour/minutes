# Privacy-B lane handoff (refreshed 2026-07-26)

You are taking over the **conversation-trust privacy epic, Slice B integration**
lane. The previous lane ran to ~723k context and was retired deliberately; this
document plus bead **minutes-ew09** is how you pick up without loss.

- Worktree: `~/Sites/minutes-conversation-trust-privacy-b-v4`
- Branch: `integrate/conversation-trust-privacy-b-v4`
- **Never push, merge, tag, sign, or release.** Steady-background only.
- Durable state: `bd show minutes-ew09` for current status, and
  `docs/investigations/privacy-b-gate-history.md` for the full verdict history.
  That history used to live in the bead's notes and was silently lost twice in
  one session, so it lives in git now and the bead carries a pointer. This file
  is the orientation.

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
| `f6232143` | track-1 gate-2 remediation — Windows sweep backed out, ceiling claim scoped |
| `204d77cc` | track-1 gate-1 remediation — a false "canary-tested on Windows CI" claim |
| `b1cc0952` | track-1 items 1, 2, 3, 4, 9 and the order-sensitive probe test |
| `8be61da0` | track-1 items 5 and 8 — descriptor-sweep bound, README:453 |
| `51146a17` | track-1 items 6 and 7 — WebM duration, ffmpeg guidance and health |
| `efd58224` | track-2 Option B closeout — Codex BLOCK on residuals, execution lens ACCEPT |
| `4481ce86` | track-2 Option B gate-4 remediation (Codex) |
| `228d68c3` | track-2 Option B gate-3 remediation — gated BLOCK by Codex |
| `7aa0d217` | track-2 Option B round-4 coverage — gated REJECT/REJECT/ACCEPT |
| `8cebbe63` | track-2 Option B round-3 remediation |
| `b341876a` | track-2 Option B round-2 — REJECTED 2/3 (P0s fixed by `8cebbe63`) |
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

**A fresh gate on `f6232143`.** Two gate rounds have run on this track's
remediation; the second was three lenses (Codex read-only, a Claude platform
lens, a Claude execution lens) and all three rejected. Every finding is
remediated. The author has now edited this prose five times and should not be the
one to read it a sixth.

**The track-1 remediation list is worked through.** All nine items plus the tenth
added 2026-07-27 are closed across `51146a17`, `8be61da0`, `b1cc0952` and
`204d77cc`. Full detail is in `docs/investigations/privacy-b-gate-history.md`.

`b1cc0952` was gated twice, split by capability: Codex read-only returned BLOCK
(3 P1 + 4 P2), a Claude execution lens returned REJECT (1 P1 + 2 P2). The lens
reproduced all eight mutations, all four declared-uncovered survivals, and every
evidence number. Every finding was re-verified locally before acting; all are
remediated at `204d77cc`.

**Next: a fresh gate on `204d77cc`.** Both reviews were of `b1cc0952`, and the
standing lesson is that the author cannot review his own prose - four of this
round's five surviving defects were prose I had written, one of them inside the
fix for that same defect class.

Then two decisions that are Mat's, not the lane's: whether track 2's documented
residuals are acceptable on that surface, and whether Option A (stable
frontmatter id) gets built as its own block.

One repo-level finding for Mat, out of scope and pre-existing: two CLI
integration test files, `tests/policy_graph_worker.rs` and
`tests/authorized_process_fd.rs`, run **zero** tests in CI, because the only
`-p minutes-cli` invocation is filtered on `copilot`. Measured:
`cargo test -p minutes-cli --no-default-features -- copilot --list` reports
`0 tests` for both. That is where the Windows inherited-handle canary lives.
Wiring them in needs someone who can watch the pipeline go green.

## State of each track

### Track 1 — compressed-import parity. Remediation list done, awaiting a gate on `204d77cc`.

Restores decoding of m4a/mp3/ogg/etc. when ffmpeg is absent, via a bounded
child running Symphonia. `origin/main` did this in-process; this branch had
deleted it and made ffmpeg mandatory, which broke `minutes watch` over iPhone
voice memos — the headline regression this track exists to close. That case is
now verified working end to end by a reviewer.

Known and unfixed, deliberately carried rather than dropped:

- The diarize fallback is asymmetric: a launchable-but-failing ffmpeg, or output
  past the diarization sample cap, still loses speaker labels without reaching
  the worker.
- Four properties of `preprocess_compressed_without_ffmpeg` have no test, listed
  at the function itself: the post-decode cancellation check, the wall clock
  handed to the child, `MAX_DIARIZATION_SAMPLES`, and the zero-remaining guard.
  A reviewer applied all four mutations simultaneously and got a green suite, so
  the list is complete as well as honest. Each needs a race or a two-hour input.
- `bounded_decode_fallback_available()` copies the whole executable to answer a
  boolean, and is called several times per import (admission, routing, probe,
  decode). On macOS/Windows that is real temp-dir I/O per file. It is also why
  cancellation is now checked before it, and why the zero-remaining guard after
  it is reachable at all.
- Three tests pollute process-global env: two set `MINUTES_FFMPEG` and one sets
  `XDG_CONFIG_HOME`. All take `test_home_env_lock()` and restore before
  asserting, but the documented dev command in CLAUDE.md is still red without
  `--test-threads=1`. CI passes only because it uses that flag.
- `verify()` re-hashes the retained descriptor while `execve` resolves the
  pathname, so a `rename()` over the snapshot defeats it on macOS. Linux is
  immune (sealed memfd); Windows incidentally so (share mode). The digest
  re-check itself now has a test, `cfg(all(unix, not(linux)))`, that **this lane
  has never run**: macOS CI is the only place it executes.
- **On Windows the decode child can inherit ambient HANDLEs and nothing sweeps
  them.** `close_extra_descriptors()` is a no-op there and `CreateProcessW` is
  called with `bInheritHandles: TRUE`. Calling `graph_worker`'s sweep was tried
  at `b1cc0952` and **backed out** at `f6232143`: it would first execute on
  Windows CI, which nobody here can watch, in the pre-authority path of a child
  that ungated CI tests require to be green, and the decode child does far more
  OS work after the sweep than the graph child does. The reasoning sits where the
  call would go. The fix is the sweep plus a decode-worker canary plus a CI
  invocation that runs it, landed by someone who can watch it.

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

B-now is **built across three commits and not yet re-gated**:

| SHA | What |
|---|---|
| `49fdf4a5` | Option B identity, oracle closure, limit semantics, test rewrites. **REJECTED** (P0 below) |
| `b341876a` | remediation of that P0 plus two false claims in `49fdf4a5`'s message. **REJECTED 2/3** |
| `8cebbe63` | CI hermeticity, proven absence, guard-3 withdrawal, third false claim corrected |

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

Two rounds of three blind reviews have run. Every round found real defects, and
three of them were false claims in my own commit messages rather than code
faults, which is this lane's standing failure mode. Worth carrying forward:

- The insight tests were **CI-breaking** and nobody noticed until a reviewer
  checked `.github/workflows/ci.yml`. The `mcp` job runs vitest with no cargo
  build, so a content-bearing tool's readiness bridge had no CLI to shell out
  to. Any new test that drives a content-bearing MCP tool must be checked
  against a CI-equivalent environment, not just a dev box.
- `existsSync` is not an absence test. It answers false for EACCES too, and a
  guard built on it reopened the exact leak it was added to close.
- A guard can be **masked by a later guard**: one fixture was refused by a
  hidden-segment rule rather than by the anchoring it was written to prove.
  Re-check old mutations after adding new guards; two of the twelve had gone
  stale and would have reported a false pass.

**Next step: track 1.** Track 2 Option B is closed out at `efd58224`: the execution lens returned ACCEPT with no P0, and Codex returns BLOCK only on documented residuals that Option A removes. Whether those residuals are acceptable is Mat s call, not a gating question. Keep alternating models between gates: one Codex pass found two real defects that nine Claude lenses missed, and one Claude execution pass found three that Codex structurally could not. The residual
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
- **The decode child's behaviour lives in a separately built binary.** Several
  tests spawn `target/debug/minutes`. Mutating child-side code and re-running
  `cargo test -p minutes-core` observes nothing: you must
  `cargo build -p minutes-cli --no-default-features` first. Forgetting it
  produces a false "mutation survived", which is the one error this lane's whole
  procedure exists to prevent.
- **A precondition that accepts any failure proves nothing.** A staleness check
  asserted `!status.success()`, which loader and argument errors satisfy just as
  well as the refusal it was written for. Assert the specific exit and message.
- **Type-check a platform-gated test by temporarily widening its cfg**, compiling,
  then narrowing it back. That is evidence, not a repeatable gate, so say which
  one you have.
- **Reusing tested code does not carry its evidence with it.** A sweep that is
  canary-tested in `graph_worker` proves something about THAT caller, in THAT
  child, doing what that child does next. Calling it from a child with a
  different protocol is new untested code wearing an old test's name.
- **A comment that names a portability hazard is not code that handles it.**
  `u64::from(rlim_cur)` sat under a comment saying `rlim_t` is not guaranteed to
  be u64, inside a cfg gate selecting two targets where it is not.

## Verification commands

```bash
cargo build -p minutes-cli --no-default-features   # FIRST: several tests spawn this binary
cargo test -p minutes-core --no-default-features --lib -- --test-threads=1   # 1636 pass / 1 ignored
cargo test -p minutes-core --no-default-features --features diarize --lib -- --test-threads=1  # 1651 / 3 ignored
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
