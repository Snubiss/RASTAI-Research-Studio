// The security UI under the daemon's content security policy. The page is served with the policy from rastai-server's
// web_assets.rs, which allows the locked focus ring's inline style by hash. Without that allowance the browser drops the
// style, and the egress badge's focus ring falls back to the theme's ring (ui-design.md 5.1; layout 2.10).

import { EgressBadge } from "@rastai/ui/security";
import { afterEach, describe, expect, test } from "vitest";

import { DAEMON_CONTENT_SECURITY_POLICY } from "../testing/daemon-csp.js";
import { inAct, mount } from "../testing/render.js";

const BADGE_ENTRY = { purpose: "model download", host: "huggingface.co", subjectId: "job_01" };

/** @type {{ container: HTMLElement, unmount: () => Promise<void> } | null} */
let mounted = null;

afterEach(async () => {
  if (mounted !== null) {
    await mounted.unmount();
    mounted = null;
  }
});

describe("the egress badge under the daemon's policy", () => {
  test("its locked focus ring still draws, so the theme's ring doesn't replace it", async () => {
    // A meta element applies the policy to this page from here on, as the daemon's header does to the app.
    const policyMeta = document.createElement("meta");
    policyMeta.httpEquiv = "Content-Security-Policy";
    policyMeta.content = DAEMON_CONTENT_SECURITY_POLICY;
    document.head.append(policyMeta);
    try {
      mounted = await mount(<EgressBadge activeEgress={[BADGE_ENTRY]} onOpen={() => {}} />);
      const badge = /** @type {HTMLButtonElement} */ (mounted.container.querySelector("button"));
      expect(badge).not.toBeNull();
      await inAct(() => badge.focus());

      const style = getComputedStyle(badge);
      expect(style.outlineStyle).toBe("solid");
      expect(style.outlineWidth).toBe("2px");
    } finally {
      // The policy is for this test only: a later test in this page must not run under it.
      policyMeta.remove();
    }
  });

  test("the policy meta is gone once its test ends", () => {
    expect(document.querySelector('meta[http-equiv="Content-Security-Policy"]')).toBeNull();
  });
});
