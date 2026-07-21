//! Cross-meeting relationship intelligence is temporarily unavailable.
//!
//! The former implementation rebuilt a relationship projection from the
//! corpus. It has been deliberately removed from Slice B while the bounded,
//! privacy-safe replacement is developed under bead `minutes-ew09`. Keeping
//! this boundary explicit prevents callers from falling back to the retired
//! durable graph index or reading meeting files directly.

use crate::config::Config;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("cross-meeting graph is temporarily unavailable while its privacy-safe bounded rebuild is completed; see roadmap issue #513")]
    TemporarilyUnavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    pub people_count: usize,
    pub meeting_count: usize,
    pub commitment_count: usize,
    pub topic_count: usize,
    pub alias_suggestions: Vec<AliasSuggestion>,
    pub alias_clusters: Vec<AliasCluster>,
    pub rebuild_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonSummary {
    pub slug: String,
    pub name: String,
    pub meeting_count: i64,
    pub last_seen: String,
    pub days_since: f64,
    pub open_commitments: i64,
    pub top_topics: Vec<String>,
    pub score: f64,
    pub losing_touch: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Commitment {
    pub text: String,
    pub status: String,
    pub due_date: Option<String>,
    pub created_at: String,
    pub commitment_type: String,
    pub meeting_title: String,
    pub meeting_date: String,
    pub person_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AliasSuggestion {
    pub name_a: String,
    pub name_b: String,
    pub shared_meetings: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AliasCluster {
    pub members: Vec<String>,
    pub slugs: Vec<String>,
    pub max_shared_meetings: usize,
}

pub fn parakeet_boost_phrases(_config: &Config, _limit: usize) -> Result<Vec<String>, GraphError> {
    Err(GraphError::TemporarilyUnavailable)
}

pub fn rebuild_index(_config: &Config) -> Result<GraphStats, GraphError> {
    Err(GraphError::TemporarilyUnavailable)
}

pub fn query_person(_config: &Config, _name: &str) -> Result<Option<PersonSummary>, GraphError> {
    Err(GraphError::TemporarilyUnavailable)
}

pub fn query_commitments(
    _config: &Config,
    _person_slug: Option<&str>,
) -> Result<Vec<Commitment>, GraphError> {
    Err(GraphError::TemporarilyUnavailable)
}

pub fn relationship_map(_config: &Config) -> Result<Vec<PersonSummary>, GraphError> {
    Err(GraphError::TemporarilyUnavailable)
}
