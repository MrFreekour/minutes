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
//! This module is the measurement primitive: the pure comparison record and
//! its logging. It deliberately carries only measurements (counts, a
//! similarity score, an outcome) and a fixed error *category*, never the
//! transcript text or a raw error message, so a shadow log can never leak
//! sensitive content. Wiring a shadow attempt into a capture path is a separate
//! step, paired with the on-hardware data run.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowError {
    /// The worker process died (crash / abort).
    WorkerCrashed,
    /// The XPC connection was interrupted or invalidated.
    XpcInterrupted,
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

/// Lowercased, punctuation-insensitive words. The two engines format
/// differently (casing, spacing, and especially punctuation — Whisper writes
/// `Hello, world!` where Apple Speech may write `hello world`), and shadow mode
/// measures word content, not typography. Non-alphanumeric, non-whitespace
/// characters are mapped to spaces before splitting, mirroring
/// `apple_speech::eval_text_for_compare_punct_insensitive` so the shadow metric
/// and the offline evaluator agree on what "same words" means.
fn normalized_words(text: &str) -> Vec<String> {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(|w| w.to_lowercase())
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
}
