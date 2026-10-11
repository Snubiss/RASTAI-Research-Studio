# ADR-0019: An in-house WebGL2 renderer for the Designer canvas and all visuals

**Status:** Accepted, 2026-10-10. Decision 5 superseded in part by [ADR-0036](0036-frontend-plain-javascript.md), 2026-10-10.

**Superseded in part:** decision 5's language. A renderer is a plain JavaScript class with short JSDoc on its public methods, still the one case where classes are allowed because it owns a WebGL context (ADR-0036 decision 5). The rest of decision 5 stands.

## Context

- The Designer needs 1,000 visible nodes at 60 fps in Chrome and in WebKitGTK, the engine Tauri uses on Linux (§11). Big models (80+ blocks) must stay under that with hierarchy collapse.
- Psychology needs a 3D data-flow view (layers × tokens) whose values change every frame, plus a vector-space view and charts.
- React Flow (DOM/SVG), three.js, deck.gl and uPlot are on the dropped list (§27). The question was whether an in-house renderer is fast enough, especially in WebKitGTK.
- Spike 06 built an in-house WebGL2 renderer with no libraries:
  - graph nodes, edges and glyph-atlas labels take **three instanced draw calls per frame at any size**;
  - the glyph atlas is drawn once with Canvas2D and uploaded as a texture;
  - the 3D flow view re-uploads cube values every frame.
- Results (Chrome for Testing 155 and WebKitGTK 2.52, 1600×1000 window):
  - both held their refresh cap (Chrome 144 fps, WebKitGTK ~62 fps) for every configuration up to **50,000 nodes with 620,000 glyphs** and **213,000 cubes**;
  - the combined scene (1,000 nodes plus a 26×512 flow view) could be drawn 16× per frame in Chrome and 32× in WebKitGTK before dropping below the cap: about **35–40× headroom** over the target;
  - a pixel read-back confirmed both really drew (28.5% vs 27.3% of the screen covered for 1,000 nodes), since WebKitGTK reports a fake GPU name;
  - WebKitGTK animates at ~60 Hz even on a 144 Hz monitor. There was no NVIDIA + Wayland blank-window problem.

## Decision

1. **`@rastai/canvas`** (the Designer graph) and **`@rastai/viz`** (charts, vector-space view, 3D data-flow view) are in-house WebGL2 renderers with no rendering libraries. They're ported from `spike/canvas/canvas_test.js`.
2. **Drawing model:** instanced draws per primitive kind (nodes, edges, glyphs, cubes, chart marks), with per-instance data in typed arrays uploaded as buffers. The draw-call count doesn't grow with the graph.
3. **Text:** a glyph atlas drawn once with Canvas2D and sampled in the shader. Canvas2D is used only for atlas building, never for scene drawing.
4. **WebGL2 is required.** If it's unavailable, the view shows `RRS-U1xxx` with a fix ("open in a browser") and falls back to the table view. Every chart has a table behind it anyway (accessibility, §7).
5. **API shape:** a renderer is a TS class, because it owns a WebGL context (the one case where contracts §12.4 allows classes). React components own a renderer and pass it plain data. No scene-graph framework. *("TS class" superseded: a plain JavaScript class, ADR-0036.)*
6. **Level of detail:** hierarchy collapse keeps visible nodes near the 1,000 target, and the 3D view switches between summaries zoomed out and heads and features zoomed in.
7. **Reduced motion** (`ui.reduced_motion`) disables camera animation and 3D auto-rotation.
8. **WebGPU is optional and later.** It's not used until every webview Tauri ships supports it.

*Note since acceptance (2026-10-10), the theme token contract (contracts §5.6, owner decision 20):*
- *Renderers never pick colors. `canvasColors(tokens)` gives the graph renderer its colors and `vizColors(tokens)` the charts; `GraphRenderer.setColors(colors)` and `setColors` on every viz class update uniforms and buffers in place, so a theme switch never recreates a WebGL context.*
- *The glyph atlas takes its font from the tokens (`createGlyphAtlas(gl, font)`); a font change rebuilds the atlas texture only.*
- *`tools/checks/check-colors` fails CI on a color literal in `packages/canvas/src/`, `packages/viz/src/`, `apps/web/src/sections/` and plugin UIs. The spike's hard-coded `#101318` background is gone from renderer code; it seeds the built-in dark theme's `canvas_background` token.*
- *The decisions are unchanged.*

**Checked by:** a Playwright test per renderer that draws a fixture scene and checks pixel coverage by read-back; a nightly performance test on the GPU runner with the spike's scenes (1,000 nodes plus the flow view) that fails below 60 fps in Chrome.

## Consequences

**Good**
- Measured 35–40× headroom over the target in both engines. Performance isn't a risk for the prototype.
- No rendering dependencies to track. The Designer and Psychology views share one approach.
- The viewer (`apps/viewer`) reuses the same packages with no daemon.

**Costs**
- We write and maintain picking, hit-testing, text layout, camera control and chart axes ourselves.
- Accessibility needs explicit work (tables, keyboard navigation), because a canvas has no DOM semantics.
- WebKitGTK caps animation at ~60 Hz on high-refresh displays. That's acceptable for this app, but users with 144 Hz monitors will notice in the desktop shell.

## Alternatives considered

| Alternative | Why not |
|---|---|
| React Flow (DOM/SVG) | DOM nodes don't scale to thousands of animated nodes. It's a dependency on the dropped list. |
| three.js or deck.gl | Large dependencies with their own churn, for three draw calls' worth of need. |
| uPlot or another chart library | Would be a second rendering stack beside the WebGL one. Dropped (§27). |
| Canvas2D for everything | No instancing, and it falls off at tens of thousands of glyphs and cubes. |
| WebGPU now | Not reliably available in WebKitGTK. |

## Links

- Spec: [§7](../spec/rastai_spec_v5.md#7-ux-system), [§9](../spec/rastai_spec_v5.md#9-psychology-first-party-plugin) (visuals), [§11](../spec/rastai_spec_v5.md#11-model-designer), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Contracts: [§1 port map](../architecture/contracts.md#1-repository-tree), [§2.3 apps and JavaScript packages](../architecture/contracts.md#23-apps-and-javascript-packages), [§5.6 theme tokens](../architecture/contracts.md#56-theme-tokens), [§12.4 JavaScript](../architecture/contracts.md#124-javascript-react-jsx-jsdoc)
- Spike: [06 canvas in webviews](../../spike/results/06_canvas_webviews.md)
- Related: [ADR-0006](0006-ui-transport-http-only.md), [ADR-0012](0012-build-in-house-policy.md), [ADR-0023](0023-owner-code-style.md), [ADR-0036](0036-frontend-plain-javascript.md)
