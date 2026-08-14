//! Quality gate and content-free measurements for provisional dictation text.
//!
//! A decoder hypothesis is not automatically UI-worthy. This gate requires a
//! stable prefix across consecutive hypotheses, keeps revisions confined to a
//! visibly provisional suffix, throttles paint churn, and records only timing
//! and token-count metrics (never dictated text).

use serde::{Deserialize, Serialize};

pub const MIN_PARTIAL_CADENCE_MS: u64 = 120;
pub const TARGET_FIRST_USEFUL_PARTIAL_MS: u64 = 700;
pub const TARGET_USEFUL_CADENCE_MAX_MS: u64 = 250;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatedPartial {
    pub text: String,
    pub stable_prefix: String,
    pub provisional_suffix: String,
    pub audio_ms: u64,
    pub decode_ms: u64,
    pub revision: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialQualityMetrics {
    pub first_useful_partial_ms: Option<u64>,
    pub useful_updates: u32,
    pub median_update_cadence_ms: Option<u64>,
    pub p95_update_cadence_ms: Option<u64>,
    pub p95_decode_ms: Option<u64>,
    pub max_provisional_revision_ratio: f64,
    pub stable_prefix_regressions: u32,
    pub rejected_hypotheses: u32,
}

#[derive(Debug, Default)]
pub struct PartialQualityGate {
    previous_tokens: Vec<String>,
    stable_tokens: Vec<String>,
    last_emitted_text: String,
    last_emit_audio_ms: Option<u64>,
    revision: u32,
    update_intervals_ms: Vec<u64>,
    decode_times_ms: Vec<u64>,
    metrics: PartialQualityMetrics,
}

impl PartialQualityGate {
    pub fn observe(&mut self, text: &str, audio_ms: u64, decode_ms: u64) -> Option<GatedPartial> {
        let tokens = useful_tokens(text)?;
        self.decode_times_ms.push(decode_ms);

        let common = common_prefix_len(&self.previous_tokens, &tokens);
        if common < self.stable_tokens.len() {
            // Never rewrite text already presented as stable. Wait for the
            // decoder to converge again; the final transcript still replaces
            // the provisional HUD wholesale.
            self.metrics.stable_prefix_regressions += 1;
            self.previous_tokens = tokens;
            return None;
        }
        if common > self.stable_tokens.len() {
            self.stable_tokens = tokens[..common].to_vec();
        }
        self.previous_tokens = tokens.clone();

        if self.stable_tokens.is_empty() {
            return None;
        }
        if self
            .last_emit_audio_ms
            .is_some_and(|last| audio_ms.saturating_sub(last) < MIN_PARTIAL_CADENCE_MS)
        {
            return None;
        }

        let stable_prefix = self.stable_tokens.join(" ");
        let provisional_suffix = tokens[self.stable_tokens.len()..].join(" ");
        let current = if provisional_suffix.is_empty() {
            stable_prefix.clone()
        } else {
            format!("{stable_prefix} {provisional_suffix}")
        };
        if current == self.last_emitted_text {
            return None;
        }

        if !self.last_emitted_text.is_empty() {
            let old = token_strings(&self.last_emitted_text);
            let unchanged = common_prefix_len(&old, &tokens);
            let stable_floor = self.stable_tokens.len().min(old.len());
            let revised = old.len().saturating_sub(unchanged.max(stable_floor));
            let provisional_len = old.len().saturating_sub(stable_floor).max(1);
            self.metrics.max_provisional_revision_ratio = self
                .metrics
                .max_provisional_revision_ratio
                .max(revised as f64 / provisional_len as f64);
        }

        if let Some(last) = self.last_emit_audio_ms {
            self.update_intervals_ms.push(audio_ms.saturating_sub(last));
        } else {
            self.metrics.first_useful_partial_ms = Some(audio_ms);
        }
        self.last_emit_audio_ms = Some(audio_ms);
        self.last_emitted_text = current.clone();
        self.revision = self.revision.saturating_add(1);
        self.metrics.useful_updates = self.metrics.useful_updates.saturating_add(1);

        Some(GatedPartial {
            text: current,
            stable_prefix,
            provisional_suffix,
            audio_ms,
            decode_ms,
            revision: self.revision,
        })
    }

    pub fn reject(&mut self) {
        self.metrics.rejected_hypotheses = self.metrics.rejected_hypotheses.saturating_add(1);
    }

    pub fn reset(&mut self) {
        self.previous_tokens.clear();
        self.stable_tokens.clear();
        self.last_emitted_text.clear();
        self.last_emit_audio_ms = None;
        self.revision = 0;
    }

    pub fn metrics(&self) -> PartialQualityMetrics {
        let mut metrics = self.metrics.clone();
        metrics.median_update_cadence_ms = percentile(&self.update_intervals_ms, 50);
        metrics.p95_update_cadence_ms = percentile(&self.update_intervals_ms, 95);
        metrics.p95_decode_ms = percentile(&self.decode_times_ms, 95);
        metrics
    }
}

fn useful_tokens(text: &str) -> Option<Vec<String>> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty()
        || !normalized.chars().any(char::is_alphanumeric)
        || (normalized.starts_with('[') && normalized.ends_with(']'))
    {
        return None;
    }
    let tokens = token_strings(&normalized);
    if tokens.len() >= 4 {
        let tail = &tokens[tokens.len() - 4..];
        if tail.windows(2).all(|pair| pair[0] == pair[1]) {
            return None;
        }
    }
    Some(tokens)
}

fn token_strings(text: &str) -> Vec<String> {
    text.split_whitespace().map(ToOwned::to_owned).collect()
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

fn percentile(values: &[u64], percentile: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) * percentile).div_ceil(100);
    sorted.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_hypothesis_is_not_mistaken_for_stable_text() {
        let mut gate = PartialQualityGate::default();
        assert!(gate.observe("Send the draft", 240, 18).is_none());
        let partial = gate.observe("Send the draft today", 420, 21).unwrap();
        assert_eq!(partial.stable_prefix, "Send the draft");
        assert_eq!(partial.provisional_suffix, "today");
        assert_eq!(gate.metrics().first_useful_partial_ms, Some(420));
    }

    #[test]
    fn revisions_never_rewrite_the_stable_prefix() {
        let mut gate = PartialQualityGate::default();
        gate.observe("book the flight", 200, 10);
        gate.observe("book the flight tomorrow", 400, 10).unwrap();
        assert!(gate
            .observe("cancel the flight tomorrow", 600, 10)
            .is_none());
        assert_eq!(gate.metrics().stable_prefix_regressions, 1);
    }

    #[test]
    fn cadence_is_throttled_and_measured_without_text() {
        let mut gate = PartialQualityGate::default();
        gate.observe("one two", 100, 8);
        gate.observe("one two three", 220, 9).unwrap();
        assert!(gate.observe("one two three four", 300, 7).is_none());
        gate.observe("one two three four five", 440, 11).unwrap();
        let metrics = gate.metrics();
        assert_eq!(metrics.useful_updates, 2);
        assert_eq!(metrics.median_update_cadence_ms, Some(220));
        assert_eq!(metrics.p95_decode_ms, Some(11));
    }

    #[test]
    fn noise_markers_and_repetition_do_not_reach_the_hud() {
        let mut gate = PartialQualityGate::default();
        assert!(gate.observe("[BLANK_AUDIO]", 200, 4).is_none());
        assert!(gate
            .observe("thanks thanks thanks thanks", 400, 4)
            .is_none());
        assert_eq!(gate.metrics().useful_updates, 0);
    }
}
