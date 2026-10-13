// Small color helpers the renderers' color functions and the palette tests share. Colors in a theme are always
// "#rrggbb" (rastai-formats rejects any other form, RRS-E2006).

const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;
const CHANNEL_MAX = 255;
const OPAQUE = 1;

/**
 * "#336699" -> [0.2, 0.4, 0.6, 1], the RGBA form WebGL takes (0 to 1). A malformed color throws: the renderers
 * must never draw with a guessed color.
 * @param {string} hex
 * @returns {number[]}
 */
export function hexToRgba(hex) {
  if (!HEX_COLOR.test(hex)) {
    throw new Error(`not a #rrggbb color: ${hex}`);
  }
  const red = Number.parseInt(hex.slice(1, 3), 16) / CHANNEL_MAX;
  const green = Number.parseInt(hex.slice(3, 5), 16) / CHANNEL_MAX;
  const blue = Number.parseInt(hex.slice(5, 7), 16) / CHANNEL_MAX;
  return [red, green, blue, OPAQUE];
}

/**
 * "#336699" -> "rgb(51, 102, 153)", the form getComputedStyle reports, so browser tests can compare a computed
 * style with a token. Kept here because color formats belong in tokens/ (check-colors).
 * @param {string} hex
 * @returns {string}
 */
export function cssRgb(hex) {
  const channels = hexToRgba(hex)
    .slice(0, 3)
    .map((channel) => Math.round(channel * CHANNEL_MAX));
  return `rgb(${channels.join(", ")})`;
}
