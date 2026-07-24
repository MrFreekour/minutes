use std::path::PathBuf;

#[allow(dead_code)]
#[path = "../tests/support/live_sidekick_engine_harness.rs"]
mod harness;

const MEETING_REFERENCE: &str = "Matt and Wesley are reviewing the Minutes Apple speech benchmark. They want to compare SpeechTranscriber and DictationTranscriber against Whisper and Parakeet before making a product decision.";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    minutes_core::install_whisper_logging_hooks();
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut output_path = repo_root.join("target/sidekick-eval/live-sidekick-media-eval.json");
    let mut audio_path = repo_root.join("tests/eval/fixtures/audio/apple-speech-meeting.wav");
    let mut model_dir = dirs::home_dir()
        .ok_or("home directory is unavailable")?
        .join(".minutes/models");
    let mut model = "tiny".to_string();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                output_path = PathBuf::from(args.next().ok_or("--out requires a path")?);
            }
            "--audio" => {
                audio_path = PathBuf::from(args.next().ok_or("--audio requires a path")?);
            }
            "--model-dir" => {
                model_dir = PathBuf::from(args.next().ok_or("--model-dir requires a path")?);
            }
            "--model" => {
                model = args.next().ok_or("--model requires a value")?;
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    let report =
        harness::run_live_sidekick_media_eval(&audio_path, MEETING_REFERENCE, &model_dir, &model)?;
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, format!("{json}\n"))?;
    eprintln!("sidekick_media_eval_artifact={}", output_path.display());
    println!("{json}");
    if !report.passed {
        std::process::exit(1);
    }
    Ok(())
}
