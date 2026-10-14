// Settings > API tokens (ui-design 6.9, layout frontend-and-plugins.md 8.2). A new token is made only after the locked
// TokenConfirm prompt is approved: the prompt says what the token can and cannot do, and then shows the new value once.
// The list and the revoke are token.list and token.revoke; the token itself is token.create. The name draft lives here,
// above the Basic and Advanced split, so switching the mode keeps what was typed.

import { AdvancedOnly, Button, Select, TextField } from "@rastai/ui";
import { usePluginHost, tokenCreate, tokenList, tokenRevoke } from "@rastai/plugin";
import { TokenConfirm } from "@rastai/ui/security";
import { useEffect, useState } from "react";

import { useShell } from "../../app/shell-context.js";
import { PRESET_LABELS, presetWords } from "./scope-words.js";

const DEFAULT_PRESET = "chat-only";
const MISSING_NAME_TEXT = "Give the token a name first.";
const NO_TOKENS_TEXT = "No API tokens yet.";

/**
 * @typedef {object} TokenPrompt
 * @property {string} name
 * @property {string} preset
 * @property {string | null} tokenValue    set once token.create has run
 */

export function TokensPage() {
  const host = usePluginHost();
  const shell = useShell();
  const [tokens, setTokens] = useState(/** @type {any[]} */ ([]));
  const [name, setName] = useState("");
  const [preset, setPreset] = useState(/** @type {string} */ (DEFAULT_PRESET));
  const [prompt, setPrompt] = useState(/** @type {TokenPrompt | null} */ (null));
  const [message, setMessage] = useState("");

  async function loadTokens() {
    const outcome = await tokenList(host, { include_revoked: false });
    if (outcome.ok) {
      setTokens(outcome.result.tokens);
    } else {
      outcome.diagnostics.forEach((diagnostic) => shell.notify(diagnostic));
    }
  }

  useEffect(() => {
    loadTokens();
  }, []);

  function askToCreate() {
    if (name.trim() === "") {
      setMessage(MISSING_NAME_TEXT);
      return;
    }
    setMessage("");
    setPrompt({ name: name.trim(), preset, tokenValue: null });
  }

  async function approve() {
    if (prompt === null) {
      return;
    }
    const outcome = await tokenCreate(host, { name: prompt.name, preset: /** @type {any} */ (prompt.preset) });
    if (outcome.ok) {
      setPrompt({ ...prompt, tokenValue: outcome.result.token });
      setName("");
      await loadTokens();
    } else {
      setPrompt(null);
      outcome.diagnostics.forEach((diagnostic) => shell.notify(diagnostic));
    }
  }

  /** @param {any} token */
  async function revoke(token) {
    const outcome = await tokenRevoke(host, { token_id: token.token_id });
    if (outcome.ok) {
      await loadTokens();
    } else {
      outcome.diagnostics.forEach((diagnostic) => shell.notify(diagnostic));
    }
  }

  const words = prompt === null ? null : presetWords(prompt.preset);
  return (
    <section className="rr-settings-group" aria-labelledby="tokens-title">
      <h3 id="tokens-title" className="rr-settings-group__title">
        API tokens
      </h3>
      <p className="rr-settings-note">
        A token lets a script or another program reach this computer. It can do only what its access allows.
      </p>
      <div className="rr-token-form">
        <TextField
          label="Token name"
          value={name}
          onChange={setName}
          help="Shown in this list, such as laptop script."
        />
        <Select
          label="Access"
          value={preset}
          options={Object.entries(PRESET_LABELS).map(([value, label]) => ({ value, label }))}
          onChange={setPreset}
        />
        <AdvancedOnly>
          <p className="rr-setting-key">
            <code>preset</code>
          </p>
        </AdvancedOnly>
        <Button variant="primary" onClick={askToCreate}>
          Create token
        </Button>
        {message ? (
          <p role="alert" className="rr-settings-note">
            {message}
          </p>
        ) : null}
      </div>

      {tokens.length === 0 ? (
        <p className="rr-settings-note">{NO_TOKENS_TEXT}</p>
      ) : (
        <ul className="rr-token-list">
          {tokens.map((token) => (
            <li key={token.token_id} className="rr-token-list__item">
              <span className="rr-token-list__name">{token.name}</span>
              <span className="rr-settings-note">{token.scopes.join(", ")}</span>
              <AdvancedOnly>
                <code className="rr-setting-key">{token.token_id}</code>
              </AdvancedOnly>
              <Button variant="quiet" ariaLabel={`Revoke ${token.name}`} onClick={() => revoke(token)}>
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      )}

      {prompt !== null && words !== null ? (
        <TokenConfirm
          tokenName={prompt.name}
          tokenValue={prompt.tokenValue}
          canDo={words.canDo}
          cannotDo={words.cannotDo}
          onApprove={approve}
          onCancel={() => {
            const created = prompt.tokenValue !== null;
            setPrompt(null);
            if (created) {
              loadTokens();
            }
          }}
        />
      ) : null}
    </section>
  );
}
