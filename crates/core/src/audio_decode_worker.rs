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
//! resource limit applies. Confining it to a child inverts that ordering, and
//! the ceiling binds before Symphonia reads a single attacker-controlled byte.
//!
//! How that ceiling is installed is platform-specific, because Darwin rejects
//! an absolute `RLIMIT_AS` below its pre-`main()` shared-cache baseline:
//!
//! - Linux installs it from the parent's `pre_exec`, so it binds before `exec`.
//! - Elsewhere the child installs a measured baseline-plus-budget ceiling
//!   itself, at [`maybe_run_audio_decode_worker`], before any decode begins.
//!   `graph_worker` binds its Darwin ceiling the same way.
//!
//! The child additionally gets its own process group, a wall-clock ceiling, a
//! capped stdout streamed into a private file it has no pathname for, a cleared
//! environment, and no ambient descriptors.
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

/// Address-space growth budget for the decode child.
///
/// This is a growth allowance over the process baseline, never an absolute
/// ceiling. A four-hour input holds roughly 921 MB of `f32` output alongside
/// 461 MB of `s16le` bytes, so the budget must clear ~1.4 GB plus allocator
/// slack while still failing an attacker-declared allocation inside the child
/// rather than exhausting the machine.
const WORKER_ADDRESS_SPACE_BYTES: u64 = 3 * 1024 * 1024 * 1024;

/// Install the child's own address-space ceiling, measured against this
/// process's baseline.
///
/// Darwin reserves a very large shared-cache virtual range before `main()`
/// (hundreds of GiB on current macOS), so an absolute `RLIMIT_AS` is below the
/// process's immutable baseline and the kernel rejects it outright. That is why
/// the ceiling is installed here, by the child itself after exec, rather than
/// from the parent's `pre_exec` as on Linux. `graph_worker` binds its Darwin
/// ceiling the same way for the same reason.
///
/// Ordering is the security property and is preserved either way: this runs
/// before the decoder is constructed and therefore before Symphonia reads a
/// single attacker-controlled byte. Only dyld and Rust runtime startup precede
/// it, and neither touches the input.
#[cfg(all(unix, not(target_os = "linux")))]
fn install_child_address_space_ceiling() -> Result<(), String> {
    let baseline = process_virtual_size()?;
    let limit = baseline
        .checked_add(WORKER_ADDRESS_SPACE_BYTES)
        .ok_or_else(|| "decode worker address-space ceiling overflowed".to_string())?;
    let rlimit = libc::rlimit {
        rlim_cur: limit,
        rlim_max: limit,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &rlimit) } != 0 {
        return Err("decode worker could not install its address-space ceiling".into());
    }
    Ok(())
}

/// Measure this process's current virtual size so the ceiling can be expressed
/// as baseline plus budget.
#[cfg(target_os = "macos")]
fn process_virtual_size() -> Result<u64, String> {
    use mach2::kern_return::KERN_SUCCESS;
    use mach2::task::task_info;
    use mach2::task_info::{
        task_basic_info_64, task_info_t, TASK_BASIC_INFO_64, TASK_BASIC_INFO_64_COUNT,
    };
    use mach2::traps::mach_task_self;

    let mut info = task_basic_info_64::default();
    let mut count = TASK_BASIC_INFO_64_COUNT;
    let status = unsafe {
        task_info(
            mach_task_self(),
            TASK_BASIC_INFO_64,
            (&mut info as *mut task_basic_info_64).cast::<libc::c_int>() as task_info_t,
            &mut count,
        )
    };
    if status != KERN_SUCCESS || count != TASK_BASIC_INFO_64_COUNT {
        return Err("decode worker could not measure its address space".into());
    }
    Ok(info.virtual_size)
}

/// Non-macOS Unix targets that also skip the `pre_exec` ceiling read their
/// baseline from `getrlimit`, falling back to the budget alone.
#[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
fn process_virtual_size() -> Result<u64, String> {
    Ok(0)
}

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
/// Self-exec is tried FIRST, because re-running our own already-running image
/// is the strongest identity available without a signature check: it is by
/// definition the same code the user already trusted to run. Only when the
/// current host does not implement the worker protocol does this fall back to
/// an adjacent Minutes binary, and then only one that sits in the very same
/// directory as the current executable.
///
/// The previous order searched the parent directory too and exec'd any regular
/// file named `minutes` found there, with no identity check. For a
/// `~/.local/bin/minutes` install that second candidate was `~/.local/minutes`,
/// so anyone able to create a file in an adjacent directory could obtain
/// execution with the user's full authority at a moment of their choosing.
///
/// Self-exec also keeps this off the macOS signed helper path: the child is the
/// same already-signed code, introducing no new packaging surface and no App
/// Sandbox conflict, unlike a worker whose job is to launch a third-party
/// engine.
fn resolve_worker_executable() -> Result<crate::bounded_child::BoundExecutable, String> {
    let current = std::env::current_exe()
        .map_err(|_| "compressed audio decode worker host was unavailable".to_string())?;
    if executable_handles_worker_protocol(&current) {
        if let Ok(executable) = crate::bounded_child::BoundExecutable::current() {
            return Ok(executable);
        }
    }
    let helper_name = format!("minutes{}", std::env::consts::EXE_SUFFIX);
    // Production searches only the current executable's own directory. That
    // covers both real layouts: a macOS bundle keeps the CLI sidecar beside
    // `minutes-app` in Contents/MacOS, and every other install reaches the
    // binary through self-exec above.
    #[allow(unused_mut)]
    let mut candidates = vec![current.parent().map(|parent| parent.join(&helper_name))];
    // The unit-test harness runs from target/debug/deps, one level below the
    // built CLI, so it needs the wider search to exercise the real child. This
    // widening exists only under cfg(test) and is never compiled into a
    // shipped binary.
    #[cfg(test)]
    candidates.push(
        current
            .parent()
            .and_then(|parent| parent.parent())
            .map(|grandparent| grandparent.join(&helper_name)),
    );
    let adjacent = candidates
        .into_iter()
        .flatten()
        .find(|candidate| candidate.is_file() && candidate != &current);
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

/// Whether the bounded fallback is both permitted and actually usable here.
///
/// Preflight surfaces must ask this rather than the config flag alone: a build
/// with no resolvable worker executable would otherwise advertise a decoder it
/// cannot run, and refuse the user at decode time instead of at admission.
pub fn bounded_decode_fallback_available(config: &crate::config::Config) -> bool {
    bounded_decode_fallback_enabled(config) && resolve_worker_executable().is_ok()
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
        .single_process()
        .close_extra_descriptors();
    // Ordering is the security property: the ceiling must bind before Symphonia
    // reads an attacker-controlled byte. On Linux `pre_exec` gives the strongest
    // form, binding before `exec` itself. Darwin rejects an absolute RLIMIT_AS
    // below its pre-main() shared-cache baseline, so there the child installs a
    // measured ceiling itself at `maybe_run_audio_decode_worker`, still ahead of
    // any decode. Setting it here as well would fail, since a process cannot
    // raise its own hard limit.
    #[cfg(target_os = "linux")]
    command.address_space_limit(WORKER_ADDRESS_SPACE_BYTES);

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
        // A stale or hostile marker must never silently swallow an ordinary
        // command such as `minutes record`, so say why the process is exiting.
        eprintln!(
            "{WORKER_MARKER} was set to an unrecognized value; refusing to run as a decode worker"
        );
        return Some(EXIT_UNDECODABLE);
    }
    // Ordering: install the ceiling before anything parses input. On Linux the
    // parent already bound it via pre_exec; elsewhere this is where it binds.
    #[cfg(all(unix, not(target_os = "linux")))]
    if let Err(error) = install_child_address_space_ceiling() {
        eprintln!("{error}");
        return Some(71);
    }
    let probe_only = std::env::args_os().any(|argument| argument == PROBE_DURATION_ARG);
    let path = std::env::args_os()
        .skip_while(|argument| argument != "--")
        .nth(1)
        .map(PathBuf::from);
    let Some(path) = path else {
        eprintln!("compressed audio decode worker requires exactly one input path");
        return Some(EXIT_UNDECODABLE);
    };
    Some(if probe_only {
        run_probe(&path)
    } else {
        run_worker(&path)
    })
}

/// Argument that switches the child into container-duration probe mode.
const PROBE_DURATION_ARG: &str = "--probe-duration";

/// Probe a compressed container's duration inside the bounded child.
///
/// Returns `None` when the container does not declare a frame count, in which
/// case the caller keeps its existing behaviour rather than paying for a full
/// decode. This exists so watcher content-type routing does not silently
/// degrade to `config.watch.type` when ffmpeg is absent, which filed long calls
/// as voice memos.
pub(crate) fn probe_compressed_duration(
    path: &Path,
    wall_clock: Duration,
) -> Option<std::time::Duration> {
    let executable = resolve_worker_executable().ok()?;
    let mut command =
        crate::bounded_child::BoundedCommand::from_bound_executable(executable).ok()?;
    retain_safe_environment(&mut command);
    command
        .env(WORKER_MARKER, "1")
        .arg(PROBE_DURATION_ARG)
        .arg("--")
        .arg(path)
        .single_process()
        .close_extra_descriptors();
    #[cfg(target_os = "linux")]
    command.address_space_limit(WORKER_ADDRESS_SPACE_BYTES);

    let run = crate::bounded_child::run(
        &mut command,
        None,
        crate::bounded_child::StdoutTarget::Capture { max_bytes: 128 },
        crate::bounded_child::ChildBudget {
            wall_clock,
            stderr_tail: 4 * 1024,
        },
    )
    .ok()?;
    if run.timed_out || !run.output.status.success() {
        return None;
    }
    let seconds: f64 = String::from_utf8_lossy(&run.output.stdout)
        .trim()
        .parse()
        .ok()?;
    (seconds.is_finite() && seconds > 0.0).then(|| std::time::Duration::from_secs_f64(seconds))
}

/// Read a container's declared duration without decoding its packets.
fn probe_duration_seconds(path: &Path) -> Result<f64, String> {
    use symphonia::core::codecs::CODEC_TYPE_NULL;
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
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "no audio track found".to_string())?;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| "container declared no sample rate".to_string())?;
    let frames = track
        .codec_params
        .n_frames
        .ok_or_else(|| "container declared no frame count".to_string())?;
    if rate == 0 {
        return Err("container declared a zero sample rate".into());
    }
    Ok(frames as f64 / f64::from(rate))
}

fn run_probe(path: &Path) -> i32 {
    match probe_duration_seconds(path) {
        Ok(seconds) => {
            println!("{seconds}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            EXIT_UNDECODABLE
        }
    }
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
        // The deadline is normally polled inside push_mono_sample, but a
        // container whose packets decode to zero frames never reaches it and
        // would spin until the parent's wall clock. Charge the budget per
        // packet so a hostile container cannot burn CPU for the full deadline.
        budget
            .check_deadline()
            .map_err(|error| format!("decode exceeded its resource budget: {error}"))?;
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A reset invalidates all later packets, so stop rather than
            // silently emitting a truncated decode as if it were complete.
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(_) => continue,
        };
        let spec = *decoded.spec();
        // Trust the decoded rate over the container's declaration. When an
        // HE-AAC/SBR stream halves the core rate, resampling by the declared
        // rate silently returns time- and pitch-scaled audio as success.
        if spec.rate != source_rate {
            return Err(format!(
                "declared sample rate {source_rate} does not match the decoded rate {}",
                spec.rate
            ));
        }
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

    /// The ceiling is the containment argument, so assert that the command the
    /// parent actually builds carries it. Comparing two literal constants
    /// proved nothing and would not notice the limit being dropped from the
    /// builder chain, which is the failure that matters.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_parent_command_carries_the_address_space_ceiling() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("tone.wav");
        write_test_wav(&source, 16_000, 1_600);
        let Ok(executable) = resolve_worker_executable() else {
            panic!("worker executable must resolve inside the test tree");
        };
        let mut command =
            crate::bounded_child::BoundedCommand::from_bound_executable(executable).unwrap();
        retain_safe_environment(&mut command);
        command
            .env(WORKER_MARKER, "1")
            .arg("--")
            .arg(&source)
            .single_process()
            .close_extra_descriptors()
            .address_space_limit(WORKER_ADDRESS_SPACE_BYTES);
        assert_eq!(
            command.configured_address_space_limit(),
            Some(WORKER_ADDRESS_SPACE_BYTES),
            "the decode child must be launched under an address-space ceiling"
        );
    }

    #[test]
    fn undecodable_input_fails_closed_rather_than_returning_silence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-audio.m4a");
        std::fs::write(&path, b"this is not a media container").unwrap();
        let error = decode_compressed_to_s16le(&path).unwrap_err();
        // Assert the failure class, not merely that some string came back:
        // an empty or wrong-class error would satisfy a non-empty check.
        assert!(
            error.contains("probe failed") || error.contains("no audio track found"),
            "unexpected failure for a non-container input: {error}"
        );
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
        // Deliberately not a silent skip. Reporting `ok` when the precondition
        // is absent is the defect class an earlier block in this epic was
        // rejected for: mutating the function under test still looked green
        // anywhere the worker could not resolve.
        resolve_worker_executable()
            .expect("a worker-capable executable must resolve for the end-to-end decode test");
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

    /// A committed one-second mono AAC/m4a fixture: the container an iPhone
    /// voice memo actually uses.
    ///
    /// Committed rather than encoded on demand so this test cannot skip in the
    /// exact environment the feature exists for, a machine with no ffmpeg.
    const M4A_FIXTURE: &[u8] = include_bytes!("../resources/decode-fixture-tone.m4a");

    /// The actual regression: an m4a voice memo must decode with no ffmpeg
    /// involved at decode time.
    #[test]
    fn compressed_m4a_decodes_without_ffmpeg_at_decode_time() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("memo.m4a");
        std::fs::write(&path, M4A_FIXTURE).unwrap();

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

    /// The end-to-end regression through the public decode entry point: with
    /// ffmpeg unavailable, a compressed import must still produce samples.
    #[test]
    fn compressed_import_survives_an_unavailable_ffmpeg() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("memo.m4a");
        std::fs::write(&path, M4A_FIXTURE).unwrap();

        let guard = crate::test_home_env_lock();
        let previous = std::env::var_os("MINUTES_FFMPEG");
        std::env::set_var("MINUTES_FFMPEG", directory.path().join("absent-ffmpeg"));
        let decoded =
            crate::transcribe::decode_compressed_for_test(&path, &crate::config::Config::default());
        match previous {
            Some(value) => std::env::set_var("MINUTES_FFMPEG", value),
            None => std::env::remove_var("MINUTES_FFMPEG"),
        }
        drop(guard);

        let samples = decoded.expect("a compressed import must decode without ffmpeg");
        assert!(
            (14_000..=18_000).contains(&samples.len()),
            "expected roughly one second at 16 kHz, got {}",
            samples.len()
        );
    }
}
