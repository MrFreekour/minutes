//! Conservative, explicit voice editing for finalized dictation utterances.
//!
//! Commands are recognized only when an entire utterance matches the grammar.
//! Ordinary prose is never searched for command-like substrings. This lets a
//! user say phrases such as "the words scratch that" without losing content.

use crate::dictation_context::DictationTextMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationCommandProvenance {
    pub kind: String,
    pub spoken_phrase: String,
}

#[derive(Debug, Clone, Copy)]
pub struct DictationCommandUtterance<'a> {
    pub raw: &'a str,
    pub cleaned: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictationCommandOutput {
    pub text: String,
    pub pre_command_text: Option<String>,
    pub commands_applied: Vec<DictationCommandProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedCommand {
    ScratchThat,
    NewLine,
    NewParagraph,
    Bullet,
    InsertSnippet(String),
    SpellLastWord(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Chunk {
    separator: &'static str,
    text: String,
}

/// Apply explicit commands to already-cleaned utterances.
///
/// Formatting commands consume exactly the following ordinary utterance. A
/// trailing formatting command, a command in an incompatible mode, an unknown
/// snippet, or a correction without a target stays literal.
pub fn apply_explicit_voice_commands(
    utterances: &[DictationCommandUtterance<'_>],
    mode: DictationTextMode,
    enabled: bool,
    snippets: &BTreeMap<String, String>,
) -> DictationCommandOutput {
    let pre_command_text = utterances
        .iter()
        .map(|utterance| utterance.cleaned.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !enabled {
        return DictationCommandOutput {
            text: pre_command_text,
            pre_command_text: None,
            commands_applied: Vec::new(),
        };
    }

    let mut chunks = Vec::<Chunk>::new();
    let mut applied = Vec::<DictationCommandProvenance>::new();
    let mut index = 0;
    while index < utterances.len() {
        let utterance = utterances[index];
        let parsed = parse_command(utterance.raw);
        match parsed {
            Some(ParsedCommand::ScratchThat)
                if !chunks.is_empty()
                    && (chunks.len() > 1
                        || has_future_literal_or_configured_snippet(
                            utterances,
                            index + 1,
                            snippets,
                        )) =>
            {
                chunks.pop();
                applied.push(provenance("scratch_that", utterance.raw));
                index += 1;
            }
            Some(ParsedCommand::SpellLastWord(replacement)) if !chunks.is_empty() => {
                let last = chunks.last_mut().expect("checked non-empty chunks");
                if replace_last_word(&mut last.text, &replacement) {
                    applied.push(provenance("confirmed_spelling", utterance.raw));
                    index += 1;
                } else {
                    push_literal(&mut chunks, utterance.cleaned);
                    index += 1;
                }
            }
            Some(ParsedCommand::InsertSnippet(name)) => {
                if let Some(snippet) = find_snippet(snippets, &name) {
                    push_chunk(&mut chunks, " ", snippet.trim().to_string());
                    applied.push(provenance("snippet", utterance.raw));
                } else {
                    push_literal(&mut chunks, utterance.cleaned);
                }
                index += 1;
            }
            Some(
                command @ (ParsedCommand::NewLine
                | ParsedCommand::NewParagraph
                | ParsedCommand::Bullet),
            ) if formatting_commands_allowed(mode)
                && index + 1 < utterances.len()
                && parse_command(utterances[index + 1].raw).is_none() =>
            {
                let next = utterances[index + 1];
                let (kind, separator, prefix) = match command {
                    ParsedCommand::NewLine => ("new_line", "\n", ""),
                    ParsedCommand::NewParagraph => ("new_paragraph", "\n\n", ""),
                    ParsedCommand::Bullet => ("bullet", "\n", "- "),
                    _ => unreachable!(),
                };
                push_chunk(
                    &mut chunks,
                    separator,
                    format!("{prefix}{}", next.cleaned.trim()),
                );
                applied.push(provenance(kind, utterance.raw));
                index += 2;
            }
            _ => {
                push_literal(&mut chunks, utterance.cleaned);
                index += 1;
            }
        }
    }

    let text = render_chunks(&chunks);
    DictationCommandOutput {
        text,
        pre_command_text: (!applied.is_empty()).then_some(pre_command_text),
        commands_applied: applied,
    }
}

fn parse_command(raw: &str) -> Option<ParsedCommand> {
    let phrase = normalized_phrase(raw);
    match phrase.as_str() {
        "scratch that" => return Some(ParsedCommand::ScratchThat),
        "new line" => return Some(ParsedCommand::NewLine),
        "new paragraph" => return Some(ParsedCommand::NewParagraph),
        "bullet" => return Some(ParsedCommand::Bullet),
        _ => {}
    }

    if let Some(name) = phrase.strip_prefix("insert snippet ") {
        if !name.trim().is_empty() {
            return Some(ParsedCommand::InsertSnippet(name.trim().to_string()));
        }
    }
    parse_confirmed_spelling(&phrase).map(ParsedCommand::SpellLastWord)
}

fn parse_confirmed_spelling(phrase: &str) -> Option<String> {
    let body = phrase
        .strip_prefix("spell last word as ")?
        .strip_suffix(" confirm")?
        .trim();
    let (case, letters) = if let Some(value) = body.strip_prefix("all caps ") {
        ("upper", value)
    } else if let Some(value) = body.strip_prefix("capital ") {
        ("capital", value)
    } else if let Some(value) = body.strip_prefix("lowercase ") {
        ("lower", value)
    } else {
        ("lower", body)
    };
    let letters = letters
        .split_whitespace()
        .map(|token| token.trim_matches(|character: char| !character.is_ascii_alphanumeric()))
        .collect::<Vec<_>>();
    if !(2..=32).contains(&letters.len())
        || letters.iter().any(|token| {
            token.chars().count() != 1 || !token.chars().all(|c| c.is_ascii_alphabetic())
        })
    {
        return None;
    }
    let mut value = letters.concat().to_ascii_lowercase();
    if case == "upper" {
        value.make_ascii_uppercase();
    } else if case == "capital" {
        let mut characters = value.chars();
        let first = characters.next()?.to_ascii_uppercase();
        value = format!("{first}{}", characters.as_str());
    }
    Some(value)
}

fn normalized_phrase(raw: &str) -> String {
    raw.trim()
        .trim_end_matches(['.', ',', '!', '?'])
        .trim()
        .to_ascii_lowercase()
}

fn formatting_commands_allowed(mode: DictationTextMode) -> bool {
    matches!(
        mode,
        DictationTextMode::AgentPrompt | DictationTextMode::Chat | DictationTextMode::EmailDocument
    )
}

fn find_snippet<'a>(snippets: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    snippets
        .iter()
        .find(|(candidate, value)| {
            candidate.trim().eq_ignore_ascii_case(name) && !value.trim().is_empty()
        })
        .map(|(_, value)| value.as_str())
}

fn has_future_literal_or_configured_snippet(
    utterances: &[DictationCommandUtterance<'_>],
    start: usize,
    snippets: &BTreeMap<String, String>,
) -> bool {
    utterances[start..]
        .iter()
        .any(|utterance| match parse_command(utterance.raw) {
            None => !utterance.cleaned.trim().is_empty(),
            Some(ParsedCommand::InsertSnippet(name)) => find_snippet(snippets, &name).is_some(),
            _ => false,
        })
}

fn replace_last_word(text: &mut String, replacement: &str) -> bool {
    let Some(end) = text
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_alphanumeric() || *character == '_')
        .map(|(index, character)| index + character.len_utf8())
    else {
        return false;
    };
    let start = text[..end]
        .char_indices()
        .rev()
        .find(|(_, character)| !(character.is_alphanumeric() || *character == '_'))
        .map_or(0, |(index, character)| index + character.len_utf8());
    text.replace_range(start..end, replacement);
    true
}

fn provenance(kind: &str, raw: &str) -> DictationCommandProvenance {
    DictationCommandProvenance {
        kind: kind.to_string(),
        spoken_phrase: raw.trim().to_string(),
    }
}

fn push_literal(chunks: &mut Vec<Chunk>, text: &str) {
    push_chunk(chunks, " ", text.trim().to_string());
}

fn push_chunk(chunks: &mut Vec<Chunk>, separator: &'static str, text: String) {
    if text.is_empty() {
        return;
    }
    chunks.push(Chunk {
        separator: if chunks.is_empty() { "" } else { separator },
        text,
    });
}

fn render_chunks(chunks: &[Chunk]) -> String {
    let mut output = String::new();
    for chunk in chunks {
        output.push_str(chunk.separator);
        output.push_str(&chunk.text);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utterance<'a>(raw: &'a str, cleaned: &'a str) -> DictationCommandUtterance<'a> {
        DictationCommandUtterance { raw, cleaned }
    }

    #[test]
    fn scratch_that_removes_one_complete_prior_utterance_and_is_reversible() {
        let result = apply_explicit_voice_commands(
            &[
                utterance("keep this", "Keep this"),
                utterance("remove this", "Remove this"),
                utterance("scratch that", "Scratch that"),
                utterance("continue", "Continue"),
            ],
            DictationTextMode::Chat,
            true,
            &BTreeMap::new(),
        );
        assert_eq!(result.text, "Keep this Continue");
        assert_eq!(
            result.pre_command_text.as_deref(),
            Some("Keep this Remove this Scratch that Continue")
        );
        assert_eq!(result.commands_applied[0].kind, "scratch_that");
    }

    #[test]
    fn formatting_commands_require_a_compatible_mode_and_next_utterance() {
        let input = [
            utterance("first", "First"),
            utterance("new paragraph", "New paragraph"),
            utterance("second", "Second"),
            utterance("bullet", "Bullet"),
            utterance("third", "Third"),
        ];
        let result = apply_explicit_voice_commands(
            &input,
            DictationTextMode::EmailDocument,
            true,
            &BTreeMap::new(),
        );
        assert_eq!(result.text, "First\n\nSecond\n- Third");

        for mode in [DictationTextMode::TerminalCode, DictationTextMode::Unknown] {
            let literal = apply_explicit_voice_commands(&input, mode, true, &BTreeMap::new());
            assert_eq!(literal.text, "First New paragraph Second Bullet Third");
            assert!(literal.commands_applied.is_empty());
        }
    }

    #[test]
    fn command_like_prose_and_bare_actually_stay_literal() {
        let result = apply_explicit_voice_commands(
            &[
                utterance(
                    "the words scratch that are in the quote",
                    "The words scratch that are in the quote",
                ),
                utterance("actually", "Actually"),
                utterance("new paragraph please", "New paragraph please"),
            ],
            DictationTextMode::EmailDocument,
            true,
            &BTreeMap::new(),
        );
        assert_eq!(
            result.text,
            "The words scratch that are in the quote Actually New paragraph please"
        );
        assert!(result.commands_applied.is_empty());
    }

    #[test]
    fn disabled_commands_are_entirely_literal() {
        let result = apply_explicit_voice_commands(
            &[
                utterance("one", "One"),
                utterance("scratch that", "Scratch that"),
            ],
            DictationTextMode::Chat,
            false,
            &BTreeMap::new(),
        );
        assert_eq!(result.text, "One Scratch that");
        assert!(result.pre_command_text.is_none());
    }

    #[test]
    fn snippets_must_be_explicitly_configured_and_exactly_named() {
        let mut snippets = BTreeMap::new();
        snippets.insert("sign off".into(), "Best,\nMat".into());
        let applied = apply_explicit_voice_commands(
            &[utterance(
                "insert snippet sign off",
                "Insert snippet sign off",
            )],
            DictationTextMode::EmailDocument,
            true,
            &snippets,
        );
        assert_eq!(applied.text, "Best,\nMat");
        assert_eq!(applied.commands_applied[0].kind, "snippet");

        let unknown = apply_explicit_voice_commands(
            &[utterance("insert snippet secret", "Insert snippet secret")],
            DictationTextMode::EmailDocument,
            true,
            &snippets,
        );
        assert_eq!(unknown.text, "Insert snippet secret");
        assert!(unknown.commands_applied.is_empty());
    }

    #[test]
    fn confirmed_spelling_requires_full_grammar_and_preserves_punctuation() {
        let result = apply_explicit_voice_commands(
            &[
                utterance("send to minuts.", "Send to minuts."),
                utterance(
                    "spell last word as capital m i n u t e s confirm",
                    "Spell last word as capital m i n u t e s confirm",
                ),
            ],
            DictationTextMode::Chat,
            true,
            &BTreeMap::new(),
        );
        assert_eq!(result.text, "Send to Minutes.");
        assert_eq!(result.commands_applied[0].kind, "confirmed_spelling");

        let ambiguous = apply_explicit_voice_commands(
            &[utterance(
                "spell this m i n u t e s",
                "Spell this m i n u t e s",
            )],
            DictationTextMode::Chat,
            true,
            &BTreeMap::new(),
        );
        assert_eq!(ambiguous.text, "Spell this m i n u t e s");
        assert!(ambiguous.commands_applied.is_empty());
    }

    #[test]
    fn scratch_without_a_target_stays_literal() {
        let result = apply_explicit_voice_commands(
            &[utterance("scratch that", "Scratch that")],
            DictationTextMode::Chat,
            true,
            &BTreeMap::new(),
        );
        assert_eq!(result.text, "Scratch that");
        assert!(result.commands_applied.is_empty());
    }

    #[test]
    fn scratch_cannot_erase_the_only_recoverable_output() {
        let result = apply_explicit_voice_commands(
            &[
                utterance("only thought", "Only thought"),
                utterance("scratch that", "Scratch that"),
            ],
            DictationTextMode::Chat,
            true,
            &BTreeMap::new(),
        );
        assert_eq!(result.text, "Only thought Scratch that");
        assert!(result.commands_applied.is_empty());
    }
}
