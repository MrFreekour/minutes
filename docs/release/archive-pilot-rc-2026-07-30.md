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
and prints the bundle identity and executable SHA-256.

Observed on 2026-07-30:

- Focused Rust tests: 47 passed, 0 failed.
- Legal retrieval benchmark: passed.
- Installed-executable document/worker smoke:
  `document_vault_smoke=passed indexed=3 current_after_mutation=2`.
- Deterministic UI interaction smoke: one approved location, two evidence
  cards, search view visible.
- Installed bundle seal: valid and satisfies its designated requirement.
- Installed executable SHA-256:
  `8e0c88abf7049c123b8280cd263687149f300397fc869142ba8ee8436ace3a6b`.
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
- Native Computer Use could not start its host pipe. The deterministic Chrome
  interaction test passed, but a human must still click-test the installed
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
