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
