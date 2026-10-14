// Settings (ui-design 6.8; layout frontend-and-plugins.md 8.2): the Appearance page and the API tokens page. Every change
// is settings.set through the shell, so the window and the other windows agree (2.3).

import { AppearancePage } from "./AppearancePage.jsx";
import { TokensPage } from "./TokensPage.jsx";

export function SettingsSection() {
  return (
    <div className="rr-settings">
      <h2 className="rr-section-title">Settings</h2>
      <AppearancePage />
      <TokensPage />
    </div>
  );
}
