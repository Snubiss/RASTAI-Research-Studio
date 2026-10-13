// A plugin's requested permissions in plain English, the sensitive ones marked, and whether each was granted
// (contracts 6.3; layout frontend-and-plugins.md 2.10). The trust prompt shows it without `granted`, before the
// user has decided. Drawn only with locked colors.

import { DANGER_TEXT_STYLE, LIST_STYLE, TEXT_STYLE } from "./locked-styles.js";
import { permissionText } from "./permission-text.js";

const SENSITIVE_MARK = " (sensitive)";

/**
 * @param {object} props
 * @param {string[]} props.requested       the manifest's permissions[]
 * @param {string[] | null} [props.granted] what the user allowed (plugins.granted_permissions); null hides the column
 */
export function PermissionList({ requested, granted = null }) {
  if (requested.length === 0) {
    return (
      <p className="rr-locked" style={TEXT_STYLE}>
        No permissions requested.
      </p>
    );
  }
  return (
    <ul className="rr-locked" style={LIST_STYLE}>
      {requested.map((permission) => {
        const described = permissionText(permission);
        return (
          <li key={permission}>
            <span style={described.sensitive ? DANGER_TEXT_STYLE : TEXT_STYLE}>
              {described.text}
              {described.sensitive ? SENSITIVE_MARK : ""}
            </span>
            {granted ? (
              <span style={TEXT_STYLE}>{granted.includes(permission) ? " - allowed" : " - not allowed"}</span>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
