// The Remote list's words: where a node is, its tone, and the heartbeat's age.

import { describe, expect, test } from "vitest";

import { heartbeatWord, nodeTone, nodeWhere } from "./node-view.js";

describe("node words", () => {
  test("the local node is this computer, and an SSH node names its address", () => {
    expect(nodeWhere({ transport: "local" })).toBe("This computer");
    expect(nodeWhere({ transport: "ssh", address: "me@gpu-box" })).toBe("SSH to me@gpu-box");
  });

  test("connected is ok, unreachable is an error, anything else is neutral", () => {
    expect(nodeTone("connected")).toBe("ok");
    expect(nodeTone("unreachable")).toBe("error");
    expect(nodeTone("starting")).toBe("neutral");
  });

  test("the heartbeat says how many seconds ago it came, or that none has", () => {
    expect(heartbeatWord(0, 5000)).toBe("No heartbeat yet");
    expect(heartbeatWord(1000, 6200)).toBe("Last heartbeat 5 s ago");
  });
});
