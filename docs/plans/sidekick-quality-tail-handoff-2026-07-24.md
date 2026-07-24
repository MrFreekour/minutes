# Sidekick Quality-Tail Handoff — 2026-07-24

This is the pause seam for a future `minutes:4` implementation lane. It is a
continuation checkpoint, not a release-readiness claim.

## Repository state

- Repository: `/home/mat/Sites/minutes-worktrees/codex-sidekick-demo`
- Branch: `feat/codex-sidekick-demo`
- Parent checkpoint: `1ee7553d75c2e7444330da37aba7fe180ce65fbe`
- No Silverbook install, signed-app acceptance, main merge, or deployment is
  authorized by this handoff.

## What this slice changes

- The output schema makes provenance closure explicit for referenced or
  rejected proposals and rationales named or dismissed in visible text.
- The strategist contract prevents a computed governing constraint from also
  being dismissed as non-decisive.
- The verifier contract treats "I can seek approval" and "I can request an
  exception" as proposed next actions, not claims that approval already exists.
- The margin fixture no longer unconditionally requires the internal
  `utterance_two` receipt when an answer never uses that rationale. A semantic
  forbidden behavior still fails any answer that mentions the logo rationale
  without citing `utterance_two`.

## Evidence already collected

Do not spend provider usage repeating these before inspecting their artifacts:

- Prompt-only provenance attempt: 1/3 margin runs passed. It was insufficient.
- Schema-level provenance attempt: 3/3 margin runs passed with complete
  receipts, no retries, and 5.392-6.846-second totals.
- Anti-inversion attempt: 3/3 runway runs passed with correct governing logic,
  no retries, and 5.827-7.315-second totals.
- Final full corpus: 7/7 graded quality, 25/25 insights, zero provider errors,
  6/7 within the complete path. One false verifier
  `incomplete_material_consequence` rejection produced a 12.833-second tail.

Ignored local artifacts are under `target/sidekick-eval/`:

- `margin-provenance-{1,2,3}.json`
- `margin-schema-{1,2,3}.json`
- `runway-reframe-{1,2,3}.json`
- `provenance-schema-full-{1,2}.json`
- `provenance-reframe-full-1.json`

## Exact next problem

The first margin candidate in `provenance-reframe-full-1.json` was:

> I can't approve a 12% cut without a written exception; let's trade a
> narrower scope, term, or prepayment for any concession. The logo is not
> decisive; the 18% margin floor governs. What price-and-cost distribution
> shows whether 12% could cross that floor?

It cited `prior_margin_floor`, `utterance_one`, and `utterance_two`. The
verifier returned `incomplete_material_consequence`, although the verifier
contract explicitly defines this price-concession shape as complete and the
semantic judge found no missing criterion.

Investigate this as verifier calibration and orchestration, not as permission
to add more fixture-specific prompt prose. Candidate approaches must be tested
against these constraints:

1. Never bypass unsupported facts, arithmetic, contradiction, privacy,
   wrong-session, visual, or provenance checks.
2. Do not rerun the strategist when a verifier-only adjudication can resolve
   the dispute.
3. Count all adjudication time in the unchanged 5s/8s latency budgets.
4. Preserve fresh-verifier isolation and immutable evidence seals.
5. Add a deterministic regression for a complete price-concession candidate
   and an almost-identical incomplete candidate.

## Cheap gates before any provider run

```bash
node --test scripts/test/*sidekick*.test.mjs scripts/test/codex_app_server.test.mjs
python3 scripts/check_live_sidekick_fixture_privacy.py
git diff --check
```

Then run one targeted margin scenario. Run a full corpus only if the targeted
case passes without weakening any gate. Do not run repeated provider corpora
until Mat confirms there is sufficient usage budget.
