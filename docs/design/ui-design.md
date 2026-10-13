# Rastai Studio UI design, version 1.0 ("Instrument")

**Status:** version 1.0, 2026-10-13. Approved by the owner: every owner decision of 2026-10-13 is recorded in §8, and §9
is the build plan. The visual reference is the owner's canvas (nine screen boards plus "Type options" and "Keyboard
shortcuts"), https://claude.ai/artifact/G5H4BFoT9F64VcpVgr8GaB. That canvas is private to the owner; this document copies
nothing from it. The second-window sign-in approval is recorded in ADR-0039.

## 0. How to read this document

- **Names.** Every name here is in `contracts.md` v1.2 unless it is marked **proposal**. §10 lists each proposal, the
  place it is used and the contract change that will register it. A proposal stays a proposal until that change lands,
  even when the owner has approved the design it belongs to (ADR-0038).
- **Layout names.** Names such as `PanelChrome` or `SidebarNav` come from the frontend layout doc
  (`docs/architecture/layout/frontend-and-plugins.md`). Where a layout name is not yet in contracts, §10 marks it.
- **Decisions.** "Approved" means the owner approved it on the date given in §8, in the owner's words where quoted.
- **Values.** Token values in §4 are the design's values. Where the code on a branch differs, §4 says "now".

Sources: spec v5.4 §1–3, §6–11, §13; layout `frontend-and-plugins.md` §2, §4, §8; contracts §5.6, §6.3, §6.4, §7.3, §9, §11.2, §15.6;
`schemas/rastai.theme/1.json`; the M1-51 branch tokens (`packages/ui/src/tokens/*`, `security/locked-styles.js`); `catalog/errors/*.json`.

---

## 1. The look in one paragraph

**Instrument.** Rastai should feel like a well-made lab instrument: a calm graphite casing, and bright readouts where the
information is. The chrome (shell, panels, controls) is neutral graphite on near-white or near-black, with one cobalt
accent for "the thing you act on next" and for live state. Everything else that carries color is data (chart series,
heatmaps, token confidence) or a state (success, the locked error and warning colors). Type is Geist for the
interface and Geist Mono for numbers, ids, shapes and token text (chosen 2026-10-13, §4.5), so a metric or a tensor
shape always reads as data. Panels are flat with 1 px borders and 10 px corners; shadows appear only on things that float (menus, the
palette, dialogs). Density is real: a 32 px control grid by default and 24 px in Compact, both meeting WCAG 2.2.

## 2. Principles

1. **Quiet chrome, loud data.** Color is spent on data, state and the one primary action per view. No decorative gradients.
2. **Numbers are mono.** Every metric, id, shape, step count and token is in the mono face with tabular figures, so columns align and data never looks like prose.
3. **Never color alone.** Every state has an icon and a word; every chart has a legend and "Show as table"; colors that must be told apart also differ in lightness.
4. **Advanced only adds.** Basic and Advanced show the same blocks in the same order; Advanced appends detail after or beside them (frontend 2.4). The Training board shows both side by side.
5. **The UI shows, the daemon decides.** Every verdict on screen ("still improving", "fits", "unsure") comes from a command or event. Where none exists yet, it is listed in section 7.
6. **Security looks different on purpose.** The locked prompts use their own palette, the system font, fixed sizes and a heavy 2 px border. No theme, density or plugin can change them.
7. **Every token is a role.** No color, size, radius, shadow or duration literal outside `@rastai/ui/src/tokens/`; the boards themselves are built only on `--rr-*` variables.

---

## 3. Information architecture

### 3.1 Sections and navigation

Left section rail (`SidebarNav`), 200 px, icon plus label, in three groups:

| Group | Sections (ids from contracts §15.6) |
|---|---|
| (top) | Home (`home`) |
| Studio | Chat in Basic / Inference in Advanced (`inference`; approved 2026-10-13), Psychology (`psychology`, plugin), Training (`training`), Evaluation (`eval`, plugin), Designer (`designer`), Data (`data`), Agent Gym (`gym`) |
| System | Remote (`remote`), Plugins (`plugins`), Settings (`settings`) |

- The current section is `aria-current="page"`, drawn with the `selection` fill and an accent icon (no left-border stripe).
- A section a plugin provides appears when the plugin is enabled; a missing one shows nothing in the rail (Home's card says why).
- A small mono count beside a section shows live jobs in it (Training "1"), from `job.state_changed`.
- Collapsible to a 56 px icon rail (Phase 2 nicety; tooltip labels).

### 3.2 Top bar (one row, 48 px)

Left to right: logo (links Home) · project switcher (`project.open`, M3) · breadcrumb of the current route ·
**palette entry** (a search-field-shaped button, "Search or run a command  Ctrl K") · **global Basic/Advanced switch**
(segmented control, `settings.set ui.mode`). On the desktop shell the native title bar stays native (M1-55).

### 3.3 Workspace model

- One layout tree per section (frontend 2.6): splits and tab groups of panels. Every panel has `PanelChrome`: title,
  the per-panel mode override chip ("Advanced · panel override" or "follows global"), and the panel menu (move right,
  bottom, new tab, open in new window, reset).
- Side docks hold cross-section panels (`psychology.live` beside chat, `core.lineage`).
- **Windows:** `view.window.open` puts the current route in a second window (Training's "Open in new window").
- Layouts are files (`workspace.layout.save/apply`, M6).

### 3.4 Where Basic/Advanced lives

Global: the top-bar switch. Per panel: the override chip in `PanelChrome`. Per palette: the palette's own
Basic/Advanced filter (it mirrors the effective mode, and "N more in Advanced" reveals the rest). Settings shows the
setting keys (`ui.theme`, …) only in Advanced.

### 3.5 Status bar (30 px, always visible)

Left to right: connection (`Connected`, `Reconnecting…`, `Offline`; statuses from contracts §15.6, dot plus word) ·
node summary (`local · RTX 5080 · GPU 54% · VRAM 4.6 / 16 GiB`, from `node.heartbeat`) · running jobs (mini progress
plus name, from `job.state_changed` and `job.progress`; click opens the job) · spacer · **egress badge** (locked, from
`egress.active`, held 10 s) · version. It wraps to two lines at phone width; the egress badge never hides.

### 3.6 Command palette

Ctrl/Cmd-K anywhere, or the top-bar entry. Two panes: the result list (Recent, Commands, Views groups; subsequence match
underlined; shortcut keycaps; "needs input" for commands with required arguments) and the detail pane for the
highlighted entry (title, id, summary, scope, kind, section; the generated `CommandForm` when arguments are needed,
prefilled from context; "Copy as" CLI/Python/JS/MCP from `copy-as.js`). Context chips under the input show which ids
will prefill (`run_id = …`). Footer: keyboard hints and the match count.

### 3.7 Keyboard shortcuts

One table for the whole app (also on the "Keyboard shortcuts" board). Ctrl means Cmd on macOS. "Existing" = already in
the spec or layout docs; "proposed" = new in this design. Every binding is a default: users rebind them in
Settings › Keybindings (`workspace.keybindings.set`), and a binding's `when` uses the context keys of contracts §15.6.

| Key | Action | Where it works | Status | Conflicts |
|---|---|---|---|---|
| Ctrl K | Open the command palette | anywhere, also inside text fields | existing (spec §7) | Browser: Chrome and Firefox use Ctrl K for the search bar; the page takes it first. Desktop shell: none. |
| Esc | Close palette, dialog or menu; focus goes back | any overlay | existing (layout 2.5) | none |
| Shift ? | Show the shortcut list | outside text fields | proposed | none |
| G then H / C / T / D / P / S | Go to Home, Chat, Training, Designer, Psychology, Settings | outside text fields | proposed | none (inactive while typing) |
| F6 | Move focus between regions: rail, panels, status bar | anywhere | proposed | Browser tab: Chrome and Firefox may send F6 to the address bar. Desktop shell: none. |
| Ctrl + / − / 0 | Zoom the interface | anywhere | existing (browser) | Desktop shell must implement it (M1-55). |
| ↑ ↓ | Move through results | palette | existing (layout 4.1) | none |
| Enter | Run the highlighted command, or open its arguments form | palette | existing (layout 2.5) | none |
| Tab | Move into the arguments form | palette, command needs input | proposed | none |
| Ctrl . | Copy the highlighted command as CLI | palette | decided 2026-10-13 (revisitable) | None known in Chrome or Firefox. Replaces Ctrl Shift C, which opens the DevTools element picker. |
| Ctrl Enter | Send the message (Enter adds a new line) | chat composer | proposed | none |
| ← → | Move token by token through an answer | a focused answer | proposed | none (same model as the timeline) |
| Enter | Inspect the focused token | a focused answer | proposed | none |
| ← → | Previous or next token | Psychology timeline | existing (layout 2.9) | none |
| ↑ ↓ | Previous or next layer | Psychology timeline | existing (layout 2.9) | none |
| Enter | Token details | Psychology timeline | existing (layout 2.9) | none |
| Space | Play or pause the replay | Psychology timeline | existing (layout 2.9) | Page scroll; taken only while the timeline has focus. |
| Ctrl S | Save a checkpoint now (`train.checkpoint` `default_shortcut`) | `section=training` | proposed | Browser Save page; taken only in Training. |
| ← → | Move the chart crosshair; values are announced | a focused chart | proposed | none |
| Ctrl Z | Undo (`document.undo`) | `focus=canvas` | proposed | Text fields keep their own undo. |
| Ctrl Shift Z (Ctrl Y on Windows and Linux) | Redo (`document.redo`) | `focus=canvas` | proposed | Text fields keep their own redo. |
| Arrows | Walk the graph through the offscreen tree mirror | `focus=canvas` | existing (layout 2.9) | none |
| Delete | Remove the selected node | `focus=canvas` | proposed | none |
| + / − / 0 | Zoom the canvas in, out, fit | `focus=canvas` | proposed | none |
| Esc | Deny or cancel | locked security prompts | proposed | none |
| Enter | Activates only the focused button; focus starts on Deny or Cancel | locked security prompts | proposed | Deliberate: Enter never approves by default. |

---

## 4. Tokens

All keys are the existing `rastai.theme/1` role names. "Now" = the value on branch `m1/M1-51`. HC = the
High-contrast built-in (base `dark`), approved on 2026-10-13 to ship now, revisited in M8. The palette, including the
cobalt accent and the blue/orange diverging default, was approved as designed on 2026-10-13. Contrast ratios are WCAG,
against `surface`.

### 4.1 Color roles

| Role | Light | Dark | HC | Light now | Dark now | Note |
|---|---|---|---|---|---|---|
| background | #f3f4f6 | #0d0f13 | #000000 | #ffffff | #101318 | shell gutter |
| surface | #ffffff | #14171c | #0b0b0b | #f6f8fa | #171b22 | panels |
| surface_raised | #ffffff | #1b1f26 | #161616 | #ffffff | #1f242d | menus, popovers |
| surface_sunken | #eef0f3 | #090b0e | #000000 | #eef1f4 | #0b0d11 | wells, code, tracks |
| overlay | #0b0d12 | #000000 | #000000 | #0b0d11 | #000000 | scrim at 50–55% |
| text | #16181d (17.8:1) | #e7e9ee (14.8:1) | #ffffff (19.7:1) | #1a1f26 | #e6ebf2 | |
| text_muted | #59606e (6.3:1) | #9ea5b2 (7.3:1) | #d9d9d9 (13.9:1) | #57606a | #9fb3c8 | |
| text_disabled | #9aa0ab (2.6, exempt) | #5b616c (2.9, exempt) | #8c8c8c (5.9:1) | #8c959f | #5c6878 | |
| text_on_accent | #ffffff (6.1:1 on accent) | #0b0e1a (8.0:1) | #000000 (11.5:1) | #ffffff | #0b0d11 | |
| border | #d7dae1 | #282d36 | #9a9a9a | #d0d7de | #2a313c | panel edges (decorative) |
| border_strong | #737a88 (4.3:1) | #6b7382 (3.8:1) | #ffffff | #6e7781 | #6b7a8e | input and control outlines (UI 3:1) |
| divider | #e6e8ed | #1f232a | #6e6e6e | #e4e8ec | #222831 | rows |
| accent | #3a56d4 (6.1:1) | #8ea2ff (7.5:1) | #9fc0ff (10.7:1) | #0a66d8 | #5aa2ff | cobalt |
| accent_hover | #2f47b8 | #a8b8ff | #c2d6ff | #0857ba | #7ab4ff | |
| accent_active | #263a99 | #7189f2 | #7fa8ff | #06479a | #3d8bf0 | |
| selection | #e0e6ff | #232c52 | #1c3a73 | #d6e8ff | #1e3a5f | text on it ≥ 11:1 |
| focus_ring | #3a56d4 (6.1:1) | #a8b8ff (9.4:1) | #4de2ff (12.8:1) | #0a66d8 | #8cc4ff | |
| success | #17784b (5.5:1) | #4bc386 (8.1:1) | #5fe39a (12.1:1) | #1a7f37 | #3fb950 | |
| series_1…8 | unchanged | unchanged | = Dark | | | they pass palettes.test today |
| sequential_low / high | unchanged | unchanged | #1b2a3f / #cfe4ff | | | |
| diverging_negative | #c75300 | #f0883e | #ffb070 | #cf222e | #ff7b72 | **blue/orange by default** |
| diverging_zero | #eef0f3 | #262a32 | #333333 | #eef1f4 | #2a2f38 | |
| diverging_positive | #0a66d8 | #5aa2ff | #9fc0ff | #1a7f37 | #3fb950 | |
| canvas_background | #f3f4f6 | #0d0f13 | #000000 | #f6f8fa | #101318 | |
| canvas_grid | #dfe2e8 | #1a1e25 | #262626 | #e4e8ec | #1c222b | dots, 20 px |
| canvas_node | #ffffff | #1b1f26 | #0b0b0b | #c9d6e6 | #50648c | nodes are cards |
| canvas_node_border | #737a88 (3.9:1) | #6b7382 (4.0:1) | #ffffff | #3d4b5c | #d9e6f2 | |
| canvas_node_selected | #3a56d4 | #8ea2ff | #9fc0ff | #0a66d8 | #5aa2ff | |
| canvas_edge | #7b8290 (3.4:1) | #5d6574 (3.3:1) | #b3b3b3 | #6e7781 | #6b8099 | |
| canvas_edge_active | #3a56d4 | #a8b8ff | #4de2ff | #0a66d8 | #8cc4ff | |
| canvas_label | #16181d | #e7e9ee | #ffffff | #1a1f26 | #f2f7ff | |
| canvas_port | #59606e | #9ea5b2 | #d9d9d9 | #57606a | #9fb3c8 | |

Board-only note: the canvas grid on the Designer board uses slightly stronger dots (#cfd3db / #262b34 / #3a3a3a) so the
grid reads at screenshot scale; the token values above are the proposal.

**Locked tokens: unchanged** (`LOCKED_TOKENS` light and dark). Checked against the new surfaces: severity-error 6.6:1 / 6.5:1,
severity-warning 4.9:1 / 11.3:1, severity-info 6.3:1 / 8.4:1, egress text on egress 7.4:1 / 12.8:1, security text on
security surface 17.8:1 / 15.9:1, security danger on security surface 6.3:1 / 7.6:1. HC uses the dark set.

**Colorblind palette: unchanged** (`COLORBLIND_COLORS`).

### 4.2 Typography

| Key | Value | Now |
|---|---|---|
| font_family_ui | `"Geist", "Noto Sans", system-ui, sans-serif` | system-ui stack with Roboto |
| font_family_mono | `"Geist Mono", "DejaVu Sans Mono", ui-monospace, monospace` | ui-monospace stack |
| font_weights | regular 400, medium 500, **bold 600** | 400 / 500 / 700 |
| font_sizes_px | xs 11, sm 12, md 14, lg 16, xl 20 | same |
| line_heights | body 1.5, heading 1.25, **code 1.45** | 1.5 / 1.25 / 1.4 |

Use: xl = section and page titles; lg = panel titles; md = body and chat; sm = labels, controls, table cells; xs =
captions, status bar, eyebrows (uppercase, +0.06em). Numbers always mono with `font-variant-numeric: tabular-nums`.
The locked security UI keeps its own fixed system-font stack (locked-styles.js), never the theme's fonts.
Geist and Geist Mono are bundled with the app (files and license in §4.5), loaded with `@font-face` in `base.css`;
nothing is fetched at runtime. The fallbacks cover the moment before the files load and any script Geist lacks.

### 4.3 Spacing (`density`), radius, elevation, focus, motion

| Token | Value | Now |
|---|---|---|
| density.comfortable.spacing_px | 0, 4, 8, 12, 16, 24, 32, 48 | same |
| density.compact.spacing_px | 0, 2, 4, 8, 12, 16, 24, 32 | same |
| radius_px | small **4** (keycaps, chips), medium **6** (controls), large **10** (panels, dialogs, pills) | 2 / 4 / 8 |
| elevation.level_1 (light) | `0 1px 2px rgba(17,20,26,.08)` | `0 1px 2px rgba(11,13,17,0.12)` |
| elevation.level_2 (light) | `0 4px 12px rgba(17,20,26,.10), 0 1px 2px rgba(17,20,26,.06)` | `0 2px 8px …0.16` |
| elevation.level_3 (light) | `0 16px 40px rgba(17,20,26,.18), 0 2px 6px rgba(17,20,26,.08)` | `0 8px 24px …0.2` |
| elevation (dark) | `0 1px 2px rgba(0,0,0,.5)` · `0 4px 12px rgba(0,0,0,.45)` · `0 16px 40px rgba(0,0,0,.6)` | similar |
| elevation (HC) | outlines instead of shadows: `0 0 0 1px #9a9a9a` · `0 0 0 1px #ffffff` · `0 0 0 2px #ffffff` | n/a |
| focus_ring | width 2 (HC 3), offset 2 | 2 / 2 |
| motion | fast 120 ms (hover, press, toggles), slow 240 ms (panels, dialogs, palette), easing `cubic-bezier(0.2, 0, 0, 1)` | same |

Control height is `--rr-space-6`: 32 px comfortable, 24 px compact (the WCAG 2.2 target minimum). No new token needed.

### 4.4 New token keys (approved 2026-10-13; proposal until the schema change lands in its own contract PR)

| Key | Light / Dark / HC | Why |
|---|---|---|
| `typography.font_sizes_px.xxl` | 28 | Home's heading and empty-state headings; xl 20 is too small for a page's one headline. |
| `colors.confidence_tint` | #c75300 / #f0883e / #ffb070 | Chat token shading mixes this into `surface` at 16 / 30 / 46%. A dedicated role stops the colorblind swap or a theme's series order from changing what "unsure" looks like. |

Nothing is renamed.

### 4.5 Type: Geist + Geist Mono (chosen 2026-10-13)

**Chosen:** pairing A, **Geist + Geist Mono**. Owner, 2026-10-13: "A text schema is fine we can revisit later if needed".

| | Geist + Geist Mono |
|---|---|
| License | SIL Open Font License 1.1. Copyright 2024 The Geist Project Authors (https://github.com/vercel/geist-font). No Reserved Font Name, so subsetting and format conversion are allowed. |
| Official source | github.com/vercel/geist-font (repository license OFL-1.1); the same files are in Google Fonts as `ofl/geist` and `ofl/geistmono`. |
| Files to bundle | `fonts/Geist/webfonts/Geist[wght].woff2` (variable, every weight we use) · `fonts/Geist/webfonts/Geist-Italic[wght].woff2` · `fonts/GeistMono/webfonts/GeistMono[wght].woff2` · `OFL.txt` (the license text, shipped beside the files) |
| Where they go | `packages/ui/src/fonts/` (a new path, added to contracts in the same PR, ADR-0038), referenced by `@font-face` in `styles/base.css` |
| Weights used | 400, 500, 600 from the variable files (`font_weights` regular / medium / bold) |
| Optional later | Subset to Latin plus Latin-1 to shrink the files; allowed because there is no Reserved Font Name. |

**Revisit later:** the owner may revisit the choice. B and C stay documented below and stay on the "Type options" board.
A swap changes only the bundled files, `OFL.txt` and the two `font_family_*` token values.

How the three were compared. IBM Plex was rejected on 2026-10-13 ("something a bit more stylish please. not over the
top but not 1950's mainframe either"). Three pairings, each a UI sans plus a matching mono from the same family, are on
the "Type options" board (Foundations page). That board shows the same slice of UI in each: headings, a run row with
numbers, a stat tile, a shaded chat line, a button row and the status-bar line. Licenses were checked in each family's
official repository and in Google Fonts' `ofl/` folders on 2026-10-13.

| | A · Geist + Geist Mono (**chosen**) | B · DM Sans + DM Mono | C · Red Hat Text + Red Hat Mono |
|---|---|---|---|
| Feel | Crisp and contemporary, tall x-height; the mono has quiet character | Soft geometric and warm; the most personality | Humanist and drawn for small text; friendly but serious |
| At 12–14 px | Best of the three: open shapes, clear 1/l/I, tabular figures | Good; the optical-size axis keeps small text open; slightly rounder and wider | Very good: designed for text sizes |
| License | SIL OFL 1.1, © 2024 The Geist Project Authors; no Reserved Font Name | SIL OFL 1.1, © 2014 The DM Sans Project Authors and © 2020 The DM Mono Project Authors; no Reserved Font Name | SIL OFL 1.1, © 2021 Red Hat, Inc., **with Reserved Font Name "Red Hat"** |
| Official source | github.com/vercel/geist-font (repo license OFL-1.1) | github.com/googlefonts/dm-fonts, github.com/googlefonts/dm-mono | github.com/RedHatOfficial/RedHatFont |
| Files to bundle | `fonts/Geist/webfonts/Geist[wght].woff2`, `Geist-Italic[wght].woff2`, `fonts/GeistMono/webfonts/GeistMono[wght].woff2`, `OFL.txt` | `Sans/fonts/webfonts/DMSans[opsz,wght].woff2`, `DMSans-Italic[opsz,wght].woff2`, `exports/DMMono-Regular.ttf`, `DMMono-Medium.ttf` (converted to woff2 once, at build), `OFL.txt` | `fonts/Proportional/RedHatText/webfonts/RedHatText-VF.woff2`, `RedHatText-Italic-VF.woff2`, `fonts/Mono/RedHatMono/webfonts/RedHatMono-VF.woff2`, `OFL.txt` |
| Catches | Vercel's house face, so less unique | DM Mono has only 300/400/500 and ships no woff2 upstream | The Reserved Font Name means any subset or conversion we make must be renamed; we must ship the upstream woff2 unmodified. Red Hat brand association |

Why A: it reads best in dense 11–14 px panels and its figures are clean for metrics. The mono has enough character to
make ids and tokens feel designed. Upstream ships variable woff2 files, so three files cover every weight we use. For
a revisit, B is the pick for more warmth and C if legibility at 11 px matters most.

---

## 5. Components (mapped to `@rastai/ui`)

### 5.1 States every interactive component has

| State | Treatment |
|---|---|
| hover | fill `surface_sunken` (secondary, ghost, rows) or `accent_hover` (primary); 120 ms |
| focus | 2 px `focus_ring` outline, 2 px offset, keyboard only (`:focus-visible`) |
| pressed / selected | `accent_active` (primary); `selection` fill (rows, nav, options); segmented: raised `surface` plus level 1 |
| disabled | `text_disabled` text, `border` outline, no hover, `cursor: not-allowed`; still focusable when it explains itself (tooltip) |
| error | outline and message in `--rr-locked-severity-error`, message carries its RRS code; `aria-invalid`, `aria-describedby` |
| loading | spinner glyph plus a verb ("Compiling…"), `aria-busy`; layout does not shift |

### 5.2 Inventory

Exists on M1-51: Button, Checkbox, Select, TextField, NumberField, FieldForm, Table, Dialog, Toast (+ showToast,
dismissToast), CommandPaletteView, PanelChrome, DiagnosticView, ModeProvider, useMode, AdvancedOnly; security:
EgressBadge, TrustPrompt, PermissionList, SignedOutScreen.

Planned in layout 4.1, not built yet (used by the boards): IconButton, Slider, Toggle, RadioGroup, Tabs, SplitView, Menu,
Tooltip, Banner, Badge, ProgressBar, Spinner, EmptyState, KeyHint, VirtualList, Tree, FileDropZone, ChartFrame,
SidebarNav, StatusBarView, DiagnosticList, TermText, Markdown, CodeBlock, Icon.

New, not in the layout or contracts yet (every name here is a proposal, §10):

| Component | What | Where used |
|---|---|---|
| SegmentedControl | 2–4 mutually exclusive buttons with `aria-pressed` (or a RadioGroup variant) | Basic/Advanced, runtime toggle, log stream filter, train mode |
| StatePill | icon + word for a state machine value (job, run, eval, node, plugin, model); color by class: live = accent, ok = success, error/preempted = locked error/warning, neutral = muted | everywhere |
| StatTile | label + big mono number (+ optional delta) | Training, Designer |
| ThemeCard | radio card with a live miniature drawn from a theme's own tokens | Settings › Appearance |
| ContextChip | "run_id = …" chip showing palette prefill | palette |
| SessionApproval (security; approved, ADR-0039 Accepted 2026-10-13; proposal until contracts lists it) | locked "let a new window sign in" prompt | second windows (decision 6) |
| TokenConfirm (security, proposal) | locked "create token" confirmation with can / cannot lists and the show-once box | Settings › API tokens |

Locked components draw only with `--rr-locked-*`, the fixed system font, fixed 14/12/18 px sizes, a 2 px
`security-text` border, 8 px radius, no shadow, and a "Rastai security check" header with a lock glyph. Deny/Cancel is
outlined and takes initial focus; Approve is filled with `security-text`.

---

## 6. Screen specs

Every screen: phone width means the section rail stacks above the content as a two-column wrap of links, toolbars wrap,
side panels stack below the main pane, wide tables scroll inside their own box, and the status bar wraps.

### 6.1 Foundations (board 1)

The language sheet: type scale, the three themes' role swatches with ratios, data ramps, locked set, spacing, radius,
elevation, focus, motion, and the core components with their states, including run-state pills (contracts §9.5),
a runs-table row (hover, selected), a job toast, and DiagnosticView Basic vs Advanced for **RRS-T2001** (texts from
`catalog/errors/RRS-T.json`).

### 6.2 Shell + Home (board 2)

| Region | Shows | Source |
|---|---|---|
| Outcome cards (6) | Chat, Teach it my data, See inside, Fix a behavior, Design a network, Train an agent; one-line promise and a readiness hint | section registry; readiness from the First Run status (request R1) |
| First run checklist | Hardware, Python environment, HF sign-in + license, model download (progress), SAE weights for 3 layers | `system.hardware`, `system.env.list`/`install`, `hf.login`, `hf.license.check`, `model.download` + `download.progress`, `psychology.sae.load`; one aggregated status per R1 |
| Advanced adds | each step's command id, download size per backend (spike numbers marked as estimates), Run doctor, Copy as CLI | `system.doctor` |
| Recent | runs, conversations, designs | request R2 |
| Status bar | as 3.5, with the egress badge lit for `hf_download` | `egress.active` |

- Empty: no project → "Create a project" card first. No GPU → the hardware step says so plainly and offers Gemma 3 270M or pairing a rented GPU.
- Loading: skeleton rows in the checklist; cards render at once.
- Error: a failed step shows its DiagnosticView inline with the fix button.
- Keyboard: Tab through cards (they are links), Enter opens; `G H` (proposed) goes Home.
- "Train an agent" is a visible but disabled card ("Arrives in a later version") rather than hidden.

### 6.3 Command palette (board 3)

As 3.6. Data: `ListCommands` descriptors (title, summary, scope, kind, `default_shortcut`, `basic`,
`arguments_schema_json`), keybindings, view actions, context from `contextStore`, recent entries from localStorage.
- Empty query: Recent, then the current section's commands.
- No match: "No command matches 'x'" plus "Search all modes" if Basic is hiding some.
- `ListCommands` failed: only view actions, with a one-line reason (frontend 2.5).
- Keyboard: ↑↓ move, Enter run (or focus the form when input is needed), Tab into arguments, Ctrl . copy as CLI (decided 2026-10-13), Esc closes and returns focus. It is a `combobox` + `listbox`; the dialog traps focus.
- Phone: the detail pane stacks under the list; the palette fills the width minus 16 px.

### 6.4 Inference (board 4)

| Region | Shows | Source |
|---|---|---|
| Models list | state per model (ready, unloaded, downloading, API "proxy") | `model.list`, `model.state_changed` |
| Conversations | title, starred / pinned flags, branches, time | `conversation.list`, `conversation.changed` |
| Chat toolbar | model picker, runtime Inspectable/Fast, Incognito, Live panel toggle | `chat.runtime.set`, `chat.create` (incognito), `psychology.live.start/stop` |
| Messages | user bubble; assistant text as token spans shaded by confidence (4 steps, legend "sure → unsure", dotted underline on the last step); branch switcher "2 / 3" | `conversation.get`, `chat.token` (logprob); token pieces from `model.vocabulary` |
| Inspected token | what it almost said (top alternatives with %), concepts it was representing (label, confidence; Advanced adds layer and feature id), entropy, Branch from this word, Open in Psychology | request R3 (alternatives), `psychology.packet` / replay for concepts |
| Composer | text, send, retention note | `chat.send`, draft store |
| Sampling (Advanced) | temperature, top-p, top-k, repetition penalty, max tokens, seed | `chat.settings.set`; defaults from `settings.get` `inference.sampling` |
| Live panel | entropy strip, tokens/s with the panel on and its overhead vs the 35% budget, concept sliders | `psychology.packet`; overhead per request R4; sliders call `chat.settings.set` |

- Fast runtime: shading still shows (logprobs), concepts show "needs the Inspectable runtime" with the RRS-G3004 fix button.
- Errors: RRS-G0005 (model not ready, fix `model.load`), RRS-G1005 (too long), RRS-S0005 (vault damaged: chat off, fix `storage.vault.recover`).
- Keyboard: Ctrl Enter sends; in a message, ←/→ moves token by token and Enter inspects (same model as the timeline).
- Phone: lists collapse into a drawer above the chat; the side panels stack below.

### 6.5 Training dashboard (board 5)

Basic and Advanced side by side for the same run (in the product, one panel shows one or the other).

| Block | Basic | Advanced adds | Source |
|---|---|---|---|
| Progress | % done, time left, step and pass | samples seen, tokens/s, run id | `job.progress`, `run.metrics` (`train.samples_seen`, `train.tokens_per_second`); time left per R5 |
| Is it learning? | verdict pill and one sentence; loss chart (train line, validation dots, "now" marker), legend, Show as table | smoothing and log scale (display-only in `@rastai/viz`, approved 2026-10-13; never feed the verdict), best-step marker, working table toggle | `run.metrics` (`train.loss`, `validation.loss`); verdict per R6 |
| Tiles | validation accuracy, best step, time left | same | recipe metric `classification.accuracy` (name to confirm), `checkpoint.saved.is_best` |
| Small multiples | — | learning rate (planned schedule dashed), grad norm, peak VRAM | `train.learning_rate`, `train.grad_norm`, `train.vram_peak_bytes` |
| Saved progress | last 3 checkpoints, best marked, Try in chat | ids, val loss, size, Export | `checkpoint.list`, `checkpoint.export` |
| What happened | 3 plain-language events | full log tail with stdout/stderr filter | run events; history per R7, live `job.log_line` |
| Config | — | recipe and hyperparameters, Rerun, Write as notebook, Compare | `run.get`, `run.rerun`, `run.notebook`, `run.compare` |

- Header actions: Checkpoint now (`train.checkpoint`), Stop and keep a checkpoint (`train.stop`), Open in new window (`view.window.open`).
- States: preparing shows the preparation steps; paused by a full disk shows RRS-T2001 (Basic text + "Free up space" / "Resume run"); failed shows its diagnostic; preempted shows "resumable" and Resume.
- Keyboard: charts are focusable; arrow keys move a crosshair point to point, announcing step and values.
- Phone: the two columns are one; small multiples wrap 1-up.

### 6.6 Model Designer (board 6)

| Region | Shows | Source |
|---|---|---|
| Toolbar | design file, breadcrumb (Model › Encoder), validity pill, undo/redo, Diff and Eject (Advanced), Compile and train | `design.validate`, `document.undo/redo`, `design.diff`, `design.eject`, `design.compile` then `train.start` |
| Layer palette | inputs, layers, blocks, pretrained backbones; Advanced shows op ids | request R8 (`design.op.list`) plus plugin node types |
| Canvas | TextCNN: token ids → embedding → conv k3/k4/k5 → max pool ×3 → concat → head (collapsed block); selected node and its edges in the active colors; zoom, fit | `GraphRenderer`; `design.edit` for every gesture |
| Inspector | kind, id, in/out shapes, params; attribute fields; train mode Freeze / LoRA / Train; Advanced: init, dtype, LoRA rank/alpha, observation point, FLOPs, fits per target | `design.stats` per node (R9), `design.edit` |
| Stats bar | params, weights, FLOPs/token, memory for batch × seq, fits, warnings count | `design.stats_updated` |
| Warning | "much larger than its training data", Advanced shows RRS-E3003 (draft code, contracts gap 22) | `design.validate` layer 4 |

- Example numbers are real for this design: 2,251,654 parameters (embedding 16,384 × 128; three conv banks of 100 filters; a 300 → 2 head), 9.0 MB in fp32.
- Keyboard: the offscreen `role="tree"` mirror (frontend 2.9); arrows walk the graph, Enter selects, Delete removes, Ctrl Z/Y.
- Phone: the canvas scrolls in its box; palette and inspector stack below it.

### 6.7 Psychology (board 7)

| Region | Shows | Source |
|---|---|---|
| Header | replay of a conversation branch, model, runtime, "recomputed from token ids"; Back to chat, Export replay | `psychology.replay.open`, `psychology.replay.export` |
| Token timeline | every generated token with an entropy bar; playback | replay summary (per-token entropy) |
| What it almost said, layer by layer | logit-lens heatmap: 7 layers × 7 tokens, top guess and %, bold when it matches the final answer; legend; Show as list | `psychology.logit_lens` lens data |
| Active concepts | labeled SAE features: label, label confidence, activation bar, example inputs; Advanced: SAE id, layer, feature index, Suppress / Boost | `psychology.sae_features`; feature details per R10; fit state per R11 |
| What it was looking at | prompt tokens shaded by attention; Advanced: per-head grid for layer 18 and a head-ablation intervention with Regenerate | `psychology.attention`; `psychology.intervention.set`, `psychology.branch.create` |

- Honest wording throughout: "what it is representing", labels as evidence with examples.
- API models: the section is greyed with "proxy only" and the observer-model picker.
- Keyboard: timeline ←/→ token, ↑/↓ layer, Enter details, Space play (frontend 2.9).
- Phone: panels stack; the heatmap scrolls horizontally.

### 6.8 Settings (board 8)

Appearance (working on the board: theme cards switch the whole board, density radios, the colorblind toggle swaps the series):

| Control | Setting | Notes |
|---|---|---|
| Theme cards (System, Light, Dark, High contrast) | `ui.theme` | each card is a miniature drawn with that theme's own tokens; HC marked M8; plugin themes appear here from M8 with their check result (R12) |
| Density | `ui.density` | radio cards with what changes |
| Reduce motion | `ui.reduced_motion` | also on when the OS asks (matchMedia) |
| Colorblind-safe chart colors | `ui.colorblind_palette` | live series preview |

Plugins: name, version, trust line, state pill (enabled, awaiting your review, incompatible with RRS-P1002); Advanced
lists the permission strings. "Review permissions" opens the locked TrustPrompt. Source: `plugin.list`,
`plugin.permissions`, `plugin.state_changed`.

Remote: nodes with state pills (connected, unreachable with RRS-N2001 and the retry countdown), hardware, heartbeat;
Advanced adds key fingerprint, cache size, protocol. Source: `node.list`, `node.status`, `node.state_changed`,
`node.heartbeat`; "Pair a computer over SSH" is `node.pair` (M9).

Every change is `settings.set`; `command.finished` refreshes all windows.

### 6.9 Locked security UI (board 9)

Three prompts over a dimmed Settings › API tokens page. The board's theme tweak includes a "Hostile test theme" that
sets every role to one grey: the app becomes unreadable, the three prompts do not change (security.test.jsx's rule).

1. **Let a new window sign in as you?** (**approved; ADR-0039 Accepted 2026-10-13**; decision 6): window title, address, opened by, "It can: do everything you can". Deny is default.
2. **Create the token "navi"?**: "It can" lists each granted scope in plain words; "It cannot" lists the rest that matter; then the show-once box with Copy. Data: `token.create` (with a dry run, R13).
3. **Install the plugin "sae-explorer-plus"?**: the TrustPrompt with plain-English permissions from `permission-text.js`; sensitive ones marked "!" and "(sensitive)" in `security-danger`, never by color alone. Data: `plugin.install` (trust step), `plugin.permissions`.

Keyboard: focus starts on the safe button; Esc = Deny/Cancel; Tab cycles inside; Enter does not approve unless the
approve button has focus. Phone: prompts stack full width.

---

## 7. Command requests (data a screen needs that no command provides)

| # | Need | Proposed | Used by |
|---|---|---|---|
| R1 | One First Run status with each step's state and next action, so the UI never decides "done" | `system.first_run.status` (read, sync): steps[]{step_id, state, detail, command_id, progress} | Home |
| R2 | Recent items across runs, conversations and designs | `project.recent.list` (read, sync) | Home |
| R3 | Top alternatives per generated token after the stream ends (the node's `TokenEvent.top_alternatives` never reaches the UI) | add `top_alternatives[]` to stored message tokens in `conversation.get` (and optionally `chat.token`) | Inference |
| R4 | Live-panel overhead (tokens/s with panel vs without) | carry `tokens_per_second` and `baseline_tokens_per_second` from `GenerateFinished` into message metadata | Inference |
| R5 | Time left and step/epoch totals | `eta_seconds`, `step`, `total_steps`, `epoch`, `total_epochs` in `job.progress` or `run.get` | Training, status bar |
| R6 | "Is it learning?" verdict (improving, plateau, overfitting, unstable) with its basis | `run.health` (read, sync) or a field on `run.get` | Training (Basic) |
| R7 | Log history when a dashboard opens (`job.log_line` is ephemeral) | `job.log.read {job_id, after_line, limit}` | Training |
| R8 | The op vocabulary with attribute schemas for the palette and inspector forms | `design.op.list` (read, sync), merged with plugin node types | Designer |
| R9 | Per-node shapes, params and FLOPs | per-node rows in `design.stats` / `design.stats_updated` | Designer |
| R10 | One feature's label, label confidence and example inputs | `psychology.feature.get {sae_id, feature_index}` | Psychology, Inference |
| R11 | Which SAE layers are loaded and their fit state (ok, stale) without rerunning the job | `psychology.sae.status` (read, sync) | Psychology, Home |
| R12 | Available themes with their accessibility check result | `theme.list` (M8) | Settings |
| R13 | A token's resolved scopes before creating it | `token.create` with `dry_run: true` returning granted[] and the preset's scopes | Security |
| R14 | Validation accuracy name for the classification recipe | confirm `classification.accuracy` in contracts §15.7 | Training |

---

## 8. Owner decisions

Answered by the owner on **2026-10-13**; the owner's words are in quotes. Row 8 is the one exception (see its note).

| # | Question | Decision (2026-10-13) | Follow-up |
|---|---|---|---|
| 1 | Fonts | **IBM Plex rejected:** "something a bit more stylish please. not over the top but not 1950's mainframe either." Then, also 2026-10-13: **Geist + Geist Mono chosen**: "A text schema is fine we can revisit later if needed". | §4.2 and §4.5; every board now uses Geist. Spec §27 register row and §37 v5.4.5 (DESIGN-01, this PR); files bundled in §9 (c). Revisitable; B and C stay documented. |
| 2 | Colors and the cobalt accent | **Approved as designed:** "colors seemed fine". | Values in §4.1 stand. |
| 3 | Blue/orange diverging scale for everyone | **Approved.** | M1-51 theme data. |
| 4 | High-contrast built-in | **Approved, ship now:** "we'll revisit in M8". | Third built-in theme in M1-51; `ui.theme` gains a high-contrast value (contract PR, §9). |
| 5 | New keys `font_sizes_px.xxl`, `colors.confidence_tint` | **Approved.** | Own contract PR before code uses them (`rastai.theme/1` schema). |
| 6 | Locked "let a new window sign in" approval | **Approved, ADR first.** | Status: **approved; ADR-0039 Accepted, 2026-10-13** (§6.9). Settles open question 24 (contracts §14). |
| 7 | Chart smoothing and log scale as display-only options | **Approved:** "probably better for realtime". | In `@rastai/viz`; never feeds a verdict (§6.5). |
| 8 | Keyboard shortcuts | Owner asked where they were: consolidated in §3.7 and on the "Keyboard shortcuts" board. **Copy as CLI is Ctrl .** (set by the design team on 2026-10-13, revisitable), because Ctrl Shift C opens the DevTools element picker. **Not yet an owner decision:** the owner has not confirmed Ctrl . in words; confirm at review. | The other "proposed" bindings in §3.7 ship as defaults; users can rebind all of them. |
| 9 | Section label | **Approved:** "Chat" in Basic, "Inference" in Advanced. | The rail label follows the effective mode on every board. |

---

## 9. Build plan: the repo work that follows, in order

Each step lands as its own PR, after the one before it is merged. Every PR follows CLAUDE.md: `just check` passes, a
test is added, the PR is signed off, and a separate reviewer agent with a naming and schema lens verifies it on a fresh
clone. Contract changes go first (ADR-0038).

### (a) Docs PR: the design, the ADR and the font register entry (DESIGN-01, this PR)

Changes no code and no contract shape. Its one name, the `docs/design/` folder, is added to contracts §1 and recorded in
`docs/architecture/contracts-changes/DESIGN-01.md` (ADR-0038). It contains:
- **`docs/design/ui-design.md`**: this document (version 1.0), with the canvas link. Linked from
  `docs/architecture/README.md` (the design map and reading order).
- **ADR-0039 "Second-window sign-in approval"** (`docs/adr/0039-second-window-sign-in-approval.md`). It settles open
  question 24 (contracts §14) and covers:
  - The opener hands its session key to a window it opened only after the owner approves the locked prompt
    "Let a new window sign in as you?" (§6.9, board 9). The prompt shows the window title, address, who opened it, and
    what it can do.
  - Deny is the default and Esc denies. The key travels only by `postMessage` to the opener's own origin, never in a
    URL. A denied or unanswered window shows `SignedOutScreen`.
  - The prompt is a new locked component, `SessionApproval`, in `@rastai/ui/security`, drawn only with locked tokens.
  - The desktop-shell case (a second `WebviewWindow` without `window.opener` signs in again with a new code, frontend
    layout 9.3).
  - A threat-model row in `docs/security/threat-model.md`. It names `SessionApproval`, which contracts does not list yet,
    so the row comes with the contract PR (step b), not here.
  - Tests (required when the mechanism is built): an e2e that a denied window cannot execute; a screenshot test under
    the hostile theme.
- **Spec §27 dependency register entry for the fonts**: an "Asset (not code)" row, modelled on the existing "License
  entry (not code)" rows, marked "owner approval of 2026-10-13". Its replacement path: swap the files, `OFL.txt` and the
  two `font_family` tokens (the DM Sans + DM Mono or Red Hat pairs in §4.5). The spec change is recorded in §37 as
  v5.4.5.

### (b) Contract PR: the new token keys and the high-contrast theme value

Only shapes, no code that uses them. Add a contracts-changes entry for the task id. It contains:
- **`rastai.theme/1`** (the `rastai-formats` source and the generated `schemas/rastai.theme/1.json`):
  - `typography.font_sizes_px.xxl` (integer, CSS pixels), required beside xs…xl;
  - `colors.confidence_tint` (`#rrggbb`, "Token confidence shading in chat"), an optional role like every color role.
- **Contracts §5.6**:
  - add `confidence_tint` to the color role table (group "Data");
  - add `--rr-font-size-xxl` and `--rr-color-confidence-tint` to the CSS variable list;
  - record that the high-contrast built-in ships now rather than at M8.
- **Contracts §11.2**: `ui.theme` gains the value `high_contrast` beside `system`, `light` and `dark`. The exact
  spelling follows the contracts naming rules; `high_contrast` is the proposal. The settings schema and
  `check-schema-diff` follow.
- **Contracts §15** (names): `packages/ui/src/fonts/` (the bundled font path) and the new component names `StatePill`,
  `SegmentedControl`, `StatTile` and `TokenConfirm`, if the contracts lists hold component names. This also unblocks
  `SessionApproval` (ADR-0039 is Accepted, 2026-10-13), and the threat-model row for it. Every name in §10 is registered
  here or in the PR that first uses it.
- No breaking change: every command and event is still `preview`, and both new keys add to the theme document.

### (c) `@rastai/ui` PR: token values, high contrast, fonts, components

This builds on the open M1-51 work (PR #256), as one follow-up PR or as amendments to its Part 2. It contains:
- **Token values** (`tokens/light.theme.json`, `tokens/dark.theme.json`) per §4.1 to §4.4:
  - colors: surfaces, text, lines, cobalt accent family, selection, focus ring, success, blue/orange diverging,
    canvas roles, and `confidence_tint`;
  - radius 4 / 6 / 10; bold 600; code line height 1.45; `xxl` 28; elevation;
  - font stacks `"Geist", "Noto Sans", system-ui, sans-serif` and `"Geist Mono", "DejaVu Sans Mono", ui-monospace, monospace`.
  - Series, sequential, colorblind and every locked token stay unchanged.
- **Built-in High contrast** (`tokens/high-contrast.theme.json`, base `dark`, the HC column of §4.1, focus ring
  width 3, outline elevations). `built-in-themes.js` gains it, and `builtInTheme` accepts it.
  `crates/rastai-formats/tests/theme_tokens.rs` validates all three files.
- **Bundled fonts** in `packages/ui/src/fonts/`: `Geist[wght].woff2`, `Geist-Italic[wght].woff2`,
  `GeistMono[wght].woff2`, `OFL.txt`, unmodified from github.com/vercel/geist-font. `styles/base.css` declares
  `@font-face` for each with `font-display: swap`; nothing is fetched at runtime.
- **New components**, each with JSDoc, CSS in `components.css` (values only through `var(--rr-*)`) and a browser test in
  Chromium and WebKit:
  - `StatePill`: state class to icon plus word (live, ok, error, warning, neutral). Error and warning use the locked
    severity colors.
  - `SegmentedControl`: buttons with `aria-pressed` in a group with a label; arrow keys move between options.
  - `StatTile`: label plus a mono value, with an optional delta.
  - `TokenConfirm` in `security/`: the locked "Create the token?" prompt with "It can", "It cannot" and the show-once
    box. Locked tokens only; exported only from `@rastai/ui/security`.
- **Every interactive component** gets the state table of §5.1 (hover, focus, disabled, error, loading).
- **Tests**:
  - `palettes.test.js`: the §4.1 pairs (accent and focus ring vs `background` and `surface`, `text_on_accent` vs
    `accent` and `accent_hover`, `border_strong` 3:1, canvas node border and edge 3:1), run on all three built-ins;
  - `tokens.test.js`: every role, including `confidence_tint`, resolves in all three;
  - `security.test.jsx`: screenshots of TokenConfirm under the hostile theme;
  - a test that the font files and `OFL.txt` are present and that base.css names them.

### (d) M1-53, M1-54 and M1-55 on top

**M1-53 (web shell)**
- The frame per §3.1 to §3.5:
  - top bar: logo, project, palette entry, global mode switch;
  - section rail with groups and `aria-current`; the `inference` label reads "Chat" in Basic and "Inference" in
    Advanced (decision 9);
  - workspace;
  - status bar: connection, node summary, jobs, the locked egress badge on the right.
- Phone-width rules from §6, and a `min-height: 100vh` frame.
- `ui.theme` `high_contrast` resolves in `state/theme.js`.
- The global shortcuts of §3.7: Ctrl K, Esc, Shift ? overlay, G-letter jumps, F6 regions.
- The status bar subscribes to `job.state_changed`, `job.progress`, `node.heartbeat` and `egress.active`.

**M1-54 (palette, copy-as, Settings, Plugins, Remote)**
- Palette per §3.6 and §6.3:
  - two panes, context chips, Basic filter with "N more in Advanced", "needs input";
  - the copy-as block, with Ctrl . for copy as CLI;
  - footer hints.
- Settings › Appearance per §6.8: theme cards for System, Light, Dark and High contrast, each drawn from that theme's
  own tokens; density; reduced motion; colorblind palette.
- Settings › API tokens, using the locked TokenConfirm. Its "can / cannot" lists come from command request R13.
- Plugins list and Remote (local node) per §6.8.
- Specs:
  - `palette.spec.js`: Ctrl K, Ctrl ., copy-as equals the golden file;
  - `basic-advanced.spec.js`: Settings, Plugins, Remote, and the Chat/Inference label;
  - `theme.spec.js`: light, dark, high contrast, `system`, compact; no `--rr-locked-*` change.

**M1-55 (desktop window)**
- Minimum window 960 × 600; below 1024 px the rail collapses to icons. Native title bar; window title
  "Rastai · <project> · <section>".
- Ctrl +/- zoom and the OS reduced-motion setting are respected. Ctrl K, Ctrl . and F6 reach the page.
- Manual check in WebKitGTK on the owner's box: Geist renders and the focus ring matches Chromium.
- Second windows wait for ADR-0039 to be accepted; then `SessionApproval` lands with the work that builds second
  windows (frontend layout 8.9 step 5).

**Command requests R1 to R14** (§7) are not part of M1. Each joins the milestone of the command area it extends, through
that milestone's contract PR.

---

## 10. Names not yet in contracts (proposals)

Each row is a name this design uses that `contracts.md` v1.2 does not list yet. The "Registered in" column says which
change adds it. None is used in code yet. A name leaves this table when its change lands (ADR-0038).

| Name | Kind | Used in | Registered in |
|---|---|---|---|
| `StatePill`, `SegmentedControl`, `StatTile`, `ThemeCard`, `ContextChip` | UI components (new, §5.2) | §5.2, §6 | §9 (b) contract PR, if contracts lists component names; otherwise the UI kit PR |
| `SessionApproval` | locked component in `@rastai/ui/security` | §5.2, §6.9, ADR-0039 | §9 (b) contract PR |
| `TokenConfirm` | locked component in `@rastai/ui/security` | §5.2, §6.9 | §9 (b) contract PR |
| `high_contrast` | `ui.theme` value (beside `system`, `light`, `dark`) | §6.8, §8 decision 4 | §9 (b) contract PR (contracts §11.2) |
| `confidence_tint` | colour role of `rastai.theme/1` | §4.1, §4.4 | §9 (b) contract PR (`rastai.theme/1`, contracts §5.6) |
| `font_sizes_px.xxl` | typography key of `rastai.theme/1` | §4.2, §4.4 | §9 (b) contract PR (`rastai.theme/1`, contracts §5.6) |
| `packages/ui/src/fonts/` | bundled font path | §4.5, §9 (c) | §9 (b) contract PR (contracts §15) |
| `tokens/high-contrast.theme.json` | token file path | §9 (c) | the UI kit PR |
| `ui.mode` | setting key (Basic / Advanced) | §3.4, §3.2 | §9 (b) contract PR (contracts §11.2) |
| `classification.accuracy` | metric name | §6.5, §7 R14 | contracts §15.7 confirmation (R14) |
| `theme.list` | command | §7 R12 | the milestone that adds themes (M8) |
| `system.first_run.status` | command | §6.2, §7 R1 | the Home milestone's contract PR |
| `project.recent.list` | command | §6.2, §7 R2 | the Home milestone's contract PR |
| `job.log.read` | command | §6.5, §7 R7 | the Training milestone's contract PR |
| `design.op.list` | command | §6.6, §7 R8 | the Designer milestone's contract PR |
| `run.health` | command | §6.5, §7 R6 | the Training milestone's contract PR |
| `psychology.feature.get` | command | §6.7, §7 R10 | the Psychology milestone's contract PR |
| `psychology.sae.status` | command | §6.2, §7 R11 | the Psychology milestone's contract PR |
| Command requests R1–R14 (other than those above) | commands and fields | §7 | each joins the milestone of the area it extends |
| `IconButton`, `Slider`, `Toggle`, `Tabs`, `SidebarNav`, `StatusBarView`, `EmptyState`, `KeyHint`, `ProgressBar` and the other planned components of §5.2 | layout names (frontend layout 4.1) | §3, §5.2 | not in contracts; a component joins contracts when it is built |
| `PanelChrome`, `CommandPaletteView`, `DiagnosticView`, `EgressBadge`, `TrustPrompt`, `PermissionList`, `SignedOutScreen` | existing components (M1-51 branch) | §5.2 | not in contracts yet (built on the M1-51 branch) |
| `contextStore`, `state/theme.js`, `built-in-themes.js`, `permission-text.js`, `locked-styles.js` | file or module names | §3.6, §6.3, §6.9 | layout names (frontend layout 2 and 4); not in contracts |
| `tokens/*.theme.json`, `security.test.jsx`, `palettes.test.js`, `tokens.test.js`, `basic-advanced.spec.js`, `theme.spec.js`, `palette.spec.js` | test and token file names | §9 | the UI kit and M1-53 to M1-55 PRs |
