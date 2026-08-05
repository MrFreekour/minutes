//! Word 97-2003, OpenDocument Text and RTF.
//!
//! These formats were counted as unsupported and never read, which for a
//! thirty-year practice leaves out most of the older agreements. They are
//! parsed by `anydoc`, a crate that was five days old when it was adopted and
//! shipped four releases on the day it was pinned. That is the reason for the
//! hostile corpus below rather than an argument against the dependency: every
//! parser here is treated as hostile, and these formats deserve it most --
//! their containers are classic exploit carriers.
//!
//! The fixtures are synthetic, produced by LibreOffice from a short synthetic
//! NDA. No client material is involved.

use minutes_archive_convert::{
    convert_bytes, ConversionError, SourceFormat, PDF_UNSUPPORTED_STRUCTURE_WARNING,
};

const NDA_DOC: &[u8] = include_bytes!("../../../tests/fixtures/archive-legacy/nda.doc");
const NDA_ODT: &[u8] = include_bytes!("../../../tests/fixtures/archive-legacy/nda.odt");
const NDA_RTF: &[u8] = include_bytes!("../../../tests/fixtures/archive-legacy/nda.rtf");

/// Word declares its outline levels, so its clauses carry their captions and
/// match as whole provisions -- the same standing DOCX has.
#[test]
fn a_word_97_document_reports_its_declared_headings() {
    let document = convert_bytes(SourceFormat::Doc, NDA_DOC).expect("convert");
    let headings = document
        .blocks
        .iter()
        .filter(|block| block.is_heading == Some(true))
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>();
    assert!(
        headings.contains(&"7. Confidentiality and Permitted Exceptions"),
        "declared headings were lost: {headings:?}"
    );
    assert!(
        document
            .blocks
            .iter()
            .all(|block| block.starts_paragraph.is_none()),
        "a format that declares its clause starts must not report paragraph \
         layout, or its clauses narrow to one paragraph each"
    );
}

#[test]
fn an_opendocument_text_reports_its_declared_headings() {
    let document = convert_bytes(SourceFormat::Odt, NDA_ODT).expect("convert");
    assert!(document
        .blocks
        .iter()
        .any(|block| block.is_heading == Some(true)
            && block.text.contains("Assignment and Change of Control")));
}

/// RTF is withheld from same-clause answers regardless of what it contains.
///
/// The first attempt keyed this on whether the parsed document contained a
/// heading, which is the document-level reasoning that caused the PDF defect:
/// one outline level anywhere would have vouched for every paragraph in the
/// file. A reviewer built the RTF that turns that into a wrong answer, and
/// `\outlinelevel` is in fact recognised by this parser, so the premise that
/// "RTF declares no headings" was wrong as well.
///
/// Word and OpenDocument carry a style system that states which paragraphs are
/// captions. RTF outline levels are an afterthought most producers never
/// write, so finding one proves little about the paragraphs around it.
#[test]
fn rtf_is_withheld_from_same_clause_answers() {
    let document = convert_bytes(SourceFormat::Rtf, NDA_RTF).expect("convert");
    assert!(
        document
            .warnings
            .iter()
            .any(|warning| warning == PDF_UNSUPPORTED_STRUCTURE_WARNING),
        "RTF must report that its clause boundaries are not declared: {:?}",
        document.warnings
    );
}

/// The same holds for an RTF that DOES carry an outline level, which is the
/// case the first attempt got wrong.
#[test]
fn an_rtf_carrying_an_outline_level_is_still_withheld() {
    let rtf = br"{\rtf1\ansi
{\pard\outlinelevel0\b 7. CONFIDENTIALITY\par}
{\pard The Recipient shall protect Confidential Information.\par}
{\pard Neither party may assign this Agreement.\par}
}";
    let document = convert_bytes(SourceFormat::Rtf, rtf).expect("convert");
    assert!(
        document
            .warnings
            .iter()
            .any(|warning| warning == PDF_UNSUPPORTED_STRUCTURE_WARNING),
        "an outline level must not license same-clause answers for the whole file"
    );
}

#[test]
fn every_legacy_format_extracts_the_same_operative_text() {
    for (format, bytes) in [
        (SourceFormat::Doc, NDA_DOC),
        (SourceFormat::Odt, NDA_ODT),
        (SourceFormat::Rtf, NDA_RTF),
    ] {
        let document = convert_bytes(format, bytes).expect("convert");
        let text = document
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("Confidential Information") && text.contains("may assign this Agreement"),
            "{format:?} lost operative text"
        );
    }
}

/// Hostile containers must fail closed, quickly, without panicking.
///
/// The worker that runs this is sandboxed under `(deny default)` with
/// `RLIMIT_AS` and `RLIMIT_CPU` bound before the decoder sees a byte, so these
/// assertions are the inner layer rather than the only one. What they check is
/// that the inner layer holds on its own.
#[test]
fn hostile_containers_fail_closed_without_panicking() {
    let cases: [(&str, SourceFormat, &[u8]); 5] = [
        (
            "truncated CFB header",
            SourceFormat::Doc,
            include_bytes!("../../../tests/fixtures/archive-legacy-hostile/truncated.doc"),
        ),
        (
            "CFB with absurd allocation table counts",
            SourceFormat::Doc,
            include_bytes!("../../../tests/fixtures/archive-legacy-hostile/absurd-fat.doc"),
        ),
        (
            "CFB whose directory sector points at itself",
            SourceFormat::Doc,
            include_bytes!("../../../tests/fixtures/archive-legacy-hostile/cyclic-dir.doc"),
        ),
        (
            "ODT decompression bomb",
            SourceFormat::Odt,
            include_bytes!("../../../tests/fixtures/archive-legacy-hostile/bomb.odt"),
        ),
        (
            "ODT declaring an external entity for /etc/passwd",
            SourceFormat::Odt,
            include_bytes!("../../../tests/fixtures/archive-legacy-hostile/xxe.odt"),
        ),
    ];

    for (label, format, bytes) in cases {
        let outcome = std::panic::catch_unwind(|| convert_bytes(format, bytes));
        let Ok(result) = outcome else {
            panic!("{label} panicked the converter");
        };
        if let Ok(document) = result {
            let text = document
                .blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                !text.contains("root:") && !text.contains("/bin/"),
                "{label} returned the contents of a local file"
            );
        }
    }
}

/// Generated rather than checked in: the interesting sizes are megabytes, and
/// a repository is a poor place to keep them.
#[test]
fn pathological_rtf_shapes_terminate() {
    let cases = [
        ("100k nested groups", {
            let mut bytes = b"{\\rtf1".to_vec();
            bytes.extend(std::iter::repeat_n(b'{', 100_000));
            bytes.push(b'x');
            bytes.extend(std::iter::repeat_n(b'}', 100_000));
            bytes.push(b'}');
            bytes
        }),
        ("200k unclosed groups", {
            let mut bytes = b"{\\rtf1".to_vec();
            for _ in 0..200_000 {
                bytes.extend_from_slice(b"{\\b ");
            }
            bytes
        }),
        ("5MB single control word", {
            let mut bytes = b"{\\rtf1\\".to_vec();
            bytes.extend(std::iter::repeat_n(b'a', 5 * 1024 * 1024));
            bytes.extend_from_slice(b" x}");
            bytes
        }),
    ];

    for (label, bytes) in cases {
        let outcome = std::panic::catch_unwind(|| convert_bytes(SourceFormat::Rtf, &bytes));
        assert!(outcome.is_ok(), "{label} panicked the converter");
    }
}

#[test]
fn an_empty_or_oversized_source_is_refused_before_parsing() {
    assert!(matches!(
        convert_bytes(SourceFormat::Doc, b""),
        Err(ConversionError::InputBudgetExceeded)
    ));
}
