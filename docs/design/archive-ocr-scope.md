# Scanned documents: scope for OCR in Minutes Archive

Status: **proposal, not implemented.** Written 2026-08-06 after a working spike.

## Why

A real census of one folder: 16,619 artifacts, **2,777 images and scans** — 17%
of the archive, and for a thirty-year practice the scans are disproportionately
the old matters. Today they are counted and never read.

## Is it feasible

Yes, and the quality is not the problem. A spike using Apple's Vision framework
(`VNRecognizeTextRequest`, via `objc2-vision`, same binding family already
pinned for `NLEmbedding`) read a synthetic rendered page and returned:

```
CONFIDENTIALITY (conf 1.00)
Section 7.1     (conf 1.00)
```

On-device, no network, no model download, no LLM. Same trust profile as the
semantic model already in use.

**An LLM is the wrong instrument here and should not be considered.** It would
mean sending client documents to a model, which is the single thing this
application exists not to do.

## The finding that shapes the design

Vision OCR **cannot run under the read-scoping the other two workers use.**
Measured by bisecting the profile against a real image:

| Sandbox profile | Result |
| --- | --- |
| Semantic worker's profile (`(deny default)`, reads limited to /System, /usr/lib, /usr/share) | **fails** |
| + `(allow iokit-open)` | fails |
| + `(allow file-read* (subpath "/private/var/db"))` | fails |
| + `/Library` + `/private/var/folders` | fails |
| `(allow file-read*)` everywhere, `(allow iokit-open)` | **works** |
| Scoped reads + `(allow iokit*)` | fails |

So it is the **filesystem read scope**, not IOKit, and Vision wants more of the
filesystem than any allowlist tried here. What can still be denied:

- `(deny default)` still applies
- **no writes anywhere** except `/dev/fd`
- **`(deny network*)`**
- the mach-lookup denylist (pasteboard, distributed notifications, logd,
  diagnosticd, analyticsd, ReportCrash, launchservicesd)
- `RLIMIT_AS` and `RLIMIT_CPU`, bound before the decoder sees a byte

The residual: a worker that receives attacker-controlled image bytes can read
the filesystem. Image decoders are a classic exploit target, so this is a real
step down from the converter and semantic workers, which cannot. It should be
stated plainly in the review packet rather than buried, and the reviewer should
be asked to weigh it directly.

Worth trying before accepting it: run OCR in a **child of the child**, so the
process holding broad read never receives the image — the parent reads the file
and passes bytes down a pipe, and the reading process is the one that dies after
each document. That does not remove the read capability, only shortens its life
and narrows what it ever sees.

## The design constraint that matters more

**OCR output is a transcription, not the source text.**

This tool's entire value is "here is the exact language in your document, at
this anchor". A scan cannot deliver that. It delivers a machine's reading of an
image, and on the material this archive actually holds — 1990s faxes, stamped
exhibits, photocopied signature pages — error rates are not negligible.

So OCR results must be a **separate evidence class**, like the meaning-similar
suggestions already are:

- never returned as `exact_excerpt`
- carry the per-line confidence Vision reports
- labelled as a reading of an image, with the page image as the citation rather
  than a text anchor
- **never** eligible for a same-clause answer: a transcription cannot establish
  that two terms share a clause when it cannot establish where the clause is

The failure to avoid is the one this project has already made twice: a number or
a card that reads as authoritative and is not.

## Scope of work

1. `minutes-archive-ocr` crate: `VNRecognizeTextRequest` behind a worker with
   the profile above, `catch_unwind`, and the existing byte-budget discipline.
2. Route image extensions (`.png .jpg .jpeg .tif .tiff .heic .bmp .gif`) and
   text-layer-less PDFs — the existing `ocr_required_files` counter already
   identifies the latter.
3. A `Transcribed` evidence class through normalize → index → card → UI,
   distinct from `EvidenceCard` at the type level so it cannot be returned as
   exact evidence by mistake.
4. Hostile corpus first, not last: malformed and adversarial PNG/JPEG/TIFF/HEIC,
   decompression bombs, huge dimensions, truncated headers. Assume the decoder
   is the attack surface.
5. Peter's disclosure and the census UI: scans become searchable *as
   transcriptions*, with the accuracy caveat in plain language.
6. Performance: measure before promising. 2,777 images at even 1 s each is 45
   minutes of indexing, and the current design rebuilds the index per session.

## Order

Item 3 first, then 1. Building the plumbing for a distinct evidence class
before there is any OCR to put in it is what stops the transcription being
quietly returned as a quote when the deadline is close.

## Not in scope

Handwriting. Vision recognises it, badly on legal material, and a wrong
transcription of a handwritten margin note is exactly the kind of confident
error this tool must not produce.
