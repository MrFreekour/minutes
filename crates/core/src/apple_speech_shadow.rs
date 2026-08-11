//! Apple Speech shadow-mode measurement (activation plan, Phase 6).
//!
//! Shadow mode runs Apple Speech alongside Whisper purely to measure the
//! real-world capability distribution and the transcript quality delta before
//! any user-facing exposure. It never affects the transcript the user sees or
//! the recording: the shadow attempt is failure-isolated (RFC 0004), gated
//! behind an off-by-default config flag, and independent of the product
//! transport gate (`apple_speech_private_audio_transport_supported`). Shadow
//! measures; it never exposes.
//!
//! This module holds the measurement primitive (the pure comparison record and
//! its logging) plus the non-blocking executor that drives it: [`ShadowRunner`]
//! runs each attempt on a dedicated thread with a bounded, drop-when-busy queue,
//! so a slow or crashing shadow attempt can never stall capture. Records carry
//! only measurements (counts, a similarity score, an outcome) and a fixed error
//! *category*, never the transcript text or a raw error message, so a shadow log
//! can never leak sensitive content. What remains is hooking
//! [`ShadowRunner::try_submit`] into a capture path (the live sidecar first),
//! paired with the on-hardware data run.

use crate::apple_speech_session::AppleSpeechSession;
use crate::config::Config;

/// What Apple Speech produced for a shadow utterance, relative to Whisper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowOutcome {
    /// Apple Speech returned a non-empty transcript.
    Usable,
    /// Apple Speech returned successfully but with no words: silence, a blip, or
    /// the native bridge's empty-segment `""`.
    Empty,
    /// Apple Speech failed: worker crash, XPC interruption, or a Speech error.
    Failed,
}

/// A fixed failure category for a shadow attempt.
///
/// `compare` takes this category directly rather than a raw error string. The
/// caller has the typed failure context (worker exit, XPC status, Speech error),
/// so it can categorize accurately; and a closed enum makes it *impossible* for a
/// raw message — which can embed recognized speech — to reach a shadow log. That
/// is a stronger guarantee than trying to redact free text after the fact.
///
/// The variants cover the known worker outcomes so the wiring step can map onto
/// them without guessing. That mapping is the wiring increment's job, and it may
/// need to enrich `apple_speech_worker` to surface a typed outcome: today
/// transport failures flatten to `MinutesError::Io`, framework failures live in
/// `AppleSpeechTranscriptionResult.error`, and `runtime_supported == false` is a
/// bool — so the wiring, not this primitive, owns turning those into the right
/// [`ShadowError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowError {
    /// The worker process died (crash / abort).
    WorkerCrashed,
    /// The XPC connection was interrupted or invalidated.
    XpcInterrupted,
    /// The device/runtime cannot run Apple Speech (`runtime_supported == false`).
    RuntimeUnsupported,
    /// Speech assets are not installed for the locale.
    AssetsUnavailable,
    /// A Speech-framework or analyzer error.
    SpeechError,
    /// Anything else; the raw message is not retained.
    Unknown,
}

impl ShadowError {
    /// Stable log token for this category.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkerCrashed => "worker_crashed",
            Self::XpcInterrupted => "xpc_interrupted",
            Self::RuntimeUnsupported => "runtime_unsupported",
            Self::AssetsUnavailable => "assets_unavailable",
            Self::SpeechError => "speech_error",
            Self::Unknown => "unknown",
        }
    }
}

/// One shadow-mode comparison of Whisper (the shipped transcript) against an
/// Apple Speech attempt for the same audio.
///
/// Carries only measurements and a fixed error category, never the transcript
/// text, so it is safe to log to disk under the same privacy rules as the rest
/// of the pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowComparison {
    /// Whether Apple Speech produced a usable transcript, an empty one, or failed.
    pub outcome: ShadowOutcome,
    /// Character count of the shipped Whisper transcript (post-normalization).
    pub whisper_chars: usize,
    /// Word count of the shipped Whisper transcript (post-normalization).
    pub whisper_words: usize,
    /// Character count of the Apple Speech transcript; 0 unless `Usable`.
    pub apple_chars: usize,
    /// Word count of the Apple Speech transcript; 0 unless `Usable`.
    pub apple_words: usize,
    /// Word-level similarity in `[0.0, 1.0]`, `1.0` meaning identical word
    /// sequences (a normalized inverse word-error-rate). `None` when there is
    /// nothing to score (`outcome != Usable`) or when the transcripts differ and
    /// exceed [`SIMILARITY_WORD_CAP`] words, where an exact score is skipped
    /// rather than faked. Never a placeholder number.
    pub similarity: Option<f32>,
    /// True when both engines produced the same normalized text.
    pub exact_match: bool,
    /// Failure category when `outcome == Failed`; `None` otherwise. A fixed
    /// category, never transcript content or a raw message.
    pub apple_error: Option<ShadowError>,
}

/// Above this normalized word count, and only when the two transcripts are not
/// already known-equal, `similarity` is reported as `None` instead of running an
/// O(n*m) word edit distance, so a pathologically long transcript can never make
/// shadow logging quadratic. Shadow runs per utterance, far below this cap, so
/// the exact metric is what actually gets used.
const SIMILARITY_WORD_CAP: usize = 3000;

/// Build a shadow comparison from the shipped Whisper transcript and the Apple
/// Speech attempt result for the same audio.
///
/// `apple` is `Ok(Some(text))` for a usable transcript, `Ok(None)` for an empty
/// result, or `Err(category)` when the attempt failed, where the caller supplies
/// the typed [`ShadowError`] category (no raw message crosses this boundary).
/// Whisper is always present because it is the shipped output. A `Some` value
/// that normalizes to zero words (e.g. the native bridge's empty-segment `""`)
/// is classified `Empty`, not `Usable`, so capability-success rates are not
/// inflated.
pub fn compare(whisper: &str, apple: Result<Option<&str>, ShadowError>) -> ShadowComparison {
    let whisper_words_vec = normalized_words(whisper);
    let whisper_chars = normalized_char_count(whisper);
    let whisper_words = whisper_words_vec.len();

    let non_usable = |outcome, apple_error| ShadowComparison {
        outcome,
        whisper_chars,
        whisper_words,
        apple_chars: 0,
        apple_words: 0,
        similarity: None,
        exact_match: false,
        apple_error,
    };

    match apple {
        Ok(Some(apple_text)) => {
            let apple_words_vec = normalized_words(apple_text);
            if apple_words_vec.is_empty() {
                // A Some("") / whitespace / punctuation-only result is not a
                // usable transcript — the native bridge returns "" when there
                // are no segments. Record it as Empty.
                return non_usable(ShadowOutcome::Empty, None);
            }
            let exact_match = whisper_words_vec == apple_words_vec;
            // Equality is cheap and definitive, so identical transcripts always
            // score 1.0 regardless of length; only differing, over-cap ones go None.
            let similarity = if exact_match {
                Some(1.0)
            } else {
                word_similarity(&whisper_words_vec, &apple_words_vec)
            };
            ShadowComparison {
                outcome: ShadowOutcome::Usable,
                whisper_chars,
                whisper_words,
                apple_chars: normalized_char_count(apple_text),
                apple_words: apple_words_vec.len(),
                similarity,
                exact_match,
                apple_error: None,
            }
        }
        Ok(None) => non_usable(ShadowOutcome::Empty, None),
        Err(category) => non_usable(ShadowOutcome::Failed, Some(category)),
    }
}

/// Persist a shadow comparison to the structured JSONL log, returning the write
/// result.
///
/// Writes through [`crate::logging::append_log`] so the measurement is durable:
/// the CLI's tracing subscriber only reaches stderr and the Tauri entry point
/// installs no subscriber at all, so tracing-only events would be dropped on
/// exactly the desktop path shadow mode most needs to measure. A write failure
/// (unwritable log dir, full disk) is surfaced two ways — a `warn` trace and the
/// returned `Err` — instead of being silently swallowed behind a success-looking
/// event. It stays failure-isolated from capture: the caller logs or ignores the
/// error and keeps recording; this function never panics.
pub fn log_comparison(source: &str, cmp: &ShadowComparison) -> std::io::Result<()> {
    let outcome = match cmp.outcome {
        ShadowOutcome::Usable => "usable",
        ShadowOutcome::Empty => "empty",
        ShadowOutcome::Failed => "failed",
    };
    let entry = serde_json::json!({
        "event": "apple_speech_shadow",
        // Each caller injects its own timestamp; append_log does not. Without it
        // measurements can't be correlated with the OS/app/worker change being
        // evaluated across sessions (daily rotation gives at most a file date).
        "ts": chrono::Utc::now().to_rfc3339(),
        "source": source,
        "outcome": outcome,
        "whisper_words": cmp.whisper_words,
        "apple_words": cmp.apple_words,
        "whisper_chars": cmp.whisper_chars,
        "apple_chars": cmp.apple_chars,
        "similarity": cmp.similarity,
        "exact_match": cmp.exact_match,
        "apple_error": cmp.apple_error.map(ShadowError::as_str),
    });
    let result = crate::logging::append_log(&entry);
    match &result {
        Ok(()) => tracing::debug!(
            target: "apple_speech_shadow",
            source,
            outcome,
            "apple-speech shadow comparison persisted"
        ),
        Err(error) => tracing::warn!(
            target: "apple_speech_shadow",
            source,
            outcome,
            %error,
            "failed to persist apple-speech shadow comparison"
        ),
    }
    result
}

/// Whether shadow mode is switched on in config.
///
/// This is only the config intent. An actual shadow attempt additionally
/// requires the macOS worker to be available (checked at the call site) and
/// respects the per-session
/// [`crate::apple_speech_session::AppleSpeechSession`] latch. It is independent
/// of the product transport gate: shadow measures, it never exposes.
pub fn shadow_enabled(config: &Config) -> bool {
    config.transcription.apple_speech_shadow
}

/// Minimum utterance length before a shadow attempt is worth making: 1 second at
/// 16 kHz, matching the live/dictation Apple Speech thresholds. Sub-second blips
/// are dropped rather than spawning a worker for noise.
const SHADOW_MIN_SAMPLES: usize = 16_000;

/// Decode a *successful* Apple Speech worker response into the `compare` apple
/// argument, following the real bridge contract:
///
/// - `runtime_supported == false` is how the bridge's `failureResponse` encodes
///   *any* thrown error — analyzer, asset, or genuinely-unsupported alike, each
///   with a message. The bridge does not distinguish them, so this is a coarse
///   [`ShadowError::SpeechError`]. Finer categories (`RuntimeUnsupported`,
///   `AssetsUnavailable`) need a typed error kind from the bridge, tracked as the
///   follow-up on [`ShadowError`].
/// - Otherwise the runtime ran. A non-empty transcript is `Ok(Some(_))`. An empty
///   one is the "analyzer completed without results" case (`runtime_supported ==
///   true` with an advisory message) and is `Ok(None)` — an `Empty` outcome, not
///   a failure, so the session keeps attempting.
///
/// No raw error string is read or carried, so no transcript content can leak.
pub fn worker_ok_to_shadow(
    runtime_supported: bool,
    transcript: &str,
) -> Result<Option<String>, ShadowError> {
    if !runtime_supported {
        Err(ShadowError::SpeechError)
    } else if transcript.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(transcript.to_string()))
    }
}

/// Map a worker-call transport failure (the `Err` arm of `transcribe_samples`)
/// into the shadow taxonomy by its `io::ErrorKind`, without reading the message.
/// `Other` is the XPC/worker layer failing (crash, interruption, or the 180s
/// budget); the rest are environment or our-side faults (missing worker
/// authority, over-budget or malformed input/response) with no Speech-capability
/// signal, recorded as `Unknown`.
pub fn transport_error_category(kind: std::io::ErrorKind) -> ShadowError {
    match kind {
        std::io::ErrorKind::Other => ShadowError::XpcInterrupted,
        _ => ShadowError::Unknown,
    }
}

/// One utterance queued for shadow comparison.
struct ShadowJob {
    samples: Vec<f32>,
    whisper_text: String,
}

/// Process-wide admission: at most one shadow attempt runs at a time across every
/// `ShadowRunner` (and therefore every recording session) in the process. A
/// per-runner flag would let rapid stop/start cycles leave several detached 180s
/// worker jobs overlapping a new recording; a single global gate bounds that to
/// one, keeping shadow's CPU/memory/asset-download load off active capture.
static SHADOW_ADMISSION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A non-blocking, failure-isolated executor for shadow comparisons.
///
/// The shadow attempt runs on a dedicated *detached* thread. A process-wide
/// admission gate ([`SHADOW_ADMISSION`]) admits exactly one attempt at a time
/// across all sessions; any utterance arriving while one is running is dropped,
/// never queued. `try_submit` never blocks and never copies audio (it takes
/// ownership). This upholds the standing "recording must never be degraded by an
/// optional consumer" decision (RFC 0004): the sidecar cannot be stalled or
/// back-pressured, and — critically — shutdown does not join the worker thread,
/// so a shadow attempt still inside the worker's 180s budget cannot delay Stop or
/// WAV preservation. Dropping the runner sets a cancel flag and closes the
/// channel: a queued-but-not-started job is skipped, and a call already inside the
/// worker finishes in the background. The last in-flight comparison is therefore
/// best-effort (it may be lost at process exit) — an acceptable trade for never
/// delaying capture. A per-session [`AppleSpeechSession`] latch stops attempts
/// after the first failure, so an incapable device is not re-probed every
/// utterance.
pub struct ShadowRunner {
    tx: std::sync::mpsc::SyncSender<ShadowJob>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    session: std::sync::Arc<AppleSpeechSession>,
}

impl ShadowRunner {
    /// Spawn a runner whose `attempt` performs one Apple Speech attempt for a set
    /// of samples and maps it into the shadow taxonomy. `source` labels the
    /// surface in the log. Comparisons are persisted via [`log_comparison`].
    ///
    /// Returns `None` if the OS refuses the worker thread (resource limits): an
    /// optional consumer disables itself rather than panicking the caller.
    pub fn spawn<F>(source: &'static str, attempt: F) -> Option<Self>
    where
        F: FnMut(&[f32]) -> Result<Option<String>, ShadowError> + Send + 'static,
    {
        Self::spawn_with_sink(source, attempt, |src, cmp| {
            let _ = log_comparison(src, cmp);
        })
    }

    /// As [`ShadowRunner::spawn`], with an injectable comparison sink for tests.
    fn spawn_with_sink<F, S>(source: &'static str, mut attempt: F, mut sink: S) -> Option<Self>
    where
        F: FnMut(&[f32]) -> Result<Option<String>, ShadowError> + Send + 'static,
        S: FnMut(&'static str, &ShadowComparison) + Send + 'static,
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (tx, rx) = std::sync::mpsc::sync_channel::<ShadowJob>(1);
        let session = std::sync::Arc::new(AppleSpeechSession::new());
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let thread_session = std::sync::Arc::clone(&session);
        let thread_cancelled = std::sync::Arc::clone(&cancelled);
        // `.ok()?` discards the JoinHandle, detaching the thread: nothing ever
        // joins it, so shutdown cannot block on an in-flight worker attempt.
        std::thread::Builder::new()
            .name("apple-speech-shadow".to_string())
            .spawn(move || {
                for job in rx {
                    // Skip a queued job if the runner was dropped (cancelled) or
                    // the session latched off, but still release admission below.
                    if !thread_cancelled.load(Ordering::Acquire) && thread_session.should_attempt()
                    {
                        let apple = attempt(&job.samples);
                        let cmp = compare(
                            &job.whisper_text,
                            apple.as_ref().map(|opt| opt.as_deref()).map_err(|e| *e),
                        );
                        match cmp.outcome {
                            ShadowOutcome::Usable => thread_session.record_success(),
                            ShadowOutcome::Failed => thread_session.record_failure(),
                            // Worker ran fine but produced no speech: not a
                            // capability failure, so the session keeps attempting.
                            ShadowOutcome::Empty => {}
                        }
                        sink(source, &cmp);
                    }
                    // Release the process-wide slot only after the attempt fully
                    // settles, whether it ran or was skipped, so a new submission
                    // (this or another session) cannot start a second worker.
                    SHADOW_ADMISSION.store(false, Ordering::Release);
                }
            })
            .ok()?;
        Some(Self {
            tx,
            cancelled,
            session,
        })
    }

    /// Submit an utterance for shadow comparison without ever blocking the caller
    /// and without copying audio: ownership of `samples` moves into the runner, so
    /// there is no infallible allocation on the capture path. Returns `false`
    /// (skips) when the utterance is too short, when the session has latched off
    /// after a failure, or when a shadow attempt is already in flight anywhere in
    /// the process. A dropped sample is acceptable for measurement.
    pub fn try_submit(&self, samples: Vec<f32>, whisper_text: String) -> bool {
        use std::sync::atomic::Ordering;
        if samples.len() < SHADOW_MIN_SAMPLES || !self.session.should_attempt() {
            return false;
        }
        // Claim the process-wide slot; if an attempt is already in flight, drop.
        if SHADOW_ADMISSION.swap(true, Ordering::AcqRel) {
            return false;
        }
        match self.tx.try_send(ShadowJob {
            samples,
            whisper_text,
        }) {
            Ok(()) => true,
            Err(_) => {
                // Receiver gone (thread ended): release the slot we claimed.
                SHADOW_ADMISSION.store(false, Ordering::Release);
                false
            }
        }
    }
}

impl Drop for ShadowRunner {
    fn drop(&mut self) {
        // Cancel a queued-but-not-started job so it can't run the worker during a
        // later session, then let `tx` drop to close the channel. We never join:
        // a call already inside the worker finishes in the background, so Stop and
        // WAV preservation are never delayed.
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Build a live-sidecar shadow runner that drives the real Apple Speech worker.
///
/// macOS only, because it calls the XPC worker. The sidecar constructs this once
/// per session when [`shadow_enabled`] is true, then feeds it finalized Whisper
/// utterances via [`ShadowRunner::try_submit`]. A successful call is decoded by
/// [`worker_ok_to_shadow`]; a transport `Err` is categorized by its
/// `io::ErrorKind` via [`transport_error_category`]. Returns `None` if the worker
/// thread cannot be spawned, disabling shadow rather than affecting capture.
#[cfg(target_os = "macos")]
pub fn spawn_live_shadow_runner(config: &Config) -> Option<ShadowRunner> {
    let language = config.transcription.language.clone();
    ShadowRunner::spawn("live-sidecar", move |samples| {
        let locale = crate::apple_speech::live_locale_hint(language.as_deref());
        match crate::apple_speech_worker::transcribe_samples(
            samples,
            locale.as_deref(),
            crate::apple_speech::AppleSpeechMode::Speech,
            true,
        ) {
            Ok(result) => worker_ok_to_shadow(result.runtime_supported, &result.transcript),
            Err(crate::error::MinutesError::Io(io)) => Err(transport_error_category(io.kind())),
            Err(_) => Err(ShadowError::Unknown),
        }
    })
}

/// Lowercased, timestamp- and punctuation-insensitive words. The two engines
/// format differently: batch Whisper prefixes lines with `[mm:ss]` timestamps
/// and punctuates (`Hello, world!`) where Apple Speech emits plain `hello world`.
/// Shadow mode measures word content, not typography, so each line is first run
/// through `pipeline::clean_transcript_line` (stripping `[...]` prefixes, exactly
/// as the offline evaluator does) and then punctuation is folded:
///
/// - word-internal apostrophes and periods are *deleted*, so contractions and
///   abbreviations survive as one token (`can't` == `cant`, `U.S.` == `us`),
/// - every other non-alphanumeric character becomes a space (a word boundary).
///
/// This follows `apple_speech::eval_text_for_compare_punct_insensitive` but folds
/// contractions the base evaluator would split. Residual heuristic limits remain
/// (e.g. hyphenated compounds still split); a shadow similarity is a proxy, not a
/// ground-truth WER.
fn normalized_words(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(crate::pipeline::clean_transcript_line)
        .flat_map(|line| {
            line.chars()
                .filter_map(|c| {
                    if c.is_alphanumeric() || c.is_whitespace() {
                        Some(c)
                    } else if c == '\'' || c == '.' {
                        // Fold word-internal punctuation: can't -> cant, U.S. -> us.
                        None
                    } else {
                        Some(' ')
                    }
                })
                .collect::<String>()
                .split_whitespace()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Character count of the normalized (lowercased, punctuation-free, single-spaced)
/// text. A size signal for the log, not used in the similarity metric.
fn normalized_char_count(text: &str) -> usize {
    normalized_words(text).join(" ").chars().count()
}

/// Word-level similarity in `[0.0, 1.0]`: `1.0 - editdistance / max(len)`, a
/// normalized inverse of the word error rate. `None` when the inputs differ and
/// exceed [`SIMILARITY_WORD_CAP`] words, so a pathological input is reported as
/// unscored rather than approximated with a misleading number. Callers handle
/// the known-equal case before calling, so this never sees two identical inputs.
fn word_similarity(a: &[String], b: &[String]) -> Option<f32> {
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return Some(1.0);
    }
    if max_len > SIMILARITY_WORD_CAP {
        return None;
    }
    let distance = word_edit_distance(a, b);
    Some(1.0 - (distance as f32 / max_len as f32))
}

/// Levenshtein edit distance over word tokens, two-row DP (O(n*m) time,
/// O(min(n,m)) space). Bounded by [`SIMILARITY_WORD_CAP`] at the call site.
fn word_edit_distance(a: &[String], b: &[String]) -> usize {
    // Iterate over the longer sequence in the outer loop so the row we allocate
    // is the shorter of the two.
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    if short.is_empty() {
        return long.len();
    }
    let mut prev: Vec<usize> = (0..=short.len()).collect();
    let mut curr: Vec<usize> = vec![0; short.len() + 1];
    for (i, long_word) in long.iter().enumerate() {
        curr[0] = i + 1;
        for (j, short_word) in short.iter().enumerate() {
            let cost = usize::from(long_word != short_word);
            curr[j + 1] = (prev[j + 1] + 1) // deletion
                .min(curr[j] + 1) // insertion
                .min(prev[j] + cost); // substitution
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[short.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punctuation_and_casing_normalize_away_to_a_perfect_match() {
        // The engines punctuate and case differently; only word content counts.
        let cmp = compare("Hello, world!", Ok(Some("hello   WORLD")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert!(
            cmp.exact_match,
            "punctuation/casing/spacing must normalize away"
        );
        assert_eq!(cmp.similarity, Some(1.0));
        assert_eq!(cmp.whisper_words, 2);
        assert_eq!(cmp.apple_words, 2);
        assert!(cmp.apple_error.is_none());
    }

    #[test]
    fn timestamped_whisper_lines_are_stripped_before_comparison() {
        // Batch Whisper output carries [mm:ss] prefixes; Apple Speech does not.
        // Identical speech must still score 1.0, not be penalized for timestamps.
        let cmp = compare("[0:05] hello world", Ok(Some("hello world")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert!(cmp.exact_match, "timestamp prefix must be stripped");
        assert_eq!(cmp.similarity, Some(1.0));
        assert_eq!(cmp.whisper_words, 2, "the 0 and 05 must not count as words");
    }

    #[test]
    fn contractions_and_abbreviations_fold_to_a_match() {
        // Engines format contractions/abbreviations differently; word-internal
        // apostrophes and periods must not create spurious word boundaries.
        let cmp = compare("I can't visit the U.S.", Ok(Some("i cant visit the us")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert!(cmp.exact_match, "can't==cant and U.S.==us");
        assert_eq!(cmp.similarity, Some(1.0));
    }

    #[test]
    fn a_one_word_difference_lowers_similarity_but_stays_usable() {
        let cmp = compare("the quick brown fox", Ok(Some("the quick red fox")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert!(!cmp.exact_match);
        // One substitution out of four words: 1 - 1/4 = 0.75.
        let s = cmp.similarity.expect("scored");
        assert!((s - 0.75).abs() < 1e-6, "got {s}");
    }

    #[test]
    fn completely_different_text_scores_zero() {
        let cmp = compare("alpha beta gamma", Ok(Some("one two three")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert_eq!(cmp.similarity, Some(0.0));
        assert!(!cmp.exact_match);
    }

    #[test]
    fn an_empty_apple_result_is_recorded_without_a_failure() {
        let cmp = compare("whisper had text", Ok(None));
        assert_eq!(cmp.outcome, ShadowOutcome::Empty);
        assert_eq!(cmp.apple_words, 0);
        assert_eq!(cmp.whisper_words, 3);
        assert_eq!(cmp.similarity, None);
        assert!(cmp.apple_error.is_none());
    }

    #[test]
    fn a_blank_or_punctuation_only_some_is_empty_not_usable() {
        // The native bridge returns "" when there are no segments; scoring that
        // as Usable/1.0 would inflate the capability-success rate.
        for blank in ["", "   ", " ...!? "] {
            let cmp = compare("whisper had text", Ok(Some(blank)));
            assert_eq!(
                cmp.outcome,
                ShadowOutcome::Empty,
                "{blank:?} should classify as Empty"
            );
            assert_eq!(cmp.apple_words, 0);
            assert_eq!(cmp.similarity, None);
        }
    }

    #[test]
    fn a_failed_attempt_records_the_caller_supplied_category() {
        // The caller passes a typed category; no raw message can cross the API,
        // so transcript content can never reach the record by construction.
        let cmp = compare("whisper had text", Err(ShadowError::XpcInterrupted));
        assert_eq!(cmp.outcome, ShadowOutcome::Failed);
        assert_eq!(cmp.apple_words, 0);
        assert_eq!(cmp.similarity, None);
        assert_eq!(cmp.apple_error, Some(ShadowError::XpcInterrupted));
        assert_eq!(cmp.whisper_words, 3);
    }

    #[test]
    fn similarity_is_symmetric_in_edit_distance() {
        let a = compare("one two three four", Ok(Some("one two three")));
        let b = compare("one two three", Ok(Some("one two three four")));
        assert_eq!(a.similarity, b.similarity);
        // One deletion out of four: 0.75.
        let s = a.similarity.expect("scored");
        assert!((s - 0.75).abs() < 1e-6, "got {s}");
    }

    #[test]
    fn differing_transcripts_over_the_cap_are_unscored_not_faked() {
        let big_a = (0..=SIMILARITY_WORD_CAP)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let big_b = (0..=SIMILARITY_WORD_CAP)
            .map(|i| format!("b{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let cmp = compare(&big_a, Ok(Some(&big_b)));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert!(!cmp.exact_match);
        // The old length-ratio proxy would have reported 1.0 here; honest is None.
        assert_eq!(cmp.similarity, None);
    }

    #[test]
    fn identical_over_the_cap_still_scores_one() {
        let big = (0..=SIMILARITY_WORD_CAP)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let cmp = compare(&big, Ok(Some(&big)));
        assert!(cmp.exact_match);
        assert_eq!(
            cmp.similarity,
            Some(1.0),
            "equality is cheap and definitive"
        );
    }

    #[test]
    fn shadow_is_off_by_default() {
        let config = Config::default();
        assert!(
            !shadow_enabled(&config),
            "shadow mode must be off by default — it is a measurement opt-in"
        );
    }

    #[test]
    fn shadow_reads_the_config_flag_when_enabled() {
        let config = Config {
            transcription: crate::config::TranscriptionConfig {
                apple_speech_shadow: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(shadow_enabled(&config));
    }

    #[test]
    fn worker_ok_maps_the_real_bridge_contract() {
        // runtime_supported=false is how the bridge encodes ANY thrown error.
        assert_eq!(
            worker_ok_to_shadow(false, "ignored"),
            Err(ShadowError::SpeechError)
        );
        // runtime_supported=true + empty transcript is the "no results" case.
        assert_eq!(worker_ok_to_shadow(true, ""), Ok(None));
        assert_eq!(worker_ok_to_shadow(true, "   "), Ok(None));
        // A real transcript flows through.
        assert_eq!(
            worker_ok_to_shadow(true, "hello world"),
            Ok(Some("hello world".to_string()))
        );
    }

    #[test]
    fn transport_errors_are_categorized_by_kind() {
        use std::io::ErrorKind;
        // The XPC/worker layer failing (crash, interruption, 180s budget).
        assert_eq!(
            transport_error_category(ErrorKind::Other),
            ShadowError::XpcInterrupted
        );
        // Environment / our-side faults carry no Speech-capability signal.
        assert_eq!(
            transport_error_category(ErrorKind::PermissionDenied),
            ShadowError::Unknown
        );
        assert_eq!(
            transport_error_category(ErrorKind::InvalidData),
            ShadowError::Unknown
        );
    }

    fn long_samples() -> Vec<f32> {
        vec![0.1_f32; SHADOW_MIN_SAMPLES]
    }

    /// Serialize the runner tests and reset the process-wide admission gate, so a
    /// prior test's detached thread cannot leave `SHADOW_ADMISSION` set and make a
    /// later test's submission spuriously drop.
    fn runner_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        SHADOW_ADMISSION.store(false, std::sync::atomic::Ordering::Release);
        guard
    }

    #[test]
    fn runner_skips_utterances_shorter_than_the_threshold() {
        let _guard = runner_test_guard();
        let runner =
            ShadowRunner::spawn_with_sink("test", |_| Ok(Some("apple".to_string())), |_, _| {})
                .expect("spawn shadow runner");
        assert!(
            !runner.try_submit(vec![0.1; 10], "whisper".to_string()),
            "sub-threshold utterance must be dropped without a worker spawn"
        );
    }

    #[test]
    fn runner_compares_a_submitted_utterance_and_persists_via_the_sink() {
        use std::sync::mpsc;
        let _guard = runner_test_guard();
        let (done_tx, done_rx) = mpsc::channel();
        let runner = ShadowRunner::spawn_with_sink(
            "test",
            |_| Ok(Some("the quick brown fox".to_string())),
            move |_, cmp| done_tx.send(cmp.clone()).unwrap(),
        )
        .expect("spawn shadow runner");
        assert!(runner.try_submit(long_samples(), "the quick brown fox".to_string()));
        let cmp = done_rx.recv().unwrap();
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert!(cmp.exact_match);
        assert_eq!(cmp.similarity, Some(1.0));
    }

    #[test]
    fn runner_latches_off_after_a_failure_and_stops_attempting() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{mpsc, Arc};
        let _guard = runner_test_guard();
        let calls = Arc::new(AtomicUsize::new(0));
        let thread_calls = Arc::clone(&calls);
        let (done_tx, done_rx) = mpsc::channel();
        let runner = ShadowRunner::spawn_with_sink(
            "test",
            move |_| {
                thread_calls.fetch_add(1, Ordering::SeqCst);
                Err(ShadowError::XpcInterrupted)
            },
            move |_, cmp| done_tx.send(cmp.outcome).unwrap(),
        )
        .expect("spawn shadow runner");

        assert!(runner.try_submit(long_samples(), "whisper text".to_string()));
        // Wait for the first attempt to be processed so the session has latched.
        assert_eq!(done_rx.recv().unwrap(), ShadowOutcome::Failed);

        // The session is now latched off: further submits are skipped and the
        // attempt closure is never called again.
        assert!(!runner.try_submit(long_samples(), "whisper text".to_string()));
        drop(runner);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "must not re-attempt after a failure"
        );
    }
}
