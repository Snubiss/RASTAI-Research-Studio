// The palette's rows: Basic filtering with the count of what it hides, experimental hiding, ranking (recent first, then
// the current section), the "Needs input" note, and the view actions. The descriptors are the daemon's shape.

import { describe, expect, test } from "vitest";

import { buildPaletteItems } from "./palette-entries.js";
import { CONTEXT_ARGUMENT_NAMES } from "./context-arguments.js";

const NO_ARGUMENTS = JSON.stringify({ type: "object", properties: {}, required: [] });
const RUN_ARGUMENT = JSON.stringify({
  type: "object",
  properties: { run_id: { type: "string" } },
  required: ["run_id"],
});
const SECTIONS = ["home", "inference", "settings"];

/** @param {Partial<any>} fields */
function command(fields) {
  return {
    commandId: "system.status",
    title: "Show status",
    section: "settings",
    basic: true,
    experimental: false,
    defaultShortcut: "",
    argumentsSchemaJson: NO_ARGUMENTS,
    ...fields,
  };
}

const COMMANDS = [
  command({ commandId: "system.status", title: "Show status", section: "settings", basic: true }),
  command({ commandId: "command.list", title: "List commands", section: "settings", basic: false }),
  command({
    commandId: "job.cancel",
    title: "Cancel job",
    section: "home",
    basic: true,
    defaultShortcut: "Mod+Shift+L",
  }),
  command({ commandId: "eval.secret", title: "Secret eval", section: "settings", experimental: true, basic: true }),
];

/**
 * @param {any} overrides
 * @returns {import("./palette-entries.js").PaletteOptions}
 */
function options(overrides) {
  return {
    commands: COMMANDS,
    sectionIds: SECTIONS,
    mode: /** @type {"basic" | "advanced"} */ ("basic"),
    query: "",
    recentIds: [],
    currentSectionId: "home",
    showAll: false,
    experimentalOn: false,
    context: {},
    ...overrides,
  };
}

describe("buildPaletteItems", () => {
  test("Basic lists only the basic commands and counts the advanced ones it hides", () => {
    const { items, hiddenCount } = buildPaletteItems(options({ mode: "basic" }));
    const commandIds = items.filter((item) => !item.isView).map((item) => item.id);
    // job.cancel is in the current section (Home), so it comes first.
    expect(commandIds).toEqual(["job.cancel", "system.status"]);
    expect(hiddenCount).toBe(1);
  });

  test("Advanced lists every command, and so does Basic with Search all modes on", () => {
    const advanced = buildPaletteItems(options({ mode: "advanced" }));
    expect(advanced.items.filter((item) => !item.isView)).toHaveLength(3);
    expect(advanced.hiddenCount).toBe(0);
    const showAll = buildPaletteItems(options({ showAll: true }));
    expect(showAll.items.filter((item) => !item.isView).map((item) => item.id)).toContain("command.list");
  });

  test("experimental commands stay hidden until plugins.experimental_apis is on", () => {
    expect(buildPaletteItems(options({ mode: "advanced" })).items.map((item) => item.id)).not.toContain("eval.secret");
    expect(
      buildPaletteItems(options({ mode: "advanced", experimentalOn: true })).items.map((item) => item.id),
    ).toContain("eval.secret");
  });

  test("a query keeps the matching commands, and a better match ranks first", () => {
    const { items } = buildPaletteItems(options({ mode: "advanced", query: "stat" }));
    expect(items.map((item) => item.id)).toEqual(["system.status"]);
    const sorted = buildPaletteItems(options({ mode: "advanced", query: "c" })).items.map((item) => item.id);
    // "Cancel job" starts with c, so it ranks above "List commands", which only has a c inside it.
    expect(sorted.indexOf("job.cancel")).toBeLessThan(sorted.indexOf("command.list"));
  });

  test("with no query, recent entries come first, then the current section's, then the rest, then the views", () => {
    const { items } = buildPaletteItems(options({ mode: "advanced", recentIds: ["system.status"] }));
    const ids = items.map((item) => item.id);
    expect(ids[0]).toBe("system.status");
    expect(ids.indexOf("job.cancel")).toBeLessThan(ids.indexOf("command.list"));
    expect(ids.at(-1)).toBe("view.section.open:settings");
  });

  test("a command with a required argument the context doesn't fill says it needs input", () => {
    const withArgument = command({
      commandId: "run.rerun",
      title: "Rerun",
      section: "home",
      argumentsSchemaJson: RUN_ARGUMENT,
    });
    const needs = buildPaletteItems(options({ commands: [withArgument], mode: "advanced" })).items[0];
    expect(needs.note).toBe("Needs input");
    const filled = buildPaletteItems(
      options({ commands: [withArgument], mode: "advanced", context: { run_id: "run_01" } }),
    );
    expect(filled.items[0].note).toBe("");
    expect(filled.items[0].prefilled).toEqual({ run_id: "run_01" });
  });

  test("the shortcut shows as its label, and a view action has no shortcut or Copy as", () => {
    const { items } = buildPaletteItems(options({ mode: "advanced" }));
    const cancel = items.find((item) => item.id === "job.cancel");
    expect(cancel?.shortcut).toBe("Ctrl+Shift+L");
    const view = items.find((item) => item.isView);
    expect(view?.command).toBeNull();
    expect(view?.detail).toBe("View");
  });

  test("the context argument names are the layout's list", () => {
    expect(CONTEXT_ARGUMENT_NAMES).toContain("run_id");
    expect(CONTEXT_ARGUMENT_NAMES).toContain("model_ref");
  });
});
