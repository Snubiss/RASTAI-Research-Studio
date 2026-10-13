// The locked focus ring is an inline style, which the daemon's content security policy allows by hash
// (rastai-server's web_assets.rs). This test keeps the two in step: change LOCKED_FOCUS_RING_CSS and the hash in the
// policy has to change with it, or the ring would silently fall back to the theme's ring in the real app.
// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { createHash } from "node:crypto";
// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import { LOCKED_FOCUS_RING_CSS } from "./locked-styles.js";

const DAEMON_WEB_ASSETS = new URL("../../../../crates/rastai-server/src/http/web_assets.rs", import.meta.url);

/**
 * The CSP source expression for an inline style with this text: 'sha256-<base64 of the sha256 digest>'.
 * @param {string} text
 * @returns {string}
 */
function styleHashSource(text) {
  return `'sha256-${createHash("sha256").update(text, "utf8").digest("base64")}'`;
}

describe("the locked focus ring under the daemon's policy", () => {
  const daemonSource = readFileSync(DAEMON_WEB_ASSETS, "utf8");

  test("the daemon's style-src allows exactly the hash of the locked focus ring", () => {
    const styleSrc = daemonSource.match(/style-src ([^;]*);/);
    expect(styleSrc).not.toBeNull();
    expect(styleSrc[1]).toBe(`'self' ${styleHashSource(LOCKED_FOCUS_RING_CSS)}`);
  });
});
