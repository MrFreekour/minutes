export type CommitmentDisplayRow = {
  text: string;
  owner: string;
  due: string;
  source: string;
  status: "open" | "stale";
};

function displayField(value: unknown, fallback: string): string {
  return typeof value === "string" && value.trim() ? value : fallback;
}

/**
 * Normalize both the historical graph row and the current live-source row.
 * Keeping this pure makes the MCP/UI compatibility contract testable without
 * booting a browser or importing the app's DOM side effects.
 */
export function normalizeCommitmentDisplayRows(data: unknown): CommitmentDisplayRow[] {
  if (!data || typeof data !== "object" || Array.isArray(data)) return [];
  const commitments = (data as { commitments?: unknown }).commitments;
  if (!Array.isArray(commitments)) return [];

  return commitments
    .filter((item): item is Record<string, unknown> =>
      Boolean(item) && typeof item === "object" && !Array.isArray(item)
    )
    .map((item) => ({
      text: displayField(item.what ?? item.text, "Commitment"),
      owner: displayField(item.who ?? item.person_name, "unassigned"),
      due: displayField(item.by_date ?? item.due_date, "no date"),
      source: displayField(item.title ?? item.meeting_title, "meeting"),
      status: item.status === "stale" ? "stale" : "open",
    }));
}
