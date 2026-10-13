// A state machine's value in words and a glyph, so color is never the only signal (ui-design.md 5.2). Used for a job,
// run, eval, node, plugin or model state. The class decides the color: live is the accent, ok is success, error and
// warning are the locked severity colors, and neutral is muted. Pills sit on the panel surfaces (background, surface,
// surface_raised), where the palette test checks every tone.

/**
 * @typedef {"live" | "ok" | "error" | "warning" | "neutral"} StateTone
 */

// Glyphs are text, so the pill reads the same in every theme and with the colorblind palette.
const TONE_GLYPHS = { live: "●", ok: "✓", error: "✕", warning: "▲", neutral: "○" };

/**
 * @param {object} props
 * @param {string} props.label    the state's word, such as "Running" or "Preempted"
 * @param {StateTone} [props.tone]
 */
export function StatePill({ label, tone = "neutral" }) {
  return (
    <span className={`rr-state-pill rr-state-pill--${tone}`}>
      <span className="rr-state-pill__glyph" aria-hidden="true">
        {TONE_GLYPHS[tone]}
      </span>
      <span>{label}</span>
    </span>
  );
}
