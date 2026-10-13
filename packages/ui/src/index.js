// @rastai/ui: the design system (layout frontend-and-plugins.md 4). Theme tokens, the Basic/Advanced primitives,
// components and the diagnostics view. The security components are not here: they are exported only from
// @rastai/ui/security, which only the app shell may import (2.10). The page loads styles/base.css and
// styles/components.css once; both read only --rr-* variables.

export { applyThemeTokens } from "./tokens/apply-theme.js";
export { canvasColors } from "./tokens/canvas-colors.js";
export { LOCKED_TOKENS } from "./tokens/locked.js";
export { builtInTheme } from "./tokens/resolve-tokens.js";
export { vizColors } from "./tokens/viz-colors.js";

export { AdvancedOnly } from "./mode/AdvancedOnly.jsx";
export { ModeProvider } from "./mode/ModeProvider.jsx";
export { useMode } from "./mode/use-mode.js";

export { Button } from "./components/Button.jsx";
export { Checkbox } from "./components/Checkbox.jsx";
export { CommandPaletteView } from "./components/CommandPaletteView.jsx";
export { Dialog } from "./components/Dialog.jsx";
export { FieldForm } from "./components/FieldForm.jsx";
export { NumberField } from "./components/NumberField.jsx";
export { PanelChrome } from "./components/PanelChrome.jsx";
export { SegmentedControl } from "./components/SegmentedControl.jsx";
export { Select } from "./components/Select.jsx";
export { StatePill } from "./components/StatePill.jsx";
export { StatTile } from "./components/StatTile.jsx";
export { Table } from "./components/Table.jsx";
export { TextField } from "./components/TextField.jsx";
export { Toast } from "./components/Toast.jsx";
export { dismissToast, showToast } from "./components/show-toast.js";

export { DiagnosticView } from "./diagnostics/DiagnosticView.jsx";

/** @typedef {import("./tokens/token-types.js").ThemeTokens} ThemeTokens */
/** @typedef {import("./tokens/token-types.js").TypographyTokens} TypographyTokens */
/** @typedef {import("./tokens/token-types.js").ThemeOptions} ThemeOptions */
/** @typedef {import("./tokens/token-types.js").BuiltInThemeName} BuiltInThemeName */
/** @typedef {import("./mode/mode-types.js").UiMode} UiMode */
/** @typedef {import("./components/FieldForm.jsx").FieldSpec} FieldSpec */
/** @typedef {import("./components/CommandPaletteView.jsx").PaletteEntry} PaletteEntry */
/** @typedef {import("./components/PanelChrome.jsx").ModeOverride} ModeOverride */
/** @typedef {import("./components/PanelChrome.jsx").PanelMenuItem} PanelMenuItem */
/** @typedef {import("./components/SegmentedControl.jsx").SegmentOption} SegmentOption */
/** @typedef {import("./components/Select.jsx").SelectOption} SelectOption */
/** @typedef {import("./components/StatePill.jsx").StateTone} StateTone */
/** @typedef {import("./components/Table.jsx").TableColumn} TableColumn */
/** @typedef {import("./components/show-toast.js").ToastEntry} ToastEntry */
