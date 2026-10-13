// Subsequence matching for the palette's filter (layout frontend-and-plugins.md 2.5): "trn" finds "Training". A query
// matches when its letters appear in the text in order, ignoring case. Pure, so it is tested in Node.

/**
 * Whether every character of the query appears in the text, in order, ignoring case. An empty query matches all.
 * @param {string} query
 * @param {string} text
 * @returns {boolean}
 */
export function matchesQuery(query, text) {
  const wanted = query.trim().toLowerCase();
  const haystack = text.toLowerCase();
  let position = 0;
  for (const character of wanted) {
    const found = haystack.indexOf(character, position);
    if (found === -1) {
      return false;
    }
    position = found + 1;
  }
  return true;
}
