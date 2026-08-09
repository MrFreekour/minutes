# Tagged-PDF structure trees as declared boundaries

Status: design note only — deliberately not implemented. Written 2026-08-09
alongside the text-layer trust work, after reviewing run-llama/liteparse's
structure-tree extraction.

## The idea

Every PDF is currently withheld from same-clause answers because nothing in a
PDF marks where a clause *ends* (see the ruling in `convert_pdf` and
`docs/design/archive-ocr-scope.md`). That ruling is correct for the PDF page
model — text placement is not structure.

But a **Tagged PDF** (PDF/UA, and ordinary exports from Word and Acrobat with
accessibility tagging on) carries a structure tree: `H1`/`H2`/`P` elements
declared by the producing software from the author's own styles. That is not
a guess about the page; it is the same class of evidence as `w:pStyle` in
DOCX — which is exactly what this pipeline already accepts as
`ProvisionBoundaries::Declared`. A tagged PDF exported by Word knows where
its headings are because Word knew.

So the upgrade path exists in principle: a PDF whose structure tree is
present and coherent could get declared boundaries and same-clause answers,
while untagged PDFs keep the refusal unchanged.

## Why not now

1. **No demonstrated need yet.** The pilot will show whether PDF same-clause
   refusals actually cost an attorney anything in practice. Exact phrase,
   whole-document search, excerpts, and anchors all work on PDFs today.
2. **Trust criteria are the hard part.** A structure tree can be junk:
   auto-tagged by a converter that guessed, partially tagged, or stale
   relative to the visible text. Accepting one requires positive coherence
   checks (tags cover the extracted text, heading tags align with pages, the
   tree round-trips to the same reading order) — the same "only
   high-confidence results may rewrite" bar as everything else here. Getting
   that wrong recreates the markerless-PDF defect the current ruling fixed,
   from a fancier direction.
3. **Parser support.** `lopdf` can walk `/StructTreeRoot`, but marked-content
   correlation (`MCID` → content-stream spans) is real work; PDFium exposes
   it directly but is a large C dependency whose adoption deserves its own
   decision (sandboxed like the other workers if ever adopted — the
   bounded-worker architecture fits, but supply chain and build cost do not
   come free).

## If it is built

- The verdict must be **typed and decided in the converter**, like
  `TextOrigin` — never inferred downstream from warnings.
- Default stays `Inferred`; only a tree that passes the coherence checks
  earns `Declared`. Fail toward refusal.
- A reviewer should get a fixture pack of hostile trees (mis-nested,
  truncated, mismatched MCIDs, tags disagreeing with visible text) before any
  trust rule ships — this feature is exactly the kind that looks done before
  it is safe.
- Revisit PDFium then, not before: if the coherence checks need marked-content
  spans anyway, the dependency decision and this feature are one decision.
