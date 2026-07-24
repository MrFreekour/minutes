# Consent Enforcement — Restricted Meetings at the Agent Layer

Wave 2 of the consent layer (bead `minutes-3yub.4`). Wave 1 introduced the
designation — meetings can carry `capture: none` and `sensitivity: restricted`
frontmatter (see [frontmatter-schema.md](frontmatter-schema.md)). Wave 2 makes
the designation an enforcement contract: a restricted meeting is **excluded by
default from every agent surface**, with an explicit, logged override where an
override exists at all. The human-readable markdown on disk is never touched —
the operator's own files stay fully readable.

## The contract

| Surface | Default | Override | Override logging |
|---|---|---|---|
| Core text search (`search`, `search_with_mode`) | excluded; indexed hits are re-read and strictly classified from live bytes | `SearchFilters::include_restricted` | caller's responsibility (CLI does it) |
| Core intent search (`search_intents`) | excluded | `SearchFilters::include_restricted` | caller's responsibility (CLI does it) |
| Core open actions (`find_open_actions`) | excluded; malformed, unknown-policy, unreadable, and out-of-root files are dropped | `include_restricted` parameter | caller's responsibility (CLI does it) |
| Core cross-meeting research (`cross_meeting_research`) | excluded | `SearchFilters::include_restricted` | caller's responsibility (CLI does it) |
| Core consistency report (`consistency_report`) | excluded | none this wave | n/a |
| Core person profile (`person_profile`) | excluded | none this wave | n/a |
| Core relationship graph (`graph.rs`) | excluded from every process-private rebuild | none | n/a; every answer is re-attested before return and the projection is discarded |
| Knowledge ingest (`knowledge.rs`, `minutes ingest`) | skipped in batch; explicit ingest of a restricted meeting is refused with a message | none this wave | n/a |
| CLI `search` / `list` / `actions` / `research` | excluded | `--include-restricted` | `sensitivity.override` event appended before results are returned |
| SDK reader (`crates/sdk/src/reader.ts`: list, search, actions, decisions, person profiles, voice memos) | excluded; explicit unknown sensitivity fails closed | `includeRestricted` option | stderr warning naming count + surface |
| SDK reader `getMeeting` by exact path | minimal stub (title, date, `sensitivity: restricted`, note) — never the body | `includeRestricted` option | stderr warning |
| Standalone MCP tools (`list_meetings`, `search_meetings`, `get_meeting`, `research_topic`, `get_person_profile`) | excluded / stub; unknown policy and uncertain files fail closed | on macOS, Linux, and Windows, trusted launch policy `MINUTES_MCP_RESTRICTED_POLICY=logged-override` **and** per-call `include_restricted: true` | Rust capability-bound, cross-process serialized durable JSONL append to `$MINUTES_HOME/audit/sensitivity-overrides.jsonl`; Windows uses a protected owner-only DACL and reparse-safe handles; audit failure denies the request |
| MCP meeting resources (recent, open actions, exact slug, recent ideas) | excluded / unavailable | no resource-level override | n/a |
| Native Recall chat | exact live normal-sensitivity context only; no MCP servers and no tools | none | n/a |
| Standalone MCP person profile (`get_person_profile`) | recomputed from policy-authorized live meeting snapshots; restricted excluded by default | same trusted-launch plus per-call override as other standalone MCP meeting tools | same durable MCP override audit |
| Standalone MCP commitments (`track_commitments`) | recomputed from normal live meeting snapshots; restricted always excluded | none | n/a |
| Standalone MCP `relationship_map` | bounded process-private core graph; restricted excluded during every rebuild | none | n/a |

Desktop app search, list, palette actions, and other in-app navigation are the
**operator's own surface**, not an agent surface: restricted meetings stay
visible to the human in their own app. Assistant-facing context builders in
the desktop app are separate trust surfaces.

Native Recall uses a desktop-owned `AgentSafeContext` boundary. It
canonicalizes each path, parses exact live frontmatter, rebuilds titles and
snippets from those bytes, binds history to the contributing snapshots, and
revalidates them immediately before process or network egress. A selected
focused meeting that is restricted, malformed, unknown-policy, unreadable, or
outside the configured root blocks locally. Ranked search candidates in any
of those states disappear. Claude runs with an empty strict MCP configuration
and `--tools ""`; Ollama is accepted only at a parsed loopback URL. Other agent
CLIs are denied until they have a verified no-tools/no-global-context mode.

PTY workspace context, proactive bundles, and automation summaries must
enforce the same default at their own final read point; Native Recall's gate
does not make those surfaces safe by implication.

## Override logging

The override is never silent. When `--include-restricted` is passed to a CLI
read command, the CLI appends a `sensitivity.override` event to the
append-only event log (`~/.minutes/events.jsonl`) **before returning
results**:

```json
{"v":1,"seq":42,"timestamp":"...","event_type":"sensitivity.override","surface":"cli.search","query":"pricing"}
```

- `surface` — the read surface that honored the override (`cli.search`,
  `cli.actions`, `cli.research`; `cli.list` routes through `cli.search` with
  an empty query).
- `query` — the query or filter context supplied by the caller, omitted when
  there is none.

CLI event append remains best-effort for a human-invoked command. MCP uses a
different, stricter authority boundary: request arguments are model-controlled
and cannot establish human consent. Only an explicit parent-process launch
grant (`logged-override`) plus the per-call flag authorizes a standalone MCP
read. The central registration wrappers enforce that contract before any tool
handler runs. Every authorized request must append a durable JSONL audit
record; failure to append denies the request. Missing, misspelled, or future
policy values resolve to `deny`. Native Recall always launches in deny mode and
currently registers no MCP server at all.

The standalone override delegates to the Rust capability layer on every
supported platform. That layer binds the complete owner-private audit
namespace, serializes writers with one retained lease, rejects incomplete
JSONL tails, appends and synchronizes through the exact leaf, then re-reads the
new record and re-attests its visible identity before content is returned. On
Windows the same boundary uses protected owner-only DACLs, reparse-safe opens,
and a non-delete-sharing leaf handle rather than emulating POSIX mode fields.

## Restricted stub (get-by-path)

Knowing a restricted meeting's path is not a bypass. `getMeeting` in the SDK
reader and the MCP `get_meeting` tool return a minimal stub without the
override:

- title, date, `sensitivity: restricted`
- a note that content is excluded by default and the `include_restricted`
  parameter is required
- never the transcript body, action items, decisions, or attendees

The SDK stub is marked with `restricted_stub: true` so callers can tell it
apart from a full meeting. MCP verifies the exact live snapshot again after
optional CLI overlay enrichment; a sensitivity or byte change during the read
fails rather than returning the second snapshot.

QMD and SQLite are ranking hints, not authorization or content sources. MCP
and core search canonicalize each candidate, re-read it inside the configured
root, strictly parse sensitivity, and derive the returned excerpt from the
verified live body. Cached snippets are never returned. If QMD candidates are
all rejected, MCP falls back to the safe CLI path.

## No override surfaces (this wave)

The core relationship graph, knowledge ingest, and core consistency/person-profile
commands have **no override**. Standalone MCP `get_person_profile` and
`track_commitments` are deliberately different because they are live
projections, not durable graph reads. The person-profile tool uses the same
trusted-launch plus per-call override and durable audit boundary as other MCP
meeting tools; commitments remain normal-only and expose no restricted
override. An explicitly named restricted meeting passed to `minutes ingest`
is refused; batch ingest skips restricted meetings and reports the count.
Core graph rows exist only in a bounded process-private projection, are bound to
the current corpus and correction revisions, and are discarded after the answer.
The signed macOS desktop bundle additionally runs that projection in a
dedicated App-Sandboxed helper whose exact CodeDirectory hash is sealed into
the enclosing app. On older macOS, standalone, source-built, and ad-hoc
channels supervise an exact second instance of the already-running executable;
the parent verifies the live child's kernel CodeDirectory hash before sending
any source bytes, and the child installs the same hard resource ceilings.
Derived annotation, insight, and live-event tools remain
absent from Native Recall until their records have live sensitivity provenance
or mandatory invalidation.

## Compatibility notes

- All changes are additive: `sensitivity` absent means normal behavior
  everywhere, and existing corpora are unaffected.
- Agents never write `sensitivity` (RFC #194 discipline) — the designation is
  set by the human-initiated flows from Wave 1.
- MCP declares and locks `minutes-sdk` 0.21.x, but does not trust the installed
  package as the final policy authority. Candidate paths from the SDK are
  canonicalized and re-read through MCP's local strict classifier before any
  fallback tool or resource derives output. This guards against a stale npm
  artifact silently treating a future sensitivity value as normal.
- Missing `sensitivity` remains legacy-normal. An explicit unsupported value,
  malformed YAML, missing required frontmatter, unreadable file, or path escape
  is policy-uncertain and excluded even when a restricted override is
  authorized.
