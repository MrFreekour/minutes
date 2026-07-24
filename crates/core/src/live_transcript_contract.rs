//! Stable live-transcript JSONL schema and cursor parser.
//!
//! Audio production remains feature-gated. Consumers that only need to parse
//! already-produced transcript evidence should not have to link Whisper.

use crate::error::MinutesError;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A finalized line in the live transcript JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLine {
    /// Sequential line number (1-based).
    pub line: usize,
    /// Wall clock timestamp (ISO 8601).
    pub ts: DateTime<Local>,
    /// Milliseconds since session start.
    pub offset_ms: u64,
    /// Utterance duration in milliseconds.
    pub duration_ms: u64,
    /// Transcribed text.
    pub text: String,
    /// Unverified speaker track label when diarization supplied one.
    pub speaker: Option<String>,
}

/// Read finalized transcript lines after `since_line` from one JSONL path.
pub fn read_since_line_from_path(
    path: &Path,
    since_line: usize,
) -> Result<Vec<TranscriptLine>, MinutesError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)?;
    read_since_line_from_reader(BufReader::new(file), since_line)
}

/// Parse finalized transcript lines after `since_line` from pinned bytes.
pub fn read_since_line_from_bytes(
    bytes: &[u8],
    since_line: usize,
) -> Result<Vec<TranscriptLine>, MinutesError> {
    read_since_line_from_reader(BufReader::new(bytes), since_line)
}

fn read_since_line_from_reader(
    reader: impl BufRead,
    since_line: usize,
) -> Result<Vec<TranscriptLine>, MinutesError> {
    let mut lines = Vec::new();
    for line_result in reader.lines() {
        let line_str = match line_result {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("skipping unreadable JSONL line: {}", error);
                continue;
            }
        };
        if line_str.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TranscriptLine>(&line_str) {
            Ok(line) if line.line > since_line => lines.push(line),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("skipping malformed JSONL line: {}", error);
            }
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_is_cursor_bound_and_skips_torn_lines() {
        let bytes = concat!(
            "{\"line\":1,\"ts\":\"2026-07-24T12:00:00+00:00\",\"offset_ms\":0,\"duration_ms\":100,\"text\":\"first\",\"speaker\":null}\n",
            "not-json\n",
            "{\"line\":2,\"ts\":\"2026-07-24T12:00:01+00:00\",\"offset_ms\":100,\"duration_ms\":100,\"text\":\"second\",\"speaker\":\"TRACK_B\"}\n",
        );
        let lines = read_since_line_from_bytes(bytes.as_bytes(), 1).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line, 2);
        assert_eq!(lines[0].speaker.as_deref(), Some("TRACK_B"));
    }
}
