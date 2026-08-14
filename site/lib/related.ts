/** Curated cross-links between resource pages.
 *
 * The resource pages were written independently and never linked to each other.
 * Six of the eighteen had no inbound link from anywhere on the site, so a reader
 * who landed on one had no route to the rest and no way to discover that the
 * neighbouring question was already answered.
 *
 * Relations are hand-written rather than derived from tags. A generated
 * "related" list produces plausible-looking but useless pairings, and the note
 * on each entry only means something if a person wrote it about that specific
 * pair. Three or four entries per page is deliberate: a longer list is a link
 * dump, and readers ignore it.
 */

export type RelatedLink = {
  href: string;
  label: string;
  /** Why a reader on this page would want that one. Shown next to the link. */
  note: string;
};

const R = (slug: string, label: string, note: string): RelatedLink => ({
  href: `/resources/${slug}`,
  label,
  note,
});

// Titles kept short here; they are link text, not the pages' own <title>.
const BOTS_HUB = "Remove AI notetaker bots from meetings";
const OTTER_ZOOM = "Remove Otter AI from Zoom";
const FIREFLIES_JOIN = "Stop Fireflies from joining your meetings";
const BUILTIN = "Turn off built-in AI notetakers in Zoom and Teams";
const LEGAL_RECORD = "Is it legal to record a meeting?";
const HIPAA_HUB = "HIPAA-compliant AI note takers";
const HIPAA_TRANSCRIPTION = "HIPAA-compliant transcription";
const OTTER_HIPAA = "Is Otter.ai HIPAA compliant?";
const FIREFLIES_HIPAA = "Is Fireflies.ai HIPAA compliant?";
const GRANOLA_HIPAA = "Is Granola HIPAA compliant?";
const LEGAL_SOFTWARE = "Legal transcription software";
const PRIVILEGE = "AI notetakers and attorney-client privilege";
const LOCAL_STT = "The best local speech-to-text apps";
const DICTATION = "Local dictation on macOS";
const OSS_GRANOLA = "Open-source alternatives to Granola AI";
const MCP_TOOLS = "Best MCP meeting memory tools";
const AGENT_TOOLS = "Best meeting tools for Claude Code and Codex";
const TEMPLATE = "Meeting minutes templates";

export const RELATED: Record<string, RelatedLink[]> = {
  // Removing notetaker bots.
  "remove-ai-notetaker-bots-from-meetings": [
    R("remove-otter-ai-from-zoom", OTTER_ZOOM, "The step-by-step version for the most common case."),
    R("stop-fireflies-from-joining-meetings", FIREFLIES_JOIN, "Fireflies auto-joins from your calendar, so the fix is different."),
    R("turn-off-built-in-ai-notetakers", BUILTIN, "Zoom and Teams run their own notetakers, which no bot-removal step touches."),
    R("is-it-legal-to-record-a-meeting", LEGAL_RECORD, "If a bot recorded you without consent, whether that was lawful depends on your state."),
  ],
  "remove-otter-ai-from-zoom": [
    R("remove-ai-notetaker-bots-from-meetings", BOTS_HUB, "The same problem across every vendor and platform."),
    R("stop-fireflies-from-joining-meetings", FIREFLIES_JOIN, "If more than one notetaker is joining, this is usually the other one."),
    R("turn-off-built-in-ai-notetakers", BUILTIN, "Removing Otter does not stop Zoom's own AI Companion."),
    R("is-otter-ai-hipaa-compliant", OTTER_HIPAA, "What Otter does with the recordings it already has."),
  ],
  "stop-fireflies-from-joining-meetings": [
    R("remove-ai-notetaker-bots-from-meetings", BOTS_HUB, "The same problem across every vendor and platform."),
    R("remove-otter-ai-from-zoom", OTTER_ZOOM, "The Otter equivalent, which works differently."),
    R("turn-off-built-in-ai-notetakers", BUILTIN, "Platform-native notetakers survive removing every third-party bot."),
    R("is-fireflies-ai-hipaa-compliant", FIREFLIES_HIPAA, "What Fireflies does with the transcripts it already holds."),
  ],
  "turn-off-built-in-ai-notetakers": [
    R("remove-ai-notetaker-bots-from-meetings", BOTS_HUB, "Third-party bots are a separate problem with separate controls."),
    R("remove-otter-ai-from-zoom", OTTER_ZOOM, "The most common third-party bot on Zoom."),
    R("stop-fireflies-from-joining-meetings", FIREFLIES_JOIN, "Calendar-driven auto-join, which platform settings do not cover."),
    R("is-it-legal-to-record-a-meeting", LEGAL_RECORD, "Consent rules apply to the platform's own recorder too."),
  ],

  // Compliance.
  "hipaa-compliant-ai-note-taker": [
    R("hipaa-compliant-transcription", HIPAA_TRANSCRIPTION, "What the rule requires of the transcription step itself."),
    R("is-otter-ai-hipaa-compliant", OTTER_HIPAA, "The per-vendor answer, with the tier and conditions."),
    R("is-fireflies-ai-hipaa-compliant", FIREFLIES_HIPAA, "The per-vendor answer, with the tier and conditions."),
    R("is-granola-hipaa-compliant", GRANOLA_HIPAA, "A vendor whose published answer is no."),
  ],
  "hipaa-compliant-transcription": [
    R("hipaa-compliant-ai-note-taker", HIPAA_HUB, "The same question asked about note-taking products."),
    R("is-otter-ai-hipaa-compliant", OTTER_HIPAA, "A worked example of the tier-and-BAA pattern."),
    R("legal-transcription-software", LEGAL_SOFTWARE, "The parallel duty when the record is privileged rather than clinical."),
    R("best-local-speech-to-text", LOCAL_STT, "On-device options, where no disclosure happens at all."),
  ],
  "is-otter-ai-hipaa-compliant": [
    R("hipaa-compliant-ai-note-taker", HIPAA_HUB, "How the vendors compare on the same question."),
    R("is-fireflies-ai-hipaa-compliant", FIREFLIES_HIPAA, "The same question about the other common choice."),
    R("hipaa-compliant-transcription", HIPAA_TRANSCRIPTION, "What the rule requires of transcription generally."),
    R("remove-otter-ai-from-zoom", OTTER_ZOOM, "If the answer sent you looking for the off switch."),
  ],
  "is-fireflies-ai-hipaa-compliant": [
    R("hipaa-compliant-ai-note-taker", HIPAA_HUB, "How the vendors compare on the same question."),
    R("is-otter-ai-hipaa-compliant", OTTER_HIPAA, "The same question about the other common choice."),
    R("hipaa-compliant-transcription", HIPAA_TRANSCRIPTION, "What the rule requires of transcription generally."),
    R("stop-fireflies-from-joining-meetings", FIREFLIES_JOIN, "If the answer sent you looking for the off switch."),
  ],
  "is-granola-hipaa-compliant": [
    R("hipaa-compliant-ai-note-taker", HIPAA_HUB, "How the vendors compare on the same question."),
    R("open-source-alternatives-to-granola-ai", OSS_GRANOLA, "Where people go when the answer rules Granola out."),
    R("is-otter-ai-hipaa-compliant", OTTER_HIPAA, "A vendor whose published answer is yes, with conditions."),
    R("hipaa-compliant-transcription", HIPAA_TRANSCRIPTION, "What the rule requires of transcription generally."),
  ],
  "legal-transcription-software": [
    R("ai-notetakers-attorney-client-privilege", PRIVILEGE, "What disclosure to a vendor does to privilege."),
    R("is-it-legal-to-record-a-meeting", LEGAL_RECORD, "Consent law by state, before the transcript exists."),
    R("hipaa-compliant-transcription", HIPAA_TRANSCRIPTION, "The clinical parallel to the same confidentiality problem."),
    R("best-local-speech-to-text", LOCAL_STT, "On-device options, where nothing is disclosed to a third party."),
  ],
  "ai-notetakers-attorney-client-privilege": [
    R("legal-transcription-software", LEGAL_SOFTWARE, "What to look for once you have accepted the duty."),
    R("is-it-legal-to-record-a-meeting", LEGAL_RECORD, "Consent law by state."),
    R("remove-ai-notetaker-bots-from-meetings", BOTS_HUB, "Removing the bot that created the exposure."),
    R("best-local-speech-to-text", LOCAL_STT, "On-device options, where there is no third party to disclose to."),
  ],
  "is-it-legal-to-record-a-meeting": [
    R("remove-ai-notetaker-bots-from-meetings", BOTS_HUB, "Removing a notetaker that joined without your consent."),
    R("ai-notetakers-attorney-client-privilege", PRIVILEGE, "The higher duty when the conversation is privileged."),
    R("legal-transcription-software", LEGAL_SOFTWARE, "Handling the recording once it exists."),
    R("meeting-minutes-template", TEMPLATE, "Writing the record up once you may lawfully keep one."),
  ],

  // Local capture and tooling.
  "best-local-speech-to-text": [
    R("local-dictation-macos", DICTATION, "The dictation-specific setup on macOS."),
    R("open-source-alternatives-to-granola-ai", OSS_GRANOLA, "The same on-device argument applied to meeting notes."),
    R("hipaa-compliant-transcription", HIPAA_TRANSCRIPTION, "Why on-device matters when the audio is regulated."),
    R("best-mcp-meeting-memory-tools", MCP_TOOLS, "Making the transcripts readable by an assistant afterwards."),
  ],
  "local-dictation-macos": [
    R("best-local-speech-to-text", LOCAL_STT, "The broader comparison, including non-dictation use."),
    R("open-source-alternatives-to-granola-ai", OSS_GRANOLA, "On-device options for meetings rather than dictation."),
    R("best-meeting-tools-for-claude-code-and-codex", AGENT_TOOLS, "Getting dictated notes in front of a coding agent."),
    R("meeting-minutes-template", TEMPLATE, "A structure to dictate into."),
  ],
  "open-source-alternatives-to-granola-ai": [
    R("is-granola-hipaa-compliant", GRANOLA_HIPAA, "The compliance answer that sends people looking."),
    R("best-local-speech-to-text", LOCAL_STT, "The transcription layer underneath most of these."),
    R("best-mcp-meeting-memory-tools", MCP_TOOLS, "Which of them an assistant can actually read."),
    R("remove-ai-notetaker-bots-from-meetings", BOTS_HUB, "Getting the existing bot out first."),
  ],
  "best-mcp-meeting-memory-tools": [
    R("best-meeting-tools-for-claude-code-and-codex", AGENT_TOOLS, "The same question from the coding-agent side."),
    R("open-source-alternatives-to-granola-ai", OSS_GRANOLA, "Where the meeting notes come from."),
    R("best-local-speech-to-text", LOCAL_STT, "The transcription layer underneath."),
    R("meeting-minutes-template", TEMPLATE, "The file format an assistant reads best."),
  ],
  "best-meeting-tools-for-claude-code-and-codex": [
    R("best-mcp-meeting-memory-tools", MCP_TOOLS, "The broader MCP comparison."),
    R("local-dictation-macos", DICTATION, "Talking notes into a session instead of typing them."),
    R("open-source-alternatives-to-granola-ai", OSS_GRANOLA, "Capture options that leave files an agent can read."),
    R("meeting-minutes-template", TEMPLATE, "A markdown structure agents parse reliably."),
  ],
  "meeting-minutes-template": [
    R("is-it-legal-to-record-a-meeting", LEGAL_RECORD, "Whether you may record the meeting you are minuting."),
    R("best-local-speech-to-text", LOCAL_STT, "Filling the template from audio instead of by hand."),
    R("best-mcp-meeting-memory-tools", MCP_TOOLS, "Making finished minutes searchable by an assistant."),
    R("local-dictation-macos", DICTATION, "Dictating the notes rather than typing them."),
  ],
};
