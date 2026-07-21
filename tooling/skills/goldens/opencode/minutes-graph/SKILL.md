---
name: minutes-graph
description: Policy-safe relationship intelligence across normal meeting history. Use for top contacts, relationship summaries, commitments, losing-touch signals, person profiles, topic research, or questions about people across meetings. Always use Minutes' native CLI projections; never build or read a durable graph cache.
compatibility: opencode
---

# /minutes-graph

Use Minutes' native, process-private relationship projection. It rebuilds from stable live Markdown, excludes restricted or policy-uncertain meetings, and revalidates the corpus before returning an answer. It does not retain a graph database or JSON index.

## Privacy boundary

- Never walk meeting files, parse frontmatter yourself, or read raw transcripts for this skill.
- Never run `graph_build.py` or read `~/.minutes/graph/index.json`; those are retired legacy surfaces.
- Never create a replacement graph cache, spreadsheet, JSON file, or database.
- Never pass `--include-restricted`. Restricted meetings are intentionally absent from this agent-facing skill.
- Treat an unavailable or resource-budget error as a hard stop. Do not fall back to filesystem reads.

## Choose the native command

| User intent | Command |
|---|---|
| Top contacts, relationship scores, losing-touch signals, top topics | `minutes people --json --limit 50` |
| One person's profile and recent history | `minutes person "<name>"` |
| Open or stale commitments | `minutes commitments --json` |
| Commitments for one person | `minutes commitments --person "<name>" --json` |
| Topic or question across normal meetings | `minutes research "<query>"` |
| Topic scoped to one attendee | `minutes research "<query>" --attendee "<name>"` |

Run the narrowest command that answers the question. Parse its stdout as the returned JSON where applicable; do not inspect stderr for meeting content.

## Workflow

1. Restate the scope in one short phrase when ambiguity matters, such as “normal meetings only.”
2. Run one native command from the table.
3. Rank or filter the returned objects in memory for this response only.
4. Cite the meeting titles/dates supplied by the command. Do not open the source paths.
5. If the native command cannot answer a cross-entity/co-occurrence question, say that the safe graph surface does not currently expose that relationship. Offer the nearest supported `people`, `person`, `commitments`, or `research` view; do not synthesize a durable index.

## Output

Prefer a compact table for ranked people or commitments. Explain signals such as `losing_touch`, `meeting_count`, `top_topics`, and `open_commitments` in plain language. Do not imply that omitted restricted meetings do not exist; say the result covers the normal meeting corpus available to the agent.

## Gotchas

- A missing person may be absent, named differently, or present only in restricted history. Report “not available in the normal meeting graph,” not “never met.”
- Relationship and commitment answers are live projections, so a later sensitivity change can intentionally remove prior results.
- Name-merge suggestions are suggestions only. Never rewrite or merge identities without the user's explicit confirmation.
- Company/product deep extraction is not available through this skill because it would require reading and retaining raw corpus content outside the native policy boundary.

