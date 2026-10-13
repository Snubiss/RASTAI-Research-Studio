// The connection banner (layout frontend-and-plugins.md 8.1): quiet while the stream is up, a line while it reconnects,
// and the RRS-U0001 Diagnostic once the daemon has been unreachable for a while (offline).

import { DiagnosticView } from "@rastai/ui";
import { useStore } from "@rastai/plugin";

import { CODE_DAEMON_UNREACHABLE } from "../constants.js";
import { useShell } from "./shell-context.js";
import { pluginDiagnostic } from "@rastai/plugin";

const QUIET_STATUSES = ["connecting", "connected"];

export function ConnectionBanner() {
  const shell = useShell();
  const connection = useStore(shell.stores.connection);
  if (QUIET_STATUSES.includes(connection.status)) {
    return null;
  }
  if (connection.status === "offline") {
    const problem = connection.problem ?? pluginDiagnostic(CODE_DAEMON_UNREACHABLE, { address: shell.client.baseUrl });
    return (
      <div className="rr-connection-banner">
        <DiagnosticView diagnostic={problem} />
      </div>
    );
  }
  return (
    <p className="rr-connection-banner" role="status">
      Reconnecting to Rastai…
    </p>
  );
}
