// The policy the browser tests apply (testing/daemon-csp.js) is the one rastai-server sends: this test reads the Rust
// literal and compares it with the copy, so the two can't drift apart.
// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import { DAEMON_CONTENT_SECURITY_POLICY } from "../testing/daemon-csp.js";

const DAEMON_WEB_ASSETS = new URL("../../../../crates/rastai-server/src/http/web_assets.rs", import.meta.url);
const POLICY_START = 'CONTENT_SECURITY_POLICY: &str = "';

/**
 * The policy string as the daemon sends it: the Rust literal with its line continuations joined.
 * @param {string} source   the text of web_assets.rs
 * @returns {string}
 */
function daemonPolicy(source) {
  const start = source.indexOf(POLICY_START);
  const end = source.indexOf('";', start);
  if (start === -1 || end === -1) {
    throw new Error("web_assets.rs has no CONTENT_SECURITY_POLICY literal");
  }
  return source.slice(start + POLICY_START.length, end).replace(/\\\n\s*/g, "");
}

describe("the policy the browser tests apply", () => {
  test("is the daemon's policy, word for word", () => {
    const source = readFileSync(DAEMON_WEB_ASSETS, "utf8");
    expect(DAEMON_CONTENT_SECURITY_POLICY).toBe(daemonPolicy(source));
  });
});
