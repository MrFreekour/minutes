# Archive pilot acceptance record — candidate b18f007b

Verification record required by
[archive-pilot-signing-and-handoff.md](archive-pilot-signing-and-handoff.md),
"Download and verify". Recorded 2026-08-07 (Pacific) by the archive lane
operator session, under explicit owner authorization from Mat Silverstein for
the acceptance tag, the `signed-dev-acceptance` gate release, and artifact
verification.

## Candidate

- Candidate SHA: `b18f007b485e15938a4c0093460eed863a48a166`
  (tip of `feat/minutes-archive-discovery` and `archive-review-2026-08-07`)
- Review gate: PR #665 (independent adversarial review) merged into
  `archive-review-base` as `cce6d2a5`, plus three post-review fix commits
  (`cf9d1bf3` retrieval provenance, `cc2af866` OCR stdin bound,
  `b18f007b` smoke-mock cleanup) — all included in the candidate and
  re-verified by `./scripts/verify-archive-dev-app.sh` before tagging.
- Acceptance tag: `acceptance-b18f007b485e15938a4c0093460eed863a48a166`
  (annotated; peels to the candidate commit exactly).

## Workflow run

- Run URL: https://github.com/silverstein/minutes/actions/runs/31235481496
- Dispatched from `main` (workflow-definition head `2f2e96e3`) by
  `silverstein`; actor and triggering actor both `silverstein`.
- Jobs, all `success`: Authorize exact protected Archive candidate; Build and
  exercise Archive without signing secrets; Sign and notarize reviewed inert
  Archive app.
- `signed-dev-acceptance` gate released after confirming actor, `main` ref,
  exact candidate SHA/tag binding, green pre-signing jobs, and that the
  waiting environment was `signed-dev-acceptance` only.

## Artifact

- Artifact name: `minutes-archive-pilot-notarized-b18f007b485e15938a4c0093460eed863a48a166`
- Contents (exactly): `minutes-archive-pilot-notarized.zip`,
  `minutes-archive-pilot-notarized.zip.sha256`,
  `signed-archive-provenance.txt`
- zip SHA-256: `290ea69e3b7abf4824234acc04b9153802e7ff99dae466a35fa3a57c67f70eaf`
- executable SHA-256: `a9f485d481bed87095a1f3da0f6fa795e7fec3340ee6d22c27ff485167da46c9`
- Team ID: `63TMLKT8HN`
- Bundle identifier: `com.useminutes.archive`
- Notarization: `notarized=true`; staple: `stapled=true` (provenance file);
  `stapler validate` — "The validate action worked!"
- Gatekeeper: `accepted`, `source=Notarized Developer ID`
- codesign: valid on disk; satisfies its Designated Requirement
- Verifier output: `artifact_verification=passed`
  (`./scripts/verify-archive-pilot-artifact.sh`, run from the exact candidate
  checkout `~/Sites/minutes-archive-review` at `b18f007b`)

## Verifier Mac

- macOS 26.6 (build 25G5065a), Mac16,5, Apple M4 Max

## Pre-tag candidate verification (same checkout)

`./scripts/verify-archive-dev-app.sh` passed end to end, including
`archive_pilot_soak` at 4,000 documents under the 256-descriptor GUI ceiling:
`indexed=4000 open_file_limit_reached=0 semantic_bound=true
semantic_partial=false broad_query=20cards/2200considered broad_seconds=0.88
peak_rss_mb=42 seconds=64.1`.

## Outstanding before handoff to Peter

Per "Human and independent acceptance" — not performed by this session:

- [ ] Operator Finder click-test with networking disabled, once
- [ ] Operator Finder click-test with networking enabled, under observation
      (no network connections from any Archive process or worker)
- [ ] Click-tests cover: native folder picker, cancellation, census export,
      content authorization, exact retrieval, stale-source withdrawal,
      close-time purge
- [ ] No canary/path/filename/content/prompt/vector leak to logs, crash
      reports, temp storage, or census export
- [ ] Independent reviewer report says approve
      (`docs/security/archive-pilot-independent-review.md`)
- [ ] Delivery hash matches the reviewed notarized zip
      (`290ea69e…f70eaf`)

Any failure quarantines the artifact; fixes require a new commit and a new
acceptance tag.
