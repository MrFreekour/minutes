//! Classify PDFs on disk by text origin, for validating the copier-signature
//! detector against real files rather than synthetic ones.
//!
//! Prints one line per file: verdict, block count, and the file name. Reads
//! nothing but what it is given and quotes no content.

use minutes_archive_convert::{convert_bytes, SourceFormat, TextOrigin};

fn main() {
    let mut author_written = 0usize;
    let mut machine_read = 0usize;
    let mut no_text = 0usize;
    let mut failed = 0usize;
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else {
            println!("unreadable            {path}");
            failed += 1;
            continue;
        };
        match convert_bytes(SourceFormat::Pdf, &bytes) {
            Ok(document) if document.blocks.is_empty() => {
                println!("no-extractable-text   {path}");
                no_text += 1;
            }
            Ok(document) => {
                let verdict = match document.text_origin {
                    TextOrigin::AuthorWritten => {
                        author_written += 1;
                        "author-written     "
                    }
                    TextOrigin::MachineReadLayer => {
                        machine_read += 1;
                        "MACHINE-READ-LAYER "
                    }
                };
                println!("{verdict}  blocks={:<4} {path}", document.blocks.len());
            }
            Err(error) => {
                println!("conversion-failed({error:?})  {path}");
                failed += 1;
            }
        }
    }
    println!(
        "\ncensus: author_written={author_written} machine_read={machine_read} no_text={no_text} failed={failed}"
    );
}
