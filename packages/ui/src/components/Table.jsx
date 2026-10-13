// A plain data table: a caption, column headers and one row per item. Lists longer than about 200 rows belong in
// VirtualList instead (layout frontend-and-plugins.md 2.8); charts use this table for "Show as table" (2.9).

/** @typedef {import("react").ReactNode} ReactNode */

/**
 * One column: which field of a row it shows and its header.
 * @typedef {object} TableColumn
 * @property {string} key
 * @property {string} title
 * @property {"start" | "end"} [align]   numbers read best aligned to the end
 */

/**
 * @param {object} props
 * @param {string} props.caption                         says what the table holds; screen readers announce it
 * @param {TableColumn[]} props.columns
 * @param {Array<Object<string, ReactNode>>} props.rows
 * @param {(row: Object<string, ReactNode>, index: number) => string} [props.rowKey]   defaults to the row index
 * @param {string} [props.emptyText]                      shown in place of rows when there are none
 */
export function Table({ caption, columns, rows, rowKey = defaultRowKey, emptyText = "Nothing to show." }) {
  return (
    <table className="rr-table">
      <caption className="rr-table__caption">{caption}</caption>
      <thead>
        <tr>
          {columns.map((column) => (
            <th key={column.key} scope="col" className={alignClass(column)}>
              {column.title}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.length === 0 ? (
          <tr>
            <td className="rr-table__empty" colSpan={columns.length}>
              {emptyText}
            </td>
          </tr>
        ) : (
          rows.map((row, index) => (
            <tr key={rowKey(row, index)}>
              {columns.map((column) => (
                <td key={column.key} className={alignClass(column)}>
                  {row[column.key]}
                </td>
              ))}
            </tr>
          ))
        )}
      </tbody>
    </table>
  );
}

/**
 * @param {Object<string, ReactNode>} _row
 * @param {number} index
 * @returns {string}
 */
function defaultRowKey(_row, index) {
  return String(index);
}

/**
 * @param {TableColumn} column
 * @returns {string | undefined}
 */
function alignClass(column) {
  return column.align === "end" ? "rr-table__cell--end" : undefined;
}
