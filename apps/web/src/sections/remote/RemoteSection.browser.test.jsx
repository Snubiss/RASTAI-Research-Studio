// The Remote list in the browser (ui-design 6.8): the heartbeat's age is a clock reading, so it counts up while the
// section is on screen, without any event arriving.

import { afterEach, describe, expect, test, vi } from "vitest";

import { ShellProvider } from "../../app/shell-context.js";
import { testShell } from "../../testing/fake-shell.js";
import { inAct, mount } from "../../testing/render.js";
import { RemoteSection } from "./RemoteSection.jsx";

/** @type {any} */
let mounted = null;

afterEach(async () => {
  await mounted?.unmount();
  mounted = null;
  vi.useRealTimers();
});

describe("the heartbeat's age", () => {
  test("counts up while the Remote section is on screen", async () => {
    // The fake clock is set before the section mounts, so its interval is the fake one and clears on unmount.
    vi.useFakeTimers();
    const { shell } = await testShell();
    shell.stores.nodes.set([
      {
        node_id: "node_local",
        name: "This computer",
        transport: "local",
        state: "connected",
        last_heartbeat_unix_ms: Date.now(),
        protocol_version: 1,
        capabilities: [],
      },
    ]);
    mounted = await mount(
      <ShellProvider value={shell}>
        <RemoteSection />
      </ShellProvider>,
    );
    expect(mounted.container.textContent).toContain("Last heartbeat 0 s ago");

    await inAct(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(mounted.container.textContent).toContain("Last heartbeat 3 s ago");
  });
});
