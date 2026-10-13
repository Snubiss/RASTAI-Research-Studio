// Basic/Advanced (layout frontend-and-plugins.md 2.4): AdvancedOnly never hides a Basic control. The ordered list
// of interactive elements (role and accessible name) in Basic must be a subsequence of the list in Advanced, and a
// value typed in Basic must survive a round trip through Advanced. The panel below is built the way sections are:
// Basic controls first, AdvancedOnly blocks after or beside them, values kept above the mode switch.

import { useState } from "react";
import { afterEach, describe, expect, test } from "vitest";

import { Button } from "../components/Button.jsx";
import { Checkbox } from "../components/Checkbox.jsx";
import { NumberField } from "../components/NumberField.jsx";
import { TextField } from "../components/TextField.jsx";
import { inAct, mount } from "../testing/render.js";
import { AdvancedOnly } from "./AdvancedOnly.jsx";
import { ModeProvider } from "./ModeProvider.jsx";
import { useMode } from "./use-mode.js";

/** @typedef {import("./mode-types.js").UiMode} UiMode */

/**
 * A sample panel: the prompt and Send are Basic; temperature and the raw-logits switch are Advanced extras.
 * @param {object} props
 * @param {string} props.prompt
 * @param {(prompt: string) => void} props.onPromptChange
 */
function SamplePanel({ prompt, onPromptChange }) {
  const [temperature, setTemperature] = useState(/** @type {number | null} */ (0.7));
  const [rawLogits, setRawLogits] = useState(false);
  return (
    <div>
      <TextField label="Prompt" value={prompt} onChange={onPromptChange} />
      <AdvancedOnly>
        <NumberField label="Temperature" value={temperature} onChange={setTemperature} />
      </AdvancedOnly>
      <Button variant="primary">Send</Button>
      <AdvancedOnly>
        <Checkbox label="Show raw logits" checked={rawLogits} onChange={setRawLogits} />
      </AdvancedOnly>
    </div>
  );
}

/**
 * The panel under a mode, with its Basic value held above the switch, as a panel store would.
 * @param {object} props
 * @param {UiMode} props.mode
 */
function SampleWindow({ mode }) {
  const [prompt, setPrompt] = useState("");
  return (
    <ModeProvider mode={mode}>
      <SamplePanel prompt={prompt} onPromptChange={setPrompt} />
    </ModeProvider>
  );
}

function ShowMode() {
  return <output>{useMode()}</output>;
}

/**
 * Role and accessible name of each interactive element, in page order.
 * @param {HTMLElement} container
 * @returns {string[]}
 */
function interactiveElements(container) {
  return [...container.querySelectorAll("button, input, select, textarea")].map((element) => {
    const role = element.tagName === "INPUT" ? `input:${element.getAttribute("type")}` : element.tagName.toLowerCase();
    return `${role} ${accessibleName(/** @type {HTMLElement} */ (element))}`;
  });
}

/**
 * @param {HTMLElement} element
 * @returns {string}
 */
function accessibleName(element) {
  const label = element.id ? document.querySelector(`label[for="${element.id}"]`) : null;
  return (element.getAttribute("aria-label") ?? label?.textContent ?? element.textContent ?? "").trim();
}

/**
 * @param {string[]} shorter
 * @param {string[]} longer
 * @returns {boolean}
 */
function isSubsequence(shorter, longer) {
  let position = 0;
  for (const item of longer) {
    if (position < shorter.length && shorter[position] === item) {
      position += 1;
    }
  }
  return position === shorter.length;
}

/**
 * Types into a React-controlled input the way a user does, so React sees the change.
 * @param {HTMLInputElement} input
 * @param {string} text
 */
function typeInto(input, text) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setValue?.call(input, text);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("Basic and Advanced", () => {
  test("AdvancedOnly never hides a Basic control: Basic's controls are a subsequence of Advanced's", async () => {
    const mounted = await mount(<SampleWindow mode="basic" />);
    const basic = interactiveElements(mounted.container);
    await mounted.rerender(<SampleWindow mode="advanced" />);
    const advanced = interactiveElements(mounted.container);
    expect(basic).toEqual(["input:text Prompt", "button Send"]);
    expect(advanced.length).toBeGreaterThan(basic.length);
    expect(isSubsequence(basic, advanced)).toBe(true);
    await mounted.unmount();
  });

  test("a value typed in Basic survives a round trip through Advanced", async () => {
    const mounted = await mount(<SampleWindow mode="basic" />);
    const input = /** @type {HTMLInputElement} */ (mounted.container.querySelector("input"));
    await inAct(() => typeInto(input, "Hello there"));
    await mounted.rerender(<SampleWindow mode="advanced" />);
    await mounted.rerender(<SampleWindow mode="basic" />);
    const after = /** @type {HTMLInputElement} */ (mounted.container.querySelector("input"));
    expect(after.value).toBe("Hello there");
    await mounted.unmount();
  });

  test("a panel's provider overrides the app's, and the default with no provider is advanced", async () => {
    const mounted = await mount(
      <>
        <ShowMode />
        <ModeProvider mode="advanced">
          <ModeProvider mode="basic">
            <ShowMode />
          </ModeProvider>
          <AdvancedOnly>
            <span>extra</span>
          </AdvancedOnly>
        </ModeProvider>
      </>,
    );
    const shown = [...mounted.container.querySelectorAll("output")].map((output) => output.textContent);
    expect(shown).toEqual(["advanced", "basic"]);
    expect(mounted.container.textContent).toContain("extra");
    await mounted.unmount();
  });
});
