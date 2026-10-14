// The core sections' UI (layout frontend-and-plugins.md 8.2): one section and one panel per section, registered as the
// pseudo plugin "core". A section whose page isn't built yet shows the honest "not available yet" state with its code
// (RRS-C0010), never fake data. Home, Settings, Plugins and Remote are built in this version.

import { CORE_PLUGIN_ID } from "@rastai/plugin";

import { HomeSection } from "../sections/home/HomeSection.jsx";
import { NotAvailableSection } from "../sections/NotAvailableSection.jsx";
import { PluginsSection } from "../sections/plugins/PluginsSection.jsx";
import { RemoteSection } from "../sections/remote/RemoteSection.jsx";
import { SettingsSection } from "../sections/settings/SettingsSection.jsx";
import { SectionPage } from "../workspace/SectionPage.jsx";

/** @typedef {import("@rastai/plugin").PluginUi} PluginUi */

/** Every section the core registers, Home first. A section id is a contracts 15.6 name. */
export const CORE_SECTION_IDS = [
  "home",
  "inference",
  "training",
  "designer",
  "data",
  "gym",
  "remote",
  "plugins",
  "settings",
];

/**
 * The core UI: each section's page, and the panel that holds each section's body (its id is `core.<section>`).
 * @returns {PluginUi}
 */
export function coreUi() {
  return {
    pluginId: CORE_PLUGIN_ID,
    sections: CORE_SECTION_IDS.map((sectionId) => ({
      id: sectionId,
      render: () => <SectionPage sectionId={sectionId} />,
    })),
    panels: CORE_SECTION_IDS.map((sectionId) => ({
      id: `${CORE_PLUGIN_ID}.${sectionId}`,
      render: () => panelBody(sectionId),
    })),
    lensViews: [],
    metricViews: [],
    settingsPages: [],
  };
}

/**
 * The body of a section's panel: the built sections' pages, or the not-available state for a section not built yet.
 * @param {string} sectionId
 * @returns {import("react").ReactNode}
 */
function panelBody(sectionId) {
  if (sectionId === "home") {
    return <HomeSection />;
  }
  if (sectionId === "settings") {
    return <SettingsSection />;
  }
  if (sectionId === "plugins") {
    return <PluginsSection />;
  }
  if (sectionId === "remote") {
    return <RemoteSection />;
  }
  return <NotAvailableSection sectionId={sectionId} />;
}
