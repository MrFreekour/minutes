# Minutes Archive private pilot release-candidate record

Date: 2026-07-30

Branch: `feat/minutes-archive-discovery`

Installed development surface:
`~/Applications/Minutes Archive Dev.app`

Bundle identifier: `com.useminutes.archive.dev`

This record distinguishes the verified local development candidate from a
Peter-ready distribution. The current app is intentionally a separate Minutes
target and does not read from, write to, or import content into the Minutes
meeting store.

## Implemented boundary

- Native multi-folder selection for Documents, iCloud Drive, external drives,
  or other individually approved locations.
- A first-pass metadata-only census that emits aggregate counts but no names,
  paths, hashes, or content.
- Explicit, separate content authorization after census review.
- In-memory legal provision and document-level retrieval for searchable PDF,
  DOCX, TXT, TEXT, and Markdown.
- Exact excerpts with document title, stable page, paragraph, or section
  anchor, source revision, and converter version.
- Final approved-root, membership, no-link, identity, byte, and SHA-256
  revision checks before any result reaches the webview.
- Automatic withdrawal of moved, replaced, mutated, or inaccessible evidence.
- PDF and DOCX conversion in a deny-by-default, network-denied, resource-limited
  parser worker.
- Separately labeled meaning-similar suggestions using Apple's pinned built-in
  English sentence model. Provision and query embeddings run in a second
  resource-limited worker that denies network access and access to user,
  volume, and network roots.
- No source content, FTS rows, or semantic vectors persisted.
- Closing the only Archive window terminates the process so an invisible app
  cannot retain source text, FTS rows, or semantic vectors. The visible footer
  tells the user that closing ends the session and discards the index.
- No downloaded model, QMD runtime, cloud AI, generated legal answer, shell,
  opener, broad filesystem permission, or webview network permission.

## Reproducible verification

Run:

```sh
./scripts/verify-archive-dev-app.sh
```

The verifier checks the installed bundle seal; runs the focused Rust tests,
legal benchmark, both real worker tests, and strict Clippy; rejects vulnerable
`quick-xml 0.37.5` if it enters the macOS Archive dependency tree; exercises
TXT, DOCX, and PDF through the installed app executable; verifies current
evidence and mutation withdrawal; runs the deterministic UI interaction smoke;
runs an installed native-window lifecycle smoke that requires a visible main
window, a real close event, and process exit; and prints the bundle identity
and executable SHA-256.

Observed on 2026-07-30:

- Focused Rust tests: 47 passed, 0 failed.
- Legal retrieval benchmark: passed.
- Installed-executable document/worker smoke:
  `document_vault_smoke=passed indexed=3 current_after_mutation=2`.
- Deterministic UI interaction smoke: one approved location, two evidence
  cards, search view visible.
- Installed native lifecycle smoke:
  `archive_native_lifecycle=passed window=visible close=purged`.
- Installed bundle seal: valid and satisfies its designated requirement.
- Installed executable SHA-256:
  `54797b481c2eb09e8e72197f5d3623f999ab3b4abf3c723b2301278093410f3f`.
- Fresh installed app process: no open network socket observed.
- macOS Archive tree: `quick-xml 0.41.0`; no `quick-xml 0.37.5`.

The whole Minutes workspace audit still reports two high-severity advisories
for `quick-xml 0.37.5`. That version is retained by the main Minutes app's
Windows-only notification dependency and is not in the macOS Archive app tree.
The whole workspace also contains informational unmaintained and unsound
transitive warnings. These facts are recorded, not waived.

## Unclosed Peter handoff gates

- The current bundle is ad-hoc signed. An Apple Development identity exists in
  the local keychain but was unavailable to the non-interactive signer
  (`errSecInternalComponent`) and is not a Developer ID distribution identity.
- No accessible `Developer ID Application` identity was found. Developer ID
  signing, notarization, staple verification, and another installed-artifact
  hash are required before sending the app to Peter.
- `.github/workflows/signed-archive-acceptance.yml` provides the bounded
  distribution path after review and merge: it accepts only an exact candidate
  protected by `acceptance-<sha>`, builds and exercises the app before any
  credential is unlocked, pauses at the existing reviewed
  `signed-dev-acceptance` environment, then signs and notarizes only the inert
  provenance-bound artifact. It cannot be dispatched until the fixed workflow
  is present on `main`, and the environment reviewer must explicitly approve
  the signing job.
- Native Computer Use could not start its host pipe. The deterministic Chrome
  interaction test and installed native window lifecycle test passed, but the
  console session was locked and a human must still click-test the installed
  Tauri app, native folder picker, cancellation, export, supported indexing,
  search, and source withdrawal.
- The end-to-end workflow has not yet been exercised by a human with networking
  disabled. Worker network denial is enforced and self-tested, but that does
  not replace the full installed-app offline test.
- Independent security review remains required. The implementation author
  cannot satisfy the independence condition.
- Real format coverage is unknown until Peter runs the metadata-only census.
  OCR, legacy Word, WordPerfect, email-container parsing, Apple packages, and
  iCloud hydration must be prioritized from those aggregate counts rather than
  guessed.

This is therefore a strong local development release candidate, not a
production claim, regulatory certification, or attorney-ready distribution.
