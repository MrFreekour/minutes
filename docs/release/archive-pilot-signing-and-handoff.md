# Archive pilot signing and handoff

This is the release-operator procedure for producing Peter's private-pilot
application. Peter does not run these commands. He receives a normal signed and
notarized `Minutes Archive` DMG and uses Finder.

No step in this procedure authorizes use of real client documents. Signing,
notarization, security review, and operator QA use only the repository's
synthetic fixtures.

## Trust boundaries

- Merging the fixed signing workflow is a repository change and requires the
  owner's explicit approval.
- Creating `acceptance-<sha>` authorizes one exact candidate for the protected
  workflow. Never create, move, replace, or force-push that tag casually.
- The `signed-dev-acceptance` environment is the human credential-release
  boundary. Approve it only after the unsigned build and tests succeed for the
  expected candidate.
- A green workflow produces a candidate for independent review. It is not
  itself an approval to send the app to Peter.

## Current control shape

Recheck this state immediately before a release; it can drift:

- `.github/workflows/signed-archive-acceptance.yml` must be present on `main`;
- the workflow must accept only a full candidate SHA protected by the exact
  `acceptance-<sha>` tag and run from `main` as `silverstein`;
- the active `protect-acceptance-tags` ruleset must cover
  `refs/tags/acceptance-*`;
- a release-tag ruleset must restrict `refs/tags/archive-v*` to the owner; a
  pushed Archive version tag publishes public release assets and advances the
  stable updater channel;
- the `signed-dev-acceptance` environment must require a reviewer and permit
  deployment only from `main`; and
- the referenced credential names must exist:
  `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_SIGNING_IDENTITY`, `APPLE_API_ISSUER`, `APPLE_API_KEY`, and
  `APPLE_API_PRIVATE_KEY`, plus `TAURI_SIGNING_PRIVATE_KEY` for updater
  artifacts.

Do not print, download, or inspect credential values during this preflight.

## Select and freeze the candidate

1. Confirm the Archive pull request is still a draft, mergeable, and green.
2. Review the complete candidate diff and record its full 40-character head
   SHA. Do not use a short SHA.
3. Run `./scripts/verify-archive-dev-app.sh` from that exact checkout.
4. Make no candidate changes after review. Any change requires a new commit,
   fresh CI, fresh review, and a different acceptance tag.
5. After explicit owner approval, merge the fixed signing-workflow pull
   request into `main`. Do not merge the Archive feature pull request merely to
   make the signing workflow available.

## Authorize the exact commit

Set `CANDIDATE_SHA` to the reviewed 40-character Archive commit:

```sh
git fetch origin main feat/minutes-archive-discovery
git cat-file -e "${CANDIDATE_SHA}^{commit}"
test "$(git rev-parse "origin/feat/minutes-archive-discovery")" = "$CANDIDATE_SHA"
git tag -a "acceptance-$CANDIDATE_SHA" "$CANDIDATE_SHA" \
  -m "Authorize Minutes Archive private pilot $CANDIDATE_SHA"
git push origin "refs/tags/acceptance-$CANDIDATE_SHA"
```

Stop if the remote tag already exists but does not resolve to the exact
candidate. Never repair an authorization mismatch by moving the tag.

Dispatch the reviewed workflow definition from `main`:

```sh
gh workflow run signed-archive-acceptance.yml \
  --ref main \
  -f "candidate_sha=$CANDIDATE_SHA"
```

Watch the run. Before approving the protected environment, confirm:

- the actor, `main` ref, candidate SHA, and acceptance tag are exact;
- the unsigned build, focused tests, strict lint, document-vault smoke, and
  native lifecycle smoke all passed; and
- the signing job is waiting at `signed-dev-acceptance`, not another
  environment.

If a signing or notarization job fails, diagnose the exact failure before a
rerun. Never weaken identity, notarization, provenance, entitlement, or
Gatekeeper checks to obtain a green run.

## Download and verify

Download only the artifact named
`minutes-archive-pilot-notarized-<candidate-sha>`. It must contain exactly:

- `minutes-archive-pilot-notarized.zip`;
- `minutes-archive-pilot-notarized.zip.sha256`;
- one notarized `Minutes_Archive_<version>_aarch64.dmg`;
- one signed `Minutes.Archive_<version>_aarch64.app.tar.gz` and its `.sig`;
- `latest-archive.json` and `archive-release-SHA256SUMS.txt`; and
- `signed-archive-provenance.txt`.

From the exact candidate checkout, verify it on a Mac before opening:

```sh
./scripts/verify-archive-pilot-artifact.sh \
  /path/to/minutes-archive-pilot-notarized-artifact
```

Record the workflow run URL, candidate SHA, zip SHA-256, executable SHA-256,
Team ID, bundle identifier, notarization result, staple result, Gatekeeper
result, verifier output, and verifier Mac details.

After independent approval, create and push the exact version tag shown in the
Archive config, for example `archive-v0.2.0`. The same protected workflow then
publishes the versioned DMG and updater archive and advances only
`archive-stable/latest-archive.json`. Confirm that URL returns 200, that its
version and asset URL match the reviewed release, and that the DMG downloaded
from the versioned release has the reviewed SHA-256 before delivery. The normal
Minutes `latest.json` channel is not involved.

## Human and independent acceptance

Generate the synthetic review folder on the test Mac:

```sh
./scripts/make-archive-qa-fixtures.sh \
  "/path/to/empty/Minutes Archive QA Fixtures"
```

Do not place the fixture folder anywhere iCloud Drive syncs -- Desktop and
Documents are synced by default on a Mac with Desktop & Documents enabled. A
QA run that put fixtures on the Desktop found the canary string in
`~/Library/Caches/CloudKit/com.apple.bird/`, which is iCloud's own upload
cache and not the application. Archive workers hold no network sockets; the
parent's expected operations are the announced launch update check and, only
when the operator presses Install, one signed download, both before the fixture
folder is approved. The app never wrote outside its own
space. The leak sweep still has to distinguish the two every time, so keep the
fixtures off synced volumes and the question does not arise.

The release operator completes the Finder interaction once with networking
disabled and once with networking enabled under observation. The independent
reviewer follows `docs/security/archive-pilot-independent-review.md` and owns
the review decision.

The app may be handed to Peter only when:

- the downloaded artifact verifier passes;
- native folder-picker, cancellation, census export, content authorization,
  exact retrieval, stale-source withdrawal, and close-time purge have been
  click-tested;
- the offline run succeeds;
- in the observed online run, no worker opens a network connection at all, and
  the parent performs exactly one launch update check, to the configured
  endpoint and any GitHub-owned release redirect, before any folder is approved,
  carrying no query string and no body. If Install is pressed, one updater
  download is additionally expected. A repeated check, repeated download, or
  any connection after a folder is approved is a failure;
- no canary, path, filename, content, prompt, or vector leaks to logs, crash
  reports, temporary storage, or the census export;
- the independent report says approve; and
- the delivered DMG hash matches the reviewed notarized DMG; and
- the stable updater manifest returns 200 and names that exact reviewed version.

Any failure leaves the artifact quarantined. Fixes require a new commit and a
new acceptance tag; do not mutate or silently replace a reviewed artifact.
