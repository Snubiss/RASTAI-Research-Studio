// The Tokens page's prompt in the browser (ui-design 6.9): a token is made only after the locked prompt is approved, and
// nothing a user does while it is being made (Enter on Cancel, Escape) closes the prompt before the answer arrives.

import { PluginHostProvider, createMockHost, pluginDiagnostic } from "@rastai/plugin";
import { afterEach, describe, expect, test, vi } from "vitest";
import { page, userEvent } from "vitest/browser";

import { ShellProvider } from "../../app/shell-context.js";
import { answered } from "../../testing/fake-shell.js";
import { inAct, mount } from "../../testing/render.js";
import { TokensPage } from "./TokensPage.jsx";

const TOKEN_VALUE = "rst_once_only_value";
const REFUSED_CODE = "RRS-C1005";

/** @type {any} */
let mounted = null;

afterEach(async () => {
  await mounted?.unmount();
  mounted = null;
});

/**
 * A host whose token.create waits for the test: `finishCreate` resolves it with the given outcome.
 * @param {any} [createOutcome]   the outcome token.create answers with; omitted, the test answers it itself
 */
function tokensHost(createOutcome) {
  /** @type {(outcome: any) => void} */
  let finishCreate = () => {};
  const createAnswer = new Promise((resolve) => {
    finishCreate = resolve;
  });
  const mock = createMockHost("core", {
    "token.list": () => answered({ tokens: [] }),
    "token.create": () => (createOutcome === undefined ? createAnswer : createOutcome),
  });
  return { ...mock, finishCreate };
}

/**
 * Types a name and presses Create token, which opens the locked prompt.
 * @param {string} name
 */
async function openPromptWithName(name) {
  await page.getByLabelText("Token name").fill(name);
  await page.getByRole("button", { name: "Create token", exact: true }).click();
}

/** The Create token button inside the open prompt (the page's own Create token is outside the modal). */
function approveButton() {
  return page.getByRole("dialog").getByRole("button", { name: "Create token", exact: true });
}

/**
 * The locked prompt's dialog element, or null when it is closed or gone.
 * @returns {HTMLDialogElement | null}
 */
function promptDialog() {
  return /** @type {HTMLDialogElement | null} */ (document.querySelector("dialog.rr-locked"));
}

/**
 * Mounts the page over a host and a shell whose notify is a spy.
 * @param {ReturnType<typeof tokensHost>} host
 */
async function mountTokens(host) {
  const shell = { notify: vi.fn() };
  mounted = await mount(
    <PluginHostProvider host={host.host}>
      <ShellProvider value={/** @type {any} */ (shell)}>
        <TokensPage />
      </ShellProvider>
    </PluginHostProvider>,
  );
  return shell;
}

describe("the create prompt while the token is made", () => {
  test("Enter on Cancel does not close the prompt, and the value shows once when the create answers", async () => {
    const host = tokensHost();
    await mountTokens(host);
    await openPromptWithName("laptop script");
    await approveButton().click();
    // Create token leaves the focus on Cancel: a second Enter presses Cancel while the token is being made.
    await userEvent.keyboard("{Enter}");
    expect(promptDialog()?.open, "the prompt is still open").toBe(true);
    expect(promptDialog()?.getAttribute("aria-busy")).toBe("true");
    // Escape is a cancel too: TokenConfirm turns it into onCancel, which must do nothing while the token is made.
    await userEvent.keyboard("{Escape}");
    expect(promptDialog()?.open, "Escape leaves the prompt open").toBe(true);

    await inAct(() => host.finishCreate(answered({ token: TOKEN_VALUE })));
    await vi.waitFor(() => expect(promptDialog()?.textContent).toContain(TOKEN_VALUE));
    expect(promptDialog()?.open).toBe(true);
  });

  test("Done closes the prompt and the token list is read again", async () => {
    const host = tokensHost();
    await mountTokens(host);
    await openPromptWithName("laptop script");
    await approveButton().click();
    await inAct(() => host.finishCreate(answered({ token: TOKEN_VALUE })));
    await vi.waitFor(() => expect(promptDialog()?.textContent).toContain(TOKEN_VALUE));
    const listsBefore = host.calls.filter((call) => call.commandId === "token.list").length;
    await page.getByRole("button", { name: "Done", exact: true }).click();
    await vi.waitFor(() => expect(promptDialog()).toBeNull());
    await vi.waitFor(() =>
      expect(host.calls.filter((call) => call.commandId === "token.list").length).toBeGreaterThan(listsBefore),
    );
  });

  test("a refused create closes the prompt and shows the daemon's diagnostic", async () => {
    const diagnostic = pluginDiagnostic(REFUSED_CODE, {});
    const host = tokensHost({ ok: false, result: null, jobId: null, diagnostics: [diagnostic], journalId: null });
    const shell = await mountTokens(host);
    await openPromptWithName("laptop script");
    await approveButton().click();
    await vi.waitFor(() => expect(promptDialog()).toBeNull());
    expect(shell.notify).toHaveBeenCalledWith(diagnostic);
    expect(shell.notify.mock.calls[0][0].code).toBe(REFUSED_CODE);
  });
});
