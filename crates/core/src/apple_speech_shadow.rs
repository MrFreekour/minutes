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
//! similarity score, an outcome), never the transcript text, so a shadow log
//! can never leak sensitive content. Wiring a shadow attempt into a capture
//! path is a separate step, paired with the on-hardware data run.

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

/// One shadow-mode comparison of Whisper (the shipped transcript) against an
/// Apple Speech attempt for the same audio.
///
/// Carries only measurements, never the transcript text, so it is safe to log
/// to disk under the same privacy rules as the rest of the pipeline.
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
    /// sequences (a normalized inverse word-error-rate). Only meaningful when
    /// `outcome == Usable`; `0.0` otherwise.
    pub similarity: f32,
    /// True when both engines produced the same normalized text.
    pub exact_match: bool,
    /// Failure detail when `outcome == Failed`; `None` otherwise. A bounded,
    /// single-line error category, never transcript content.
    pub apple_error: Option<String>,
}

/// Above this normalized word count, `similarity` uses a cheap length-ratio
/// proxy instead of an O(n*m) word edit distance, so a full-meeting batch
/// transcript can never make shadow logging quadratic. Per-utterance shadow
/// runs are far below this, so the exact metric is what actually gets used.
const SIMILARITY_WORD_CAP: usize = 3000;

/// Cap on the stored `apple_error` length. Worker errors are structured status
/// strings, but `compare` accepts an arbitrary `&str`, so the message is
/// collapsed to one line and truncated to defend the "no transcript content in
/// shadow logs" guarantee against a caller whose error embeds recognized text.
const MAX_ERROR_CHARS: usize = 160;

/// Build a shadow comparison from the shipped Whisper transcript and the Apple
/// Speech attempt result for the same audio.
///
/// `apple` is `Ok(Some(text))` for a usable transcript, `Ok(None)` for an empty
/// result, or `Err(msg)` when the attempt failed. Whisper is always present
/// because it is the shipped output. A `Some` value that normalizes to zero
/// words (e.g. the native bridge's empty-segment `""`) is classified `Empty`,
/// not `Usable`, so capability-success rates are not inflated.
pub fn compare(whisper: &str, apple: Result<Option<&str>, &str>) -> ShadowComparison {
    let whisper_words_vec = normalized_words(whisper);
    let whisper_chars = normalized_char_count(whisper);
    let whisper_words = whisper_words_vec.len();

    let empty = |outcome, apple_error| ShadowComparison {
        outcome,
        whisper_chars,
        whisper_words,
        apple_chars: 0,
        apple_words: 0,
        similarity: 0.0,
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
                return empty(ShadowOutcome::Empty, None);
            }
            ShadowComparison {
                outcome: ShadowOutcome::Usable,
                whisper_chars,
                whisper_words,
                apple_chars: normalized_char_count(apple_text),
                apple_words: apple_words_vec.len(),
                similarity: word_similarity(&whisper_words_vec, &apple_words_vec),
                exact_match: whisper_words_vec == apple_words_vec,
                apple_error: None,
            }
        }
        Ok(None) => empty(ShadowOutcome::Empty, None),
        Err(msg) => empty(ShadowOutcome::Failed, Some(bounded_error(msg))),
    }
}

/// Persist a shadow comparison to the structured JSONL log.
///
/// Writes through [`crate::logging::append_log`] so the measurement is durable:
/// the CLI's tracing subscriber only reaches stderr and the Tauri entry point
/// installs no subscriber at all, so tracing-only events would be dropped on
/// exactly the desktop path shadow mode most needs to measure. The write is
/// failure-isolated — a logging error is swallowed so it can never affect
/// capture — and a `debug` trace is emitted alongside for live tailing.
pub fn log_comparison(source: &str, cmp: &ShadowComparison) {
    let outcome = match cmp.outcome {
        ShadowOutcome::Usable => "usable",
        ShadowOutcome::Empty => "empty",
        ShadowOutcome::Failed => "failed",
    };
    let entry = serde_json::json!({
        "event": "apple_speech_shadow",
        "source": source,
        "outcome": outcome,
        "whisper_words": cmp.whisper_words,
        "apple_words": cmp.apple_words,
        "whisper_chars": cmp.whisper_chars,
        "apple_chars": cmp.apple_chars,
        "similarity": cmp.similarity,
        "exact_match": cmp.exact_match,
        "apple_error": cmp.apple_error,
    });
    // Failure-isolated: a logging failure must never affect capture.
    let _ = crate::logging::append_log(&entry);
    tracing::debug!(target: "apple_speech_shadow", source, outcome, "apple-speech shadow comparison");
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
/// normalized inverse of the word error rate. Two empty transcripts are treated
/// as identical (`1.0`); one empty and one not is `0.0`.
fn word_similarity(a: &[String], b: &[String]) -> f32 {
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    // Guard against a quadratic blow-up on very long (batch) transcripts.
    if max_len > SIMILARITY_WORD_CAP {
        let min_len = a.len().min(b.len());
        return min_len as f32 / max_len as f32;
    }
    let distance = word_edit_distance(a, b);
    1.0 - (distance as f32 / max_len as f32)
}

/// Levenshtein edit distance over word tokens, two-row DP (O(n*m) time,
/// O(min(n,m)) space). Inputs are per-utterance word lists, so this is cheap.
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

/// Collapse an error to a bounded, single-line diagnostic (see `MAX_ERROR_CHARS`).
fn bounded_error(msg: &str) -> String {
    let one_line = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > MAX_ERROR_CHARS {
        let truncated: String = one_line.chars().take(MAX_ERROR_CHARS).collect();
        format!("{truncated}…")
    } else {
        one_line
    }
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
        assert_eq!(cmp.similarity, 1.0);
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
        assert!(
            (cmp.similarity - 0.75).abs() < 1e-6,
            "got {}",
            cmp.similarity
        );
    }

    #[test]
    fn completely_different_text_has_low_similarity() {
        let cmp = compare("alpha beta gamma", Ok(Some("one two three")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert_eq!(cmp.similarity, 0.0);
        assert!(!cmp.exact_match);
    }

    #[test]
    fn an_empty_apple_result_is_recorded_without_a_failure() {
        let cmp = compare("whisper had text", Ok(None));
        assert_eq!(cmp.outcome, ShadowOutcome::Empty);
        assert_eq!(cmp.apple_words, 0);
        assert_eq!(cmp.whisper_words, 3);
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
            assert_eq!(cmp.similarity, 0.0);
        }
    }

    #[test]
    fn a_failed_apple_attempt_carries_a_bounded_error_not_the_transcript() {
        let cmp = compare("whisper had text", Err("worker crashed (XPC interrupted)"));
        assert_eq!(cmp.outcome, ShadowOutcome::Failed);
        assert_eq!(cmp.apple_words, 0);
        assert_eq!(
            cmp.apple_error.as_deref(),
            Some("worker crashed (XPC interrupted)")
        );
        // The Whisper side is still measured so the log shows what shipped.
        assert_eq!(cmp.whisper_words, 3);
    }

    #[test]
    fn a_long_multiline_error_is_bounded_to_one_line() {
        let huge = format!("failure: {}", "context ".repeat(80));
        let cmp = compare("hi", Err(&huge));
        let stored = cmp.apple_error.expect("failed attempt records an error");
        assert!(!stored.contains('\n'));
        assert!(
            stored.chars().count() <= MAX_ERROR_CHARS + 1,
            "bounded to {} chars, got {}",
            MAX_ERROR_CHARS,
            stored.chars().count()
        );
        assert!(stored.ends_with('…'), "truncation marker present");
    }

    #[test]
    fn similarity_is_symmetric_in_edit_distance() {
        let a = compare("one two three four", Ok(Some("one two three")));
        let b = compare("one two three", Ok(Some("one two three four")));
        assert!((a.similarity - b.similarity).abs() < 1e-6);
        // One deletion out of four: 0.75.
        assert!((a.similarity - 0.75).abs() < 1e-6, "got {}", a.similarity);
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
