// The components of the design's inventory that have no other home (ui-design.md 5.2): StatePill, SegmentedControl and
// StatTile. Each shows exactly the text and state it is given, and the bundled Geist faces load from the package
// (ui-design.md 4.5), never from the network.

import { useState } from "react";
import { afterEach, beforeAll, describe, expect, test } from "vitest";
import { userEvent } from "vitest/browser";

import { cssRgb } from "../tokens/color-math.js";
import { applyThemeTokens } from "../tokens/apply-theme.js";
import { LOCKED_TOKENS } from "../tokens/locked.js";
import { builtInTheme } from "../tokens/resolve-tokens.js";
import { loadStyles } from "../testing/load-styles.js";
import { mount } from "../testing/render.js";
import { SegmentedControl } from "./SegmentedControl.jsx";
import { StatePill } from "./StatePill.jsx";
import { StatTile } from "./StatTile.jsx";

/** @typedef {import("./StatePill.jsx").StateTone} StateTone */

const OPTIONS = { density: /** @type {const} */ ("comfortable"), reducedMotion: false, colorblind: false };
const MIN_TARGET_PX = 24;
const TONES = /** @type {StateTone[]} */ (["live", "ok", "error", "warning", "neutral"]);
const LOG_OPTIONS = [
  { value: "all", label: "All" },
  { value: "warnings", label: "Warnings" },
  { value: "errors", label: "Errors" },
];

beforeAll(async () => {
  await loadStyles();
  applyThemeTokens(document.documentElement, builtInTheme("light", OPTIONS));
});

afterEach(() => {
  document.body.replaceChildren();
});

/**
 * A segmented control whose value is kept in state, with every change recorded.
 * @param {object} props
 * @param {string[]} props.changes
 * @param {string} [props.initial]
 */
function SegmentHarness({ changes, initial = "all" }) {
  const [value, setValue] = useState(initial);
  return (
    <SegmentedControl
      label="Log stream"
      options={LOG_OPTIONS}
      value={value}
      onChange={(next) => {
        changes.push(next);
        setValue(next);
      }}
    />
  );
}

describe("StatePill", () => {
  test("shows the word and a glyph that is hidden from screen readers, in every tone", async () => {
    const mounted = await mount(
      <div>
        {TONES.map((tone) => (
          <StatePill key={tone} label={`Job ${tone}`} tone={tone} />
        ))}
      </div>,
    );
    const pills = [...mounted.container.querySelectorAll(".rr-state-pill")];
    expect(pills.map((pill) => pill.textContent)).toEqual(TONES.map((tone) => `${glyphFor(tone)}Job ${tone}`));
    for (const tone of TONES) {
      const pill = mounted.container.querySelector(`.rr-state-pill--${tone}`);
      expect(pill, tone).not.toBeNull();
      expect(pill?.querySelector("[aria-hidden=true]"), tone).not.toBeNull();
    }
    await mounted.unmount();
  });

  test("the tone sets the glyph's color: live is the accent, error is the locked severity color", async () => {
    const mounted = await mount(
      <div>
        <StatePill label="Running" tone="live" />
        <StatePill label="Failed" tone="error" />
      </div>,
    );
    const live = /** @type {HTMLElement} */ (
      mounted.container.querySelector(".rr-state-pill--live .rr-state-pill__glyph")
    );
    const error = /** @type {HTMLElement} */ (
      mounted.container.querySelector(".rr-state-pill--error .rr-state-pill__glyph")
    );
    expect(getComputedStyle(live).color).toBe(cssRgb(builtInTheme("light", OPTIONS).colors.accent));
    expect(getComputedStyle(error).color).toBe(cssRgb(LOCKED_TOKENS.light["severity-error"]));
    await mounted.unmount();
  });
});

describe("SegmentedControl", () => {
  test("is a labelled group of buttons, one pressed, each at least 24 px tall", async () => {
    const mounted = await mount(<SegmentHarness changes={[]} />);
    const group = /** @type {HTMLElement} */ (mounted.container.querySelector("[role=group]"));
    expect(group.getAttribute("aria-label")).toBe("Log stream");
    const buttons = [...group.querySelectorAll("button")];
    expect(buttons.map((button) => button.textContent)).toEqual(["All", "Warnings", "Errors"]);
    expect(buttons.map((button) => button.getAttribute("aria-pressed"))).toEqual(["true", "false", "false"]);
    for (const button of buttons) {
      expect(button.getBoundingClientRect().height).toBeGreaterThanOrEqual(MIN_TARGET_PX);
    }
    await mounted.unmount();
  });

  test("a click presses the option and reports its value", async () => {
    /** @type {string[]} */
    const changes = [];
    const mounted = await mount(<SegmentHarness changes={changes} />);
    await userEvent.click(/** @type {HTMLElement} */ ([...mounted.container.querySelectorAll("button")][2]));
    expect(changes).toEqual(["errors"]);
    const pressed = [...mounted.container.querySelectorAll("button")].map((button) =>
      button.getAttribute("aria-pressed"),
    );
    expect(pressed).toEqual(["false", "false", "true"]);
    await mounted.unmount();
  });

  test("Space presses the focused option, as for any button", async () => {
    /** @type {string[]} */
    const changes = [];
    const mounted = await mount(<SegmentHarness changes={changes} />);
    const warnings = /** @type {HTMLElement} */ ([...mounted.container.querySelectorAll("button")][1]);
    warnings.focus();
    await userEvent.keyboard(" ");
    expect(changes).toEqual(["warnings"]);
    await mounted.unmount();
  });

  test("the arrow keys move focus between options and wrap at the ends, without pressing anything", async () => {
    /** @type {string[]} */
    const changes = [];
    const mounted = await mount(<SegmentHarness changes={changes} />);
    const buttons = /** @type {HTMLElement[]} */ ([...mounted.container.querySelectorAll("button")]);
    buttons[0].focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(buttons[1]);
    await userEvent.keyboard("{ArrowRight}{ArrowRight}");
    expect(document.activeElement).toBe(buttons[0]);
    await userEvent.keyboard("{ArrowLeft}");
    expect(document.activeElement).toBe(buttons[2]);
    await userEvent.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(buttons[1]);
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(buttons[2]);
    expect(changes).toEqual([]);
    await mounted.unmount();
  });

  test("the group is one Tab stop: the pressed option is in the Tab order and the others are not", async () => {
    const mounted = await mount(<SegmentHarness changes={[]} />);
    const buttons = [...mounted.container.querySelectorAll("button")];
    expect(buttons.map((button) => button.tabIndex)).toEqual([0, -1, -1]);
    await mounted.unmount();
  });

  test("Tab enters the group at the pressed option, and Shift+Tab after an arrow leaves from the arrowed-to option", async () => {
    const mounted = await mount(<TabOrderHarness initial="all" />);
    const [before, all, warnings] = /** @type {HTMLElement[]} */ ([...mounted.container.querySelectorAll("button")]);
    before.focus();
    await userEvent.tab();
    expect(document.activeElement).toBe(all);
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(warnings);
    // Shift+Tab leaves the group to the button before it: the arrowed-to option is the stop, the pressed one is not.
    await userEvent.tab({ shift: true });
    expect(document.activeElement).toBe(before);
    await mounted.unmount();
  });

  test("ArrowRight that wraps to the first option, then Tab, leaves the group to the button after it", async () => {
    const mounted = await mount(<TabOrderHarness initial="errors" />);
    const [before, all, , errors, after] = /** @type {HTMLElement[]} */ ([
      ...mounted.container.querySelectorAll("button"),
    ]);
    before.focus();
    await userEvent.tab();
    expect(document.activeElement).toBe(errors);
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(all);
    await userEvent.tab();
    expect(document.activeElement).toBe(after);
    await mounted.unmount();
  });

  test("when focus leaves the group, the next Tab into it lands on the pressed option again", async () => {
    const mounted = await mount(<TabOrderHarness initial="all" />);
    const [before, all, warnings, , after] = /** @type {HTMLElement[]} */ ([
      ...mounted.container.querySelectorAll("button"),
    ]);
    before.focus();
    await userEvent.tab();
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(warnings);
    await userEvent.tab();
    expect(document.activeElement).toBe(after);
    await userEvent.tab({ shift: true });
    expect(document.activeElement).toBe(all);
    await mounted.unmount();
  });

  test("when the options shrink while focus is inside, the one Tab stop moves to the last option", async () => {
    const mounted = await mount(<ShrinkHarness count={3} />);
    const group = /** @type {HTMLElement} */ (mounted.container.querySelector("[role=group]"));
    const buttons = /** @type {HTMLElement[]} */ ([...group.querySelectorAll("button")]);
    buttons[2].focus();
    expect(document.activeElement).toBe(buttons[2]);
    await mounted.rerender(<ShrinkHarness count={2} />);
    const shrunk = /** @type {HTMLButtonElement[]} */ ([...group.querySelectorAll("button")]);
    expect(shrunk).toHaveLength(2);
    expect(shrunk.map((button) => button.tabIndex)).toEqual([-1, 0]);
    await mounted.unmount();
  });

  test("a disabled group presses nothing and shows its options in the disabled text color", async () => {
    /** @type {string[]} */
    const changes = [];
    const mounted = await mount(
      <SegmentedControl
        label="Log stream"
        options={LOG_OPTIONS}
        value="all"
        onChange={(next) => changes.push(next)}
        disabled
      />,
    );
    const buttons = /** @type {HTMLButtonElement[]} */ ([...mounted.container.querySelectorAll("button")]);
    expect(buttons.every((button) => button.disabled)).toBe(true);
    buttons[1].click();
    expect(changes).toEqual([]);
    const colors = builtInTheme("light", OPTIONS).colors;
    expect(getComputedStyle(buttons[1]).color).toBe(cssRgb(colors.text_disabled));
    await mounted.unmount();
  });
});

/**
 * A segmented group whose option count the test changes, to see the Tab stop follow the shorter list.
 * @param {object} props
 * @param {number} props.count
 */
function ShrinkHarness({ count }) {
  const options = LOG_OPTIONS.slice(0, count);
  return <SegmentedControl label="Log stream" options={options} value="all" onChange={() => {}} />;
}

/**
 * A button, the segmented group, and another button, so a test can see where Tab and Shift+Tab land.
 * @param {object} props
 * @param {string} props.initial
 */
function TabOrderHarness({ initial }) {
  return (
    <div>
      <button type="button">before</button>
      <SegmentHarness changes={[]} initial={initial} />
      <button type="button">after</button>
    </div>
  );
}

describe("StatTile", () => {
  test("shows the label, the number in the mono face with tabular figures, and the change when there is one", async () => {
    const mounted = await mount(
      <div>
        <StatTile label="Steps" value="1,284" delta="+12 this run" />
        <StatTile label="Loss" value="0.412" />
      </div>,
    );
    const tiles = [...mounted.container.querySelectorAll(".rr-stat-tile")];
    expect(tiles[0].querySelector(".rr-stat-tile__label")?.textContent).toBe("Steps");
    expect(tiles[0].querySelector(".rr-stat-tile__value")?.textContent).toBe("1,284");
    expect(tiles[0].querySelector(".rr-stat-tile__delta")?.textContent).toBe("+12 this run");
    expect(tiles[1].querySelector(".rr-stat-tile__delta")).toBeNull();
    const value = /** @type {HTMLElement} */ (tiles[0].querySelector(".rr-stat-tile__value"));
    expect(getComputedStyle(value).fontVariantNumeric).toContain("tabular-nums");
    expect(getComputedStyle(value).fontFamily).toContain("Geist Mono");
    await mounted.unmount();
  });
});

describe("the bundled fonts", () => {
  test("Geist and Geist Mono load from the package's own files", async () => {
    const loaded = await Promise.all([
      document.fonts.load('400 16px "Geist"'),
      document.fonts.load('600 16px "Geist"'),
      document.fonts.load('400 16px "Geist Mono"'),
    ]);
    for (const faces of loaded) {
      expect(faces.length).toBeGreaterThan(0);
    }
    expect(document.fonts.check('16px "Geist"')).toBe(true);
    expect(document.fonts.check('16px "Geist Mono"')).toBe(true);
  });
});

/**
 * @param {StateTone} tone
 * @returns {string}
 */
function glyphFor(tone) {
  return { live: "●", ok: "✓", error: "✕", warning: "▲", neutral: "○" }[tone];
}
