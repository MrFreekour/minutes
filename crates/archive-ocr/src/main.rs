//! The OCR worker process.
//!
//! Installs the security boundary before it reads a byte of the image, then
//! reads one page from stdin and writes the result to stdout. One process per
//! document: it exits when the page is read.

fn main() {
    let operation = std::env::args().nth(1).unwrap_or_default();
    if std::env::args().nth(2).is_some() {
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
    let mut image = Vec::new();
    if std::io::stdin().lock().read_to_end(&mut image).is_err() {
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
