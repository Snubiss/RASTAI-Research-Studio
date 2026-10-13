// One of the six outcome cards on Home (design 6.2): a promise, and a link to the section that delivers it (its page is
// #/<section_id>, layout 2.6). A card whose section isn't built in this version is visible and disabled, with the reason,
// rather than hidden.

/**
 * @typedef {object} OutcomeCardSpec
 * @property {string} title
 * @property {string} promise
 * @property {string} sectionId     the section it opens; "" when this version has none
 * @property {string} unavailableText   why it is disabled, shown when sectionId is ""
 */

/**
 * @param {object} props
 * @param {OutcomeCardSpec} props.card
 */
export function OutcomeCard({ card }) {
  const ready = card.sectionId !== "";
  return (
    <article className="rr-outcome-card" aria-labelledby={titleId(card)}>
      <h2 id={titleId(card)} className="rr-outcome-card__title">
        {card.title}
      </h2>
      <p className="rr-outcome-card__promise">{card.promise}</p>
      {ready ? (
        <a className="rr-outcome-card__link" href={`#/${card.sectionId}`}>
          Open {card.title}
        </a>
      ) : (
        <p className="rr-outcome-card__note" aria-disabled="true">
          {card.unavailableText}
        </p>
      )}
    </article>
  );
}

/**
 * A stable id for the card's heading, from its title.
 * @param {OutcomeCardSpec} card
 * @returns {string}
 */
function titleId(card) {
  return `outcome-${card.title.toLowerCase().replaceAll(" ", "-")}`;
}
