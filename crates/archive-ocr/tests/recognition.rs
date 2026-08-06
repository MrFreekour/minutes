//! Reading a real rendered page, and refusing hostile images.
//!
//! The fixture is a synthetic NDA rendered by macOS. No client material is
//! involved, and none ever should be: a scan of a real matter must not enter
//! this repository or any test run.

#![cfg(target_os = "macos")]

use minutes_archive_ocr::{recognize_page, OcrError, MAX_IMAGE_BYTES};

const SCANNED_NDA: &[u8] = include_bytes!("../../../tests/fixtures/archive-ocr/scanned-nda.png");

#[test]
fn a_rendered_page_is_read_with_its_operative_language_intact() {
    let page = recognize_page(SCANNED_NDA).expect("recognize");
    let text = page
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        text.contains("CONFIDENTIALITY"),
        "the caption was not read: {text}"
    );
    assert!(
        text.contains("Confidential Information"),
        "the operative term was not read: {text}"
    );
    assert!(
        text.contains("assign this Agreement"),
        "the assignment clause was not read: {text}"
    );

    // Every line carries a usable confidence, since that number is what the
    // card shows a reader deciding whether to check the original.
    for line in &page.lines {
        assert!(
            (0.0..=1.0).contains(&line.confidence),
            "line confidence out of range: {line:?}"
        );
    }
    let lowest = page.lowest_confidence();
    assert!(
        (0.0..=1.0).contains(&lowest),
        "page confidence out of range: {lowest}"
    );
}

/// Clean rendered type should read well. This is a floor, not a promise about
/// real scans -- a 1990s fax will do far worse, which is exactly why the output
/// is never presented as an exact quotation.
#[test]
fn clean_rendered_type_reads_with_high_confidence() {
    let page = recognize_page(SCANNED_NDA).expect("recognize");
    assert!(
        page.lowest_confidence() > 0.3,
        "clean rendered type read poorly ({}); the fixture or the recognizer changed",
        page.lowest_confidence()
    );
}

/// Hostile and malformed images must fail closed, without panicking or hanging.
///
/// Image decoders are a classic attack surface and these bytes are attacker
/// controlled. The worker's sandbox and resource limits are the outer layer;
/// this asserts the inner one holds on its own.
#[test]
fn hostile_images_fail_closed_without_panicking() {
    let mut truncated = SCANNED_NDA[..SCANNED_NDA.len() / 3].to_vec();
    let mut corrupt_ihdr = SCANNED_NDA.to_vec();
    // Absurd dimensions in the PNG header: a decoder that trusts them tries to
    // allocate the product.
    corrupt_ihdr[16..24].copy_from_slice(&[0x7f, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff]);
    let mut zero_dimensions = SCANNED_NDA.to_vec();
    zero_dimensions[16..24].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not an image at all", b"7. CONFIDENTIALITY".to_vec()),
        ("PNG signature then garbage", {
            let mut bytes = SCANNED_NDA[..8].to_vec();
            bytes.extend(std::iter::repeat_n(0xA5, 4096));
            bytes
        }),
        ("truncated mid-stream", std::mem::take(&mut truncated)),
        (
            "header claims 2GB square",
            std::mem::take(&mut corrupt_ihdr),
        ),
        (
            "header claims zero pixels",
            std::mem::take(&mut zero_dimensions),
        ),
        ("a single byte", vec![0x89]),
    ];

    for (label, bytes) in cases {
        let outcome = std::panic::catch_unwind(|| recognize_page(&bytes));
        let Ok(result) = outcome else {
            panic!("{label} panicked the recognizer");
        };
        if let Ok(page) = result {
            // Decoding something is acceptable; inventing legal text from
            // noise is not the failure mode we can assert on, but a page that
            // decodes must still respect the budgets.
            assert!(
                page.lines.len() <= minutes_archive_ocr::MAX_LINES,
                "{label} exceeded the line budget"
            );
        }
    }
}

#[test]
fn an_image_larger_than_the_budget_is_refused_before_decoding() {
    let oversized = vec![0u8; MAX_IMAGE_BYTES + 1];
    assert_eq!(recognize_page(&oversized), Err(OcrError::ImageRefused));
}

/// The boundary and the recognizer have to work together, not just separately.
///
/// Installing a seatbelt profile is irreversible for the process, so this runs
/// the real worker binary rather than calling into the library: the same thing
/// the application spawns, doing the same work, with the profile already
/// applied before the image reaches it.
#[test]
fn the_worker_reads_a_page_from_inside_its_security_boundary() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let worker = env!("CARGO_BIN_EXE_minutes-archive-ocr-worker");

    let selftest = Command::new(worker)
        .arg("sandbox-self-test")
        .output()
        .expect("spawn self-test");
    assert!(
        selftest.status.success(),
        "the boundary did not hold: {selftest:?}"
    );

    let mut child = Command::new(worker)
        .arg("recognize")
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(SCANNED_NDA)
        .expect("write image");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "the sandboxed worker failed to read the page: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let page: minutes_archive_ocr::RecognizedPage =
        serde_json::from_slice(&output.stdout).expect("decode worker output");
    let text = page
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains("Confidential Information"),
        "sandboxed recognition lost the operative term: {text}"
    );
}
