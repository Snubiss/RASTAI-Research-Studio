// What the window shows when it can't reach the daemon at all, before the frame exists (RRS-U0001, design 6.9).

import { DiagnosticView } from "@rastai/ui";

import { CODE_DAEMON_UNREACHABLE } from "../constants.js";
import { pluginDiagnostic } from "@rastai/plugin";

/**
 * @param {object} props
 * @param {import("@rastai/proto").Diagnostic | null} props.diagnostic
 */
export function OfflineScreen({ diagnostic }) {
  const problem = diagnostic ?? pluginDiagnostic(CODE_DAEMON_UNREACHABLE, { address: window.location.origin });
  return (
    <main className="rr-main">
      <DiagnosticView diagnostic={problem} onFix={() => window.location.reload()} />
    </main>
  );
}
