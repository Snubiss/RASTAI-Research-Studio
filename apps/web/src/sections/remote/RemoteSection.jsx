// Remote (ui-design 6.8; layout frontend-and-plugins.md 8.2): the nodes this window knows, with their state and their last
// heartbeat. This version shows the local node; pairing another computer over SSH comes with M9. Advanced adds the
// protocol and the capabilities each node reported.

import { AdvancedOnly, DiagnosticView, StatePill } from "@rastai/ui";
import { pluginDiagnostic, useStore } from "@rastai/plugin";

import { useShell } from "../../app/shell-context.js";
import { heartbeatWord, nodeTone, nodeWhere } from "./node-view.js";

// The catalog code for a node the daemon can't reach (RRS-N2001); its text comes from the catalog.
const UNREACHABLE_CODE = "RRS-N2001";

export function RemoteSection() {
  const shell = useShell();
  const nodes = /** @type {any[]} */ (useStore(shell.stores.nodes));
  return (
    <div className="rr-remote">
      <h2 className="rr-section-title">Remote</h2>
      {nodes.length === 0 ? (
        <p className="rr-settings-note">No computers are known yet.</p>
      ) : (
        <ul className="rr-card-list" aria-label="Known computers">
          {nodes.map((node) => (
            <li key={node.node_id} className="rr-card">
              <h3 className="rr-card__title">{node.name}</h3>
              <p className="rr-settings-note">{nodeWhere(node)}</p>
              <StatePill label={node.state} tone={nodeTone(node.state)} />
              {node.state === "unreachable" ? (
                <DiagnosticView diagnostic={pluginDiagnostic(UNREACHABLE_CODE, {})} />
              ) : null}
              <p className="rr-settings-note">{heartbeatWord(node.last_heartbeat_unix_ms, Date.now())}</p>
              <AdvancedOnly>
                <p className="rr-setting-key">{`node id ${node.node_id}`}</p>
                <p className="rr-settings-note">{`Protocol ${node.protocol_version}`}</p>
                <p className="rr-settings-note">{`Capabilities: ${(node.capabilities ?? []).join(", ") || "none"}`}</p>
              </AdvancedOnly>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
