// The plain words for a token's scopes (contracts 6.3: the scope table's "Grants" column) and the two presets. The
// token prompt says what a token holds from these, until the daemon's dry run (command request R13, ui-design 7) can
// name the scopes itself. The words are presentation; the scopes and presets are the daemon's.

/** The scope presets a token can be made with (contracts 6.3). */
/** @type {Object<string, string[]>} */
export const PRESET_SCOPES = {
  "chat-only": ["chat"],
  "mcp-default": ["read", "chat", "project"],
};

/** The name of each preset in the form. */
/** @type {Object<string, string>} */
export const PRESET_LABELS = {
  "chat-only": "Chat only",
  "mcp-default": "Scripts and MCP clients",
};

// The scopes that matter most to someone deciding whether to trust a token: a token without them can't do these things.
const SCOPES_THAT_MATTER = [
  "system.admin",
  "plugins.manage",
  "secrets",
  "sandbox.spawn",
  "gateway.spend",
  "conversations.read",
  "training",
  "inference.manage",
];

/** @type {Object<string, string>} */
const SCOPE_WORDS = {
  read: "Read models, runs, designs, nodes and settings",
  chat: "Chat with local models and use the /v1 endpoints",
  project: "Read and change projects, files and documents",
  "system.admin": "Change settings and tokens, collect garbage, and update",
  "plugins.manage": "Install, remove, enable or disable plugins",
  secrets: "Read and store secrets, and sign in to Hugging Face",
  "sandbox.spawn": "Create and remove sandboxes",
  "gateway.spend": "Call paid third-party APIs",
  "conversations.read": "Read conversation history and gateway logs",
  training: "Start and stop training runs",
  "inference.manage": "Download, load and unload models, and configure adapters",
};

/**
 * What a preset's token can do, and what it cannot do from the scopes that matter.
 * @param {string} preset   a PRESET_SCOPES key
 * @returns {{ canDo: string[], cannotDo: string[] }}
 */
export function presetWords(preset) {
  const granted = PRESET_SCOPES[preset] ?? [];
  const withheld = SCOPES_THAT_MATTER.filter((scope) => !granted.includes(scope));
  return {
    canDo: granted.map(wordOf),
    cannotDo: withheld.map(wordOf),
  };
}

/**
 * @param {string} scope
 * @returns {string}
 */
function wordOf(scope) {
  return SCOPE_WORDS[scope] ?? scope;
}
