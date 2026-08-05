# Peter private-pilot acceptance run

Peter should receive a normal signed and notarized Mac application, not a
Terminal command, development checkout, QMD setup, or generic ChatGPT upload
workflow. A release operator completes the artifact verification and security
review before this run.

## Before Peter receives the app

The release operator records the exact candidate commit and verifies the
downloaded notarized artifact with
`scripts/verify-archive-pilot-artifact.sh`. An independent reviewer approves the
security packet in `docs/security/archive-pilot-independent-review.md`. The
operator then performs the complete installed-app interaction once with
networking disabled and once with networking enabled under network
observation.

Do not use Peter's documents for those release tests. Use synthetic legal
fixtures with distinctive canary text. The release operator creates the
review folder with `scripts/make-archive-qa-fixtures.sh` and follows
`docs/release/archive-pilot-signing-and-handoff.md`.

## Peter's first session

1. Peter opens the delivered `Minutes Archive` application in Finder. macOS
   should open it normally without an unidentified-developer override.
2. He selects one small, well-understood pilot folder containing roughly
   100–500 documents. He does not need to reorganize or move them.
3. He runs **Private census**. The application reports only aggregate format,
   size, package, placeholder, permission, and error counts.
4. He saves the aggregate census report with the normal Save dialog. Before it
   is shared, the operator confirms that it contains no filename, source path,
   hash, or document text.
5. Peter reviews the supported and unsupported counts. Only then does he choose
   **Build private search index**, which is the separate authorization to read
   supported documents.
6. He tries questions he already knows how to judge, beginning with:
   “Find confidentiality provisions no more than three sentences covering
   affiliates, compelled disclosure, and survival.”
7. For every useful result, he checks the displayed source title and exact
   page, paragraph, or section anchor against the original document before
   relying on it.
8. He closes the window when finished. Closing ends the private session and
   discards the in-memory index.

## Adding the rest of the archive

After the bounded folder proves useful, Peter may add Documents, locally
available iCloud Drive folders, other cloud-sync folders, and external drives
one at a time through the native picker. An iCloud item that has not been
downloaded is counted as a placeholder; it is not silently searched.

The application reports only the locations Peter approved. It must never be
described as searching the whole computer when a folder, external volume,
cloud item, or protected location was unavailable.

## Expected pilot limitations

The searchable formats are searchable PDF, Word (`.docx` and legacy `.doc`),
OpenDocument Text (`.odt`), Rich Text Format (`.rtf`), TXT, TEXT, and Markdown.
Scanned PDFs are reported as requiring OCR.

WordPerfect, Pages packages, PST/OLM/MSG mail containers, spreadsheets,
presentations, macro-enabled documents (`.docm`), encrypted documents, and
other unsupported formats remain coverage signals, not searchable claims.
Spreadsheets and presentations are excluded deliberately rather than for want
of a parser: this tool segments prose into clauses and quotes them as evidence,
and a worksheet has no clauses to find, so putting one through would produce
confident-looking results assembled from cell text.

Search results are research assistance. They are exact retrieved excerpts, not
legal conclusions, and Peter reviews the source before use. Meaning-similar
suggestions are separately labeled and are never presented as proof that a
clause satisfies the question.

## Two things Peter is told before he starts

These are accepted, disclosed limitations of the pilot, not defects to be
discovered. The operator states both in plain language when handing over the
app. Neither should be softened.

**1. If the app is force-quit or crashes, the location of the folder you chose
may remain on your Mac until you next open the app.**

When you click "choose folder", macOS itself remembers the last folder you
picked. That is a standard macOS behaviour, not something Minutes Archive asks
for, and it cannot be switched off. The app erases that record when it closes
normally, and again the next time it opens. Neither erase can run if the app is
force-quit or crashes.

What could remain is the folder's FULL PATH -- its name, the names of every
folder above it, and the name and identifier of the disk it is on. It is not any
document, not any text from a document, and nothing is ever sent anywhere. It
sits in the app's own settings file on your Mac. In a law practice those folder
names are often client names, which is why this is stated rather than left
unsaid. Opening Minutes Archive again clears it.

**2. PDFs and RTFs never answer "in the same clause".**

Word and OpenDocument files record where each section begins. PDF and RTF do
not -- a PDF records only where ink sits on the page.

The app reads clause captions out of a PDF's layout, and does that well: your
results are titled and cited by clause. But a caption shows where a clause
*starts* and never where it *ends*, and nothing in the page marks the end. Two
attempts were made to work around that and an independent reviewer defeated
both, each time with a document that produced a confident and wrong answer. So
the app no longer makes that claim about a PDF or an RTF at all.

Everything else about them still works: exact phrase, "which documents mention
X and Y", excerpts, and page or section citations. What you will not get is
"these two things are in the same clause".

**Word and OpenDocument answer it, with one thing to know.** Those files state
where their sections begin, so the app trusts them. What a Word heading proves
is that a *section* starts there -- not that everything beneath it is a single
legal clause. If an agreement puts several numbered sub-clauses under one Word
heading without styling them as headings themselves, the app treats that whole
section as one clause and may report two of those sub-clauses as sharing it.
Read the excerpt, which is always shown, before relying on such an answer.

This is deliberate throughout: a wrong "same clause" answer to a lawyer is
worse than no answer.

## Stop and contact the pilot operator

Peter should stop the session if macOS shows an unidentified-developer warning,
the app requests network or unrelated privacy permissions, a census export
contains a filename or path, a result lacks a source anchor, a changed source
remains available, the app claims to search an unavailable location, or the
Archive process remains running after its only window closes.

No real client document should be sent to support, copied into an email, or
uploaded to a model to diagnose the issue. Reproduce with a synthetic document
or share only the aggregate census report.
