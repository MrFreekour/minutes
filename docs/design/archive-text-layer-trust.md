# Archive text-layer trust

Status: implemented (branch `archive-text-layer-trust`), 2026-08-09.
Prompted by reviewing run-llama/liteparse's per-page complexity routing
against this pipeline's provenance invariant.

## The side door

`TextProvenance` says provenance is decided once at conversion and is the only
thing that determines whether a passage may be quoted. Until this change, the
decision for PDFs was: text layer extracts → `Extracted` → quotable.

Two classes of real file walk through that door:

1. **Copier OCR.** A scanner or copier that embeds OCR produces a PDF whose
   every page is a page-sized image with the machine's reading drawn behind it
   in an invisible text rendering mode. The text extracts like any text layer.
   It is a machine's reading of an image — often a fax, a stamped exhibit, a
   photocopied signature page, exactly the material OCR gets wrong — and it
   was quotable as the exact language of the source.
2. **Garbled layers.** A broken `ToUnicode` CMap yields a text layer that is
   mojibake: replacement characters, control characters, private-use-area
   glyphs. It extracted, so it was quoted.

## The guard

Both are decided in the converter (`crates/archive-convert`), which is where
provenance is supposed to be decided, and both verdicts are typed:

- `ConvertedDocument.text_origin: TextOrigin` — `AuthorWritten` or
  `MachineReadLayer`. Required field, **no serde default**: a worker payload
  that does not state its origin fails to parse instead of parsing as
  quotable. This deliberately repeats the provision-boundaries lesson — a
  hard rule keyed on a warning string a caller can omit is not a rule.
- The copier signature requires **both** signals on one page: text drawn in
  an invisible rendering mode (`3 Tr` or `7 Tr`) *and* an image XObject of at
  least 250,000 pixels (a letterhead logo is far under it; a fax page is
  seven times it). Either signal alone appears in authored documents;
  together they are how every OCR-embedding scanner builds its output.
  Detection is bounded at 500 pages (`TEXT_ORIGIN_PAGES_EXAMINED`).
- A garbled layer (≥ 200 chars, > 20% detectable damage) converts as
  `ocr_required_or_no_extractable_text` with no blocks — the same shape as a
  PDF with no text layer, which is what it functionally is. The pages remain
  eligible for a real recogniser.

On the core side (`crates/archive-core/src/vault.rs`),
`normalize_conversion_by_origin` routes a `MachineReadLayer` conversion down
`normalize_transcribed_document` — the **only** entry point that produces
`Transcribed` — with converter name `pdf-embedded-ocr-layer-v1` so the card
attributes the reading to what actually read it. Page anchors are recovered
from the converter's `page:NNNN` anchors; confidence is `None`, because the
embedded layer reports none and inventing one would be its own provenance
failure (`normalize_transcribed_document` now takes `Option<f32>` for this).
The build report counts a demoted PDF under `transcribed_documents`, not
`searchable_pdf_documents` — counted by what the index holds, not by file
extension.

What a reader sees for a demoted document is what they see for any scan: a
`TranscribedCard` that says the characters are a machine's reading and must be
checked against the page. Never an `EvidenceCard`, never an exact-phrase
claim, never a same-provision answer.

## Deliberate limits

- **Whole-document demotion.** A 40-page typed agreement with one copier-OCR
  exhibit page demotes entirely. Per-page mixed provenance is a real future
  design (the index already stores provenance per provision row), but the
  conservative direction is the safe one: a false demotion loses quotability,
  never truth. Do not loosen to per-page without carrying the same tests.
- **Undetectable damage.** A CMap that maps to the *wrong valid letters* is
  invisible to any ratio heuristic. The garbled guard catches the detectable
  classes only; this is documented, not solved.
- **Past page 500.** A document whose only copier pages sit past the
  examination bound keeps `AuthorWritten`. Known limitation, bounded on
  purpose so a hostile page count cannot buy unbounded work.
- **Visible-text OCR layers.** A producer that embeds its OCR as *visible*
  text over the image does not match the signature. None of the mainstream
  scanner pipelines do this; if one surfaces in the pilot, it becomes a new
  signal, not a loosened threshold.

## Tests

- `crates/archive-convert`: copier signature detected end-to-end through real
  PDF parsing (`a_scanners_embedded_ocr_layer_is_reported_as_machine_read`),
  both-signals requirement in both directions, clip-mode variant, garbled
  ratio positive/negative/minimum cases.
- `crates/archive-core`: `a_machine_read_text_layer_normalizes_as_transcribed_not_extracted`
  pins provenance, boundaries, converter attribution, page-anchor recovery,
  absent confidence, and the extracted control.
- The existing retrieval invariants (`a_transcribed_document_is_never_returned_as_exact_evidence`,
  the document-scope provenance tests) apply to demoted documents unchanged,
  because demotion produces the same `Transcribed` documents those tests
  govern.
