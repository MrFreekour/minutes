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

**A clause that carries no heading of any kind cannot be bounded.** The
segmenter closes a provision at a heading. Where a PDF is written as continuous
prose, with clauses simply following one another and nothing marking where one
ends, neither the file nor the layout offers a boundary. Those documents are
withheld from same-provision search rather than guessed at.

This was previously much broader, and the reason is worth stating because it
was a defect rather than a limit. The paragraph-break threshold was the median
gap in the document times 1.3. That assumes most gaps are within-paragraph line
spacing; it is false for any document whose paragraphs are mostly a single
line, where the median IS the paragraph gap. The threshold then landed above
every gap in the document and no boundary could ever be found -- in documents
whose captions were plainly visible to a reader. The reference is now the font
size, which no document statistic can distort.

Scope and mitigation, all verified by execution:

- DOCX is unaffected: `w:pStyle` reports the structure directly.
- PDFs with numbered captions ("7. CONFIDENTIALITY"), real heading styles, or
  a larger caption size were unaffected throughout.
- PDFs set in one uniform size with unnumbered title-case captions are now
  segmented correctly. `tests/fixtures/archive-real-pdf/uniform-spacing-captions.pdf`
  covers this and its test no longer asserts withdrawal; it asserts the
  captions are recovered AND that the confidentiality and assignment clauses
  stay separate, so the fix cannot pass by over-detecting.
- Over-detection is guarded in the other direction. Gap width alone cannot
  distinguish a caption from a signature block, and in a double-spaced document
  the ordinary paragraph gap exceeds any size-derived threshold. A caption is
  therefore also required to introduce prose, since a signature line is
  followed by another short line.
  `tests/fixtures/archive-real-pdf/double-spaced-signature.pdf` asserts that no
  signature line becomes a caption and that the real captions in that same
  document survive.
- A caption on the first line of a page is still not marked. Treating that
  position as a paragraph start was tried and reverted: running headers sit
  there, and it severed a carve-out from the liability cap it limits. Page ends
  are hard boundaries, so provisions do not run together across the break.
- The excerpt is always displayed, so any remaining overstatement is visible
  rather than hidden.
- Cards making a conjunction claim on a provision with no caption still say so:
  "This provision carries no section caption, so its extent was inferred from
  the page layout; check the excerpt that the terms are in one clause."

An earlier geometric converter that attempted this was built, reviewed and
reverted, because it silently deleted section captions from documents with no
running header. The present change is narrower: it alters only the reference
the existing gap test measures against, plus the requirement that a caption
introduce something.

Three of the four conditions are mutation-verified. The gap threshold's exact
multiplier is not -- widening it to 0.1 changes no fixture -- and the code
records that rather than implying coverage it does not have. The reviewer
should judge whether the residual, and that untested multiplier, are acceptable
for the pilot.

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
