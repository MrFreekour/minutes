//! Derived-view refresh — the single owner of "this artifact changed on disk,
//! so everything computed from it is now stale."
//!
//! Minutes keeps markdown as the canonical store, but several consumers cache
//! views derived from it:
//!
//! | View | Refresh | Idempotent? |
//! |---|---|---|
//! | `graph.db` (people, meetings, topics, commitments) | [`crate::graph::rebuild_index`] | yes — full rebuild from files |
//! | `search.db` (full-text index) | [`crate::search_index::SearchIndex::upsert_file`] | yes — unconditional per-file reindex |
//! | Vault copy (`strategy = "copy"` only) | [`crate::vault::sync_file`] | yes — overwrite |
//! | QMD collection index | `qmd update -c <collection>` | yes — reindex |
//! | Knowledge base (wiki/PARA/Obsidian) | [`crate::knowledge::ingest_file`] | **no** — facts dedup, but the chronological log always appends |
//!
//! The search index earns its place despite syncing lazily on read: that sync
//! (`SyncMode::Auto`) compares only **mtime and size**, and the enum's own
//! docs note that `SyncMode::Force` exists to catch "mtime-collision edge
//! cases". A regenerated summary of identical byte length, written within the
//! filesystem's mtime granularity of the last sync, would therefore stay
//! stale. `upsert_file` re-reads and reindexes unconditionally, which closes
//! that window for the one file we know just changed — without paying for a
//! whole-corpus re-walk.
//!
//! MCP and QMD document reads go straight to the markdown and need nothing.
//!
//! The first four run automatically after any write that changes
//! summary-derived frontmatter. Knowledge ingestion is opt-in
//! ([`RefreshOptions::ingest_knowledge`]) precisely because it is not
//! idempotent: re-ingesting the same meeting writes a second entry into the
//! user's append-only knowledge log even when no fact changed.
//!
//! Every step is **best-effort**. A stale derived view is a nuisance; a failed
//! refresh must never fail an artifact write that already succeeded, so this
//! module has no error return — failures land in [`RefreshReport::warnings`]
//! and the tracing log.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::graph::GraphStats;
use crate::knowledge::UpdateResult;

/// Which derived views a refresh should touch.
#[derive(Debug, Clone, Default)]
pub struct RefreshOptions {
    /// Re-run knowledge-base ingestion for this artifact.
    ///
    /// Off by default. [`crate::knowledge::update_from_meeting`] deduplicates
    /// facts, but always appends a chronological log entry, so an automatic
    /// re-ingest on every rewrite would accumulate duplicate lines in a
    /// user-owned file. Callers expose this as an explicit opt-in
    /// (`minutes resummarize --ingest`).
    pub ingest_knowledge: bool,
}

/// Outcome of a QMD collection reindex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QmdRefresh {
    /// No `search.qmd_collection` configured — the common case.
    NotConfigured,
    /// `qmd update` ran and exited successfully.
    Refreshed,
    /// `qmd` could not be spawned, or exited non-zero. The index is stale.
    Failed(String),
}

/// What a refresh actually managed to do.
///
/// Fields are `None`/`false` both when a view is not configured (no vault, no
/// QMD collection, knowledge disabled) and when its refresh failed — the
/// distinction is in [`RefreshReport::warnings`], which is populated only on
/// failure.
#[derive(Debug, Default)]
pub struct RefreshReport {
    /// Graph rebuild stats, when the rebuild succeeded.
    pub graph: Option<GraphStats>,
    /// Whether the artifact was reindexed into the full-text search index.
    pub search_indexed: bool,
    /// Destination path, when a vault copy was written (`strategy = "copy"`).
    pub vault: Option<PathBuf>,
    /// Whether a QMD collection reindex ran and succeeded.
    pub qmd_refreshed: bool,
    /// Knowledge-base result, when `ingest_knowledge` was requested and the
    /// knowledge base is actually enabled. A disabled knowledge base leaves
    /// this `None` and records a warning — never a zero-fact "success".
    pub knowledge: Option<UpdateResult>,
    /// Human-readable failures. Empty on a fully clean refresh.
    pub warnings: Vec<String>,
}

impl RefreshReport {
    /// Did every attempted step succeed?
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }
}

/// Refresh every derived view that depends on `path`, best-effort.
///
/// Call this after any write that changes an artifact's summary-derived
/// frontmatter (`entities`, `people`, `intents`, `action_items`, `decisions`)
/// or its AI-owned body sections. It never fails: each step records a warning
/// and the rest continue.
///
/// The graph rebuild is whole-corpus, not per-file —
/// [`crate::graph::rebuild_index`] clears the tables and rewalks every
/// markdown file under `config.output_dir`. That is the existing pipeline
/// convention (`jobs.rs`, `watch.rs`), it is what makes the refresh inherently
/// idempotent, and it is accepted here as a deliberate trade-off: a corpus of
/// ~90 meetings rebuilds in well under a second.
pub fn refresh_derived_views(path: &Path, config: &Config, opts: &RefreshOptions) -> RefreshReport {
    refresh_derived_views_at(
        path,
        config,
        opts,
        &crate::graph::db_path(),
        &crate::search_index::SearchIndex::default_db_path(),
    )
}

/// [`refresh_derived_views`] against explicit database paths.
///
/// Crate-private and mirroring [`crate::graph::rebuild_index_at`]: the public
/// entry point always targets the real `~/.minutes/{graph,search}.db`, while
/// tests point at temporary files. Without this seam a unit test that merely
/// sets `config.output_dir` to a `TempDir` would still clear the developer's
/// real indexes and repopulate them from the fixture corpus — both db paths
/// are global, not derived from the config.
pub(crate) fn refresh_derived_views_at(
    path: &Path,
    config: &Config,
    opts: &RefreshOptions,
    graph_db: &Path,
    search_db: &Path,
) -> RefreshReport {
    let mut report = RefreshReport::default();

    match crate::graph::rebuild_index_at(config, graph_db) {
        Ok(stats) => report.graph = Some(stats),
        Err(error) => {
            tracing::warn!(error = %error, artifact = %path.display(), "graph index rebuild failed");
            report
                .warnings
                .push(format!("graph index rebuild: {error}"));
        }
    }

    match crate::search_index::SearchIndex::open_at(search_db, config)
        .and_then(|index| index.upsert_file(path))
    {
        Ok(()) => report.search_indexed = true,
        Err(error) => {
            tracing::warn!(error = %error, artifact = %path.display(), "search index upsert failed");
            report.warnings.push(format!("search index: {error}"));
        }
    }

    match crate::vault::sync_file(path, config) {
        Ok(Some(vault_path)) => {
            crate::events::append_event(crate::events::MinutesEvent::VaultSynced {
                source_path: path.display().to_string(),
                vault_path: vault_path.display().to_string(),
                strategy: config.vault.strategy.clone(),
            });
            report.vault = Some(vault_path);
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(error = %error, artifact = %path.display(), "vault sync failed");
            report.warnings.push(format!("vault sync: {error}"));
        }
    }

    match refresh_qmd_collection(config) {
        QmdRefresh::Refreshed => report.qmd_refreshed = true,
        QmdRefresh::NotConfigured => {}
        QmdRefresh::Failed(reason) => {
            report.warnings.push(format!("qmd reindex: {reason}"));
        }
    }

    if opts.ingest_knowledge {
        refresh_knowledge(path, config, &mut report);
    }

    report
}

/// Re-ingest `path` into the knowledge base, recording the outcome.
///
/// A disabled or unconfigured knowledge base is reported as a warning rather
/// than a zero-fact success: [`crate::knowledge::update_from_meeting`] returns
/// `Ok` with empty counts in that case, which is indistinguishable from a real
/// run that found nothing.
fn refresh_knowledge(path: &Path, config: &Config, report: &mut RefreshReport) {
    if !config.knowledge.enabled || config.knowledge.path.as_os_str().is_empty() {
        report.warnings.push(
            "knowledge ingest requested, but the knowledge base is not enabled \
             (set `knowledge.enabled` and `knowledge.path` in config.toml)"
                .to_string(),
        );
        return;
    }

    match crate::knowledge::ingest_file(path, config) {
        Ok(update) => {
            if update.facts_written > 0 {
                crate::events::append_event(crate::events::MinutesEvent::KnowledgeUpdated {
                    meeting_path: path.display().to_string(),
                    facts_written: update.facts_written,
                    facts_skipped: update.facts_skipped,
                    people_updated: update.people_updated.clone(),
                });
            }
            report.knowledge = Some(update);
        }
        Err(error) => {
            // Two distinct cases arrive here, both correctly non-fatal:
            // the deliberate loud refusal for `sensitivity: restricted`
            // artifacts (a policy exclusion, not a failure), and a genuine
            // partial write — `update_from_meeting` writes person files and
            // always appends its chronological log *before* returning this
            // error, so a retry adds a second log entry.
            tracing::warn!(error = %error, artifact = %path.display(), "knowledge ingest failed");
            report.warnings.push(format!("knowledge ingest: {error}"));
        }
    }
}

/// Ask QMD to reindex the configured collection.
pub fn refresh_qmd_collection(config: &Config) -> QmdRefresh {
    refresh_qmd_collection_with(config, "qmd")
}

/// [`refresh_qmd_collection`] against an explicit program name, so the
/// spawn-failure branch is testable on machines where `qmd` *is* installed.
///
/// Note that a wrong collection name is not detectable here: `qmd update -c
/// <unknown>` exits 0, so only a genuine spawn failure or non-zero exit is
/// reported.
fn refresh_qmd_collection_with(config: &Config, program: &str) -> QmdRefresh {
    let Some(collection) = config.search.qmd_collection.as_ref() else {
        return QmdRefresh::NotConfigured;
    };
    let status = crate::engine_process::command(program)
        .args(["update", "-c", collection])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => QmdRefresh::Refreshed,
        // A non-zero exit means the collection did not reindex. Reporting it
        // as success would let the CLI claim a fresh index over a stale one.
        Ok(status) => QmdRefresh::Failed(format!(
            "`{program} update -c {collection}` exited {status}"
        )),
        Err(error) => {
            tracing::debug!(error = %error, collection = %collection, "qmd update skipped");
            QmdRefresh::Failed(format!(
                "could not run `{program}` for `{collection}`: {error}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A config whose meetings live in `dir`, plus the temp graph and search
    /// db paths that keep the real `~/.minutes/*.db` out of the test.
    fn fixture(dir: &TempDir) -> (Config, PathBuf, PathBuf) {
        let mut config = Config::default();
        config.output_dir = dir.path().to_path_buf();
        (
            config,
            dir.path().join("graph.db"),
            dir.path().join("search.db"),
        )
    }

    fn write_meeting(dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(
            &path,
            "---\ntitle: Test\ntype: meeting\ndate: 2026-07-26\n---\n\n## Summary\n\nhi\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn refresh_options_default_leaves_knowledge_opt_in() {
        // The append-only knowledge log makes ingestion non-idempotent, so it
        // must never be the default.
        assert!(!RefreshOptions::default().ingest_knowledge);
    }

    #[test]
    fn qmd_refresh_is_skipped_when_no_collection_configured() {
        let dir = TempDir::new().unwrap();
        let (config, _, _) = fixture(&dir);
        assert!(config.search.qmd_collection.is_none());
        assert_eq!(refresh_qmd_collection(&config), QmdRefresh::NotConfigured);
    }

    #[test]
    fn qmd_refresh_reports_failure_when_the_binary_cannot_be_spawned() {
        let dir = TempDir::new().unwrap();
        let (mut config, _, _) = fixture(&dir);
        config.search.qmd_collection = Some("meetings".into());

        // Pin the program name rather than relying on `qmd` being absent: it
        // is installed on plenty of dev machines, and `qmd update -c <bogus>`
        // exits 0 there — so a collection-name-based test would assert nothing
        // and would pass for opposite reasons on different machines.
        let result = refresh_qmd_collection_with(&config, "minutes-no-such-binary-qmd");

        match result {
            QmdRefresh::Failed(reason) => {
                assert!(
                    reason.contains("minutes-no-such-binary-qmd"),
                    "failure should name the program, got {reason:?}"
                );
            }
            other => panic!("expected a spawn failure, got {other:?}"),
        }
    }

    #[test]
    fn search_index_upsert_targets_the_given_db_and_never_the_global_one() {
        // Same regression class as the graph db: `SearchIndex::default_db_path`
        // is global, so a test that only redirects `output_dir` would rebuild
        // the developer's real ~/.minutes/search.db.
        let dir = TempDir::new().unwrap();
        let (config, graph_db, search_db) = fixture(&dir);
        let artifact = write_meeting(&dir, "meeting.md");

        let report = refresh_derived_views_at(
            &artifact,
            &config,
            &RefreshOptions::default(),
            &graph_db,
            &search_db,
        );

        assert!(report.search_indexed, "warnings: {:?}", report.warnings);
        assert!(
            search_db.exists(),
            "the injected search db path must be the target"
        );
    }

    #[test]
    fn graph_rebuild_targets_the_given_db_and_never_the_global_one() {
        // Regression guard: `graph::db_path()` is global, so a test that only
        // redirects `output_dir` would wipe the developer's real graph index.
        let dir = TempDir::new().unwrap();
        let (config, graph_db, search_db) = fixture(&dir);
        let artifact = write_meeting(&dir, "meeting.md");

        let report = refresh_derived_views_at(
            &artifact,
            &config,
            &RefreshOptions::default(),
            &graph_db,
            &search_db,
        );

        assert!(report.graph.is_some(), "graph should rebuild");
        assert!(graph_db.exists(), "the injected db path must be the target");
    }

    #[test]
    fn missing_output_dir_warns_instead_of_failing() {
        // Best-effort contract: a graph rebuild that cannot run must not be
        // able to fail a write that already succeeded.
        let dir = TempDir::new().unwrap();
        let (mut config, graph_db, search_db) = fixture(&dir);
        config.output_dir = dir.path().join("does-not-exist");
        let artifact = dir.path().join("meeting.md");

        let report = refresh_derived_views_at(
            &artifact,
            &config,
            &RefreshOptions::default(),
            &graph_db,
            &search_db,
        );

        assert!(report.graph.is_none());
        assert!(!report.is_clean());
        assert!(
            report.warnings.iter().any(|w| w.contains("graph index")),
            "expected a graph warning, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn clean_refresh_reports_no_warnings_and_skips_unconfigured_views() {
        let dir = TempDir::new().unwrap();
        let (config, graph_db, search_db) = fixture(&dir);
        let artifact = write_meeting(&dir, "meeting.md");

        let report = refresh_derived_views_at(
            &artifact,
            &config,
            &RefreshOptions::default(),
            &graph_db,
            &search_db,
        );

        assert!(
            report.is_clean(),
            "unexpected warnings: {:?}",
            report.warnings
        );
        assert!(report.graph.is_some());
        // Vault and QMD are unconfigured by default; knowledge was not opted in.
        assert!(report.vault.is_none());
        assert!(!report.qmd_refreshed);
        assert!(report.knowledge.is_none());
    }

    #[test]
    fn ingest_on_a_disabled_knowledge_base_warns_instead_of_faking_success() {
        // `update_from_meeting` returns Ok with zero counts when knowledge is
        // disabled, which is indistinguishable from "ran, found nothing".
        let dir = TempDir::new().unwrap();
        let (config, graph_db, search_db) = fixture(&dir);
        let artifact = write_meeting(&dir, "meeting.md");
        assert!(!config.knowledge.enabled);

        let opts = RefreshOptions {
            ingest_knowledge: true,
        };
        let report = refresh_derived_views_at(&artifact, &config, &opts, &graph_db, &search_db);

        assert!(
            report.knowledge.is_none(),
            "a disabled knowledge base must not report a zero-fact success"
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("knowledge base is not enabled")),
            "expected a disabled-knowledge warning, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn restricted_artifacts_are_refused_by_knowledge_ingest() {
        let dir = TempDir::new().unwrap();
        let knowledge_dir = TempDir::new().unwrap();
        let (mut config, graph_db, search_db) = fixture(&dir);
        config.knowledge.enabled = true;
        config.knowledge.path = knowledge_dir.path().to_path_buf();

        let artifact = dir.path().join("restricted.md");
        std::fs::write(
            &artifact,
            "---\ntitle: Private\ntype: meeting\ndate: 2026-07-26\nsensitivity: restricted\n---\n\n## Summary\n\nhi\n",
        )
        .unwrap();

        let opts = RefreshOptions {
            ingest_knowledge: true,
        };
        let report = refresh_derived_views_at(&artifact, &config, &opts, &graph_db, &search_db);

        assert!(report.knowledge.is_none());
        assert!(
            report.warnings.iter().any(|w| w.contains("restricted")),
            "expected the restricted refusal to surface, got {:?}",
            report.warnings
        );
    }
}
