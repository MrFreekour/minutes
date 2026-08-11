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
    /// Apple Speech returned successfully but with no text (silence or a blip).
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
    /// Failure detail when `outcome == Failed`; `None` otherwise. This is an
    /// error string, never transcript content.
    pub apple_error: Option<String>,
}

/// Above this normalized word count, `similarity` uses a cheap length-ratio
/// proxy instead of an O(n*m) word edit distance, so a full-meeting batch
/// transcript can never make shadow logging quadratic. Per-utterance shadow
/// runs are far below this, so the exact metric is what actually gets used.
const SIMILARITY_WORD_CAP: usize = 3000;

/// Build a shadow comparison from the shipped Whisper transcript and the Apple
/// Speech attempt result for the same audio.
///
/// `apple` is `Ok(Some(text))` for a usable transcript, `Ok(None)` for an empty
/// result, or `Err(msg)` when the attempt failed. Whisper is always present
/// because it is the shipped output.
pub fn compare(whisper: &str, apple: Result<Option<&str>, &str>) -> ShadowComparison {
    let whisper_words_vec = normalized_words(whisper);
    let whisper_chars = normalized_char_count(whisper);
    let whisper_words = whisper_words_vec.len();

    match apple {
        Ok(Some(apple_text)) => {
            let apple_words_vec = normalized_words(apple_text);
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
        Ok(None) => ShadowComparison {
            outcome: ShadowOutcome::Empty,
            whisper_chars,
            whisper_words,
            apple_chars: 0,
            apple_words: 0,
            similarity: 0.0,
            exact_match: false,
            apple_error: None,
        },
        Err(msg) => ShadowComparison {
            outcome: ShadowOutcome::Failed,
            whisper_chars,
            whisper_words,
            apple_chars: 0,
            apple_words: 0,
            similarity: 0.0,
            exact_match: false,
            apple_error: Some(msg.to_string()),
        },
    }
}

/// Emit a shadow comparison to the structured log under the `apple_speech_shadow`
/// target. Logging only, so it is failure-isolated from capture: a shadow
/// attempt that crashed still lands here as an event, never as a panic.
pub fn log_comparison(source: &str, cmp: &ShadowComparison) {
    match cmp.outcome {
        ShadowOutcome::Usable => tracing::info!(
            target: "apple_speech_shadow",
            source,
            outcome = "usable",
            whisper_words = cmp.whisper_words,
            apple_words = cmp.apple_words,
            whisper_chars = cmp.whisper_chars,
            apple_chars = cmp.apple_chars,
            similarity = cmp.similarity,
            exact_match = cmp.exact_match,
            "apple-speech shadow comparison"
        ),
        ShadowOutcome::Empty => tracing::info!(
            target: "apple_speech_shadow",
            source,
            outcome = "empty",
            whisper_words = cmp.whisper_words,
            "apple-speech shadow: empty result while whisper produced text"
        ),
        ShadowOutcome::Failed => tracing::warn!(
            target: "apple_speech_shadow",
            source,
            outcome = "failed",
            error = cmp.apple_error.as_deref().unwrap_or(""),
            "apple-speech shadow: attempt failed, whisper used"
        ),
    }
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

/// Lowercased whitespace-split words. Casing and spacing differ between the two
/// engines' formatting, and shadow mode measures word content, not typography,
/// so both are normalized away before comparison.
fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace().map(|w| w.to_lowercase()).collect()
}

/// Character count of the normalized (lowercased, single-spaced) text. A size
/// signal for the log, not used in the similarity metric.
fn normalized_char_count(text: &str) -> usize {
    normalized_words(text).join(" ").chars().count()
}

/// Word-level similarity in `[0.0, 1.0]`: `1.0 - editdistance / max(len)`, a
/// normalized inverse of the word error rate. Two empty transcripts are treated
/// as identical (`1.0`); one empty and one not is `0.0`.
fn word_similarity(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_transcripts_are_a_perfect_match() {
        let cmp = compare("Hello world", Ok(Some("hello   WORLD")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert!(cmp.exact_match, "casing/spacing must normalize away");
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
    fn an_empty_apple_result_is_recorded_without_latching_a_failure() {
        let cmp = compare("whisper had text", Ok(None));
        assert_eq!(cmp.outcome, ShadowOutcome::Empty);
        assert_eq!(cmp.apple_words, 0);
        assert_eq!(cmp.whisper_words, 3);
        assert!(cmp.apple_error.is_none());
    }

    #[test]
    fn a_failed_apple_attempt_carries_the_error_not_the_transcript() {
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
    fn similarity_is_symmetric_in_edit_distance() {
        let a = compare("one two three four", Ok(Some("one two three")));
        let b = compare("one two three", Ok(Some("one two three four")));
        assert!((a.similarity - b.similarity).abs() < 1e-6);
        // One deletion out of four: 0.75.
        assert!((a.similarity - 0.75).abs() < 1e-6, "got {}", a.similarity);
    }

    #[test]
    fn both_empty_is_a_perfect_match() {
        let cmp = compare("", Ok(Some("")));
        assert_eq!(cmp.outcome, ShadowOutcome::Usable);
        assert_eq!(cmp.similarity, 1.0);
        assert!(cmp.exact_match);
        assert_eq!(cmp.whisper_words, 0);
        assert_eq!(cmp.apple_words, 0);
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
