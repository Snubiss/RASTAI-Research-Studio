// CommandForm and JobProgress in a real browser (layout frontend-and-plugins.md 7.1): the form draws a command's
// arguments schema and runs it with the values filled in, and the job view follows job.get, the progress events
// and the final state change's Diagnostic.

import { afterEach, describe, expect, test } from "vitest";
import { userEvent } from "vitest/browser";

import { pluginDiagnostic } from "../plugin-diagnostic.js";
import { sampleBusEvent } from "../testing/host-contract.js";
import { createMockHost } from "../testing/mock-host.js";
import { inAct, mount } from "../testing/render.js";
import { CommandForm } from "./CommandForm.jsx";
import { DiagnosticList } from "./DiagnosticList.jsx";
import { JobProgress } from "./JobProgress.jsx";
import { PluginHostProvider } from "./PluginHostProvider.jsx";

/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("../host-api.js").BusEvent} BusEvent */
/** @typedef {import("../testing/render.js").Mounted} Mounted */

// settings.set's arguments schema, as the daemon sends it in CommandDescriptor.arguments_schema_json.
const SETTINGS_SET_SCHEMA = JSON.stringify({
  title: "SettingsSetArgs",
  type: "object",
  properties: {
    key: { type: "string", description: "A dotted key, such as ui.theme." },
    level: { type: "string", enum: ["user", "project"], description: "Which settings.json to write." },
    value: { description: "Any JSON value the key allows." },
  },
  required: ["key", "value"],
  additionalProperties: false,
});

/** @type {Mounted[]} */
let mounted = [];

afterEach(async () => {
  for (const page of mounted) {
    await page.unmount();
  }
  mounted = [];
});

/**
 * @param {import("react").ReactNode} element
 * @returns {Promise<Mounted>}
 */
async function show(element) {
  const page = await mount(element);
  mounted.push(page);
  return page;
}

/** Lets the mock host's answers (resolved promises) arrive and React render them. */
async function settle() {
  await inAct(() => new Promise((resolve) => setTimeout(resolve, 0)));
}

/**
 * @param {any} result
 * @returns {ExecuteOutcome}
 */
function okOutcome(result) {
  return { ok: true, result, jobId: null, diagnostics: [], journalId: null };
}

/**
 * A bus event with its payload, as the event hub delivers it.
 * @param {string} eventType
 * @param {string} subjectId
 * @param {object} payload   the EventPayload oneof: { case, value }
 * @returns {BusEvent}
 */
function eventWithPayload(eventType, subjectId, payload) {
  return /** @type {BusEvent} */ ({
    ...sampleBusEvent(eventType, subjectId),
    payload: { $typeName: "rastai.events.v1.EventPayload", payload },
  });
}

/**
 * @param {HTMLElement} container
 * @param {string} label
 * @returns {HTMLElement}
 */
function fieldLabelled(container, label) {
  const labelElement = [...container.querySelectorAll("label")].find((element) =>
    element.textContent?.startsWith(label),
  );
  const control = labelElement?.htmlFor ? container.querySelector(`#${CSS.escape(labelElement.htmlFor)}`) : null;
  if (!(control instanceof HTMLElement)) {
    throw new Error(`no field labelled ${label}`);
  }
  return control;
}

describe("CommandForm", () => {
  test("draws one field per argument and runs the command with the values filled in", async () => {
    const mock = createMockHost("core", {
      "settings.set": (args) => okOutcome({ ...args, source: "user" }),
    });
    /** @type {ExecuteOutcome[]} */
    const outcomes = [];
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <CommandForm
          commandId="settings.set"
          argumentsSchemaJson={SETTINGS_SET_SCHEMA}
          submitLabel="Save"
          onOutcome={(outcome) => outcomes.push(outcome)}
        />
      </PluginHostProvider>,
    );
    expect(page.container.querySelectorAll("input, select, textarea")).toHaveLength(3);
    await userEvent.fill(fieldLabelled(page.container, "Key"), "ui.density");
    await userEvent.fill(fieldLabelled(page.container, "Value"), '"compact"');
    await userEvent.click(/** @type {HTMLElement} */ (page.container.querySelector("button[type=submit]")));
    await settle();
    // The level was left empty, so the daemon applies its default.
    expect(mock.calls.map((call) => [call.commandId, call.args])).toEqual([
      ["settings.set", { key: "ui.density", value: "compact" }],
    ]);
    expect(outcomes.map((outcome) => outcome.ok)).toEqual([true]);
  });

  test("starts with the initial values, and reports the arguments they make on every change", async () => {
    const mock = createMockHost("core", {});
    /** @type {object[]} */
    const seen = [];
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <CommandForm
          commandId="settings.set"
          argumentsSchemaJson={SETTINGS_SET_SCHEMA}
          initialValues={{ key: "ui.theme" }}
          onArgumentsChange={(args) => seen.push(args)}
        />
      </PluginHostProvider>,
    );
    expect(/** @type {HTMLInputElement} */ (fieldLabelled(page.container, "Key")).value).toBe("ui.theme");
    await userEvent.fill(fieldLabelled(page.container, "Value"), '"dark"');
    expect(seen.at(-1)).toEqual({ key: "ui.theme", value: "dark" });
    expect(mock.calls).toEqual([]);
  });

  test("shows the daemon's diagnostics when the command fails", async () => {
    const mock = createMockHost("core", {
      "settings.set": () => ({
        ok: false,
        result: null,
        jobId: null,
        diagnostics: [pluginDiagnostic("RRS-C0002", { found: "a string", expected: "a JSON value" })],
        journalId: null,
      }),
    });
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <CommandForm commandId="settings.set" argumentsSchemaJson={SETTINGS_SET_SCHEMA} />
      </PluginHostProvider>,
    );
    await userEvent.fill(fieldLabelled(page.container, "Key"), "ui.theme");
    // Not JSON: the form sends it as text and the daemon's diagnostic says what is wrong.
    await userEvent.fill(fieldLabelled(page.container, "Value"), "dark");
    await userEvent.click(/** @type {HTMLElement} */ (page.container.querySelector("button[type=submit]")));
    await settle();
    expect(mock.calls.map((call) => call.args)).toEqual([{ key: "ui.theme", value: "dark" }]);
    expect(page.container.querySelector("[role=alert]")).not.toBeNull();
    expect(page.container.textContent).toContain("RRS-C0002");
  });
});

describe("JobProgress", () => {
  test("shows job.get's state, the progress events of its own job and the final Diagnostic", async () => {
    let state = "running";
    const mock = createMockHost("core", {
      "job.get": () => okOutcome({ job: { job_id: "job_01", node_id: "local", kind: "train", state } }),
    });
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <JobProgress jobId="job_01" />
      </PluginHostProvider>,
    );
    await settle();
    expect(page.container.textContent).toContain("running");
    expect(page.container.querySelector("progress")).toBeNull();

    await inAct(() => {
      mock.emit(
        eventWithPayload("job.progress", "job_02", {
          case: "jobProgress",
          value: { fraction: 0.9, message: "other job" },
        }),
      );
      mock.emit(
        eventWithPayload("job.progress", "job_01", {
          case: "jobProgress",
          value: { fraction: 0.25, message: "epoch 1 of 4" },
        }),
      );
    });
    const bar = /** @type {HTMLProgressElement} */ (page.container.querySelector("progress"));
    expect(bar.value).toBe(0.25);
    expect(page.container.textContent).toContain("epoch 1 of 4");
    expect(page.container.textContent).not.toContain("other job");

    state = "failed";
    const diagnostic = pluginDiagnostic("RRS-J1002", { job_id: "job_01" });
    await inAct(() =>
      mock.emit(eventWithPayload("job.state_changed", "job_01", { case: "jobStateChanged", value: { diagnostic } })),
    );
    await settle();
    expect(page.container.textContent).toContain("failed");
    expect(page.container.querySelector("[role=alert]")).not.toBeNull();
    expect(page.container.textContent).toContain("RRS-J1002");
  });

  test("shows the Diagnostic of a job that failed before it opened, from job.get's error_code", async () => {
    const failedJob = { job_id: "job_01", node_id: "local", kind: "train", state: "failed", error_code: "RRS-J1002" };
    const mock = createMockHost("core", { "job.get": () => okOutcome({ job: failedJob }) });
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <JobProgress jobId="job_01" />
      </PluginHostProvider>,
    );
    await settle();
    expect(page.container.textContent).toContain("failed");
    expect(page.container.querySelector("[role=alert]")).not.toBeNull();
    expect(page.container.textContent).toContain("This computer ran out of memory");
  });
});

describe("DiagnosticList", () => {
  test("a fix whose arguments aren't a JSON object shows an invalid arguments Diagnostic and runs nothing", async () => {
    const mock = createMockHost("core", {});
    const diagnostic = {
      ...pluginDiagnostic("RRS-J0005", { job_id: "job_404" }),
      fixCommandId: "job.list",
      fixArgumentsJson: "{not json",
    };
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <DiagnosticList diagnostics={[diagnostic]} />
      </PluginHostProvider>,
    );
    await inAct(() => /** @type {HTMLElement} */ (page.container.querySelector("button")).click());
    await settle();
    expect(mock.calls).toEqual([]);
    expect(mock.shownDiagnostics.map((shown) => shown.map((item) => item.code))).toEqual([["RRS-C0002"]]);
  });
});
