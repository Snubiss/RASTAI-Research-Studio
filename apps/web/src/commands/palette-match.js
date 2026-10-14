// Subsequence matching for the palette's filter (layout frontend-and-plugins.md 2.5): "trn" finds "Training". A query
// matches when its letters appear in the text in order, ignoring case. Pure, so it is tested in Node.

// How well a query matches a text, best first. A match at the start beats one inside the text, and both beat a scattered
// subsequence, so "set" ranks "Settings" above "Show status" for the same query.
const RANK_PREFIX = 0;
const RANK_CONTAINS = 1;
const RANK_SUBSEQUENCE = 2;
const RANK_NONE = -1;

/**
 * Whether a rank from matchRank is a match.
 * @param {number} rank
 * @returns {boolean}
 */
export function isMatch(rank) {
  return rank !== RANK_NONE;
}

/**
 * How well the query matches the text: a lower number is a better match, and RANK_NONE (-1) is no match.
 * @param {string} query
 * @param {string} text
 * @returns {number}
 */
export function matchRank(query, text) {
  const wanted = query.trim().toLowerCase();
  const haystack = text.toLowerCase();
  if (wanted === "") {
    return RANK_PREFIX;
  }
  if (haystack.startsWith(wanted)) {
    return RANK_PREFIX;
  }
  if (haystack.includes(wanted)) {
    return RANK_CONTAINS;
  }
  return isSubsequence(wanted, haystack) ? RANK_SUBSEQUENCE : RANK_NONE;
}

/**
 * @param {string} wanted   lowercase, not empty
 * @param {string} haystack lowercase
 * @returns {boolean}
 */
function isSubsequence(wanted, haystack) {
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
