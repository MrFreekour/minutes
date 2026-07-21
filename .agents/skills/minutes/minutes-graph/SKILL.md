---
name: minutes-graph
description: Policy-safe live-source person profiles and topic research, with honest fail-closed handling for deferred graph rankings, commitments, aliases, and losing-touch signals. Always use Minutes' bounded native CLI surfaces; never build or read a durable graph cache.
---

# /minutes-graph

The durable relationship projection is temporarily unavailable while the bounded privacy-safe rebuild tracked in [roadmap issue #513](https://github.com/silverstein/minutes/issues/513) is completed. Bounded live-source person profiles and topic research remain available. Do not fall back to the retired index or read meeting files directly.

## Privacy boundary

- Never walk meeting files, parse frontmatter yourself, or read raw transcripts for this skill.
- Never run `graph_build.py` or read `~/.minutes/graph/index.json`; those are retired legacy surfaces.
- Never create a replacement graph cache, spreadsheet, JSON file, or database.
- Never pass `--include-restricted`. Restricted meetings are intentionally absent from this agent-facing skill.
- Treat the temporary-unavailability response or any resource-budget error as a hard stop. Do not fall back to filesystem reads.

## Temporary availability

The durable graph commands are fail-closed during this deferral. Do not run
`minutes people`, `minutes people merge`, or `minutes commitments`; rankings,
aliases, losing-touch signals, and graph commitments remain unavailable until
issue #513 is resolved. The bounded live-source `minutes person` and
`minutes research` commands remain available and do not read the retired index.

## Workflow

1. Classify the request. For a person profile, run `minutes person "<name>"`; for topic research, run `minutes research "<topic>"`.
2. Require exit status 0. Use only the bounded native result and never substitute filesystem reads.
3. For rankings, losing-touch, aliases, or commitments, state that the graph projection is temporarily unavailable and link roadmap issue #513.
4. Do not imply that restricted history or a relationship fact is absent when any command fails.

## Output

Return bounded live-source profiles or topic research when requested. For graph-only
requests, tell the user the graph projection is temporarily unavailable and link
issue #513. Never invent rankings, commitments, or relationship signals from raw files.

## Gotchas

- A failed person profile cannot be interpreted as “never met.” Report the source unavailable.
- Do not imply that a later sensitivity change removed a result; no graph result is being produced during the deferral.
- Alias and merge operations remain unavailable with the graph projection. Never simulate them from names in live results.
- Use `minutes research "<topic>"` for bounded company, product, or topic research. If it fails, report the source unavailable; never fall back to raw corpus reads.

