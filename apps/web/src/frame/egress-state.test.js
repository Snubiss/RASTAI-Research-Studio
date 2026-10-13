// The egress badge's list (layout frontend-and-plugins.md 8.4; spec 24): only egress.active reports count, each pair is
// held for EGRESS_BADGE_HOLD_MS after its last report, and a report refreshes its hold.

import { describe, expect, test } from "vitest";

import { EGRESS_BADGE_HOLD_MS } from "../constants.js";
import { activeEgress, egressReportOf, heldEgress, recordEgress } from "./egress-state.js";

const DOWNLOAD = { purpose: "hf_download", host: "huggingface.co", subjectId: "model_01" };
const START_MS = 1_000_000;

describe("recordEgress and activeEgress", () => {
  test("a report is on the list until its hold time has passed", () => {
    const entries = recordEgress([], DOWNLOAD, START_MS);
    expect(activeEgress(entries, START_MS + EGRESS_BADGE_HOLD_MS - 1)).toEqual([DOWNLOAD]);
    expect(activeEgress(entries, START_MS + EGRESS_BADGE_HOLD_MS)).toEqual([]);
  });

  test("a repeated report refreshes its hold rather than adding a second entry", () => {
    let entries = recordEgress([], DOWNLOAD, START_MS);
    entries = recordEgress(entries, DOWNLOAD, START_MS + EGRESS_BADGE_HOLD_MS - 1);
    expect(entries).toHaveLength(1);
    expect(activeEgress(entries, START_MS + EGRESS_BADGE_HOLD_MS + 1)).toEqual([DOWNLOAD]);
  });

  test("the same host for another subject is another entry", () => {
    const entries = recordEgress(
      recordEgress([], DOWNLOAD, START_MS),
      { ...DOWNLOAD, subjectId: "model_02" },
      START_MS,
    );
    expect(activeEgress(entries, START_MS)).toHaveLength(2);
  });

  test("heldEgress drops what is no longer held", () => {
    const entries = recordEgress(
      recordEgress([], DOWNLOAD, START_MS),
      { ...DOWNLOAD, host: "x.example" },
      START_MS + 5000,
    );
    expect(heldEgress(entries, START_MS + EGRESS_BADGE_HOLD_MS).map((entry) => entry.host)).toEqual(["x.example"]);
  });
});

describe("egressReportOf", () => {
  test("reads the report of an egress.active event", () => {
    const event = {
      eventType: "egress.active",
      payload: {
        case: "egressActive",
        value: { purpose: "hf_download", host: "huggingface.co", subjectId: "model_01" },
      },
    };
    expect(egressReportOf(event)).toEqual(DOWNLOAD);
  });

  test("any other event carries no report", () => {
    expect(egressReportOf({ eventType: "job.progress", payload: { case: "jobProgress", value: {} } })).toBeNull();
    expect(egressReportOf({ eventType: "egress.active", payload: undefined })).toBeNull();
  });
});
