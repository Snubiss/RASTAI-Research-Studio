# ADR-0036: The frontend is plain JavaScript + React, with short JSDoc types checked by `checkJs`

**Status:** Accepted, 2026-10-10. Supersedes [ADR-0023](0023-owner-code-style.md)'s TypeScript section (and its TS wording in decisions 1 and 7) and, in part, [ADR-0019](0019-in-house-webgl2-canvas.md) decision 5 ("a renderer is a TS class").

## Context

- **The owner reviews every line of code, and TypeScript isn't their strength.** Code the reviewer can't read fluently isn't really reviewed. That is the risk ADR-0023 exists to prevent (§28 risk 1).
- AI agents write most of the code. In TypeScript they drift toward exactly what ADR-0023 forbids: generic type parameters, conditional and mapped types, type-level tricks.
- Type checking still pays for itself. It catches a wrong field name or a missing argument in agent-written code before review.
- The JavaScript tooling already on the list can check plain `.js` files: the TypeScript compiler reads JSDoc comments in `checkJs` mode and emits nothing.
- Most frontend types are generated anyway (protobuf-es, document types, the catalog), and generated code isn't reviewed (contracts §2.4 rule 2).

## Decision

1. **All hand-written frontend, plugin-UI and SDK code is plain JavaScript ES modules with React and JSX:** `.js` for modules, `.jsx` for React components. This covers `apps/web`, `apps/viewer`, `apps/desktop`'s front config, every `@rastai/*` package, first-party plugin UIs, and the `rastai plugin new` templates. Nobody writes TypeScript syntax.
2. **Types live in short JSDoc comments:** `@typedef` with `@property` lines, `@param`, `@returns`, `@callback`. Keep them minimal and readable. **No JSDoc type gymnastics:** no conditional or mapped types, no long generic templates, no `@template` chains. A function that would need one is split, or takes a plain object instead.
3. **A dev-only checker.** `just check-js` (inside `just check`) runs `tsc --noEmit -p jsconfig.json` with `checkJs` on; settings live in `jsconfig.base.json`, and each JS package has a `jsconfig.json` that extends it. The `typescript` package is on §27 as dev tooling for this checker only. It never emits or ships code.
4. **Generated code may be `.d.ts` declarations or JSDoc `@typedef` `.js` modules.** It lives only in generated folders (every `src/generated/`, `packages/proto/src/generated/` included), is never edited by hand and isn't reviewed. protobuf-es generates with target `js+dts`; connect-es is used from JavaScript. `@rastai/plugin` ships generated declarations so plugin authors' editors complete names.
5. **Classes only where a browser API demands one:** a renderer that owns a WebGL context (`GraphRenderer`, the viz classes). They are plain JavaScript classes with JSDoc on their public methods. Everything else is function components and plain functions.
6. **Polymorphism through a shared object shape,** documented with one `@typedef`: a `CommandExecutor` is any object with `execute`, a `Panel` any object with `id` and `render`. There are no interfaces to implement.
7. **Names.** Packages keep `@rastai/*`. "TS packages" are JS packages; `tsconfig` is `jsconfig`; `.ts`/`.tsx` are `.js`/`.jsx`; tests are `*.test.js` and `tests/e2e/*.spec.js`; CI jobs `test-js` and `test-js-macos`; recipe `just test-js`. Plugin authors may still use any language inside their own iframe.
8. **Enforced by** `tools/checks/check-no-typescript`: CI fails on any `.ts`, `.tsx` or `.d.ts` file outside the generated folders. Formatting is prettier (M1).

*Note since acceptance (2026-10-11): decision 4 named all of `packages/proto/` as generated. Only `packages/proto/src/generated/` is; the package's `index.js` and tests are hand-written and reviewed, and `check-no-typescript` and `check-colors` exempt only that folder (M1-08). The decision is unchanged.*

```js
// packages/canvas/src/graph-renderer.js
// A class only because it owns a WebGL context (ADR-0036 decision 5).

/** @typedef {import("./colors.js").CanvasColors} CanvasColors */

export class GraphRenderer {
  /** @param {WebGL2RenderingContext} gl */
  constructor(gl) {
    this.gl = gl;
  }

  /**
   * Update colors in place, so a theme switch never recreates the WebGL context.
   * @param {CanvasColors} colors
   */
  setColors(colors) {
    // write the new values into the color uniforms
  }
}

// Not allowed: /** @template {keyof Events} K @param {K} type @param {(e: Events[K]) => void} handler */
// Write one plain function per event type, or take {type, handler} as a plain object.
```

**Checked by:** `check-no-typescript` and `just check-js` in `just check` on every PR; `check-generated` (regenerating the `.d.ts` and `@typedef` files changes nothing); the review checklist in the PR template (JSDoc kept short, no gymnastics).

## Consequences

**Good**
- The owner can read and review every hand-written frontend line.
- Agents keep type checking: a wrong field or a missing argument still fails `just check`.
- No build step for types: Vite's esbuild JSX transform runs the source as written, and packages export `./src/index.js`.
- Generated types still reach editors through `.d.ts` files nobody reviews.

**Costs**
- JSDoc is wordier than inline type syntax. About +8 size-days of code across the UI milestones.
- `checkJs` is less strict than a `.ts` file with `strict` on, and some library types are awkward to name in JSDoc. Where a type would need gymnastics, the code is reshaped instead.
- Contributors who know TypeScript must write JavaScript here.

## Alternatives considered

| Alternative | Why not |
|---|---|
| TypeScript (ADR-0023 as accepted) | The reviewer doesn't read it fluently, and its type system invites the tricks ADR-0023 forbids. |
| Plain JavaScript with no checker | Agents' field-name and argument mistakes would surface only at run time or in review. |
| TypeScript with a lint ban on advanced types | Still TypeScript syntax on every line; the reviewer's problem stays. |
| Flow, ReScript, Elm | Another type language to learn and allowlist, with smaller ecosystems. |
| JSDoc with full TypeScript-style generics | The same gymnastics, moved into comments. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 22, [§16](../spec/rastai_spec_v5.md#16-architecture-overview), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new), [§29](../spec/rastai_spec_v5.md#29-repo-and-engineering-setup), [§30](../spec/rastai_spec_v5.md#30-adrs-to-write-now)
- Contracts: [§1 repository tree](../architecture/contracts.md#1-repository-tree) (no TypeScript source), [§2.3 apps and JavaScript packages](../architecture/contracts.md#23-apps-and-javascript-packages), [§2.4 rule 11](../architecture/contracts.md#24-layering-rules), [§12.4 JavaScript](../architecture/contracts.md#124-javascript-react-jsx-jsdoc), [§15.11 names replaced](../architecture/contracts.md#1511-names-rejected-or-replaced)
- Related: [ADR-0019](0019-in-house-webgl2-canvas.md), [ADR-0023](0023-owner-code-style.md), [ADR-0004](0004-schema-ownership.md), [ADR-0015](0015-plugins-and-licensing.md)
