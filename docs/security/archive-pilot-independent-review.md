# Minutes Archive private-pilot security review packet

This packet is for the independent reviewer of the Peter pilot. It describes
the exact product under review and the evidence required before the installer
may be handed to an attorney. It is not a certification, legal opinion, or
substitute for the attorney's professional-responsibility analysis.

## Review object

The review object is one notarized `Minutes Archive.app` produced from an exact
40-character candidate commit by the protected
`Signed Archive Pilot Acceptance` workflow. The download must contain:

- `minutes-archive-pilot-notarized.zip`;
- `minutes-archive-pilot-notarized.zip.sha256`; and
- `signed-archive-provenance.txt`.

Run the repository's verifier on a Mac before opening the application:

```sh
./scripts/verify-archive-pilot-artifact.sh /path/to/downloaded-artifact-directory
```

The verifier fails unless the zip digest, provenance, Developer ID signature,
Minutes Team ID `63TMLKT8HN`, production identifier
`com.useminutes.archive`, notarization ticket, staple, Gatekeeper assessment,
executable digest, and forbidden-entitlement checks all agree.

## Product boundary to verify

The pilot is a separate Minutes target. It does not import material into the
Minutes meeting store and has no cloud mode.

| Boundary | Required behavior | Primary evidence |
| --- | --- | --- |
| Location authority | Only folders chosen in the native picker are scanned; overlapping roots are rejected | root-approval tests and native picker test |
| Census privacy | Census reads directory entries and file metadata, not regular-file bytes; export contains no names, paths, hashes, or content | census unit tests and exported synthetic report inspection |
| Content authority | Opening documents is a separate action available only after a reviewed census | UI smoke and native interaction test |
| Parser isolation | PDF and DOCX bytes cross bounded pipes into a resource-limited, network-denied worker; paths do not | converter worker tests and sandbox source review |
| Semantic isolation | Apple's built-in revision-pinned model runs in a separate resource-limited, network-denied worker; no model download API is called | semantic worker tests and dependency/source review |
| Derivative lifetime | Source text, FTS rows, and vectors are in memory only; closing the sole window exits the process | persistence search, native close lifecycle smoke, process inspection |
| Evidence fidelity | Results are exact excerpts with source revision and page, paragraph, or section anchors | legal benchmark and document-vault smoke |
| Live-source fence | Root membership, link status, file identity, bytes, and SHA-256 are rechecked before display; stale evidence is withdrawn | mutation/replacement tests and document-vault smoke |
| Webview authority | No filesystem, shell, opener, updater, autostart, global shortcut, or network capability is exposed | Tauri capability and CSP inspection |
| Distribution | Exact protected commit is built and exercised before credentials unlock; candidate code is not executed afterward | fixed workflow policy tests and run log |

## Adversarial review cases

The reviewer should independently attempt at least these cases:

| Case | Required result |
| --- | --- |
| Add a parent and its child as separate roots | overlap is refused |
| Place a symbolic link inside an approved root | link is skipped and never traversed |
| Cancel a census | no partial report is retained for export |
| Cancel content indexing | no partial vault remains searchable |
| Use a PDF containing prompt-like instructions | text is treated as evidence, never orchestration |
| Replace or mutate a matched source after indexing | result is withdrawn |
| Remove or disconnect an approved root | results from that root become unavailable |
| Search with a wrong or empty vault scope | no content is returned |
| Ask for three required concepts in one clause | only one-provision conjunctions qualify |
| Ask for criteria anywhere in one document | each criterion is tied to exact evidence in that same document |
| Exceed the candidate budget | search fails closed rather than claiming completeness |
| Disable networking for the entire installed-app session | census, indexing, exact search, and supported semantic suggestions still work |
| Close the only window after indexing | process exits and cannot answer without rebuilding the vault |

## Egress and observation checks

Use synthetic documents only for security testing. With networking disabled,
exercise census, content authorization, PDF and DOCX conversion, semantic
suggestions, export, and close. Repeat with networking enabled while observing
the app and both workers. Any network connection attributable to the Archive
processes is a stop-ship finding.

Inspect unified logs, crash reports, window titles, exported census JSON, and
temporary directories for synthetic canary strings, filenames, source paths,
extracted text, prompts, and vectors. Any confidential derivative outside the
authorized evidence UI or explicit export is a stop-ship finding.

## Known retrieval limitations disclosed to the reviewer

These are open defects the implementation author found and did not fix. They
are listed so the reviewer tests them deliberately rather than discovering them
as surprises, and so the boundary between "known and bounded" and "stop-ship"
is drawn by the reviewer rather than assumed.

**PDF clause extent is inferred, so a same-provision claim is confined to one
paragraph.** The segmenter closes a provision at a heading. A heading marks
where a provision begins and never where it ends, so a clause that carries no
heading the converter can see is absorbed by the clause above it, and a
provision may hold more than one clause.

An earlier candidate tried to solve this by detecting captions better. It made
matters worse, and the reason is the core of this limitation. Boundary
confidence was decided per document: any single heading marked every provision
in the file trustworthy. A reviewer's document with one administrative line
("Attention General Counsel") over two unheaded operative clauses therefore
returned a card asserting a conjunction the document never made. That
reproduction is now a checked-in regression fixture,
`tests/fixtures/archive-real-pdf/unheaded-clauses-under-one-notice.pdf`.

The fix is at retrieval rather than detection. Every required term must fall
inside one *clause unit*: the whole provision for a format that declares where
clauses begin, and one paragraph for a format that reports only layout.
Paragraph separation is the part of PDF structure that is genuinely observable,
which is why it is the span the app is willing to stand behind.

Scope and mitigation, all verified by execution:

- DOCX is unaffected and matches whole provisions: `w:pStyle` declares clause
  starts, so a conjunction spanning two paragraphs of one clause is real. The
  converter deliberately reports no paragraph layout for DOCX, and a test in
  `minutes-archive-convert` asserts that contract, because reporting it would
  silently narrow every DOCX clause to a single paragraph.
- Caption recovery from uniformly formatted PDFs is restored, now that it is
  safe. The paragraph-break reference was the median gap, which IS the
  paragraph gap when paragraphs are single-line, so the threshold sat above
  every gap in such a file and no boundary was ever found. It is anchored to
  font size instead.
- Over-detection is guarded: gap width alone cannot separate a caption from a
  signature block, so a caption must also introduce prose.
  `double-spaced-signature.pdf` asserts no signature line becomes a caption
  while the real captions in the same document survive.
- A caption on the first line of a page is still not marked. Treating that
  position as a paragraph start was tried and reverted: running headers sit
  there, and it severed a carve-out from the liability cap it limits.
- The excerpt is always displayed, so any residual overstatement is visible.
- Cards claiming a conjunction on an uncaptioned provision still say so.

What the reviewer should attack: whether a paragraph is reliably detected in
real producer output, since a *missed* paragraph split is now the way a
conjunction could still span two clauses. Over-splitting only withholds an
answer and is the safe direction; under-splitting is not. Five mutations of the
rule and its inputs are each caught by the suite.

**PDF page-boundary segmentation is layout-derived.** Provision extents in PDFs
come from page and paragraph layout, not from a structure the file declares.
The "Evidence fidelity" row above is exact about excerpts, revisions, and
anchors -- the excerpt is genuinely the source text at the cited anchor -- but
provision *extent* in a structureless PDF is inferred.

## Stop-ship criteria

The pilot must not be delivered if the reviewer finds any unresolved issue
that can:

- disclose a filename, path, document byte, excerpt, prompt, or derivative
  outside the approved local operation;
- escape an approved root or follow a link or reparse point;
- return evidence after its source is no longer current and authorized;
- persist source text or vectors across application exit;
- execute document text or candidate-controlled code as instructions;
- silently use a network or downloaded model;
- present a generated legal conclusion as source evidence;
- bypass the protected signing, notarization, or provenance boundary; or
- make a materially broader claim than the tested format and location coverage.

## Review record

The final review report should identify the candidate commit, notarized zip
SHA-256, executable SHA-256, macOS version, test Mac architecture, review date,
reviewer, methods used, findings with severity, fixes retested, residual risks,
and a clear approve or do-not-approve decision. The reviewer—not the
implementation author—owns that decision.

An author-run adversarial pass was completed before this review and is logged
in `docs/security/archive-pilot-pre-review-findings.md`. It records five
findings and their fixes, what was probed and held, the residual risks, and --
most usefully -- what it did not reach. It is explicitly not an independent
review and not approval: it was commissioned by the implementer, on the
implementer's code. Read it to avoid duplicating covered ground, not to
shorten the review.

Use `docs/security/archive-pilot-review-record-template.md` as the reviewer-
owned record. Its initial `NOT REVIEWED` state is intentional and must never be
treated as approval.
