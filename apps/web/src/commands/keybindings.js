// Keyboard shortcuts as the palette and the window read them (layout frontend-and-plugins.md 2.5 step 2; design 3.7). A
// shortcut is text such as "Mod+Shift+L": "Mod" is Ctrl, or Cmd on macOS. The daemon's default_shortcut is the one source
// of a command's binding; a user's own bindings (workspace.keybindings) come later. Pure, so it is tested in Node.

// Copy as CLI (design 3.7): Ctrl . Ctrl Shift C is the browser's element picker, so it is not used.
export const COPY_AS_CLI_SHORTCUT = "Mod+.";

const MODIFIER_MOD = "mod";
const MODIFIER_SHIFT = "shift";
const MODIFIER_ALT = "alt";
const PART_SEPARATOR = "+";

/**
 * @typedef {object} Shortcut
 * @property {boolean} mod     Ctrl or Cmd
 * @property {boolean} shift
 * @property {boolean} alt
 * @property {string} key     the key, lowercased, as KeyboardEvent.key names it
 */

/**
 * @typedef {object} KeyPress   the parts of a KeyboardEvent a shortcut reads
 * @property {string} key
 * @property {boolean} ctrlKey
 * @property {boolean} metaKey
 * @property {boolean} shiftKey
 * @property {boolean} altKey
 */

/**
 * The parts of a shortcut text. Empty or unreadable text is no shortcut.
 * @param {string} text
 * @returns {Shortcut | null}
 */
export function parseShortcut(text) {
  const parts = text
    .split(PART_SEPARATOR)
    .map((part) => part.trim().toLowerCase())
    .filter((part) => part !== "");
  if (parts.length === 0) {
    return null;
  }
  const key = parts[parts.length - 1];
  const modifiers = parts.slice(0, -1);
  if (!modifiers.every((modifier) => [MODIFIER_MOD, MODIFIER_SHIFT, MODIFIER_ALT].includes(modifier))) {
    return null;
  }
  return {
    mod: modifiers.includes(MODIFIER_MOD),
    shift: modifiers.includes(MODIFIER_SHIFT),
    alt: modifiers.includes(MODIFIER_ALT),
    key,
  };
}

/**
 * Whether a key press is the shortcut. Shift and Alt must match exactly, so Ctrl+K never fires Ctrl+Shift+K's command.
 * @param {string} text   the shortcut text
 * @param {KeyPress} press
 * @returns {boolean}
 */
export function shortcutMatches(text, press) {
  const shortcut = parseShortcut(text);
  if (shortcut === null) {
    return false;
  }
  return (
    shortcut.key === press.key.toLowerCase() &&
    shortcut.mod === (press.ctrlKey || press.metaKey) &&
    shortcut.shift === press.shiftKey &&
    shortcut.alt === press.altKey
  );
}

/**
 * The command whose default shortcut this key press is, or null. A command with no shortcut never matches.
 * @param {any[]} commands   CommandDescriptor[]
 * @param {KeyPress} press
 * @returns {any | null}
 */
export function commandForKeys(commands, press) {
  return commands.find((command) => shortcutMatches(command.defaultShortcut ?? "", press)) ?? null;
}

/**
 * The shortcut as the screen shows it: "Mod+Shift+L" reads "Ctrl+Shift+L". Empty text reads as nothing.
 * @param {string} text
 * @returns {string}
 */
export function shortcutLabel(text) {
  const shortcut = parseShortcut(text);
  if (shortcut === null) {
    return "";
  }
  const parts = [];
  if (shortcut.mod) {
    parts.push("Ctrl");
  }
  if (shortcut.alt) {
    parts.push("Alt");
  }
  if (shortcut.shift) {
    parts.push("Shift");
  }
  parts.push(shortcut.key.length === 1 ? shortcut.key.toUpperCase() : capitalized(shortcut.key));
  return parts.join(PART_SEPARATOR);
}

/**
 * @param {string} word
 * @returns {string}
 */
function capitalized(word) {
  return word.charAt(0).toUpperCase() + word.slice(1);
}
