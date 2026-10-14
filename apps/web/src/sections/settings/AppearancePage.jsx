// Settings > Appearance (ui-design 6.8). Each theme card is a miniature drawn with that theme's own tokens, so the choice
// shows before it is made. A change is applied at once through the shell (settings.set). The setting keys show in
// Advanced only; Basic says what each control does.

import { AdvancedOnly, Checkbox, SegmentedControl, builtInTheme } from "@rastai/ui";
import { useStore } from "@rastai/plugin";

import { useShell } from "../../app/shell-context.js";
import { globalMode } from "../../state/mode.js";
import { readSetting } from "../../state/settings.js";
import { themeNameFor } from "../../state/theme.js";

const THEME_CHOICES = [
  { value: "system", label: "System", note: "Light or dark, as your computer is set." },
  { value: "light", label: "Light", note: "Dark text on a light page." },
  { value: "dark", label: "Dark", note: "Light text on a dark page." },
  { value: "high_contrast", label: "High contrast", note: "Stronger text and borders." },
];
const DENSITY_CHOICES = [
  { value: "comfortable", label: "Comfortable", note: "The usual spacing." },
  { value: "compact", label: "Compact", note: "Tighter rows and panels, so more fits on screen." },
];
const MODE_OPTIONS = [
  { value: "basic", label: "Basic" },
  { value: "advanced", label: "Advanced" },
];
const DEFAULT_DENSITY = "comfortable";
const DARK_SCHEME_QUERY = "(prefers-color-scheme: dark)";
// The series a chart draws in, in order: the live preview of the colorblind choice shows these.
const SERIES_ROLES = ["series_1", "series_2", "series_3", "series_4", "series_5", "series_6", "series_7", "series_8"];

export function AppearancePage() {
  const shell = useShell();
  const settings = useStore(shell.stores.settings).values;
  const themeSetting = readSetting(settings, "ui.theme", "system");
  const density = readSetting(settings, "ui.density", DEFAULT_DENSITY);
  const reducedMotion = readSetting(settings, "ui.reduced_motion", false) === true;
  const colorblind = readSetting(settings, "ui.colorblind_palette", false) === true;
  const mode = globalMode(settings);
  const prefersDark = window.matchMedia(DARK_SCHEME_QUERY).matches;
  const previewTokens = builtInTheme(themeNameFor(themeSetting, prefersDark), {
    density: density === "compact" ? "compact" : "comfortable",
    reducedMotion: reducedMotion,
    colorblind: colorblind,
  });

  return (
    <section className="rr-settings-group" aria-labelledby="appearance-title">
      <h3 id="appearance-title" className="rr-settings-group__title">
        Appearance
      </h3>
      <fieldset className="rr-settings-field">
        <legend>Theme</legend>
        <AdvancedOnly>
          <p className="rr-setting-key">
            <code>ui.theme</code>
          </p>
        </AdvancedOnly>
        <div className="rr-theme-cards">
          {THEME_CHOICES.map((choice) => (
            <ThemeCard
              key={choice.value}
              choice={choice}
              selected={themeSetting === choice.value}
              onChoose={() => shell.setSetting("ui.theme", choice.value)}
              colors={
                builtInTheme(themeNameFor(choice.value, prefersDark), {
                  density: "comfortable",
                  reducedMotion: false,
                  colorblind: false,
                }).colors
              }
            />
          ))}
        </div>
      </fieldset>

      <fieldset className="rr-settings-field">
        <legend>Density</legend>
        <AdvancedOnly>
          <p className="rr-setting-key">
            <code>ui.density</code>
          </p>
        </AdvancedOnly>
        {DENSITY_CHOICES.map((choice) => (
          <label key={choice.value} className="rr-radio-row">
            <input
              type="radio"
              name="density"
              value={choice.value}
              checked={density === choice.value}
              onChange={() => shell.setSetting("ui.density", choice.value)}
            />
            <span>
              {choice.label}
              <span className="rr-settings-note">{` ${choice.note}`}</span>
            </span>
          </label>
        ))}
      </fieldset>

      <div className="rr-settings-field">
        <AdvancedOnly>
          <p className="rr-setting-key">
            <code>ui.reduced_motion</code>
          </p>
        </AdvancedOnly>
        <Checkbox
          label="Reduce motion"
          checked={reducedMotion}
          onChange={(checked) => shell.setSetting("ui.reduced_motion", checked)}
          help="Also on when your computer asks for reduced motion."
        />
      </div>

      <div className="rr-settings-field">
        <AdvancedOnly>
          <p className="rr-setting-key">
            <code>ui.colorblind_palette</code>
          </p>
        </AdvancedOnly>
        <Checkbox
          label="Colorblind-safe chart colors"
          checked={colorblind}
          onChange={(checked) => shell.setSetting("ui.colorblind_palette", checked)}
          help="Charts use a palette that stays apart for the most common kinds of color blindness."
        />
        <div className="rr-series-preview" role="img" aria-label="Chart colors, in order">
          {SERIES_ROLES.map((role) => (
            <span key={role} className="rr-series-preview__swatch" style={{ background: previewTokens.colors[role] }} />
          ))}
        </div>
      </div>

      <fieldset className="rr-settings-field">
        <legend>Interface</legend>
        <AdvancedOnly>
          <p className="rr-setting-key">
            <code>ui.mode</code>
          </p>
        </AdvancedOnly>
        <SegmentedControl
          label="Interface mode"
          options={MODE_OPTIONS}
          value={mode}
          onChange={(value) => shell.setSetting("ui.mode", value)}
        />
      </fieldset>
    </section>
  );
}

/**
 * One theme as a radio card with a miniature in its own colors.
 * @param {object} props
 * @param {{ value: string, label: string, note: string }} props.choice
 * @param {boolean} props.selected
 * @param {() => void} props.onChoose
 * @param {Object<string, string>} props.colors   the theme's color roles, as hex
 */
function ThemeCard({ choice, selected, onChoose, colors }) {
  return (
    <label className={selected ? "rr-theme-card rr-theme-card--selected" : "rr-theme-card"}>
      <input type="radio" name="theme" value={choice.value} checked={selected} onChange={onChoose} />
      <span
        className="rr-theme-card__preview"
        aria-hidden="true"
        style={{ background: colors.surface, borderColor: colors.border }}
      >
        <span className="rr-theme-card__bar" style={{ background: colors.accent }} />
        <span className="rr-theme-card__line" style={{ background: colors.text }} />
        <span className="rr-theme-card__line rr-theme-card__line--short" style={{ background: colors.text_muted }} />
      </span>
      <span className="rr-theme-card__name">{choice.label}</span>
      <span className="rr-settings-note">{choice.note}</span>
    </label>
  );
}
