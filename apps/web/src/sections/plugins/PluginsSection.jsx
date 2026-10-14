// Plugins (ui-design 6.8; layout frontend-and-plugins.md 8.2): the installed plugins, their state and what they were
// granted. This version lists them only; installing and reviewing come with the plugin work. Advanced adds the permission
// names the manifest asks for.

import { AdvancedOnly, DiagnosticView, StatePill } from "@rastai/ui";
import { useStore } from "@rastai/plugin";

import { useShell } from "../../app/shell-context.js";
import { firstProblem, pluginTitle, sourceWord, stateTone, trustLine } from "./plugin-view.js";

export function PluginsSection() {
  const shell = useShell();
  const plugins = /** @type {any[]} */ (useStore(shell.stores.plugins));
  return (
    <div className="rr-plugins">
      <h2 className="rr-section-title">Plugins</h2>
      {plugins.length === 0 ? (
        <p className="rr-settings-note">No plugins are installed.</p>
      ) : (
        <ul className="rr-card-list" aria-label="Installed plugins">
          {plugins.map((plugin) => (
            <PluginRow key={plugin.plugin_id} plugin={plugin} />
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * @param {object} props
 * @param {any} props.plugin   a PluginSummary
 */
function PluginRow({ plugin }) {
  const problem = firstProblem(plugin);
  return (
    <li className="rr-card">
      <h3 className="rr-card__title">{pluginTitle(plugin)}</h3>
      <p className="rr-settings-note">{`Version ${plugin.version} · ${sourceWord(plugin.source)}`}</p>
      <p>{trustLine(plugin)}</p>
      <StatePill label={plugin.state} tone={stateTone(plugin.state)} />
      {problem ? <DiagnosticView diagnostic={problem} /> : null}
      <AdvancedOnly>
        <p className="rr-setting-key">{`plugin id ${plugin.plugin_id}`}</p>
        <ul className="rr-permission-words">
          {
            /** @type {string[]} */ (plugin.manifest?.permissions ?? []).map((permission) => (
              <li key={permission}>
                <code>{permission}</code>
              </li>
            ))
          }
        </ul>
      </AdvancedOnly>
    </li>
  );
}
