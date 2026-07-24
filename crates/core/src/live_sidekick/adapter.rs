//! Production evidence adapters shared by native Sidekick and replay gates.

use super::{EvidenceId, LiveSidekickEngine, ReasoningTranscriptEvidence};
use crate::error::MinutesError;
use crate::live_transcript_contract::{self, TranscriptLine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Bounded receipt for one incremental JSONL-to-engine adapter pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptAdapterReceipt {
    pub cursor: usize,
    pub new_items: usize,
    pub accepted_evidence_ids: Vec<EvidenceId>,
}

/// Read the production live-transcript JSONL path from a cursor and reduce
/// accepted items into the active Sidekick engine.
pub fn observe_transcript_jsonl_from_path(
    engine: &mut LiveSidekickEngine,
    cursor: &mut usize,
    path: &Path,
    evidence_id_prefix: &str,
) -> Result<TranscriptAdapterReceipt, MinutesError> {
    let lines = live_transcript_contract::read_since_line_from_path(path, *cursor)?;
    Ok(observe_transcript_lines(
        engine,
        cursor,
        lines,
        evidence_id_prefix,
    ))
}

/// Parse already-pinned JSONL bytes through the same cursor and reducer path.
///
/// Native acceptance uses this after verifying file ownership, mode, identity,
/// and digest. The autonomous harness uses it with committed synthetic bytes.
pub fn observe_transcript_jsonl_from_bytes(
    engine: &mut LiveSidekickEngine,
    cursor: &mut usize,
    bytes: &[u8],
    evidence_id_prefix: &str,
) -> Result<TranscriptAdapterReceipt, MinutesError> {
    let lines = live_transcript_contract::read_since_line_from_bytes(bytes, *cursor)?;
    Ok(observe_transcript_lines(
        engine,
        cursor,
        lines,
        evidence_id_prefix,
    ))
}

fn observe_transcript_lines(
    engine: &mut LiveSidekickEngine,
    cursor: &mut usize,
    lines: Vec<TranscriptLine>,
    evidence_id_prefix: &str,
) -> TranscriptAdapterReceipt {
    let mut accepted_evidence_ids = Vec::new();
    for line in lines {
        *cursor = (*cursor).max(line.line);
        if line.text.trim().is_empty() {
            continue;
        }
        let evidence_id = EvidenceId::new(format!("{evidence_id_prefix}{}", line.line));
        if engine
            .observe_transcript(ReasoningTranscriptEvidence {
                evidence_id: evidence_id.clone(),
                text: line.text,
                speaker_label: line.speaker,
                speaker_verified: false,
                offset_ms: line.offset_ms,
                duration_ms: line.duration_ms,
            })
            .is_ok_and(|reduction| reduction.accepted)
        {
            accepted_evidence_ids.push(evidence_id);
        }
    }
    TranscriptAdapterReceipt {
        cursor: *cursor,
        new_items: accepted_evidence_ids.len(),
        accepted_evidence_ids,
    }
}
