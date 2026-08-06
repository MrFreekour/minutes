//! The worker must execute in place, never from a copy.
//!
//! A Developer ID signature with the hardened runtime is bound to its bundle.
//! When the worker was copied to a private temp directory and run from there,
//! the copy failed validation -- `codesign` reports "invalid Info.plist (plist
//! or signature have been modified)" -- and the kernel SIGKILLed it on exec.
//! Every notarized build was therefore unable to index a single document,
//! while signature, staple, Gatekeeper and launch all passed and the window
//! opened normally.
//!
//! Nothing caught it: local testing used an ad-hoc-signed app, whose copy runs
//! fine, and CI exercised the unsigned build. This asserts the invariant on
//! every build rather than only on signed ones, because the signed path is the
//! one nobody exercises until a human clicks the button.

use minutes_archive_convert::BoundedConverter;
use std::path::Path;

fn worker_copies_in_temp() -> usize {
    // Counted, not diffed by name: sibling tests bind concurrently.
    std::fs::read_dir(std::env::temp_dir())
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("minutes-archive-worker-")
                })
                .count()
        })
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
#[test]
fn binding_the_converter_copies_the_executable_nowhere() {
    let executable = env!("CARGO_BIN_EXE_minutes-archive-convert-worker");
    let before = worker_copies_in_temp();
    let converter = BoundedConverter::bind(Path::new(executable)).expect("bind");
    let during = worker_copies_in_temp();
    assert_eq!(
        during, before,
        "binding copied the executable into the temp directory; a copy of a \
         bundle-signed binary is killed on exec"
    );
    drop(converter);
}

/// The pinned executable is the one that runs, and a swap is still refused.
///
/// Running in place gave up the guarantee that the bytes cannot change between
/// verification and use, so identity is checked against the descriptor opened
/// at bind time instead. Replacing the file at the path leaves that descriptor
/// pointing at the original inode, and the mismatch is what refuses the launch.
#[cfg(target_os = "macos")]
#[test]
fn a_worker_swapped_after_binding_is_refused() {
    let executable = env!("CARGO_BIN_EXE_minutes-archive-convert-worker");
    let directory = tempfile::tempdir().expect("temp");
    let path = directory.path().join("worker");
    std::fs::copy(executable, &path).expect("stage worker");

    let converter = BoundedConverter::bind(&path).expect("bind");
    // A different file at the same path: new inode, so the pinned descriptor
    // and the path no longer agree.
    std::fs::remove_file(&path).expect("remove");
    std::fs::copy(executable, &path).expect("replace worker");

    let error = converter
        .convert(minutes_archive_convert::SourceFormat::Docx, b"irrelevant")
        .expect_err("a replaced worker must be refused");
    assert!(
        matches!(
            error,
            minutes_archive_convert::WorkerError::ExecutableUnavailable
        ),
        "expected the swap to be refused on identity, got {error:?}"
    );
}
