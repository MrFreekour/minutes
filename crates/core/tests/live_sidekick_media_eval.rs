#![cfg(feature = "whisper")]

#[allow(dead_code)]
#[path = "support/live_sidekick_engine_harness.rs"]
mod harness;

const MEETING_REFERENCE: &str = "Matt and Wesley are reviewing the Minutes Apple speech benchmark. They want to compare SpeechTranscriber and DictationTranscriber against Whisper and Parakeet before making a product decision.";

#[test]
fn prerecorded_audio_crosses_asr_adapter_and_sidekick_engine() {
    minutes_core::install_whisper_logging_hooks();
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let model_dir = dirs::home_dir().unwrap().join(".minutes/models");
    if !model_dir.join("ggml-tiny.bin").is_file() {
        eprintln!("SKIPPED: local tiny Whisper model is unavailable");
        return;
    }
    let report = harness::run_live_sidekick_media_eval(
        &repo_root.join("tests/eval/fixtures/audio/apple-speech-meeting.wav"),
        MEETING_REFERENCE,
        &model_dir,
        "tiny",
    )
    .expect("media replay should complete");
    assert!(report.passed, "{report:#?}");
    assert!(report.production_batch_asr);
    assert!(!report.native_microphone_capture);
    assert!(!report.native_live_asr);
    assert!(!report.native_diarization);
    assert!(!report.release_ready_from_this_report_alone);
}
