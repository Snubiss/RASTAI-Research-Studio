// The thin main (layout frontend-and-plugins.md 8.3): each step is one function in app/boot.js, app/daemon.js,
// app/shell.js or plugin-host/. Sign-in comes first; a window without a session shows SignedOutScreen (RRS-U0002/U0003),
// and one the daemon can't be reached from shows the offline screen (RRS-U0001).

/// <reference types="vite/client" />
import "@rastai/ui/styles/base.css";
import "@rastai/ui/styles/components.css";
import "./app/frame.css";
import "./app/surfaces.css";

import { SignedOutScreen } from "@rastai/ui/security";
import { createRoot } from "react-dom/client";

import { App } from "./app/App.jsx";
import { loadCommands, loadPlugins, loadSettings, noSessionScreen, signIn } from "./app/boot.js";
import { connectDaemon, versionProblem } from "./app/daemon.js";
import { OfflineScreen } from "./app/OfflineScreen.jsx";
import { createShell } from "./app/shell.js";
import { registerShellUis } from "./plugin-host/load-plugins.js";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

async function main() {
  const root = createRoot(/** @type {HTMLElement} */ (document.getElementById("root")));
  const session = await signIn(window.location);
  if (session.sessionKey === null) {
    if (noSessionScreen(session.diagnostic) === "offline") {
      root.render(<OfflineScreen diagnostic={session.diagnostic} />);
    } else {
      root.render(<SignedOutScreen code={session.diagnostic?.code} />);
    }
    return;
  }
  const connected = await connectDaemon(window.location.origin, session.sessionKey);
  if (connected.client === null) {
    root.render(<OfflineScreen diagnostic={connected.diagnostic} />);
    return;
  }
  const client = connected.client;
  const shell = createShell(client);
  const settings = await loadSettings(client);
  shell.applySettings(settings.result ?? { values: {}, sources: [] });
  const commands = await loadCommands(client);
  shell.stores.commands.set(commands.commands);
  const plugins = await loadPlugins(client);
  shell.stores.plugins.set(plugins.plugins);
  const uis = await registerShellUis(plugins.plugins);
  shell.loadedPluginIds = uis.loadedIds;
  reportAll(shell, [settings.diagnostic, commands.diagnostic, plugins.diagnostic, ...uis.diagnostics]);
  reportAll(shell, [versionProblem(client.daemonVersion)]);
  shell.start();
  root.render(<App shell={shell} />);
}

/**
 * Shows each Diagnostic that isn't null as a notice.
 * @param {import("./app/shell.js").Shell} shell
 * @param {(Diagnostic | null)[]} diagnostics
 */
function reportAll(shell, diagnostics) {
  for (const diagnostic of diagnostics) {
    if (diagnostic !== null) {
      shell.notify(diagnostic);
    }
  }
}

main();
