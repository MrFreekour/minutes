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
- `minutes-archive-pilot-notarized.zip.sha256`;
- one notarized `Minutes_Archive_<version>_aarch64.dmg`;
- one signed `Minutes.Archive_<version>_aarch64.app.tar.gz` and its `.sig`;
- `latest-archive.json` and `archive-release-SHA256SUMS.txt`; and
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
| Update window | The parent makes one automatic check and at most one user-consented signed download, both before any folder is approved; both are refused thereafter | update-gate tests in `archive/src-tauri/src/main.rs` and network observation |
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
| Approve a folder, then remove it, then try to make the app check for updates | the check is refused; removing the location does not reopen the window |
| Rebuild with the endpoint pointed at a host you control, and serve a manifest signed with your own minisign key | the download is refused and the installed application is unchanged |
| Same, with a signature that is valid but was made over different bytes | the download is refused |
| Drive `check_for_archive_update` in a loop from the webview console before approving anything | exactly one request leaves the process |
| Serve a manifest that never finishes, or a 500, or malformed JSON | the app reports that it could not check and is otherwise unaffected |
| Close the only window after indexing | process exits and cannot answer without rebuilding the vault |

## The bounded update surface, and what it costs

Minutes Archive is no longer network-free. The parent process makes one
automatic outbound check and may make one user-consented update download. The
reviewer should treat that as a real change to the product rather than a
footnote, because every earlier round of this review was
conducted against a binary that made none.

**Why it exists.** The pilot is delivered as a signed application to an
attorney who will not track releases and cannot be asked to re-download it. A
security fix that reaches nobody is not a fix. The alternative considered and
rejected was mailing him a new build each time, which moves the same problem to
an unauthenticated channel and depends on him acting.

**What it is.** A GET of a static JSON file at the endpoint configured in
`archive/src-tauri/tauri.conf.json`. It carries no query string: the endpoint
contains none of the `{{current_version}}`, `{{target}}` or `{{arch}}`
placeholders `tauri-plugin-updater` would otherwise substitute, and
`check_for_archive_update` calls `clear_headers()` before building the updater
so nothing can be attached to it. It runs before a folder has been approved, so
no value derived from the operator's archive exists in the process yet.

**The residual, stated plainly.** The request is not zero-information. It
discloses to whoever serves the endpoint, and to anything on the path, that a
copy of Minutes Archive was opened from that IP at that time. TLS conceals the
path, but SNI and DNS name the host. The request also carries a fixed
`User-Agent` of the form `tauri-plugin-updater/<version>`, which is set by the
library on its own client and cannot be removed through its builder API; it
identifies the library, not the installation. There is no cookie, no
identifier, and no state carried between launches, so two launches are not
linkable except by IP. For an attorney whose network is observed, "this Mac
opened Minutes Archive at 09:14" is the disclosure, and it is not nothing.
Nothing about the archive, the folder, or any document is in it.

**How each step is kept to one.** The gate is `claim_network_window` in
`archive/src-tauri/src/main.rs`. It refuses when either the live session state
shows the process has taken on the operator's archive -- an approved location,
a census report, an open index, or a scan in flight -- or when a monotone latch
was set as soon as the folder picker opened. The latch exists because the live
read alone lets approve-then-remove reopen the window. Both are proven by
mutation: removing the live read makes
`an_update_check_is_refused_once_a_location_is_approved` and
`an_update_check_is_refused_once_an_index_exists` fail; removing the latch makes
`removing_an_approved_location_does_not_reopen_the_window` fail; replacing the
compare-exchange with an unconditional store makes
`a_launch_session_that_has_seen_nothing_may_check_once` fail. The window is also
closed when the folder picker opens, before the operator has chosen anything,
so cancelling the panel does not restore it.

**Signature verification.** Installation goes through
`Update::download_and_install`, which verifies the minisign signature against
the public key in `tauri.conf.json` before writing. That is the same key the
main Minutes application uses; no second key was introduced. There is no other
install path in the file -- the plugin's bare `install` would accept whatever
bytes it was given, and adding a second entry point is how an unverified one
appears later. A download only happens if the operator presses the button
against a visible offer.

**What did not change.** `archive-main` still does not carry `updater:default`,
so the plugin's own commands remain unreachable from the webview; the "Webview
authority" row above is unchanged. Both workers still run under
`(deny network*)`. The CSP still has no `connect-src` beyond IPC.

**Release and recovery path.** Archive uses its own fixed `archive-stable`
manifest, never the normal Minutes `latest.json`. An `archive-vX.Y.Z` tag runs
the protected signing workflow and stages the signed updater archive and
notarized DMG in a private draft release. It does not advance the updater feed.
The reviewer records the checksum-record hash for those exact bytes. A separate
protected promotion checks that hash, makes the same assets public, verifies
their public hashes, and replaces the stable manifest last. The reviewer must
confirm the stable manifest returns 200 and names the exact reviewed version
before delivery. If a release must be stopped,
the stable manifest can be replaced or removed so older clients no longer see
it; clients that already installed it require a higher patch release or a
manual DMG. The UI states that the installed app stays unchanged after a failed
update and names the signed DMG as the recovery path.

## Egress and observation checks

Use synthetic documents only for security testing. With networking disabled,
exercise census, content authorization, PDF and DOCX conversion, semantic
suggestions, export, and close. Repeat with networking enabled while observing
the app and both workers.

The expected observation without accepting an update is exactly one updater
check, from the parent process only, to the configured update endpoint, before
any folder is approved. A test that presses Install additionally observes one
signed download operation. GitHub may redirect either request to its
release-asset host; those redirect hops
belong to the same bounded operation and must stay on GitHub-owned HTTPS hosts.
Any connection from either worker, any repeated check or download, any
connection after a folder has been approved, and any check request carrying a
query string, cookie, or body is a stop-ship finding.

Inspect unified logs, crash reports, window titles, exported census JSON, and
temporary directories for synthetic canary strings, filenames, source paths,
extracted text, prompts, and vectors. Any confidential derivative outside the
authorized evidence UI or explicit export is a stop-ship finding.

## Known retrieval limitations disclosed to the reviewer

These are open defects the implementation author found and did not fix. They
are listed so the reviewer tests them deliberately rather than discovering them
as surprises, and so the boundary between "known and bounded" and "stop-ship"
is drawn by the reviewer rather than assumed.

**No PDF or RTF answers a same-provision query.** A caption marks where a
clause starts and never where it ends, and nothing in a PDF marks the end.

Two narrower rules were tried and an independent reviewer defeated both. The
first trusted a document because it contained a heading, so one administrative
line vouched for two unrelated clauses. The second confined the claim to a
paragraph, which assumed paragraph breaks are reliably visible; a ReportLab PDF
of two separately labelled clauses at ordinary line spacing reported starts of
`true, true, false` and put both in one span. Both reproductions are checked in
as `unheaded-clauses-under-one-notice.pdf` and
`labelled-clauses-at-uniform-spacing.pdf`.

RTF is withheld on the same basis, and the accurate statement is that its
structure signals are not trusted as complete clause boundaries rather than
that it records none: `\outlinelevel` IS recognised by this parser. The first
attempt keyed on whether the parsed file contained a heading -- the same
document-level reasoning -- so a single outline level switched a whole file to
whole-provision matching. `an_rtf_carrying_an_outline_level_is_still_withheld`
pins that closed.

The prohibition is enforced on `SourceFormat`, not on a converter warning. It
previously read a warning string, which let a reviewer pass a markerless PDF
`ConvertedDocument` through the public normalizer and obtain declared
boundaries and a same-clause answer; ingestion was safe only because the
shipped converters always set it.
`a_markerless_pdf_or_rtf_document_still_cannot_declare_its_boundaries` covers
that, and the paragraph-unit machinery that made the bypass reachable is
deleted rather than left in place looking like policy.

What remains for PDFs: exact phrase, whole-document search, excerpts, and page
or section anchors. Caption recovery from uniformly formatted PDFs is retained
because it titles and cites provisions accurately; it simply no longer licenses
a same-clause claim.

**Word, OpenDocument and DOCX do answer, on their declared structure, and the
reviewer's objection applies to them.** A `w:pStyle` heading proves a section
starts there, not that every paragraph beneath it is one legal clause. An
agreement with numbered sub-clauses set as body text under a single styled
heading will be treated as one clause. This is the contract DOCX has had since
the candidate that two rounds found fit, and `.doc` and `.odt` now inherit it
rather than introducing it -- verified by converting the reviewer's own
reproduction to `.docx` and observing identical behaviour. It is disclosed to
the attorney in those terms, alongside the instruction to read the excerpt.

The reviewer should judge whether that residual is acceptable, and should note
that meaning-similarity suggestions embed and return the whole provision; they
carry a "not a determination" label and are not the same-clause path.

**PDF page-boundary segmentation is layout-derived.** Provision extents in PDFs
come from page and paragraph layout, not from a structure the file declares.
The "Evidence fidelity" row above is exact about excerpts, revisions, and
anchors -- the excerpt is genuinely the source text at the cited anchor -- but
provision *extent* in a structureless PDF is inferred.

**Reveal is a check-then-open race.** `source_path_for_reveal` re-verifies the
source revision and then hands a path to `/usr/bin/open -R`. A process that
rewrites the file between that check and Finder resolving the path can make
Finder show a document that no longer matches the quotation. The race requires
a process actively changing the owner's own filesystem, and it affects only
what Finder displays: the quoted text was checked against the source bytes at
response time. Rechecking immediately before reveal was tried and retained;
it cannot make Finder consume the already-verified file handle because Finder's
reveal interface accepts a path.

**Folder exclusions can be defeated by inode reuse.** A skipped folder is
bound to its device and inode. If it is deleted and the filesystem is forced to
reuse that inode for a new folder at the same relative path, the skip applies
to the wrong folder without a separate warning. This requires deliberate local
manipulation of the owner's own filesystem between choosing the skip and
building the vault. Path-only binding was tried and rejected because an
ordinary rename defeated it; inspection and identity capture now happen
together, but closing the inode-reuse gap would require holding an open
directory capability across the build, which is a separate design.

**A PDF page whose visible area is mostly raster imagery is not quotable.** The
converter walks each page's bounded content graph and tracks image placement on
a 32 by 32 coverage grid. The grid uses the inherited CropBox intersected with
the MediaBox, matching the part a reader can actually see. A page is treated as
machine-read when raster draws cover at least half that visible grid and the
contributing images contain at least 250,000 source pixels. This catches one
page image, strips, tiles, cropped scans, inline images, and images reached
through Forms without decoding their pixel data.

Rectangular clipping paths and Form bounding boxes narrow credited coverage;
graphics-state save and restore carries both transforms and clipping. More
complex path geometry is intentionally conservative: it is not allowed to
reduce credited image coverage. That can withhold quotations from a page whose
large image is clipped by a curve or text outline, but it cannot hide a visible
scan and present OCR as the author's exact words.

The verdict is page-local. Provisions on a machine-read page are shown only as
a machine reading and must be checked against the source; independently typed
pages in the same PDF remain eligible for exact quotations. A born-digital
chart or photograph covering most of one page can therefore cost that page's
quotability, but does not demote the rest of the document. Signatures, logos,
stamps, and letterhead that occupy only a small visible region do not meet the
coverage rule.

## Stop-ship criteria

The pilot must not be delivered if the reviewer finds any unresolved issue
that can:

- disclose a filename, path, document byte, excerpt, prompt, or derivative
  outside the approved local operation;
- escape an approved root or follow a link or reparse point;
- return evidence after its source is no longer current and authorized;
- persist source text or vectors across application exit;
- execute document text or candidate-controlled code as instructions;
- use the network at any moment other than the single announced launch check,
  send anything in that request, repeat it, or make it after a folder has been
  approved;
- install an update whose minisign signature does not verify against the key in
  `tauri.conf.json`, or install one without the operator asking;
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
