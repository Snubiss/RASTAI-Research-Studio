// A label and one big number in the mono face, with an optional change line (ui-design.md 5.2): Training and Designer
// show their counts this way. The caller formats the number, so the tile shows exactly the text it is given.

/**
 * @param {object} props
 * @param {string} props.label
 * @param {string} props.value      the number as text, already formatted, such as "1,284"
 * @param {string} [props.delta]    the change, such as "+12 this run"; left out when empty
 */
export function StatTile({ label, value, delta = "" }) {
  return (
    <div className="rr-stat-tile">
      <p className="rr-stat-tile__label">{label}</p>
      <p className="rr-stat-tile__value">{value}</p>
      {delta ? <p className="rr-stat-tile__delta">{delta}</p> : null}
    </div>
  );
}
