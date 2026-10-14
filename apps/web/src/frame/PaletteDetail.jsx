// The palette's detail pane (design 3.6, 6.3): what the highlighted entry is, its arguments form when it needs input,
// and its call written four ways (Copy as). The text comes from copy-as.js; nothing here invents a line.

import { Button, SegmentedControl } from "@rastai/ui";
import { CommandForm, PluginHostProvider } from "@rastai/plugin";
import { useEffect } from "react";

import { copyAsFormats } from "../commands/copy-as.js";
import { kindWord } from "../commands/command-index.js";

/** @typedef {import("../commands/palette-entries.js").PaletteItem} PaletteItem */
/** @typedef {import("@rastai/plugin").PluginHost} PluginHost */

// The four formats a call is copied in, in the order the palette shows them (layout 2.5 step 7).
export const COPY_FORMATS = [
  { value: "cli", label: "CLI" },
  { value: "python", label: "Python" },
  { value: "javascript", label: "JavaScript" },
  { value: "mcp", label: "MCP" },
];

/**
 * @param {object} props
 * @param {PaletteItem | null} props.item
 * @param {PluginHost} props.host
 * @param {string} props.copyFormat          one of COPY_FORMATS' values
 * @param {(format: string) => void} props.onCopyFormatChange
 * @param {string} props.copyStatus          what the last copy said
 * @param {() => void} props.onCopy
 * @param {Object<string, any>} props.argumentsOfItem   the arguments the item's form holds, or its prefilled ones
 * @param {(itemId: string, args: Object<string, any>) => void} props.onArgumentsChange
 * @param {(itemId: string) => void} props.onRun        runs the item (or opens its view)
 * @param {(itemId: string) => void} props.onFormRan     the item's form ran and succeeded
 */
export function PaletteDetail({
  item,
  host,
  copyFormat,
  onCopyFormatChange,
  copyStatus,
  onCopy,
  argumentsOfItem,
  onArgumentsChange,
  onRun,
  onFormRan,
}) {
  // A command's form starts from its prefilled values, so the arguments the palette copies start from them too.
  useEffect(() => {
    if (item && !item.isView) {
      onArgumentsChange(item.id, item.prefilled);
    }
  }, [item?.id]);

  if (item === null) {
    return <p className="rr-palette__empty">Nothing is highlighted.</p>;
  }
  if (item.isView) {
    return (
      <section className="rr-palette-detail" aria-label={item.title}>
        <h3 className="rr-palette-detail__title">{item.title}</h3>
        <Button variant="primary" onClick={() => onRun(item.id)}>
          Open
        </Button>
      </section>
    );
  }

  const command = item.command;
  const formats = copyAsFormats(command.commandId, argumentsOfItem);
  return (
    <section className="rr-palette-detail" aria-label={command.title}>
      <h3 className="rr-palette-detail__title">{command.title}</h3>
      <p className="rr-palette-detail__summary">{command.summary}</p>
      <dl className="rr-palette-detail__facts">
        <dt>Command</dt>
        <dd>
          <code>{command.commandId}</code>
        </dd>
        <dt>Section</dt>
        <dd>{item.detail}</dd>
        <dt>Scope</dt>
        <dd>
          <code>{command.scope}</code>
        </dd>
        <dt>Kind</dt>
        <dd>{kindWord(command.kind)}</dd>
        {item.shortcut ? (
          <>
            <dt>Shortcut</dt>
            <dd>{item.shortcut}</dd>
          </>
        ) : null}
      </dl>
      {item.note ? (
        <PluginHostProvider host={host}>
          <CommandForm
            key={item.id}
            commandId={command.commandId}
            argumentsSchemaJson={command.argumentsSchemaJson}
            submitLabel="Run"
            initialValues={item.prefilled}
            onArgumentsChange={(args) => onArgumentsChange(item.id, args)}
            onOutcome={(outcome) => {
              if (outcome.ok) {
                onFormRan(item.id);
              }
            }}
          />
        </PluginHostProvider>
      ) : (
        <Button variant="primary" onClick={() => onRun(item.id)}>
          Run
        </Button>
      )}
      <section className="rr-palette-detail__copy" aria-labelledby="copy-as-title">
        <h4 id="copy-as-title" className="rr-palette-detail__subtitle">
          Copy as
        </h4>
        <SegmentedControl label="Copy as" options={COPY_FORMATS} value={copyFormat} onChange={onCopyFormatChange} />
        <pre className="rr-palette-detail__code">
          <code>{formats[copyFormat]}</code>
        </pre>
        <Button onClick={onCopy}>Copy</Button>
        <p role="status" className="rr-palette-detail__status">
          {copyStatus}
        </p>
      </section>
    </section>
  );
}
