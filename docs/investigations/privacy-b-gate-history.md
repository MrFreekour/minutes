# Privacy-B lane: gate history and verdicts

> Why this file exists. This history lived in the `notes` field of bead
> `minutes-ew09` and was silently truncated twice in one session: once by the
> known bd concurrent-write clobber, which cut it from 8761 to 7150 characters
> and deleted a whole gate verdict plus a nine-item remediation list, and once
> when the field grew past a limit in bd's own event log and the write was
> rejected. Both times the loss was invisible until a length check caught it.
> Version control cannot be clobbered by a concurrent bd session, so the
> long-form record lives here now and the bead carries a pointer plus the
> current state. Recovery of the first loss was possible only because beads is
> Dolt-backed; see the `bd-concurrency-race` note for the `AS OF` procedure.

Entries are append-only and oldest first. Every verdict below was produced by
spawning three subagents with distinct lenses against an exact SHA, told not to
trust the commit message.

---

Graph defer approved by Mat on 2026-07-21; fail-closed module and every canonical consumer point to roadmap issue #513. Superseding smaller B candidate frozen at 9131bbd1a7c5e7fb9cdf85db494f95ea252e4ea5 on accepted Slice A 7856314ff4aef07474f7305eb5c78a8186e829ad. 00d95ece rejected and invalid. Structural remediation: two persistent shared sentinel slots with random-token acknowledgement and no ambient unlink; cumulative async operation abort plus bounded policy child; raw traversal entry/path/deadline charging; desktop/skills/docs distinguish graph unavailable from empty. Gates green: no-default core 1434/1 ignored plus integrations, default core 1433/1 ignored plus integrations, app 263, SDK 132/1 skipped, MCP 10, strict core/CLI clippy, skills compile/check/dry, fmt/diff. No push/sign/tag.
2026-07-21 steady-background block 1: rebased the A+B spine onto origin/main 146d062f (A 90b2d6c4, B 8e20df91) and restored the dirty smaller-B remediation without conflicts. Recovered the last graph prototype only as a held experimental baseline, then replaced three rejected primitives: graph/search now share the owner-private cross-process projection lease; vocabulary corrections use the capability-bound exact-file reader; every negative alias comparison consumes the cumulative work budget. Added a verified 64 MiB SQLite main-page ceiling, 8 MiB cache setting, memory-only temp storage, and no durable production graph cache. Graph module: 59/59 tests green; no-default core check green; graph/lib/overlay/name-correction rustfmt green. Not accepted or shippable yet. Next blockers: replace or prove the remaining SQLite transient-heap bound, restore MCP/CLI/UI graph consumers without fallback, and adversarially validate full graph semantics. Whole-tree cargo fmt remains blocked by three pre-existing dirty Slice-C Recall formatting lines, left untouched.
2026-07-26 GATE RESULT on track-1 candidate c6badc34: REJECTED 3/3. Not a
checkpoint. Fourth consecutive rejected round on this track. Nothing pushed.

Lenses: scope/routing, platform/containment, test-integrity with independent
mutation reproduction.

THE CENTRAL FINDING, and it is a claim defect not a code defect. The commit
states "Every fix above is covered by a test that was mutation-verified". That
procedure was performed for four of seven bullets. The test-integrity reviewer
reverted the other three, individually and simultaneously, and both cited
evidence numbers reproduced exactly (1625/0/1 and 1640/0/3):
- P0 the diarize fallback (the largest fix) has NO test. Reverting the config
  source, availability check, both cancellation checks, the remaining() wall
  clock and the MAX_DIARIZATION_SAMPLES cap leaves the suite fully green.
- P0 the_probe_command_carries_the_same_ceiling_as_the_decode_command asserts
  the HELPER, not the production path: it calls build_decode_command directly
  and never asserts probe_compressed_duration uses it. Restoring the inline
  builder with no ceiling at all passes. The commit's own phrase "that child's
  containment was asserted by nothing" is still literally true after the fix.
- P1 the health.rs message fix has no test; existing assertions match both
  wordings.
Two of the four that WERE verified are weaker than claimed:
- P1 the chained-OGG fixture yields 0 packets, so pre-fix code also failed
  closed. The test distinguishes only by the word "reset"; with a realistic
  fixture (131 packets) the mutation returns 3.01 s of a 5 s file as a
  successful decode while the test stays green.
- P1 disabled_duration_routing_... passes WITH the bug whenever
  target/debug/minutes is absent; its precondition is assumed, not asserted.

OTHER P1s:
- WebM/MKV duration probe is ~44x short. Symphonia populates n_frames for
  Matroska with segment duration in MILLISECONDS, so frames/rate yields
  seconds*1000/sample_rate. Measured: 150 s .webm probes as 3.4 s. webm is a
  default watch extension, so with no ffmpeg every browser/Meet recording is
  filed as a memo and never diarized, while the same file WITH ffmpeg routes
  correctly. Same defect class as the zero-duration ffmpeg bug, polarity
  inverted: the fallback is now the wrong probe. Formula predates this work but
  origin/main's routing did not vary by decoder.
- compressed_audio_ffmpeg_guidance() is reachable in the state its own doc
  comment declares impossible. Symphonia has no Opus decoder, so Opus-in-WebM
  (browser MediaRecorder), Opus-in-OGG (WhatsApp/Telegram voice notes) and
  ALAC-in-m4a exit 65, and the watcher then tells the user to enable
  compressed_decode_fallback which is already true. In the same state
  `minutes health` prints a green check saying it works with no extra install.
  This is the exact defect fixed in health.rs one file over, missed in the
  message the watcher, Recovery Center and Tauri retry actually print.
- README:453 claimed fixed and NOT fixed. The commit changed one README line,
  not two, in the same bullet that accuses its predecessor of exactly that.
  Root cause worth recording: the line reads "compressed formats require
  [ffmpeg](https://ffmpeg.org/)" and both the edit and the confirming grep used
  the same plain literal, so the verification shared the edit's blind spot.
  Lesson: verify an edit with a different method than the one that made it.
- The ETXTBSY story survives as an EXECUTABLE assertion.
  path_backed_snapshot_execs_only_after_its_writer_is_dropped is #[cfg(unix)]
  and asserts ETXTBSY, which the Mac measurements refute; the suite is
  deterministically RED on macOS and CI never sees it. Prose was corrected, the
  assertion was not, and the known-unfixed list omits it.
- The non-Linux descriptor sweep is O(RLIMIT_NOFILE) uncapped. macOS permits an
  unprivileged `ulimit -n unlimited`, measured at 101 ns/iter -> 217 s per child.
  That makes the 60 s duration probe always time out, falling through to
  config.watch.type: long meetings filed as memos with no diarization, i.e. the
  regression this round exists to fix. This lane put macOS on that path;
  graph_worker's equivalent is cfg(not(macos)).
- "no ambient descriptors" is stated unconditionally and is false on Windows;
  graph_worker already implements and canary-tests the NT handle sweep.

P2s carried: module doc's platform split contradicts the in-code comment; the
diarize asymmetry leaves NO trace in the artifact (no speaker_map, no
processing_warnings, no degraded status) so it is indistinguishable from a
genuine single-speaker recording; fail-closed on reset refuses containers ffmpeg
decodes fully; the parallel dev command is red across a varying 4-6 test set
spanning knowledge:: and transcribe::, broader than the two disclosed; MCP
process_audio is WAV-only where main accepted five formats, so the whole
"works without ffmpeg" story never reaches an agent surface; Windows env policy
diverges from graph_worker (no SystemRoot/WINDIR); the diarize fallback
normalizes amplitude where its ffmpeg sibling does not, so diarization sees
different input depending on decoder; stale allow(dead_code) on macOS suppresses
the signal that would report the macOS decode path being dropped.

VERIFIED SOUND: cfg matrix has exactly one ceiling implementation per platform
with no gap and the Windows Job Object path is reached; no third command
builder exists; canonicalize-before-bind weakens nothing; dictation_threshold=0
and non-faststart routing both hold empirically with and without ffmpeg;
demuxer-level reset fails closed; all evidence numbers reproduce; both fixtures
are committed, not gitignored, and unskippable; PARENT COMMIT d5945d1c IS
HONEST, all three of its tests genuinely mutation-verified including the real
pyannote ONNX one.

Reviewers judged the production code in the untested fixes substantively
correct: the defect is coverage, not behaviour, except README:453 which is a
live user-facing inaccuracy.

REMEDIATION LIST for the next lane, structural order:
1. Write tests for the three uncovered fixes (diarize fallback, probe-command
   routing via probe_compressed_duration itself, health message) and
   mutation-verify each individually.
2. Rebuild the chained-OGG fixture so the first logical stream delivers packets,
   then assert the success/failure verdict rather than the message.
3. Assert the worker precondition in disabled_duration_routing_... instead of
   assuming it.
4. Fix the ETXTBSY assertion (gate to linux or assert per-platform behaviour).
5. Cap the non-Linux descriptor sweep.
6. Fix the WebM/MKV millisecond n_frames unit bug.
7. Make compressed_audio_ffmpeg_guidance honest for the codec-unsupported state,
   and make health agree with it.
8. README:453.
9. Decide on Windows close_extra_descriptors (call graph_worker's sweep or stop
   claiming it).
2026-07-27 track-2 Option B candidate created: 49fdf4a5. NOT yet gated; three
blind reviews were in flight when this note was written. Nothing pushed.

FIRST, A LOSS TO RECORD. The handoff points at this bead for "the full options
write-up and tradeoffs" for the track-2 identity decision (three options: stable
frontmatter id, canonical relative path, content hash; plus sub-decisions on
archived meetings and a rebuild path). That write-up is NOT in this bead. The
notes field ends at the track-1 remediation list, and a scan of all 50 beads
found no copy. It was lost, most likely to the known bd concurrent-write
clobber. The DECISION itself survived in the handoff (Option B now, Option A
later as its own block) and was sufficient to proceed, but the reasoning behind
it is gone. If Option A is picked up, its tradeoffs will have to be rederived.

ROOT CAUSE OF THE ZERO-RECORDS BUG, measured rather than reasoned. Against the
real event log on this machine: 1537 insight records, 355 distinct
source_meeting values, 0 released, and all 1537 reported withheld with the
message "designated restricted, or has been archived, moved, or deleted". That
message was false; the meetings were present and readable. The pipeline records
source_meeting as the absolute path it processed, and release required that
exact path to be inside the live corpus. Every value names
/Users/silverbook/meetings/... while the live root here is /home/mat/meetings,
so every record failed the corpus-membership check. Any corpus that changes
machine or home directory, or is restored from backup, loses its whole insight
projection and is told the reason was policy.

WHAT 49fdf4a5 DOES.
- Option B identity: a source is identified by its path relative to the live
  corpus root. Relative values resolve against the root; historical absolute
  values are normalised by stripping a recognised root prefix, anchored on the
  live root's own final path segment. Measured effect on the real log: 452 of
  1537 released, 1085 withheld whose sources genuinely are not in this corpus.
  Option A is untouched and remains the durable fix.
- Anchoring on the root segment rather than the filename is load-bearing for
  policy, not cosmetic: 273 of the 355 distinct values are macOS temp paths left
  by test runs, and a basename fallback would bind them to whatever live meeting
  shares a filename, releasing one meeting's insights under another meeting's
  policy.
- The limit-differencing oracle is closed, and so is the same defect on the
  since axis. Both now run in-process over a fixed MCP_INSIGHT_SCAN_WINDOW, so
  no caller-supplied value reaches the CLI at all and the withheld tally is a
  function of corpus state alone. since loses no reach because it is a lower
  bound.
- limit means "max results" again instead of "size of the pre-policy window",
  which is what had cost filtered queries their reach.
- The two withheld reason buckets are left deliberately conflated. Splitting
  "not resolvable into this corpus" from "resolved and refused" would publish a
  clean count of restricted meetings to a caller with no override. The
  user-facing message is corrected to admit both causes, and the truncation note
  no longer advertises a remedy that does not exist.

TEST DISCIPLINE. Twelve mutations, each applied on its own, full MCP suite run
each time, each caught by the test naming its property, then restored from a
pristine copy. The two tautological tests were deleted and rewritten rather than
patched: one captured the withheld tally in a const and re-asserted that same
immutable local in a loop, the other named the partial-view contract but
exercised only the release helper, which has no notion of partial. The handler
tests drive the registered tool through an in-memory MCP client via a
dependency seam whose defaults are the live functions, because MINUTES_BIN is
resolved at module load with no env override and an end-to-end assertion would
otherwise no-op wherever a built CLI is absent. The fake CLI honours --limit and
--since faithfully, without which the oracle assertions would be vacuous.

NOT COVERED, stated rather than implied: the producer still writes absolute
paths, so this is read-side only; the agent-trust gate on the tool is unchanged
and unasserted; no macOS or Windows execution is possible from this lane.

GATES: MCP vitest 256 passed / 1 skipped on three consecutive runs, tsc clean,
npm run build clean, MCP integration 11/11, check:llms up to date. No Rust
changed.

NEW TRACK-1 FINDING, for that remediation list. minutes-core
--no-default-features --lib --test-threads=1 was run twice. Second run: 1625
passed / 0 failed / 1 ignored, matching the number previous reviewers cited.
First run: one failure in
audio_decode_worker::tests::the_probe_command_carries_the_same_ceiling_as_the_decode_command,
panicking with "no Minutes binary was found next to this process" even though
target/debug/minutes exists. It passes in isolation, so it is order- or
load-sensitive, not a code defect, but it means the documented dev command is
not deterministically green even single-threaded. This is the same
assumed-precondition family as remediation item 3 and should be folded into it:
the test asserts a ceiling but assumes a discoverable worker binary.
2026-07-27 track-2 Option B, two full gate rounds run. NOT accepted. Candidate is
now 8cebbe63. Nothing pushed.

ROUND 1 on 49fdf4a5: 1 ACCEPT, 1 REJECT, 1 REJECT.
ROUND 2 on b341876a: 1 REJECT, 1 REJECT, plus the round-1 test-integrity lens
reporting late on 49fdf4a5.

Every round found real defects, and THREE of them were false claims in my own
commit messages rather than code faults. That is the lane's standing failure
mode reproducing itself, so the specifics are worth keeping:

1. "273 of the 355 distinct values are macOS temp paths." 274 are; 273 is the
   count that resolves to nothing. Both readings are true of the same data,
   which is how the error arose, and 273 was the number carrying the
   anchoring-is-safe argument.
2. "Without this seam the assertions would no-op wherever a built CLI is
   absent." False. The readiness bridge reintroduces the CLI dependency one
   layer up, which a reviewer proved with strace: one agent-readiness spawn per
   tool call.
3. "Two handler tests were left at the 5 s default ... both now carry 60 s."
   Five were; two were raised; three that issue real tool calls were still at
   the default, measured at 80% of budget under concurrency.

THE FINDING THAT MATTERED MOST, and it was nearly missed: the eight handler
tests added in 49fdf4a5 CANNOT PASS IN CI. Insights are content-bearing, so
every call routes through requireAgentTrustReadiness(), which shells out to the
CLI whatever runner is injected, and the `mcp` job in .github/workflows/ci.yml
runs npm ci and vitest with NO cargo build on ubuntu, macos and windows.
Measured with no binary reachable: 8 failed / 142 passed. b341876a's response
was to document the precondition, which was the wrong answer to a CI-breaking
dependency. 8cebbe63 makes readiness injectable with the live bridge as the
default; the same suite is now 160/160 with no CLI on PATH, no target directory
and HOME pointed at an empty tree. Standing rule for this lane: any new test
that drives a content-bearing MCP tool must be run against a CI-equivalent
environment, not just a dev box.

TWO P0 LEAKS FOUND AND CLOSED, both in the path normaliser:
- The anchor branch joined everything after the anchor onto the live root
  without inspecting what came before, so an inactive component to the left was
  discarded, and a recorded path that still existed was normalised anyway. A
  reviewer demonstrated a restored-backup corpus releasing a restricted
  meeting's insight under an unrestricted namesake with withheld.total 0.
- The guard added for that used existsSync, which answers false for EVERY stat
  failure. With the duplicate corpus's parent chmod 000 the same leak
  reproduced exactly. Absence is now proven with statSync throwIfNoEntry:false;
  EACCES, ELOOP and EIO all mean "treat as present". Not exotic: another user's
  home on a shared Mac is drwxr-x---, and an MCP server without Full Disk
  Access cannot stat ~/Documents.

A GUARD WAS WITHDRAWN, because its justification was wrong. It screened every
segment of the recorded path, but the example its comment gave is already
refused by the single-anchor rule as ambiguous. What decides whether a record
sat in an active part of its corpus is its own corpus-relative tail, which
isActiveCorpusMeetingPath already checks. Screening the discarded left-hand side
only rejected corpora that had lived under ~/Archive/ or ~/.local/share/, which
is the case the normaliser exists to serve.

TEST-DISCIPLINE LESSONS, beyond the ones already recorded:
- A NEW GUARD CAN MASK THE FIXTURE THAT PROVED AN OLD ONE. The misattribution
  test used a path containing `.tmpXXXX`; once a hidden-segment guard existed,
  that fixture was refused by the guard rather than by the anchoring it was
  written to prove. Re-check earlier mutations after adding guards.
- Two of the twelve original mutations went stale against rewritten code and
  would have reported a false pass had they been re-run blindly.
- Equivalent mutants exist and should be disclosed, not dropped: hardcoding
  `matched: 0` in the empty-result branch cannot fail, because matching.length
  is zero exactly when selected.length is.

STATE. 27 mutations verified individually across the three commits, each applied
alone with the full suite run and restored from a pristine copy. Gates: MCP
vitest 269 passed / 1 skipped, and 160/160 for index.test.ts in a CI-equivalent
environment; tsc clean; build clean; MCP integration 11/11; check:llms up to
date; real-log release split held at 452 released / 1085 withheld across every
change in rounds 2 and 3. No Rust changed. Reviewers independently reproduced
1537/355/0 before and 452/1085 after, and one cross-checked 452 real releases
against frontmatter titles finding zero misbindings.

NEXT: re-gate 8cebbe63 with three fresh blind reviews. Do not treat the two
prior ACCEPT-adjacent verdicts as transferable; the candidate has changed
substantially since both.
2026-07-27 addendum. Round-2 test-integrity lens returned ACCEPT on b341876a and
independently reproduced all fifteen mutation claims, the 452/1085 split, the
0/1537 baseline, and the 273-vs-274 correction. Its findings were coverage gaps,
not behavioural defects. Closed at 7aa0d217; candidate is now 7aa0d217.

The gap worth remembering: nothing pinned that the absolute branch carries the
WHOLE tail over. Swapping the tail join for a basename bind left the suite green
at b341876a and would have bound /elsewhere/meetings/memos/x.md to <root>/x.md,
a different meeting under a different meeting's policy, moving the real split to
410/1127 while green. At 8cebbe63 that mutation is caught only INCIDENTALLY, by
two tests that expect an inactive tail to be refused and happen to break when the
tail is dropped. Incidental is not asserted; there is now a positive assertion
with a same-named file at the corpus root so a basename bind picks the wrong one.

Also pinned there: the NUL guard, `truncated` comparing against the scan window
rather than the caller's limit, and the three argument descriptions. Those
descriptions had no coverage at all and check:llms cannot supply it, because the
generator reads manifest.json rather than the zod .describe() strings, yet they
are the only thing an agent reads before choosing arguments and two of the three
had already been rewritten in this series for being false.

Still uncovered by choice and disclosed in the commits: the 64 MiB maxBuffer has
no test, and `matched` in the empty-result branch is an equivalent mutant.

Gates at 7aa0d217: MCP vitest 272 passed / 1 skipped full workspace, 163/163 for
index.test.ts in a CI-equivalent environment with no CLI reachable, tsc clean,
build clean, integration 11/11, check:llms up to date, no Rust changed.

31 mutations now verified individually across the four code commits.

NEXT, unchanged: re-gate 7aa0d217 with three fresh blind reviews. Six reviews
have run over two rounds; the candidate has changed substantially since every
one of them, so no prior verdict transfers.
2026-07-28 GATE 3 on 7aa0d217: REJECT, REJECT, ACCEPT. Remediated at 228d68c3,
which is the current candidate. Nothing pushed.

No reviewer found a behavioural defect this round. The test-integrity lens built
51 mutations from its own reading of the diff, found every claimed behaviour
went red, reproduced the CI claim to the digit in BOTH directions (8 failed/142
passed at b341876a, 163/163 at 7aa0d217), and confirmed the basename mutation
moves the real split to exactly 410/1127 as claimed. Its verdict was ACCEPT with
one P1.

All three lenses rejected or flagged the same class: statements that are not
true. Three of them:
1. MCP_MEETING_INSIGHTS_DESCRIPTION, the agent-facing one, still said each
   insight is released only after "the meeting the pipeline recorded as its
   source" is re-read. True at ec29d9eb; 49fdf4a5 put resolution in between, so
   in the residual case a DIFFERENT meeting is re-read. A reviewer also showed
   released records carry the vanished meeting's title and path, so restricted
   metadata travels with the insight, which nothing documented.
2. The resolver docstring asserted the whole-path hidden/inactive screen that
   8cebbe63 itself deleted. Two lenses independently demonstrated four shapes it
   claimed were refused and which resolve. That paragraph is the only written
   bound on a knowingly accepted risk and it was wrong permissively.
3. The test harness PRECONDITION block claimed the tests need a built CLI,
   twelve lines above the stub that removed the need. All three lenses caught it.

That is now FOUR rounds where the defect was a claim rather than code. The
pattern is specific and worth naming: every one came from changing behaviour and
updating the commit message correctly while leaving an ADJACENT prose statement
describing the old behaviour. Checking the diff is not enough; the surrounding
docstring and any mirrored copy have to be re-read after every behavioural
change.

TWO METHOD LESSONS worth carrying:
- Pointing HOME at an empty tree does NOT prove hermeticity. A reviewer found
  that when a mutation made a test reach isCliAvailable(), the suite's
  auto-installer downloaded a 45 MB binary and a 78 MB model into that HOME and
  silently converted the environment into one WITH a CLI, contaminating every
  later run. The pristine tree never triggers it, so the claim stood, but the
  verification method could not tell "no binary needed" from "the suite
  installed the binary it lacked". Every CI-hermeticity check must now assert
  post-run that no binary appeared. Done for 228d68c3.
- Asserting a dependency by function .name passes against any stub that sets
  .name. Exported bindings are now compared by reference; the reviewer's own
  spoof is a regression test.

DELIBERATELY NOT FIXED, recorded so it is not rediscovered as new:
- Symlinked roots stay unrecovered. The anchor is basename(realpath(root)), so a
  ~/meetings pointing at an external volume recovers nothing; a reviewer judged
  this the commonest real moved-corpus shape. It fails closed. Widening the
  anchor set is a permissive change to the one function that has produced two
  P0s by being made more permissive. Option A's business.
- 1037 of the log's 1537 records are unreachable past the fixed 500 window. That
  is the price of closing the differencing oracle and leaves this MCP surface
  less capable than the CLI.

PRODUCT GAP FOUND, out of scope but real: get_meeting_insights appears in NO
skill. Every /minutes-* skill shells `minutes insights` directly, which does no
source revalidation at all. This whole fix therefore improves only the MCP host
and Tauri Recall paths; the primary agent path bypasses the policy boundary.
Worth its own issue.

Gates at 228d68c3: MCP vitest 272 passed / 1 skipped; 163/163 CI-equivalent with
the no-binary-installed assertion; tsc, build, integration 11/11, check:llms all
clean; real-log split unchanged at 452/1085 through every commit since the
first. manifest.json, manifest.mcpb.json and the generated site artifacts updated
for the corrected description. No Rust changed. 34 mutations verified
individually across five code commits, plus the reviewer's independent 51.

NEXT: gate 4 on 228d68c3. Three rounds have now run; every round found real
problems and the trend is that they are shrinking and moving from code into
prose. A fresh context should run it, since the author cannot review his own
prose for the fourth time with fresh eyes.

---

## 2026-07-29 GATE 4 on 228d68c3, run by Codex (cross-model)

Run via the `/codex` skill, `codex exec` adversarial mode, gpt-5.x through
codex-cli 0.145.0. Chosen over more Claude subagents because the surviving
findings in rounds 1-3 were prose I had written and re-edited, and the author is
the worst available reviewer of his own prose. Verdict: BLOCK with findings.
Remediated at 4481ce86.

**Two real defects, both missed by all nine prior Claude lens-reviews.**

1. `.trim()` on the recorded source pointer could rebind one real file to
   another. Leading and trailing whitespace is legal in POSIX and macOS
   filenames, so with both `<root>/notes.md ` and `<root>/notes.md` present, a
   record naming the first resolved to the second. Different meeting, different
   policy. Verified locally against real files before fixing. Trimming is now
   only an emptiness probe; any other change refuses the value.
2. The empty-result message claimed something the code never evaluated. Filters
   run only over records that already survived policy, so when everything was
   withheld the filter was never tested, yet the reply said "No meeting insights
   matched the filter criteria." Now says no RELEASABLE insights matched, and
   that withheld records are not filter-tested. The capped note had the same
   defect.

**Two false comments, both mine.** The resolver claimed `..` and inactive dirs
were "rejected before any filesystem access"; they are normalised away by `join`
first, so `archive/../notes.md` resolves to `notes.md`. Measured. The outcome is
correct (that tail denotes `notes.md`, and the control `archive/notes.md` still
withholds), so it was a comment defect, not a leak. The Windows-to-POSIX note
said the candidate "simply does not exist"; backslash and colon are legal POSIX
filename characters, so it is a real path that withholds only because nothing is
there.

**Recorded, not fixed.** A path is a mutable locator, not an identity: replace
the recorded file, or retarget a symlink, and the re-read validates the
replacement and releases the old record under the new file's policy. Relative
recorded values are accepted although the pipeline only writes absolute ones.
Both fail toward releasing and are exactly what Option A removes; both are now
written at the function. Codex also showed the withheld tally is an aggregate
restricted-count in a healthy corpus and is differenceable across corpus
CHANGES (observe, append a record, observe again) even though it is not
differenceable across caller arguments. That angle is new; removing counts is
the only complete fix and would cost the partial-view contract.

**Method note.** Codex could not execute anything here: its sandbox fails before
process launch (`bwrap: loopback: Failed RTM_NEWADDR`). It correctly refused to
invent findings and asked for the diff, which was pasted inline. So this was a
pure code-reading review, weaker than the Claude gates that could run tests and
mutations, and stronger exactly where those were weakest. Every acted-on finding
was re-verified locally before any change. Note also that `codex review --base`
no longer accepts a prompt argument in codex-cli 0.145.0, and that the default
base would have handed it the whole epic (88k insertions); scope to the
candidate range explicitly.

Candidate is now **4481ce86**. Gates: MCP vitest 273 passed / 1 skipped, tsc,
build, integration 11/11, check:llms clean, real-log split unchanged at 452/1085.

---

## 2026-07-29 CLOSEOUT on 4481ce86 (two lenses, split by capability)

Deliberately split: Codex re-read cold (it has no stake in prose I had edited
four times), one Claude agent executed (Codex's sandbox cannot launch processes
here). Candidate ended at **efd58224**.

**Codex: BLOCK.** It was right that my dispute of its earlier finding was wrong,
and that is the round's real catch. I had argued `archive/../notes.md`
normalising to `notes.md` was harmless because the tail "denotes notes.md and
always did". That holds only if the cancelled component exists, is a directory,
and is not a symlink. Measured on Linux with real files:

    missing/../board.md   kernel ENOENT    -> denotes nothing
    afile/../board.md     kernel ENOTDIR   -> denotes nothing
    alink/../board.md     kernel opens <base>/outside/board.md, OUTSIDE the corpus

`join` cancels all three lexically to `<root>/board.md`. The symlink case
launders an out-of-corpus file past the active-corpus check. `.` and `..` are now
refused before normalisation. Fixed at c3c44a30.

Codex also caught three overclaims in my prose, one of which I wrote WHILE
fixing an overclaim: the scan-window comment said a fixed window means
"differencing across any pair of requests yields nothing". It closes differencing
across caller ARGUMENTS only; state differencing (observe, append, observe)
still works, and the tally is an aggregate restricted count when every record
resolves. Both now written as accepted leaks rather than absent ones.

Codex still returns BLOCK overall. Its standing objections are recorded, not
disputed: path-as-identity is not provenance; "otherwise we return 0 of 1537" is
an availability argument, not an identity one; seq/timestamp and buffering limits
are accepted rather than closed. All are Option A's scope or explicit product
acceptances. **Whether documented residuals are acceptable on this surface at all
is Mat's call, not the lane's.**

**Claude execution lens: ACCEPT, no P0.** It built ~30 single-line mutations
itself and confirmed all 33 tests added in the range are killed by a mutation
matching what their name claims. It reproduced the CI-hermeticity result with a
better control than mine: it verified the network was live from inside the
scrubbed environment (302 to the releases URL), so a pass could not be confused
with an environment where the auto-installer simply had no route. Adopt that
control for future hermeticity checks.

Its three surviving mutations, all closed at efd58224: an untested inclusive
`since` boundary (`>=` vs `>`, distinguishable only by a record on exact local
midnight, which neither the suite nor the real log had); an untested middle
withheld-bucket branch whose conflation my comment located in the wrong function
(the caller discards the reason, so `source-policy-denied` is never
distinguishable in output); and an unreachable anchor-position guard whose
apparent test coverage came from the lexical guard instead.

**Evidence-line precisions worth not repeating.** "452 released / 1085 withheld"
measures the release HELPER over the whole log; the tool examines only the newest
500, so a real call reports 300/200. "273 passed / 1 skipped" holds only after
`npm run build` (271/3 on a clean checkout). "Integration 11/11" needs a built
`target/debug/minutes` (2 passed / 9 failed without one). And on this corpus
`include_restricted: true` returns the same split, because nothing here is
restricted, so the audited override is an unexercised path on Mat's own data.

---

## 2026-07-29 TRACK 1, the whole nine-item list plus the tenth. Candidates b1cc0952 then 204d77cc.

Items 5, 6, 7 and 8 closed at 51146a17 and 8be61da0. Items 1, 2, 3, 4, 9 and the
item added 2026-07-27 closed at b1cc0952, gated, remediated at 204d77cc. Nothing
pushed.

GATE: one Codex read-only pass (BLOCK, 3 P1 + 4 P2) and one Claude execution lens
(REJECT, 1 P1 + 2 P2). Split by capability on purpose, as at the track-2 closeout.
Codex cannot execute here; the Claude lens reproduced all eight mutations, all
four declared-uncovered survivals, and every evidence number. Both found real
defects, and neither found what the other did.

A DEFECT INTRODUCED BY THE FIX FOR THE SAME DEFECT CLASS. 51146a17 rewrote
compressed_audio_ffmpeg_guidance and left the OLD doc paragraph stacked above the
new one, so the function still claimed it was "only reachable when ffmpeg is
missing and the bounded worker is unavailable" - the exact false statement that
commit existed to remove. Found by reading the commit's own diff back a day
later. That makes five consecutive rounds where a surviving defect was prose.

THREE MORE PROSE DEFECTS FOUND BY REVIEWERS IN THIS ROUND, all mine, all written
the same day:
1. The new ceiling check accepted an ambient `ulimit -v` while its comment said
   it refused one (Codex). Fixed by requiring soft AND hard limits to equal the
   worker budget exactly, which is what makes it a provenance check. There is now
   a test that launches a child under a foreign 2 GiB ceiling and requires
   refusal; under the looser form that child exits 0.
   **[SUPERSEDED by the 2026-07-29 gate-3 entry: "which is what makes it a
   provenance check" was itself an overclaim. It compares two numbers to a
   constant; a foreign launcher setting exactly that value is accepted.]**
2. The uncovered list said three items and omitted the zero-remaining guard
   (Codex). It says four, at the function, in one place.
3. "canary-tested through graph_worker on Windows CI" was false (Claude lens).
   The canary exists in crates/cli/tests/policy_graph_worker.rs, but CI's only
   `-p minutes-cli` invocation is filtered on `copilot`, under which that file
   reports 0 tests. That sentence was the whole justification for adding an
   unexercised call to the decode child's pre-authority path. Corrected at
   204d77cc; wiring the file into CI is the real repair and was deliberately not
   done blind from a lane that cannot watch the runner.

A TEST OF MINE SURVIVED ITS OWN MUTATION, caught by running it. Cancelling and
asserting the message passed with the pre-decode check DELETED, because the
post-decode check reports the same string. The fix asserts two things one message
cannot: the ORDER against the availability probe (disable the fallback in the
config, so a passing order check yields the cancellation string and a failing one
yields the unavailability string) and that no decode was attempted (name a file
that does not exist, so an attempt reports the missing input).

A PRECONDITION THAT ACCEPTS ANY FAILURE PROVES NOTHING. The probe test's
staleness check asserted only `!status.success()`, which any argument or loader
error satisfies (Codex). It now requires exit 71 and the ceiling named in stderr.
The reviewer then built a deliberately stale child and confirmed the precondition
fires.

CONTAINMENT MADE OBSERVABLE IS WHAT MADE THE PROBE ASSERTABLE. The old test
asserted build_decode_command and never asserted the production probe used it, so
an inline builder with no ceiling passed. The child now refuses to parse input
unless it can see its own ceiling. Defence in depth on its own, and it turns the
ceiling into something a test can see through probe_compressed_duration itself.
The reviewer also removed the ceiling from build_decode_command entirely and
found four tests die, so the decode path is covered end to end too.

THE CHAINED-OGG NUMBER REPRODUCED TO THE DIGIT. The old fixture was 0.25 s and
delivered no packets, so pre-fix code failed closed too. The rebuilt fixture is
3 s at 44.1 kHz plus 2 s at 48 kHz. With the reset arms restored to `break` the
decode returns 48134 samples, 3.01 s of a 5 s file, as a success: exactly what
the gate-3 reviewer predicted.

ITEM 10 WAS A MISREPORT, NOT A MISSING BUILD. resolve_worker_executable found the
adjacent binary, failed to BIND it, and fell through to a branch whose text says
no binary was found. Binding copies a 256 MB debug executable into an immutable
snapshot, so it can fail under memory pressure or a full temp filesystem. The
cause is now carried into the message for that branch. The flake did not recur in
five full runs and is not fixed, only made diagnosable.

WINDOWS, CHECKED BEFORE IMPLEMENTED, which was Codex's advice and was right.
1.95.0's sys/process/windows.rs sets `inherit_handles: true` at line 193 and
passes it to CreateProcessW at line 417; nothing in this crate calls the setter
that would change it. Both reviewers confirmed it independently. So the sweep is
load-bearing rather than theatre, and the decode child now calls it.
**[SUPERSEDED by the 2026-07-29 gate-2 entry: the call was backed out. The
sweep's load-bearing-ness stands; "the decode child now calls it" does not.]**

TWO TECHNIQUES WORTH REUSING:
- Type-check a platform-gated test by temporarily widening its cfg, compiling,
  then narrowing it back. Running the Darwin digest test under the widened cfg
  fails at chmod with EPERM, which is direct evidence that Linux's memfd seal is
  real and the digest re-check genuinely is a non-Linux control. It is evidence,
  not a repeatable gate, as Codex noted.
- The decode child's behaviour lives in a SEPARATELY built target/debug/minutes.
  Any mutation of child-side code needs `cargo build -p minutes-cli
  --no-default-features` before a test can observe it. Forgetting that is a false
  "mutation survived" waiting to happen.

STILL UNCOVERED IN THE DIARIZE FALLBACK, verified complete by the reviewer, who
applied all four simultaneously and got a fully green suite: the post-decode
cancellation check, the wall clock handed to the child, MAX_DIARIZATION_SAMPLES,
and the zero-remaining guard. Each needs a race or a two-hour input.

REPO-LEVEL FINDING FOR MAT, out of scope and pre-existing: two CLI integration
test files, tests/policy_graph_worker.rs and tests/authorized_process_fd.rs, run
zero tests in CI because the only `-p minutes-cli` invocation is filtered on
`copilot`. Worth its own change by someone who can watch the pipeline.

GATES at 204d77cc: core --no-default-features --lib --test-threads=1 1635 passed
/ 0 failed / 1 ignored, up from 1629; --features diarize 1650 / 0 / 3;
minutes-app 272 / 0; fmt clean; clippy clean for both documented invocations.
`--all-targets` clippy is red on four PRE-EXISTING findings in copilot/control.rs,
live_session.rs and resummarize.rs, untouched here and outside the documented
gate.

NEXT: the track-1 list is worked through. A fresh gate on 204d77cc is the honest
next step, since both reviews were of b1cc0952 and the author cannot review his
own prose for the sixth time. After that, Mat's call on whether track 2's
documented residuals are acceptable, and Option A as its own block if he wants it.

---

## 2026-07-29 GATE 2 on track 1, three lenses. Candidate 204d77cc, remediated at f6232143.

Three blind reviews, all rejecting: Codex read-only BLOCK (6 P1), a Claude
platform/containment lens REJECT (3 P1), a Claude execution lens REJECT (2 P1).
Run concurrently with only the execution lens permitted to mutate the tree, so
the read-only lenses could not observe deliberately broken code. Every finding
was re-verified locally before acting. Nothing pushed.

THE ONE THAT REVERSED A DECISION. The Windows inherited-handle sweep is backed
out. I added it believing it was existing canary-tested code called from one more
place. The platform lens established, and I confirmed both halves, that `ci.yml`
runs the minutes-core lib tests on windows-latest with no guard while
`bounded_worker_child_round_trips_pcm_into_a_private_file` has no cfg gate and
spawns the real decode child, so the sweep would execute on Windows CI
immediately; and that the two children are not equivalent afterwards, since the
graph child only reads stdin and writes stdout while this one opens files and
runs Symphonia container probing, which can pull delay-loaded imports, against a
sweep that retains only the three std handles.

My framing had been wrong in both directions at once: Codex said "covered by a
test nothing currently executes" still claims coverage, and the platform lens
said "read it as the same sweep, called from here" understates an exposure the
change creates. Item 9's own wording offered "call graph_worker's sweep OR stop
claiming it"; with a measured reason not to land it blind and a failure mode of
losing compressed import on Windows entirely, stopping the claim is the correct
half. General lesson: reusing tested code does not transfer its evidence. The
test covered that caller, in that child, doing what that child does next.

THE PROVENANCE OVERCLAIM, and it is the same shape as the two before it. I called
the new ceiling check a provenance check. It compares two numbers to a constant.
A foreign launcher setting exactly the worker budget is accepted and nothing can
tell the difference. What exact equality buys over the "finite and no looser"
form it replaced is only that an ambient `ulimit -v` under the budget no longer
satisfies it. Third round running where a comment claimed an authentication or
coverage property the code did not have.

TWO PROVED MUTATION SURVIVORS, both mine:
1. The `rlim_max` half of the ceiling check could be deleted with the entire
   suite green, while its docstring called it load-bearing. Neither sibling test
   could see it: `BoundedCommand::address_space_limit` sets both limits together,
   so no command they can build varies them apart. The new test uses a shell to
   lower the soft limit alone. Nothing can RAISE a hard limit, which is exactly
   why the case is reachable.
2. `health_and_watch_guidance_agree_about_the_unsupported_codecs` is named for
   agreement between two producers and read only one. Replacing the whole
   guidance body with text asserting the OPPOSITE of health left it green. Three
   siblings caught that mutation so nothing escaped, but the test named for
   agreement was the only one blind to a disagreement. Both mutations were
   reproduced here before fixing.

A COMPILE ERROR INSIDE MY OWN CFG GATE. `u64::from(rlim_cur)` does not compile
where `rlim_t` is `i64` (FreeBSD family) or `uintptr_t` (Haiku), both selected by
`cfg(all(unix, not(macos)))`, under a comment saying rlim_t is not guaranteed to
be u64. The sibling code it was copied from uses `try_into` for exactly that
reason. Not a shipped target, but the comment named the hazard and the code had
it.

A SILENT DEGRADATION PATH THE CHANGE WIDENED. `probe_compressed_duration` maps
any child failure to `None`, indistinguishable to the watcher from "this
container declares no duration", whose consequence is falling back to
`config.watch.type`: long calls filed as memos. The containment check added a new
way to reach it. It now logs before returning None.

THREE PLATFORM COMMENTS CONTRADICTING EACH OTHER, one with my new code block
directly underneath, and one materially misleading: `WORKER_ADDRESS_SPACE_BYTES`
said "a growth allowance over the process baseline, never an absolute ceiling",
which is true on macOS only. On Linux it is absolute and the ~250 MB image comes
out of it.

A MEASURED COST, MITIGATED RATHER THAN DISCLOSED. The execution lens measured the
suite 26% slower than before the range against a CI step with a three-minute
timeout, from per-test binds that each copy the ~250 MB debug executable. The
precondition is now answered once per process (113 s -> 94 s locally) and the CI
timeout goes 3 -> 5. Loosening a timeout cannot turn a passing run red, which is
why that is safe from a lane that cannot watch the runners; ADDING invocations is
not, so the dead CLI test files are still left for someone who can.

WHAT THE LENSES CONFIRMED RATHER THAN FOUND, worth recording because it is the
first round where the substance held: all eight declared mutations reproduce
independently, all four declared-uncovered properties genuinely survive when
applied simultaneously, every evidence number reproduces including 48134 to the
digit, the fixture provenance commands reproduce a file of identical byte length
containing two BOS pages with distinct serials, the toolchain claim is exact to
the line and the setter that would change it is unstable and therefore unusable
here, the CI precondition holds on all three runners, and the exact-equality
check has no legitimate-user break under ambient ulimits, cgroups, containers,
systemd or 32-bit. The `pre_exec` registration order was checked too: the ceiling
closure is registered before the direct-exec closure, and had it been reversed
the child-side check would refuse every legitimate Linux launch.

NOT FIXED, RECORDED: b1cc0952's message still contains the false "canary-tested"
sentence. Commit messages are immutable evidence, so the correction lives in
204d77cc, f6232143 and here rather than in a rewrite of history.

GATES at f6232143: core --no-default-features --lib --test-threads=1 1636 passed
/ 0 failed / 1 ignored in 94 s; --features diarize 1651 / 0 / 3; minutes-app
272 / 0; fmt clean; clippy clean for both documented invocations.

NEXT: a fresh gate on f6232143. Three rounds of three lenses have now run on this
track. The trend is the same one track 2 showed: findings shrinking and moving
out of code into prose, with the code substance holding up under independent
mutation. The author should not be the one to review this prose a seventh time.

---

## 2026-07-29 GATE 3 on track 1, three lenses. Candidate f6232143, remediated at a9662ba4.

Codex read-only BLOCK (7 P1), a Claude next-maintainer lens REJECT (3 P1), a
Claude execution lens REJECT (3 P1, every one proved by mutation with the suite
green). Same isolation as gate 2: only the execution lens could touch the tree.
Every finding re-verified locally. Nothing pushed.

THE FINDING THAT MATTERS MOST, because it is a fix failing the same way twice.
Gate 2 found that the health/watch agreement test read only one of the two
producers it is named for. The gate-2 remediation made it read both, by asserting
each contained "Opus" and "ALAC", and the docstring said "Both sides are read
now". The gate-3 execution lens then replaced the guidance with "The bundled
decoder decodes Opus and ALAC perfectly well, so ffmpeg is never required for
those": contains both words, passes, and tells a watcher user the opposite of
what `minutes health` tells the same user, with all 1636 tests green.

VOCABULARY AGREEMENT IS NOT AGREEMENT. The fix that finally held is structural
rather than another assertion: both surfaces build their text around one shared
sentence via a macro, so they cannot disagree without one of them dropping it,
which a test can see. Worth generalising: when a test must check that two things
AGREE, checking that both mention the same subject is not it. Either share the
bytes or assert the relationship.

NUMBERS I REPEATED WITHOUT MEASURING. I claimed the memoised precondition took
the suite from 113 s to 94 s. Measured A/B by the execution lens on one box:
102.40 s pre-range, 102.82 s as landed, 103.37 s without the memoisation. It
saves ~0.4 s. The whole range costs ~0.4 s, not the 26% a gate-2 reviewer
estimated and I repeated in three places. Run-to-run noise exceeds the entire
effect. The CI timeout bump built on that number is reverted; `git diff
8be61da0 -- .github/` is empty again. Lesson, and it is the lane's own lesson
arriving from a new direction: a reviewer's number is a claim too, and repeating
it without measuring makes it mine.

A DEGRADE PATH CLOSED FOR ONE BRANCH OF FIVE. The next-maintainer lens walked a
real bug report end to end and found `probe_compressed_duration` returns `None`
down five paths while only the one gate 2 fixed logs. The worst discards the bind
error that item 10 exists to preserve, with a real TOCTOU behind it: the watcher
binds once for `bounded_decode_fallback_available`, the probe binds again
immediately after. All five now report their cause.

AN ORDERING CLAIM THAT SURVIVED ITS OWN CORRECTION, proved by mutation. f6232143
added "no test here can see whether the availability probe ran"; eight lines
below, the surviving bullet still said the test "pins the ORDER" and the probe
"must not run". Hoisting the probe above the cancellation check while preserving
error precedence left the suite green.

macOS: THE LEAST-VERIFIED PATH CARRIED THE MOST CONFIDENT DOCS. It is the only
platform where the child installs its own ceiling, and nothing tests it: the sole
macOS-gated test asserts the PARENT installs nothing, and deleting the install
call leaves that suite green. The non-macOS sibling meanwhile carried a
per-branch TESTED BY list. Disclosure moved to where the gap is. The
cross-reference naming a Unix-only test was being emitted on Windows, where that
test does not compile.

A FOURTH CONTRADICTING PLATFORM COMMENT, inside the block gate 2 rewrote to fix
three. Also: the budget doc grouped Windows Job Object limits with Unix
RLIMIT_AS, which bound committed memory versus reserved address space, so the
image-size reasoning does not transfer; and "the strongest ordering of any
platform" was an unmeasured superlative in the same file where "commonest state"
had just been deleted for being one.

THE STRUCTURAL OBSERVATION, and it should shape whoever gates next. Almost every
surviving finding in gates 2 and 3 concerns macOS or Windows behaviour, the two
platforms this lane cannot execute. That is not fixable by rewording. The right
response is to make fewer claims about them, not better ones: state the mechanism,
name the platform, and say plainly that it is unverified here. Where a claim
about those platforms is load-bearing, the work belongs to someone with a runner.

WHAT HELD. The three tests added at f6232143 all survive adversarial probing: the
soft-ceiling test kills its mutation and the reviewer could not construct a
wrong-reason green; the fixture-premise assertion fails correctly against both a
short first stream and an unchained one; the ceiling keystone kills five tests
when removed. The revert of the Windows sweep is complete and honestly described,
`graph_worker` is byte-identical to its pre-range state, and the memoisation
weakens nothing. All evidence numbers except the timings reproduce exactly.

CLAIM CORRECTED: "clippy clean with -D warnings for both documented invocations"
was true of the two crate-scoped runs but CLAUDE.md's gate also lists
`cargo clippy --all`, which is red with 66 pre-existing errors in
tauri/src-tauri. Verified identical at 24e3a117 with changes stashed. CI runs
clippy on the macOS runner only, so it is not a CI break.

GATES at a9662ba4: core 1636 / 0 / 1 ignored; --features diarize 1651 / 0 / 3;
minutes-app 272 / 0; fmt clean.

NEXT: gate 4 on a9662ba4. Three rounds, nine lenses, and the pattern is stable:
the code substance holds under independent mutation every time, and what survives
is prose about platforms nobody here can run. A fourth round should be scoped to
that question specifically, or the lane should stop gating and hand the platform
claims to someone with macOS and Windows runners.

---

## 2026-07-29 GATE 4 on track 1, scoped platform-claim lens. Working tree atop e25bdb48, remediated at f0977d67.

The prepared Codex read-only lens was recovered from the interrupted Claude
session and regenerated with the current complete
`crates/core/src/audio_decode_worker.rs`. Its only question was whether every
remaining platform claim was true of the code or explicitly labelled
unverified. The first pass BLOCKED on one P1 and five P2 findings.

THE P1 WAS FALSE PROVENANCE AGAIN. The diagnostic said the ceiling was "not the
one this worker installs", but the verifier compares two values with a constant
and cannot identify who installed them. It now reports only the property it
observes: both ceiling values must equal the configured worker budget.

THE P2 FINDINGS WERE ALL CLAIMS AHEAD OF EXECUTION. The module summary described
Windows process-group isolation without platform scope, repeated unmeasured
Darwin virtual-memory behavior, asserted no App Sandbox conflict, described the
adjacent executable search as covering every real layout, and called one
Windows builder assertion the only ceiling coverage while overlooking the probe
builder test. Each finding was checked against the code before editing.

The remediation centralises the disclosure instead of rewriting those claims:
Linux was exercised through the real child path; macOS control flow was
inspected but ceiling installation and effective kernel enforcement were not
executed here; Windows builder configuration was inspected and tested but Job
Object attachment, ordering, and effective enforcement were not executed here.
The unsupported claims were removed, and the Windows test comment now names
both builder tests while explicitly denying caller-sensitive or runtime
enforcement coverage.

The regenerated second pass ACCEPTED with zero P1/P2. It explicitly confirmed
that the disclosures match the evidence and treated the remaining correction
history as editorial verbosity rather than a platform-honesty defect.

This was one scoped gate-4 lens against the working-tree contents, not three
independent blind exact-SHA accepts. `f0977d67` is therefore a local unaccepted
candidate, not a checkpoint.

Exact Linux gates after remediation: `cargo build -p minutes-cli
--no-default-features`; strict core+CLI no-default clippy; strict core
no-default+diarize clippy; fmt and diff checks; core no-default lib 1636 passed,
0 failed, 1 ignored; core diarize lib 1651 passed, 0 failed, 3 ignored;
minutes-app no-default 272 passed, 0 failed. The app test build emitted 43
pre-existing no-default warnings and still passed. Nothing pushed, merged,
tagged, signed, or released.

NEXT: execute the disclosed macOS and Windows enforcement paths with runners, or
make an explicit decision to stop gating track 1. Do not convert missing runtime
evidence into more confident platform prose.

---

## 2026-07-29 PLATFORM EXECUTION FOLLOW-UP to gate 4. Candidate d301d277.

The macOS half of the disclosed runtime gap was executed natively in an
isolated temporary checkout on `jexs-imac` (arm64 macOS 26.5, pinned Rust
1.95.0). The existing iMac Minutes checkout was not modified. Candidate Git
objects travelled in a local bundle; nothing was pushed.

NATIVE MAC COMPILE FOUND A REAL CANDIDATE-OWNED DEFECT. Strict no-default
core+CLI clippy failed on `clippy::type_complexity` for the Darwin-only
five-tuple return type of `immutable_unix_executable_snapshot`. Linux could not
see that cfg branch. Blame traced it to this lane's path-backed snapshot work.
`d301d277` introduces a private Darwin/non-Linux-Unix tuple alias; strict native
no-default core+CLI and core+diarize clippy are green afterwards.

THE MACOS INSTALLER AND KERNEL REFUSAL ARE NOW EXECUTED SEPARATELY. Existing
native end-to-end decode and duration-probe tests launch the real Minutes child;
success proves the in-child ceiling install returned success before Symphonia
construction because an install failure exits 71. The new macOS-only
`the_macos_child_ceiling_refuses_an_over_budget_mapping` runs the same installer
in an isolated test subprocess, then requests 3 GiB plus one 16 KiB Darwin page
of `PROT_NONE` anonymous address space. Darwin refused the mapping, and the
exact `d301d277` source blobs passed the focused regression 1/1.

ONE MACOS GAP REMAINS, NARROWER THAN BEFORE. The enforcement regression calls
the installer directly. Deleting the production entry point's call to the
installer would therefore leave the suite green. The installer and effective
Darwin `RLIMIT_AS` behavior are verified; the production caller relationship is
not mutation-sensitive.

MAC EVIDENCE: the code-equivalent full no-default core suite passed 1648, failed
0, ignored 1; the code-equivalent full core+diarize suite passed 1663, failed 0,
ignored 3 in 371.53 s; the final production source plus new macOS-only test
passed minutes-app 284/0. The focused ceiling regression added afterwards
passed 1/0 against source blobs independently matched to exact
`d301d2776e470da7f19ab4ec8d4d036292b87b53`. Native fmt and both strict
crate-scoped clippy invocations are green. The app build rewrote two tracked
`mic_check` build artifacts; they were identified by exact-tree comparison and
restored in the disposable checkout before final source verification.

LINUX AND WINDOWS CROSS-CHECK: exact `d301d277` passes fmt, diff check, strict
core+CLI no-default clippy, and strict core+diarize clippy on Linux. The full
repo Linux `cargo clippy --all --no-default-features -- -D warnings` remains red
on 65 unrelated pre-existing Tauri warnings, none in candidate files. Exact
Windows GNU-target core no-default check is green with 11 known cfg warnings.
No Windows runtime host was available, so Job Object attachment, suspension
ordering, and effective memory enforcement remain unexecuted.

STATUS: `d301d277` is a clean local candidate, not an accepted checkpoint. The
earlier scoped gate-4 second pass accepted before this exact SHA; there are not
three independent blind exact-SHA accepts. Nothing pushed, merged, tagged,
signed, or released.

NEXT: execute the Job Object path on real Windows, obtain three fresh blind
reviews against exact `d301d277`, or have Mat explicitly stop gating track 1.

---

## 2026-07-29 HOSTED WINDOWS EXECUTION AND INTEGRATION DIAGNOSTICS. Draft PR #604.

Mat authorized a temporary validation branch and draft PR, but did not
authorize merge, tag, signing, publication, deployment, or release. The current
delivery boundary is therefore an exact-SHA validation candidate that must
remain draft and unmerged.

THE FOCUSED WINDOWS ENFORCEMENT GAP WAS CLOSED. Hosted `windows-latest` workflow
run 30498965752 executed exact candidate `e9b2ac90` and passed the focused Job
Object committed-memory regression. Release-readiness and installer jobs were
skipped because the workflow ran in validation-only mode. Earlier
one-control-at-a-time mutations had independently made that regression fail
before each control was restored.

THAT FOCUSED PASS DID NOT MAKE THE INTEGRATION READY. Ordinary exact-tree CI run
30498818041 failed the Windows no-default core suite and timed out while a QMD
concurrency test waited indefinitely. Three independent exact-SHA reviews
therefore returned BLOCK. The first remediation candidate, `a62cc0b0`, made
hosted libtest assertions visible, increased the expanded Windows core suite's
bounded timeout, made Rust fixtures platform-native, bounded the QMD gate wait,
normalized nested QMD snapshot keys, and selected the existing Windows MCP
junction regression in CI.

THE DIAGNOSTIC MATRIX THEN FOUND MORE REAL WINDOWS-ONLY TEST DEFECTS. Run
30499792662 proved the nested QMD regression on hosted Windows, while the newly
selected MCP path suite exposed mixed separators in `$HOME` expansion,
Unix-literal containment fixtures, and cleanup racing reader-held directories.
The replacement batch uses native joins for home expansion and fixtures, and
retires bound readers before removing their temporary roots. It also replaces
a stale hard-coded public site test claim with the generated number of
test declarations; that number is not described as a pass count.

The Windows core output collapsed most failures onto four platform defects.
Private-store hardening had replaced the DACL but retained an
Administrators-group owner on elevated runners; the replacement sets both the
current-user owner and a protected current-user-only DACL through each exact
opened handle. Graph-worker opaque source IDs were incorrectly validated as
host paths, so their slash-delimited wire namespace is now validated
independently of the host platform. The Win32 descriptor-relative no-replace
rename form returned invalid-parameter when given the retained target-directory
handle; the Windows implementation now uses the native NT relative rename on
the exact source and target handles. Unix rename/swap race fixtures that cannot
execute while Windows retains a no-delete-share handle are Unix-scoped beside
the stronger Windows retained-handle tests, and the QMD parser fixture now
builds a platform-native absolute path.

The companion packaging-parity check correctly detected that the complete
graph-worker file changed. The replacement refreshes its whole-file golden
after reviewing the portable fixture and host-independent wire validation in
the complete authority file; the structural and mutation self-tests remain
green.

THE FIRST REPLACEMENT REMAINS BLOCKED. Hosted run 30501091431 at `91d8ee43`
made the Windows MCP/corpus lane and all short cross-platform checks green and
reduced the ordinary Windows core result from 175 failures to 74, but did not
make that suite green. Visible assertions identified a retained Windows
executable-snapshot setup handle with write access, which conflicts with the
image loader's sharing request, and a relative `NtCreateFile` root descriptor
without native traverse access. The next batch closes the setup writer, binds
the completed snapshot read-only with no write/delete sharing, verifies it
against the digest captured before the close/reopen boundary, and adds a
Windows child-launch regression. It also grants the private directory
descriptor traverse access, makes the graph event-filter fixture
platform-native, and emits graph-journal dirty-cause details only in test
builds so any residual hosted false-positive is attributable. The run's macOS
full-workspace clippy step separately found an unused and uncalled Recall
streaming/executable-authority prototype. The batch removes that dead prototype
and its lone `File` import instead of suppressing the lint; active Recall
source reauthorization and local Ollama behavior are unchanged.

THE SECOND REPLACEMENT REMAINS BLOCKED, BUT CLOSED THE CHILD-LAUNCH DEFECT.
Hosted run 30502676456 at `d5d5f94f` passed the Windows Job memory controls,
the read-only executable-snapshot child-launch regression, the Windows
installer build, and every completed non-core Windows lane. It reduced the
ordinary Windows core result again, from 74 failures to 69; the audio-decode,
diarization-fallback, duration-routing, and unrelated-state graph failures
disappeared. The remaining output exposed four shared Windows contracts rather
than 69 independent product defects. POSIX-style Windows disposition removes a
link when the disposition handle closes, so exact private retirement now uses
a dedicated clone and closes it before checking namespace absence. Windows
reports retained-lease CREATE_NEW and lock contention as raw sharing/lock
violations (32/33), so the binder safely reopens the no-delete-sharing identity
after 32 and the nonblocking lock wrapper normalizes 33. Relative private file
creation grants add-file access on the retained directory, and a missing
private-audio authority fixture now uses a native absent path rather than a
Unix-shaped URI. Source/sibling inode-swap fixtures are POSIX-scoped because
Windows safely moves the already-authorized exact handle; adjacent
Windows-capable atomic-transfer tests preserve and assert the new pathname
winner.

THE THIRD REPLACEMENT REMAINS BLOCKED, BUT MADE THE RESIDUAL CONTRACT EXACT.
Hosted run 30504083202 at `82030290` made lint, Linux, every short
cross-platform lane, and the Windows CLI install green, but the ordinary
Windows core result remained red: 1,441 passed, 60 failed, and 5 ignored.
The shared private-retirement output proved that a `File::try_clone` is not an
independent Windows POSIX-delete file object: disposition on the duplicate
still conflicted with the retained original. Exact retirement now consumes the
capability, closes its confirming pathname handles, applies disposition to the
retained exact file object, and closes it before asserting namespace absence.
The API shape prevents a caller from accidentally keeping that internal
authority alive after retirement.

The same run also proved that generic mutation-capable file attestation cannot
be reused for a Windows no-delete-sharing lease. It requested DELETE access
while the retained lease deliberately denied delete sharing, producing raw
sharing violation 32 against Minutes' own handle. Lease files now rebind and
identity-attest through the same no-delete-sharing open contract before and
after lock/audit operations. The remaining isolated search fixture retires its
intentionally stale mutation authority before opening a fresh explicit
restricted-content override, and split-state watcher errors carry the audio
and sidecar recovery locations in the outer typed error context.

STATUS: `e9b2ac90`, `a62cc0b0`, `91d8ee43`, `d5d5f94f`, and `82030290` are not
accepted.
Exact terminal receipts for the replacement candidate are recorded on draft PR
#604 rather than in a post-validation receipt-only commit. Acceptance still
requires ordinary CI and the focused validation-only Windows workflow to be
green for the same exact current-main-based SHA, followed by three fresh
independent non-blocking reviews of that SHA. The draft PR must remain
unmerged, and no release action is authorized.
