//! The OCR worker process.
//!
//! Installs the security boundary before it reads a byte of the image, then
//! reads one page from stdin and writes the result to stdout. One process per
//! document: it exits when the page is read.

fn main() {
    // Accepts the marker form the application uses (`<marker> <operation>`)
    // and the bare form the tests use, so the same binary is exercised either
    // way rather than the tests proving something about a different path.
    let mut arguments = std::env::args().skip(1);
    let first = arguments.next().unwrap_or_default();
    let operation = if first == minutes_archive_ocr::WORKER_MARKER {
        arguments.next().unwrap_or_default()
    } else {
        first
    };
    if arguments.next().is_some() {
        std::process::exit(64);
    }
    if minutes_archive_ocr::install_worker_security_boundary().is_err() {
        std::process::exit(70);
    }
    if operation == "sandbox-self-test" {
        std::process::exit(minutes_archive_ocr::sandbox_self_test());
    }
    if operation != "recognize" {
        std::process::exit(64);
    }

    use std::io::{Read, Write};
    // Bounded on the way in, not only on the way out.
    //
    // `recognize_page` refuses anything over MAX_IMAGE_BYTES, but it does so
    // after the bytes are already in memory: an unbounded `read_to_end` would
    // allocate until RLIMIT_AS aborted the process, three gigabytes later, for
    // an input the next line was always going to reject at sixty-four
    // megabytes. The parent bounds what it sends, and this worker is written
    // not to depend on that -- the output side already takes the same care.
    let mut image = Vec::new();
    if std::io::stdin()
        .lock()
        .take(minutes_archive_ocr::MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut image)
        .is_err()
    {
        std::process::exit(65);
    }
    // Any panic inside Apple's decoder is contained here rather than becoming
    // an abort the parent has to interpret.
    let outcome = std::panic::catch_unwind(|| minutes_archive_ocr::recognize_page(&image));
    let Ok(result) = outcome else {
        std::process::exit(71);
    };
    let Ok(page) = result else {
        std::process::exit(66);
    };
    let Ok(encoded) = serde_json::to_vec(&page) else {
        std::process::exit(67);
    };
    if std::io::stdout().lock().write_all(&encoded).is_err() {
        std::process::exit(68);
    }
    std::process::exit(0);
}
