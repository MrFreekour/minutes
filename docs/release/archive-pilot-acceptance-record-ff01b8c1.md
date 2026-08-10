# Archive pilot acceptance record — candidate ff01b8c1

Verification record required by
[archive-pilot-signing-and-handoff.md](archive-pilot-signing-and-handoff.md),
"Download and verify". Recorded 2026-08-10 by the archive lane operator
session under explicit owner authorization from Mat Silverstein for the
acceptance tag, the `signed-dev-acceptance` gate release, and artifact
verification.

Supersedes
[archive-pilot-acceptance-record-b18f007b.md](archive-pilot-acceptance-record-b18f007b.md).
That artifact remains verified and valid; it simply predates the work below,
and the procedure requires a new commit and a new acceptance tag for any
change after review.

## What this candidate adds over b18f007b

- **Copier-OCR side door closed** (PR #687). A scanned PDF whose scanner
  embedded an OCR text layer used to index as `Extracted`, so a machine's
  reading of a fax was quotable as the exact language of the source. The
  converter now issues a typed `TextOrigin` verdict on the two-signal copier
  signature and the vault routes such a document down the transcription road.
  A text layer garbled past reading converts as needing OCR rather than being
  quoted as mojibake. See `docs/design/archive-text-layer-trust.md`.
- **Setup-screen defects** (PR #715). The approved-location glyph rendered as
  a misaligned sliver (missing font glyph *and* a CSS specificity collision);
  the window opened too short for its own first screen; the route back to
  adding folders was unfindable.
- **Approved-location reveal** (PR #715). "Show in Finder" per row: the
  interface sends the opaque id, the path is resolved in Rust, handed to
  Finder, and dropped. The folder name is deliberately never shown — in a
  practice the folder name is the sensitive part.
- **Per-location counts** (PR #720). Rows now carry item and byte counts so an
  owner with several matters approved can tell them apart. The totals are
  `serde(skip)`, so the privacy-reviewed export shape
  (`minutes.archive-census.v1`) is byte-identical to what was reviewed.
- **Plain-language sweep** (PR #720). The app spoke to an attorney like a
  design document. Every user-facing string was rewritten; the claims are
  unchanged. Two changes went past wording: "Private legal work product" was
  a term of art the app cannot assert, and the "opaque location numbers"
  panel described a mechanism whose only felt effect was an annoyance, so it
  now states the benefit the reader can act on — a saved report is safe to
  share.

## Candidate

- Candidate SHA: `ff01b8c1994594c9dbeae7eabd2746c88ba23b18`
  (tip of `main` and of `feat/minutes-archive-discovery`; the branch was
  fast-forwarded from `b18f007b`, which was an ancestor)
- Acceptance tag: `acceptance-ff01b8c1994594c9dbeae7eabd2746c88ba23b18`
  (annotated; peels to the candidate commit exactly)

## Workflow run

- Run URL: https://github.com/silverstein/minutes/actions/runs/31422059657
- Dispatched from `main` by `silverstein`; actor and triggering actor both
  `silverstein`.
- Jobs, all `success`: Authorize exact protected Archive candidate; Build and
  exercise Archive without signing secrets; Sign and notarize reviewed inert
  Archive app.
- `signed-dev-acceptance` released after confirming actor, `main` ref, exact
  tag→candidate binding, green pre-signing jobs, and that it was the only
  environment waiting.

## Artifact

- Artifact name: `minutes-archive-pilot-notarized-ff01b8c1994594c9dbeae7eabd2746c88ba23b18`
- Contents (exactly three): `minutes-archive-pilot-notarized.zip`,
  `minutes-archive-pilot-notarized.zip.sha256`,
  `signed-archive-provenance.txt`
- zip SHA-256: `1eae27407c134b094b375fbf5a5f1aa5f5c992e8baa2f29700dfd33aca9e1614`
- executable SHA-256: `169ff64c24a4231805aa011bad30964f601c1c0ff2942992b4e7a13b1dfe764f`
- Team ID: `63TMLKT8HN`
- Bundle identifier: `com.useminutes.archive`
- Notarization `notarized=true`; staple `stapled=true`; `stapler validate` —
  "The validate action worked!"
- Gatekeeper: `accepted`, `source=Notarized Developer ID`
- codesign: valid on disk; satisfies its Designated Requirement
- Verifier output: `artifact_verification=passed`
  (`./scripts/verify-archive-pilot-artifact.sh`, run from the exact candidate
  checkout `~/Sites/minutes-archive-review` at `ff01b8c1`)

## Verifier Mac

- macOS 26.6 (build 25G5065a), Mac16,5, Apple M4 Max

## Pre-tag candidate verification (same checkout)

`./scripts/verify-archive-dev-app.sh` passed end to end, including
`archive_pilot_soak` at 4,000 documents under the 256-descriptor GUI ceiling:
`indexed=4000 open_file_limit_reached=0 semantic_bound=true
semantic_partial=false broad_query=20cards/2200considered broad_seconds=0.90
peak_rss_mb=42 seconds=66.7`.

Every screen was additionally driven end to end in the installed dev app
(choose folders → count → open documents → search) with the rendered result
read back, which is how the setup-screen defects and the remaining jargon were
found; unit tests and type checks do not catch either.

## Outstanding before handoff to Peter

Per "Human and independent acceptance" — not performed by this session:

- [ ] Operator Finder click-test with networking disabled, once
- [ ] Operator Finder click-test with networking enabled, under observation
      (no network connections from any Archive process or worker)
- [ ] Click-tests cover: native folder picker, cancellation, census export,
      content authorization, exact retrieval, stale-source withdrawal,
      close-time purge
- [ ] No canary, path, filename, content, prompt, or vector leak to logs,
      crash reports, temporary storage, or the saved report
- [ ] Independent reviewer report says approve
      (`docs/security/archive-pilot-independent-review.md`)
- [ ] Delivery hash matches the reviewed notarized zip
      (`1eae2740…ca9e1614`)

An earlier QA pass on `b18f007b` exercised much of the click-test list against
that artifact and found no leaks — the app held zero network sockets, exited on
window close, and the saved report contained only counts. Those results
informed the fixes above but do **not** carry over: this is a different
artifact and the list has to be walked again on it.

Any failure leaves the artifact quarantined. Fixes require a new commit and a
new acceptance tag; do not mutate or silently replace a reviewed artifact.
