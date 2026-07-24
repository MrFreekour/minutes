//! Bounded compressed-audio decode worker.
//!
//! ffmpeg remains the preferred decoder: its resampler and AAC decoder produce
//! samples that whisper transcribes correctly across languages, while
//! Symphonia's AAC decoder produces subtly different samples that trigger
//! hallucination loops on non-English audio (issue #21). This worker exists so
//! that a user who has never installed ffmpeg keeps the behaviour they had
//! before the conversation-trust work: iPhone/iCloud voice memos are `.m4a`,
//! and the folder watcher is a headline input mode, so requiring ffmpeg would
//! be a default-user regression.
//!
//! Symphonia is never linked into a decode that runs in this process. Container
//! probing can allocate attacker-declared tables, and the objection to
//! in-process use was one of ordering: the allocation happens before any
//! resource limit applies. Confining it to a child inverts that ordering.
//! [`crate::bounded_child::BoundedCommand::address_space_limit`] installs
//! `RLIMIT_AS` from `pre_exec`, so the ceiling is in force before `exec`, and
//! therefore before Symphonia's first probe call reads a single byte. The child
//! additionally gets its own process group, a wall-clock ceiling, a capped
//! stdout, and no ambient descriptors.
//!
//! The worker emits the same bytes ffmpeg is asked for, raw 16 kHz mono
//! `s16le` PCM on stdout, so both decoders share one downstream path.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Set on the child so a re-executed Minutes binary knows to decode instead of
/// running its ordinary command line. Mirrors the policy graph worker's marker
/// contract.
const WORKER_MARKER: &str = "MINUTES_AUDIO_DECODE_WORKER_V1";

/// Address-space ceiling for the decode child.
///
/// Generous enough for a long meeting's decoded PCM plus decoder tables, small
/// enough that an attacker-declared allocation fails inside the child rather
/// than exhausting the machine.
const WORKER_ADDRESS_SPACE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Child exit code used when the input could not be decoded at all, as opposed
/// to a resource or plumbing failure.
const EXIT_UNDECODABLE: i32 = 65;

/// Environment the decode child is allowed to inherit.
///
/// The decoder needs no configuration, no `HOME`, and no `PATH`: it is handed
/// one already-opened input path and writes to stdout.
fn retain_safe_environment(command: &mut crate::bounded_child::BoundedCommand) {
    command.env_clear();
    for name in ["LANG", "LC_ALL", "LC_CTYPE"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

/// Executables known to dispatch [`maybe_run_audio_decode_worker`] before they
/// parse a command line.
///
/// Self-exec is only safe against a binary that actually honours the marker.
/// Re-executing an arbitrary host, notably a test harness, would run that
/// host's real work with the marker set and could recurse, so anything not on
/// this list fails closed instead.
const WORKER_CAPABLE_EXECUTABLES: [&str; 2] = ["minutes", "minutes-app"];

fn executable_handles_worker_protocol(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| WORKER_CAPABLE_EXECUTABLES.contains(&stem))
}

/// Resolve the executable that will run the decode.
///
/// Prefers an adjacent Minutes binary, then re-executes the current one when it
/// is itself worker-capable. Self-exec keeps this off the macOS signed helper
/// path entirely: the child is the same already-signed code, so it introduces
/// no new packaging surface and no App Sandbox conflict, unlike a worker whose
/// job is to launch a third-party engine.
fn resolve_worker_executable() -> Result<crate::bounded_child::BoundExecutable, String> {
    let current = std::env::current_exe()
        .map_err(|_| "compressed audio decode worker host was unavailable".to_string())?;
    let helper_name = format!("minutes{}", std::env::consts::EXE_SUFFIX);
    let adjacent = current.parent().and_then(|parent| {
        [
            parent.join(&helper_name),
            parent.parent()?.join(&helper_name),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
    });
    if let Some(helper) = adjacent {
        if let Ok(executable) = crate::bounded_child::BoundExecutable::bind(&helper) {
            return Ok(executable);
        }
    }
    if !executable_handles_worker_protocol(&current) {
        return Err(
            "compressed audio decode worker is unavailable because no Minutes binary was found \
             next to this process"
                .into(),
        );
    }
    crate::bounded_child::BoundExecutable::current()
        .map_err(|_| "compressed audio decode worker executable could not be resolved".to_string())
}

/// Whether a compressed decode may fall back to the bounded Symphonia worker.
///
/// Default-on by design. Shipping this opt-in would leave the very user it
/// exists for, someone who never installed ffmpeg, still regressed; the flag is
/// for an operator who wants to refuse the extra decoder, not something a
/// default user must discover.
pub fn bounded_decode_fallback_enabled(config: &crate::config::Config) -> bool {
    config.transcription.compressed_decode_fallback
}

/// Decode a compressed file to 16 kHz mono `s16le` PCM inside a bounded child.
///
/// Returns the raw PCM bytes written by the child. The caller converts them
/// with the same reader used for ffmpeg output.
pub(crate) fn decode_to_private_pcm(
    path: &Path,
    destination: &mut crate::pipeline::PrivateAudioTempFile,
    max_output_bytes: u64,
    wall_clock: Duration,
) -> Result<(), String> {
    let executable = resolve_worker_executable()?;
    let mut command = crate::bounded_child::BoundedCommand::from_bound_executable(executable)
        .map_err(|_| "compressed audio decode worker authority could not be bound".to_string())?;
    retain_safe_environment(&mut command);
    command
        .env(WORKER_MARKER, "1")
        .arg("--")
        .arg(path)
        // Order matters and is the whole security argument: `address_space_limit`
        // is applied in `pre_exec`, so `RLIMIT_AS` binds before `exec` and thus
        // before Symphonia probes the container.
        .address_space_limit(WORKER_ADDRESS_SPACE_BYTES)
        .single_process()
        .close_extra_descriptors();

    let output = crate::pipeline::output_with_authorized_audio_stdin_to_private_file_with_budget(
        &mut command,
        None,
        destination,
        max_output_bytes,
        wall_clock,
    )
    .map_err(|error| {
        if crate::bounded_child::is_spawn_failure(&error) {
            format!("compressed audio decode worker could not be started: {error}")
        } else {
            format!("compressed audio decode worker failed: {error}")
        }
    })?;

    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.lines().last().unwrap_or("unknown error").to_string();
    if output.status.code() == Some(EXIT_UNDECODABLE) {
        Err(format!("the audio could not be decoded: {detail}"))
    } else {
        Err(format!(
            "compressed audio decode worker failed closed: {detail}"
        ))
    }
}

/// Child entry point. Returns `None` in an ordinary process.
///
/// Called before any argument parsing so a decode child never runs a user
/// command. The marker is removed immediately so it cannot be inherited any
/// further, matching the policy graph worker.
pub fn maybe_run_audio_decode_worker() -> Option<i32> {
    let marker = std::env::var_os(WORKER_MARKER)?;
    std::env::remove_var(WORKER_MARKER);
    if marker != "1" {
        return Some(EXIT_UNDECODABLE);
    }
    let path = std::env::args_os()
        .skip_while(|argument| argument != "--")
        .nth(1)
        .map(PathBuf::from);
    let Some(path) = path else {
        eprintln!("compressed audio decode worker requires exactly one input path");
        return Some(EXIT_UNDECODABLE);
    };
    Some(run_worker(&path))
}

fn run_worker(path: &Path) -> i32 {
    match decode_compressed_to_s16le(path) {
        Ok(pcm) => {
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(&pcm).is_err() || stdout.flush().is_err() {
                return 74;
            }
            0
        }
        Err(error) => {
            eprintln!("{error}");
            EXIT_UNDECODABLE
        }
    }
}

/// Decode `path` with Symphonia into 16 kHz mono `s16le` bytes.
///
/// Runs only inside the bounded child. The address-space ceiling is already in
/// force here, so an attacker-declared table allocation fails this process
/// rather than the caller's.
fn decode_compressed_to_s16le(path: &Path) -> Result<Vec<u8>, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|error| format!("input unavailable: {error}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("probe failed: {error}"))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "no audio track found".to_string())?;
    let track_id = track.id;
    let source_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let channels = track
        .codec_params
        .channels
        .map(|value| value.count())
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| format!("decoder unavailable: {error}"))?;

    // Resample and bound in one streaming pass so a hostile declared duration
    // cannot force an unbounded intermediate buffer even under the ceiling.
    let budget = crate::audio_budget::AudioWorkBudget::new();
    budget
        .validate_stream(source_rate, channels)
        .map_err(|error| error.to_string())?;
    let mut resampler = crate::audio_budget::StreamingMonoResampler::new(
        source_rate,
        crate::audio_budget::CANONICAL_SAMPLE_RATE,
        budget,
        crate::audio_budget::MAX_CANONICAL_SAMPLES,
    )
    .map_err(|error| error.to_string())?;

    let mut decoded_any = false;
    // Any packet-level error ends the stream: a truncated or hostile container
    // yields whatever decoded cleanly so far rather than an error spiral.
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(_) => continue,
        };
        let spec = *decoded.spec();
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let frame_channels = spec.channels.count().max(1);
        for frame in buffer.samples().chunks(frame_channels) {
            if frame.len() < frame_channels {
                continue;
            }
            let mono = frame.iter().copied().sum::<f32>() / frame_channels as f32;
            if !mono.is_finite() {
                return Err("decoded audio contains a non-finite sample".into());
            }
            resampler
                .push_mono_sample(mono)
                .map_err(|error| format!("decode exceeded its resource budget: {error}"))?;
            decoded_any = true;
        }
    }
    if !decoded_any {
        return Err("no decodable audio was found".into());
    }
    let samples = resampler
        .finish()
        .map_err(|error| format!("decode exceeded its resource budget: {error}"))?;
    if samples.is_empty() {
        return Err("no decodable audio was found".into());
    }

    let mut pcm = Vec::with_capacity(samples.len() * std::mem::size_of::<i16>());
    for sample in samples {
        let clamped = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        pcm.extend_from_slice(&clamped.to_le_bytes());
    }
    Ok(pcm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_marker_is_absent_in_an_ordinary_process() {
        // The ordinary path must never be mistaken for a decode child.
        std::env::remove_var(WORKER_MARKER);
        assert!(maybe_run_audio_decode_worker().is_none());
    }

    #[test]
    fn the_fallback_is_on_by_default_and_can_be_refused() {
        // Shipping this opt-in would leave the user it exists for, someone who
        // never installed ffmpeg, still regressed. The flag is for an operator
        // who wants to refuse the extra decoder.
        let mut config = crate::config::Config::default();
        assert!(
            bounded_decode_fallback_enabled(&config),
            "compressed-import fallback must default to on"
        );
        config.transcription.compressed_decode_fallback = false;
        assert!(!bounded_decode_fallback_enabled(&config));
    }

    #[test]
    fn an_existing_config_without_the_field_keeps_the_fallback() {
        // A user upgrading into this build has no such key in their
        // config.toml. They must not silently lose compressed imports.
        let existing: crate::config::TranscriptionConfig =
            toml::from_str("engine = \"whisper\"\n").unwrap();
        assert!(existing.compressed_decode_fallback);
    }

    #[test]
    fn address_space_ceiling_is_bounded_and_nonzero() {
        // The ceiling is the containment argument; a zero or absent limit
        // would silently turn this back into an in-process decode.
        assert!(WORKER_ADDRESS_SPACE_BYTES > 0);
        assert!(WORKER_ADDRESS_SPACE_BYTES <= 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn undecodable_input_fails_closed_rather_than_returning_silence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-audio.m4a");
        std::fs::write(&path, b"this is not a media container").unwrap();
        let error = decode_compressed_to_s16le(&path).unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn missing_input_is_reported_rather_than_panicking() {
        let directory = tempfile::tempdir().unwrap();
        let error = decode_compressed_to_s16le(&directory.path().join("absent.mp3")).unwrap_err();
        assert!(error.contains("input unavailable"));
    }

    #[test]
    fn a_non_minutes_host_refuses_to_self_exec() {
        // Re-executing a test harness would run the suite again with the
        // marker set. Resolution must fail closed instead of recursing.
        assert!(!executable_handles_worker_protocol(Path::new(
            "/tmp/minutes_core-0123456789abcdef"
        )));
        assert!(!executable_handles_worker_protocol(Path::new(
            "/usr/bin/env"
        )));
        assert!(executable_handles_worker_protocol(Path::new(
            "/usr/local/bin/minutes"
        )));
        assert!(executable_handles_worker_protocol(Path::new(
            "/Applications/Minutes.app/Contents/MacOS/minutes-app"
        )));
    }

    /// Build a WAV that Symphonia decodes through the same path as a
    /// compressed container, so the resample and PCM emission are covered
    /// without depending on an external encoder.
    fn write_test_wav(path: &Path, sample_rate: u32, frames: usize) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for index in 0..frames {
            let phase = index as f32 / sample_rate as f32 * 440.0 * std::f32::consts::TAU;
            writer
                .write_sample((phase.sin() * 16_000.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn decode_resamples_to_canonical_sixteen_khz_mono_pcm() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tone.wav");
        write_test_wav(&path, 44_100, 44_100);

        let pcm = decode_compressed_to_s16le(&path).unwrap();
        assert_eq!(pcm.len() % 2, 0, "s16le output must be whole samples");
        let samples = pcm.len() / 2;
        // One second at 44.1 kHz resamples to about one second at 16 kHz.
        assert!(
            (15_500..=16_500).contains(&samples),
            "expected ~16000 samples, got {samples}"
        );
        assert!(
            pcm.chunks_exact(2)
                .any(|pair| i16::from_le_bytes([pair[0], pair[1]]).abs() > 1_000),
            "decoded tone must carry real signal"
        );
    }

    #[test]
    fn decode_preserves_already_canonical_audio_length() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("canonical.wav");
        write_test_wav(&path, 16_000, 16_000);

        let pcm = decode_compressed_to_s16le(&path).unwrap();
        assert_eq!(pcm.len() / 2, 16_000);
    }

    /// Full parent-to-child path: spawn the bounded worker and read back PCM
    /// through the private file that the child never has a pathname for.
    /// Requires a built `minutes` binary next to the test harness.
    #[test]
    fn bounded_worker_child_round_trips_pcm_into_a_private_file() {
        if resolve_worker_executable().is_err() {
            eprintln!("skipping: no adjacent minutes binary to act as the decode worker");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("tone.wav");
        write_test_wav(&source, 44_100, 44_100);

        let mut destination =
            crate::pipeline::PrivateAudioTempFile::new("minutes-decode-test-", ".s16le").unwrap();
        decode_to_private_pcm(
            &source,
            &mut destination,
            crate::audio_budget::AudioWorkBudget::max_pcm_s16le_bytes(),
            Duration::from_secs(120),
        )
        .unwrap();

        let mut reader = destination.try_clone_reader().unwrap();
        let mut pcm = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut pcm).unwrap();
        let samples = pcm.len() / 2;
        assert!(
            (15_500..=16_500).contains(&samples),
            "expected ~16000 samples through the child, got {samples}"
        );
    }

    /// The actual regression: an m4a voice memo must decode without ffmpeg.
    /// Skipped where no encoder is available to produce the fixture.
    #[test]
    fn compressed_m4a_decodes_without_ffmpeg_at_decode_time() {
        let Ok(ffmpeg) = crate::ffmpeg::resolve_ffmpeg() else {
            eprintln!("skipping: no ffmpeg available to build the m4a fixture");
            return;
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("memo.m4a");
        let encoded = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-ar",
                "44100",
                "-ac",
                "1",
                "-c:a",
                "aac",
            ])
            .arg(&path)
            .arg("-y")
            .status();
        if !encoded.map(|status| status.success()).unwrap_or(false) {
            eprintln!("skipping: ffmpeg could not build the aac fixture");
            return;
        }

        // Decoded entirely by Symphonia, which is what a user without ffmpeg
        // installed would get.
        let pcm = decode_compressed_to_s16le(&path).unwrap();
        let samples = pcm.len() / 2;
        assert!(
            (14_000..=18_000).contains(&samples),
            "expected roughly one second of 16 kHz audio, got {samples}"
        );
        assert!(
            pcm.chunks_exact(2)
                .any(|pair| i16::from_le_bytes([pair[0], pair[1]]).abs() > 1_000),
            "decoded memo must carry real signal"
        );
    }
}
