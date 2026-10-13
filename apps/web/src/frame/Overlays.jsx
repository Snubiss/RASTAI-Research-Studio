// The dialogs the frame opens: the shortcut list (Shift ?, design 3.7) and the egress details (the badge's click, layout
// 8.4). Each is a Dialog, opened by the overlay store, so one place says which dialog is open.

import { Dialog } from "@rastai/ui";
import { useStore } from "@rastai/plugin";

import { useShell } from "../app/shell-context.js";
import { SHORTCUT_LIST } from "../app/keyboard.js";

/** @typedef {import("./egress-state.js").EgressEntry} EgressEntry */

export function Overlays() {
  const shell = useShell();
  const overlay = useStore(shell.stores.overlay);
  const egress = /** @type {EgressEntry[]} */ (useStore(shell.stores.egress));
  const close = () => shell.stores.overlay.set("");
  return (
    <>
      <Dialog open={overlay === "shortcuts"} title="Keyboard shortcuts" onClose={close}>
        <dl className="rr-shortcuts">
          {SHORTCUT_LIST.map((shortcut) => (
            <div key={shortcut.keys} className="rr-shortcuts__row">
              <dt>
                <kbd>{shortcut.keys}</kbd>
              </dt>
              <dd>{shortcut.action}</dd>
            </div>
          ))}
        </dl>
      </Dialog>
      <Dialog open={overlay === "egress"} title="Data leaving this machine" onClose={close}>
        <ul className="rr-egress-list">
          {egress.map((entry) => (
            <li key={`${entry.purpose}|${entry.host}|${entry.subjectId}`}>
              {entry.purpose} to {entry.host} for {entry.subjectId}
            </li>
          ))}
        </ul>
      </Dialog>
    </>
  );
}
