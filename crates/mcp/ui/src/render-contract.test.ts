import { describe, expect, it } from "vitest";

import { normalizeCommitmentDisplayRows } from "./render-contract.js";

describe("commitment renderer contract", () => {
  it("renders the historical graph payload without losing its fields", () => {
    expect(normalizeCommitmentDisplayRows({
      commitments: [{
        text: "Send the plan",
        status: "stale",
        due_date: "2026-07-20",
        created_at: "2026-07-10T12:00:00Z",
        commitment_type: "action_item",
        meeting_title: "Planning",
        meeting_date: "2026-07-10T12:00:00Z",
        person_name: "Avery Quinn",
      }],
    })).toEqual([{
      text: "Send the plan",
      owner: "Avery Quinn",
      due: "2026-07-20",
      source: "Planning",
      status: "stale",
    }]);
  });

  it("also renders current live-source rows and rejects non-array payloads", () => {
    expect(normalizeCommitmentDisplayRows({
      commitments: [{
        what: "Review the memo",
        who: "Case Morgan",
        by_date: null,
        title: "Review",
        status: "open",
      }],
    })[0]).toEqual({
      text: "Review the memo",
      owner: "Case Morgan",
      due: "no date",
      source: "Review",
      status: "open",
    });
    expect(normalizeCommitmentDisplayRows({ commitments: {} })).toEqual([]);
  });
});
