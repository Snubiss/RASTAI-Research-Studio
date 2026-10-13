// The M1 components in a real engine: each one's controlled value goes out through its callback, keyboard use
// works, and the view-only components show exactly the data they are given (layout frontend-and-plugins.md 4).

import { useState } from "react";
import { afterEach, beforeAll, describe, expect, test } from "vitest";
import { userEvent } from "vitest/browser";

import { loadStyles } from "../testing/load-styles.js";
import { inAct, mount } from "../testing/render.js";
import { applyThemeTokens } from "../tokens/apply-theme.js";
import { builtInTheme } from "../tokens/resolve-tokens.js";
import { Button } from "./Button.jsx";
import { CommandPaletteView } from "./CommandPaletteView.jsx";
import { Dialog } from "./Dialog.jsx";
import { FieldForm } from "./FieldForm.jsx";
import { PanelChrome } from "./PanelChrome.jsx";
import { Table } from "./Table.jsx";
import { Toast } from "./Toast.jsx";

/** @typedef {import("./FieldForm.jsx").FieldSpec} FieldSpec */
/** @typedef {import("./PanelChrome.jsx").ModeOverride} ModeOverride */

const OPTIONS = { density: /** @type {const} */ ("comfortable"), reducedMotion: false, colorblind: false };
// WCAG 2.2's minimum target size, which every button meets in both densities (layout 2.9).
const MIN_TARGET_PX = 24;

/** @type {FieldSpec[]} */
const FIELDS = [
  { name: "model", label: "Model", kind: "text", required: true, help: "A model reference.", choices: [] },
  { name: "steps", label: "Steps", kind: "number", required: false, help: "", choices: [] },
  { name: "resume", label: "Resume", kind: "boolean", required: false, help: "", choices: [] },
  { name: "device", label: "Device", kind: "choice", required: false, help: "", choices: ["cpu", "cuda:0"] },
  { name: "extra", label: "Extra", kind: "json", required: false, help: "", choices: [] },
];
const PALETTE_ENTRIES = [
  { id: "model.load", title: "Load model", detail: "Inference", shortcut: "Ctrl+L" },
  { id: "train.start", title: "Start training", detail: "Training" },
  { id: "view.section.open", title: "Open Designer" },
];

beforeAll(async () => {
  await loadStyles();
});

afterEach(() => {
  document.body.replaceChildren();
  document.documentElement.removeAttribute("style");
});

/**
 * The form with its values kept in state, and every onChange call recorded.
 * @param {object} props
 * @param {Array<Object<string, any>>} props.changes
 * @param {FieldSpec[]} [props.fields]
 * @param {Object<string, any>} [props.initialValues]
 */
function FormHarness({ changes, fields = FIELDS, initialValues = { device: "cpu" } }) {
  const [values, setValues] = useState(initialValues);
  return (
    <FieldForm
      fields={fields}
      values={values}
      onChange={(next) => {
        changes.push(next);
        setValues(next);
      }}
    />
  );
}

describe("Button", () => {
  test("calls onClick, and is at least 24 px tall in both densities", async () => {
    for (const density of /** @type {const} */ (["comfortable", "compact"])) {
      applyThemeTokens(document.documentElement, builtInTheme("light", { ...OPTIONS, density }));
      let clicks = 0;
      const mounted = await mount(<Button onClick={() => (clicks += 1)}>Save</Button>);
      const button = /** @type {HTMLButtonElement} */ (mounted.container.querySelector("button"));
      await inAct(() => button.click());
      expect(clicks).toBe(1);
      expect(button.getBoundingClientRect().height).toBeGreaterThanOrEqual(MIN_TARGET_PX);
      await mounted.unmount();
    }
  });

  test("a disabled button doesn't call onClick", async () => {
    let clicks = 0;
    const mounted = await mount(
      <Button disabled onClick={() => (clicks += 1)}>
        Save
      </Button>,
    );
    await inAct(() => mounted.container.querySelector("button")?.click());
    expect(clicks).toBe(0);
    await mounted.unmount();
  });
});

describe("FieldForm", () => {
  test("renders one labelled control per field kind", async () => {
    const mounted = await mount(<FormHarness changes={[]} />);
    const labels = [...mounted.container.querySelectorAll("label")].map((label) => label.textContent);
    expect(labels).toEqual(["Model *", "Steps", "Resume", "Device", "Extra"]);
    expect(mounted.container.querySelector("input[type=text]")?.hasAttribute("required")).toBe(true);
    expect(mounted.container.querySelector("input[type=number]")).not.toBeNull();
    expect(mounted.container.querySelector("input[type=checkbox]")).not.toBeNull();
    expect(mounted.container.querySelectorAll("option")).toHaveLength(2);
    expect(mounted.container.querySelector("textarea")).not.toBeNull();
    expect(mounted.container.textContent).toContain("A model reference.");
    await mounted.unmount();
  });

  test("every change reports all the values, numbers as numbers", async () => {
    /** @type {Array<Object<string, any>>} */
    const changes = [];
    const mounted = await mount(<FormHarness changes={changes} />);
    await userEvent.type(/** @type {HTMLElement} */ (mounted.container.querySelector("input[type=text]")), "tiny");
    await userEvent.type(/** @type {HTMLElement} */ (mounted.container.querySelector("input[type=number]")), "12");
    await userEvent.click(/** @type {HTMLElement} */ (mounted.container.querySelector("input[type=checkbox]")));
    await userEvent.selectOptions(/** @type {HTMLElement} */ (mounted.container.querySelector("select")), "cuda:0");
    expect(changes.at(-1)).toEqual({ device: "cuda:0", model: "tiny", steps: 12, resume: true });
    await mounted.unmount();
  });

  test("a choice with no value shows Choose…, not the first choice, and a required one is invalid", async () => {
    /** @type {Array<Object<string, any>>} */
    const changes = [];
    /** @type {FieldSpec[]} */
    const fields = [
      { name: "device", label: "Device", kind: "choice", required: true, help: "", choices: ["cpu", "cuda:0"] },
    ];
    const mounted = await mount(<FormHarness fields={fields} initialValues={{}} changes={changes} />);
    const select = /** @type {HTMLSelectElement} */ (mounted.container.querySelector("select"));
    expect(select.value).toBe("");
    expect(select.selectedOptions[0].textContent).toBe("Choose…");
    expect(select.validity.valueMissing).toBe(true);

    await userEvent.selectOptions(select, "cpu");
    expect(changes).toEqual([{ device: "cpu" }]);
    expect(select.value).toBe("cpu");
    expect(select.validity.valueMissing).toBe(false);
    const labels = [...select.options].map((option) => option.textContent);
    expect(labels).toEqual(["cpu", "cuda:0"]);
    await mounted.unmount();
  });

  test("a choice whose value is empty or not one of the choices shows Choose…, never the first choice", async () => {
    /** @type {FieldSpec[]} */
    const fields = [
      { name: "device", label: "Device", kind: "choice", required: true, help: "", choices: ["cpu", "cuda:0"] },
    ];
    // "" and a stale value the choices no longer hold: the browser would select "cpu" while the form reports these.
    for (const unknownValue of ["", "tpu"]) {
      /** @type {Array<Object<string, any>>} */
      const changes = [];
      const mounted = await mount(
        <FormHarness fields={fields} initialValues={{ device: unknownValue }} changes={changes} />,
      );
      const select = /** @type {HTMLSelectElement} */ (mounted.container.querySelector("select"));
      expect(select.value).toBe("");
      expect(select.selectedOptions[0].textContent).toBe("Choose…");
      expect(select.validity.valueMissing).toBe(true);
      expect(changes).toEqual([]);
      await mounted.unmount();
    }
  });
});

describe("Table", () => {
  test("shows a caption, the headers and one row per item", async () => {
    const columns = [
      { key: "name", title: "Name" },
      { key: "loss", title: "Loss", align: /** @type {const} */ ("end") },
    ];
    const rows = [
      { name: "run 1", loss: "0.42" },
      { name: "run 2", loss: "0.37" },
    ];
    const mounted = await mount(<Table caption="Runs" columns={columns} rows={rows} />);
    expect(mounted.container.querySelector("caption")?.textContent).toBe("Runs");
    const headers = [...mounted.container.querySelectorAll("th")].map((cell) => cell.textContent);
    expect(headers).toEqual(["Name", "Loss"]);
    const cells = [...mounted.container.querySelectorAll("tbody td")].map((cell) => cell.textContent);
    expect(cells).toEqual(["run 1", "0.42", "run 2", "0.37"]);
    await mounted.rerender(<Table caption="Runs" columns={columns} rows={[]} emptyText="No runs yet." />);
    expect(mounted.container.querySelector("tbody")?.textContent).toBe("No runs yet.");
    await mounted.unmount();
  });
});

describe("Dialog", () => {
  test("opens as a modal, and Escape asks the caller to close it", async () => {
    let closes = 0;
    const mounted = await mount(
      <Dialog open title="Delete run?" onClose={() => (closes += 1)}>
        <p>This can be undone.</p>
      </Dialog>,
    );
    const dialog = /** @type {HTMLDialogElement} */ (mounted.container.querySelector("dialog"));
    expect(dialog.open).toBe(true);
    expect(dialog.getAttribute("aria-labelledby")).toBe(dialog.querySelector("h2")?.id);
    await userEvent.keyboard("{Escape}");
    expect(closes).toBe(1);
    expect(dialog.open).toBe(true);
    await mounted.rerender(
      <Dialog open={false} title="Delete run?" onClose={() => {}}>
        <p>This can be undone.</p>
      </Dialog>,
    );
    expect(dialog.open).toBe(false);
    await mounted.unmount();
  });
});

describe("Toast", () => {
  test("lists the toasts in a polite live region and dismisses by id", async () => {
    /** @type {string[]} */
    const dismissed = [];
    const toasts = [
      { id: "a", message: "Saved.", tone: /** @type {const} */ ("success") },
      { id: "b", message: "Run started.", tone: /** @type {const} */ ("info") },
    ];
    const mounted = await mount(<Toast toasts={toasts} onDismiss={(id) => dismissed.push(id)} />);
    const region = /** @type {HTMLElement} */ (mounted.container.querySelector("[role=status]"));
    expect(region.getAttribute("aria-live")).toBe("polite");
    expect([...region.querySelectorAll("p")].map((item) => item.textContent)).toEqual(["Saved.", "Run started."]);
    await inAct(() => region.querySelectorAll("button")[1].click());
    expect(dismissed).toEqual(["b"]);
    await mounted.unmount();
  });
});

describe("CommandPaletteView", () => {
  test("Up and Down move the highlight, Enter runs it, Escape closes", async () => {
    /** @type {string[]} */
    const runs = [];
    let closes = 0;
    const mounted = await mount(
      <CommandPaletteView
        entries={PALETTE_ENTRIES}
        query=""
        onQueryChange={() => {}}
        onRun={(id) => runs.push(id)}
        onClose={() => (closes += 1)}
      />,
    );
    const input = /** @type {HTMLInputElement} */ (mounted.container.querySelector("input"));
    expect(document.activeElement).toBe(input);
    await userEvent.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(runs).toEqual(["view.section.open"]);
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(runs).toEqual(["view.section.open", "model.load"]);
    const active = mounted.container.querySelector("[aria-selected=true]");
    expect(input.getAttribute("aria-activedescendant")).toBe(active?.id);
    await userEvent.keyboard("{Escape}");
    expect(closes).toBe(1);
    await mounted.unmount();
  });

  test("shows the shortcut and the empty text, and reports typing", async () => {
    /** @type {string[]} */
    const queries = [];
    const mounted = await mount(
      <CommandPaletteView
        entries={PALETTE_ENTRIES}
        query=""
        onQueryChange={(query) => queries.push(query)}
        onRun={() => {}}
        onClose={() => {}}
      />,
    );
    expect(mounted.container.querySelector("kbd")?.textContent).toBe("Ctrl+L");
    await userEvent.keyboard("x");
    expect(queries).toEqual(["x"]);
    await mounted.rerender(
      <CommandPaletteView entries={[]} query="zzz" onQueryChange={() => {}} onRun={() => {}} onClose={() => {}} />,
    );
    expect(mounted.container.textContent).toContain("No matching commands.");
    await mounted.unmount();
  });
});

describe("PanelChrome", () => {
  test("changes the panel's mode override and runs a menu item", async () => {
    /** @type {ModeOverride[]} */
    const overrides = [];
    /** @type {string[]} */
    const selected = [];
    const mounted = await mount(
      <PanelChrome
        title="Live"
        modeOverride=""
        onModeOverrideChange={(value) => overrides.push(value)}
        menuItems={[{ id: "reload", label: "Reload panel" }]}
        onMenuSelect={(id) => selected.push(id)}
      />,
    );
    expect(mounted.container.querySelector("h2")?.textContent).toBe("Live");
    await userEvent.selectOptions(/** @type {HTMLElement} */ (mounted.container.querySelector("select")), "basic");
    expect(overrides).toEqual(["basic"]);
    const menuButton = /** @type {HTMLButtonElement} */ (mounted.container.querySelector("[aria-haspopup=menu]"));
    expect(mounted.container.querySelector("[role=menu]")).toBeNull();
    await userEvent.click(menuButton);
    expect(menuButton.getAttribute("aria-expanded")).toBe("true");
    await userEvent.click(/** @type {HTMLElement} */ (mounted.container.querySelector("[role=menuitem]")));
    expect(selected).toEqual(["reload"]);
    expect(mounted.container.querySelector("[role=menu]")).toBeNull();
    await mounted.unmount();
  });
});
