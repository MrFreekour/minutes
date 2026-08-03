#[cfg(target_os = "macos")]
#[test]
fn bound_worker_denies_network_and_personal_files_before_embedding() {
    use minutes_archive_semantic::{BoundedSemanticEngine, APPLE_ENGLISH_SENTENCE_DIMENSION};
    use std::path::Path;

    let executable = env!("CARGO_BIN_EXE_minutes-archive-semantic");
    let engine = BoundedSemanticEngine::bind(Path::new(executable)).expect("bind and self-test");
    let mut session = engine.open_session().expect("sandboxed session");
    let vector = session
        .embed("The recipient shall not disclose proprietary information.")
        .expect("embedded in worker");
    assert_eq!(vector.len(), APPLE_ENGLISH_SENTENCE_DIMENSION);
    let second = session
        .embed("A clause requiring prior written consent before assignment.")
        .expect("second request on same bounded worker");
    assert_eq!(second.len(), APPLE_ENGLISH_SENTENCE_DIMENSION);
}

/// The app terminates with `app_handle().exit(0)`, which does not unwind, so
/// no destructor runs at exit. The worker snapshot directory is owned by a
/// `TempDir` whose cleanup is `Drop`, and it was surviving the process as a
/// 40 MB copy of the executable in $TMPDIR. The close handler now releases
/// the session explicitly while the process is still alive; this asserts the
/// drop chain that relies on actually reclaims the snapshot.
#[cfg(target_os = "macos")]
#[test]
fn dropping_the_engine_reclaims_its_worker_snapshot() {
    use minutes_archive_semantic::BoundedSemanticEngine;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    // Track the specific directory this engine creates. Counting them is
    // racy: sibling tests in the same binary bind their own engines.
    fn snapshots() -> HashSet<PathBuf> {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return HashSet::new();
        };
        entries
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("minutes-archive-semantic-")
            })
            .map(|entry| entry.path())
            .collect()
    }

    let executable = env!("CARGO_BIN_EXE_minutes-archive-semantic");
    let before = snapshots();
    let engine = BoundedSemanticEngine::bind(Path::new(executable)).expect("bind");
    let created = &snapshots() - &before;
    assert_eq!(
        created.len(),
        1,
        "binding must create exactly one private snapshot"
    );
    let snapshot = created.into_iter().next().expect("snapshot path");
    assert!(snapshot.exists(), "snapshot must exist while bound");
    drop(engine);
    assert!(
        !snapshot.exists(),
        "dropping the engine must reclaim {}",
        snapshot.display()
    );
}
