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
