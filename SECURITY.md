# Security Policy

Minutes captures, transcribes, and stores private conversations. Meetings,
calls, voice memos, and the things people said believing no one else would hear
them. A vulnerability here is not a degraded service, it is potential disclosure
of that material. Reports are taken seriously and are welcome.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Report privately through GitHub:

1. Go to the [Security tab](https://github.com/silverstein/minutes/security)
2. Click **Report a vulnerability**

This opens a private advisory visible only to you and the maintainer.

If GitHub private reporting is not available to you, contact
[@silverstein](https://github.com/silverstein) through GitHub and ask for a
private channel. Do not include vulnerability details in a public message.

### What to include

- What the issue is and roughly how severe you believe it is
- Steps to reproduce, or a proof of concept
- Affected version, platform, and configuration
- Anything about your setup that seems relevant (transcription engine,
  diarization on or off, MCP client, desktop app versus CLI)

### What to expect

Minutes is maintained by one person, so these are honest targets rather than a
commercial SLA:

| Stage | Target |
| --- | --- |
| First acknowledgement | within 5 days |
| Initial assessment | within 14 days |
| Fix or mitigation plan for confirmed high-severity issues | within 30 days |

You will get a real answer either way, including if the report is assessed as
out of scope or as accepted risk. Credit is given in the advisory unless you
prefer otherwise.

Please give a reasonable window to ship a fix before public disclosure.

## Supported versions

Minutes ships frequently and support tracks the latest minor release. Fixes land
on `main` and go out in the next release rather than being backported.

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Anything older | No, please upgrade |

## Scope

### In scope

- The Rust engine and CLI (`crates/core`, `crates/cli`, `crates/reader`)
- `crates/whisper-guard`
- The MCP server and TypeScript SDK (`crates/mcp`, `crates/sdk`)
- The Tauri desktop application (`tauri/`)
- Published artifacts: the `minutes-sdk` and `minutes-mcp` npm packages, the
  `minutes-core`, `minutes-cli`, and `whisper-guard` crates, the Homebrew cask,
  the MCPB bundle, and the Cowork extension

Issues of particular interest, given what this software handles:

- Anything causing conversation content, audio, or transcripts to leave the
  machine without the user configuring it
- Bypasses of file permission handling on meeting output (written `0600`)
- Bypasses of consent or sensitivity controls
- Paths that expose restricted conversation content to an agent, MCP client, or
  model provider that should not receive it
- Memory-safety or parsing issues reachable from untrusted audio, including
  files placed in a watched folder
- Privilege or sandbox escapes in the desktop app
- Supply-chain weaknesses in how releases are built, signed, or published

### Out of scope

- Vulnerabilities in upstream dependencies with no Minutes-specific
  exploitation path. Report those upstream; tell us if Minutes amplifies them
- Anything requiring an attacker who already has local user-level access to the
  machine, since a local-first tool cannot defend against a compromised host
- Content that a user deliberately configured a cloud summarizer to receive.
  That path is opt-in and documented
- Model quality problems, including transcription errors and hallucinated text,
  unless they cross a trust boundary. Transcript fabrication is handled by
  `whisper-guard`; open a normal issue
- Missing hardening with no demonstrated impact, and automated scanner output
  without analysis

## Design notes relevant to security

- Audio never leaves the device. Transcription, diarization, and speaker
  matching run locally, and there is no cloud backend to breach
- No account and no server-side profile of conversations
- Meeting output is written with `0600` owner-only permissions
- HTTP uses `ureq` rather than shelling out, so credentials never appear in
  process arguments
- PID file lifecycle uses `flock` for atomic check-and-write, closing a TOCTOU
  race on recording state
- Automated summarization is off by default and needs no API key

Architecture and privacy detail: <https://useminutes.app/security.md>

## Contributor note

Please do not submit real meeting recordings, transcripts, or personal audio in
issues, pull requests, or test fixtures. Use synthetic samples. If you need to
share a reproducer containing sensitive audio, ask for a private channel first.
