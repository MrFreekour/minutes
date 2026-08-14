//! Offline safety evaluation for a proposed implicit `actually` correction detector.
//!
//! This example is deliberately not linked into the product path. It measures
//! whether a surface-form heuristic could safely earn promotion.

use serde::Deserialize;
use std::collections::BTreeMap;

const FIXTURES: &str =
    include_str!("../../../docs/evals/fixtures/dictation-semantic-corrections.json");

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    category: String,
    text: String,
    should_rewrite: bool,
}

#[derive(Debug, Default)]
struct CategoryScore {
    total: usize,
    candidates: usize,
    false_rewrites: usize,
    missed_corrections: usize,
}

fn proposed_candidate_detector(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let Some(index) = lower.find("actually,") else {
        return false;
    };
    let before = lower[..index].trim_end();
    let after = lower[index + "actually,".len()..]
        .trim()
        .trim_end_matches(['.', '!', '?']);
    let explicit_break = before.ends_with('—') || before.ends_with('-');
    let replacement_words = after.split_whitespace().count();
    explicit_break && (1..=5).contains(&replacement_words)
}

fn main() {
    let cases: Vec<Case> = serde_json::from_str(FIXTURES).expect("valid correction fixtures");
    let mut categories = BTreeMap::<String, CategoryScore>::new();
    let mut positives = 0usize;
    let mut negatives = 0usize;
    let mut true_positives = 0usize;
    let mut false_rewrites = Vec::new();
    let mut misses = Vec::new();

    for case in &cases {
        let candidate = proposed_candidate_detector(&case.text);
        let score = categories.entry(case.category.clone()).or_default();
        score.total += 1;
        score.candidates += usize::from(candidate);
        if case.should_rewrite {
            positives += 1;
            if candidate {
                true_positives += 1;
            } else {
                score.missed_corrections += 1;
                misses.push(case.id.as_str());
            }
        } else {
            negatives += 1;
            if candidate {
                score.false_rewrites += 1;
                false_rewrites.push(case.id.as_str());
            }
        }
    }

    let recall = true_positives as f64 / positives as f64;
    let false_rewrite_rate = false_rewrites.len() as f64 / negatives as f64;
    let promotion_passed = false_rewrites.is_empty() && recall >= 0.90 && cases.len() >= 1_000;

    println!("cases={}", cases.len());
    println!("genuine_corrections={positives}");
    println!("non_corrections={negatives}");
    println!("correction_recall={:.2}%", recall * 100.0);
    println!("false_rewrite_rate={:.2}%", false_rewrite_rate * 100.0);
    println!("false_rewrites={}", false_rewrites.join(","));
    println!("missed_corrections={}", misses.join(","));
    for (category, score) in categories {
        println!(
            "category={category} total={} candidates={} false_rewrites={} missed_corrections={}",
            score.total, score.candidates, score.false_rewrites, score.missed_corrections
        );
    }
    println!("promotion_threshold=false_rewrites:0,recall:>=90%,cases:>=1000");
    println!("promotion_passed={promotion_passed}");
    println!("product_behavior=bare actually remains literal; implicit correction unshipped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_actually_never_becomes_a_candidate() {
        for text in [
            "actually",
            "Actually.",
            "I actually meant it.",
            "Keep the word actually in this sentence.",
        ] {
            assert!(!proposed_candidate_detector(text));
        }
    }

    #[test]
    fn corpus_covers_every_required_adversarial_category() {
        let cases: Vec<Case> = serde_json::from_str(FIXTURES).unwrap();
        for category in [
            "intentional_actually",
            "discourse_marker",
            "quoted_speech",
            "technical_prose",
            "multiple_antecedents",
            "genuine_correction",
        ] {
            assert!(cases.iter().any(|case| case.category == category));
        }
    }
}
