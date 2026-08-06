//! A build at archive scale, under the constraints the application really has.
//!
//! Every failure the owner of this pilot hit in person got past the existing
//! checks for the same two reasons, and this exists to close both.
//!
//! **Scale.** `document_vault_smoke` indexes three documents. A build that
//! stops at 237 of 16,621, or loses its semantic worker nine thousand
//! documents in, looks perfect at three.
//!
//! **Environment.** The tests run from a terminal, which inherits a soft
//! `RLIMIT_NOFILE` in the thousands. launchd gives a GUI application 256. The
//! vault holds one descriptor per indexed document, so the real application
//! stopped at 237 and called the rest unreadable while every local run passed.
//! This harness lowers its own ceiling to the GUI's 256 before building, which
//! is the only way a test can be in the same situation the application is in.
//!
//! Runs headless against the installed executable -- the same binary the
//! application spawns its workers from -- with no window, no folder picker and
//! nobody clicking anything.
//!
//! Usage: `archive_pilot_soak <minutes-archive-app executable> [documents]`

use minutes_archive_convert::BoundedConverter;
use minutes_archive_core::approve_roots;
use minutes_archive_core::retrieval::VaultId;
use minutes_archive_core::vault::{
    build_authorized_document_vault, raise_open_file_ceiling, DocumentVaultLimits, ExcludedFolder,
};
use minutes_archive_semantic::BoundedSemanticEngine;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tempfile::TempDir;

/// The soft limit launchd hands a GUI application.
#[cfg(unix)]
const GUI_OPEN_FILE_SOFT_LIMIT: libc::rlim_t = 256;

/// Pin this process to the ceiling the application starts life with.
#[cfg(unix)]
fn adopt_gui_open_file_limit() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0,
        "could not read the open-file limit"
    );
    let lowered = libc::rlimit {
        rlim_cur: GUI_OPEN_FILE_SOFT_LIMIT,
        rlim_max: limit.rlim_max,
    };
    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lowered) },
        0,
        "could not lower the open-file limit to the GUI's"
    );
}

#[cfg(not(unix))]
fn adopt_gui_open_file_limit() {}

/// A document with real provisions, so the semantic path does work per file
/// rather than being skipped for want of text.
fn synthetic_matter(index: usize) -> String {
    format!(
        "MATTER {index:05}\n\n\
         7. CONFIDENTIALITY\n\
         Confidential Information includes affiliate data disclosed under this Agreement.\n\n\
         8. ASSIGNMENT\n\
         Neither party may assign this Agreement without prior written consent.\n\n\
         9. GOVERNING LAW\n\
         This Agreement is governed by the laws of the State of New York.\n"
    )
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let worker_path = arguments
        .next()
        .expect("usage: archive_pilot_soak <minutes-archive-app executable> [documents]");
    // Well past the 256 the application is given, so a build that cannot raise
    // its own ceiling fails here instead of on the owner's archive.
    let documents: usize = arguments
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(700);
    assert!(
        documents > 300,
        "a soak below the 256-descriptor ceiling proves nothing"
    );

    adopt_gui_open_file_limit();
    // Exactly what the application does at startup, from the same code.
    raise_open_file_ceiling();

    let temp = TempDir::new().expect("temporary fixture");
    let root = temp.path().join("approved");
    fs::create_dir(&root).expect("approved root");

    // Nested, because a flat folder never exercises the descent, and one
    // folder that must be skipped entirely.
    let matters = root.join("matters");
    let deep = matters.join("2019").join("q3");
    let skipped = root.join("attachments");
    fs::create_dir_all(&deep).expect("nested folders");
    fs::create_dir(&skipped).expect("skipped folder");

    for index in 0..documents {
        let directory: &PathBuf = match index % 3 {
            0 => &root,
            1 => &matters,
            _ => &deep,
        };
        fs::write(
            directory.join(format!("matter-{index:05}.txt")),
            synthetic_matter(index),
        )
        .expect("write matter");
    }
    // These must never be read, and must not be counted as indexed.
    let skipped_documents = 40;
    for index in 0..skipped_documents {
        fs::write(
            skipped.join(format!("screenshot-{index:03}.txt")),
            synthetic_matter(900_000 + index),
        )
        .expect("write skipped");
    }

    let converter =
        BoundedConverter::bind(Path::new(&worker_path)).expect("bind embedded converter worker");
    // Optional exactly as in the application: a machine without the on-device
    // model must still complete the build.
    let semantic_engine = BoundedSemanticEngine::bind(Path::new(&worker_path)).ok();
    let semantic_bound = semantic_engine.is_some();

    let approved = approve_roots(&[root]).expect("approve root");
    let started = Instant::now();
    let vault = build_authorized_document_vault(
        VaultId::parse("archive-pilot-soak").expect("vault id"),
        &approved,
        DocumentVaultLimits {
            excluded_paths: vec![ExcludedFolder {
                root_index: 0,
                relative_path: PathBuf::from("attachments"),
            }],
            ..DocumentVaultLimits::default()
        },
        &AtomicBool::new(false),
        &converter,
        None,
        None,
        semantic_engine,
    )
    .expect("build the document vault at archive scale");
    let elapsed = started.elapsed();
    let report = vault.build_report();

    // The failure that reached the owner: a build that stops partway and
    // reports the remainder as something else.
    assert_eq!(
        report.open_file_limit_reached, 0,
        "the build ran out of descriptors; the ceiling was not raised"
    );
    assert_eq!(
        report.indexed_documents, documents as u64,
        "the build indexed {} of {documents} documents",
        report.indexed_documents
    );
    assert!(
        !report.budget_reached,
        "a default-limit build hit a budget at {documents} documents"
    );
    assert_eq!(
        report.excluded_directories, 1,
        "the excluded folder was entered"
    );

    // Exact evidence is the product and must be complete regardless of what
    // the optional workers did.
    let response = vault
        .interpret_and_search("Find assignment provisions within three sentences covering assign.")
        .expect("exact search over the soaked index");
    assert!(
        !response.evidence.is_empty(),
        "exact search returned nothing over {documents} indexed documents"
    );
    assert!(
        response
            .evidence
            .iter()
            .all(|card| !card.exact_excerpt.is_empty()),
        "an evidence card carried no source language"
    );

    println!(
        "archive_pilot_soak=passed documents={} indexed={} excluded_dirs={} \
         open_file_limit_reached={} semantic_bound={} semantic_partial={} seconds={:.1}",
        documents,
        report.indexed_documents,
        report.excluded_directories,
        report.open_file_limit_reached,
        semantic_bound,
        report.semantic_coverage_partial,
        elapsed.as_secs_f64(),
    );
}
