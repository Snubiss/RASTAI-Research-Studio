// Home (design 6.2): the six outcome cards, the First Run checklist, and what is recent. Each region reads the daemon's
// commands through the stores and says plainly what this version can't show yet. No region computes a verdict of its
// own: "is this step done" is the daemon's to say, and it doesn't say it in this version (request R1, design 7).

import { DiagnosticView } from "@rastai/ui";

import { notAvailableDiagnostic } from "../not-available.js";
import { FirstRunChecklist } from "./FirstRunChecklist.jsx";
import { OutcomeCard } from "./OutcomeCard.jsx";

/** @typedef {import("./OutcomeCard.jsx").OutcomeCardSpec} OutcomeCardSpec */

// The six outcome cards of design 6.2, in its order. "Train an agent" is visible and disabled, not hidden.
/** @type {OutcomeCardSpec[]} */
const OUTCOME_CARDS = [
  {
    title: "Chat",
    promise: "Talk with a local model and see how it answers.",
    sectionId: "inference",
    unavailableText: "",
  },
  {
    title: "Teach it my data",
    promise: "Train a model on your own examples, then check what it learned.",
    sectionId: "training",
    unavailableText: "",
  },
  {
    title: "See inside",
    promise: "Watch the concepts a model uses while it answers.",
    sectionId: "",
    unavailableText: "Arrives with the Psychology plugin.",
  },
  {
    title: "Fix a behavior",
    promise: "Find what drives an unwanted behavior and check a fix.",
    sectionId: "",
    unavailableText: "Arrives in a later version.",
  },
  {
    title: "Design a network",
    promise: "Build a model from blocks on a canvas and see its size before training.",
    sectionId: "designer",
    unavailableText: "",
  },
  {
    title: "Train an agent",
    promise: "Teach an agent to finish tasks from its own attempts.",
    sectionId: "",
    unavailableText: "Arrives in a later version.",
  },
];

export function HomeSection() {
  return (
    <div className="rr-home">
      <section aria-labelledby="outcomes-title" className="rr-home-panel">
        <h2 id="outcomes-title" className="rr-home-panel__title">
          What do you want to do?
        </h2>
        <div className="rr-outcome-grid">
          {OUTCOME_CARDS.map((card) => (
            <OutcomeCard key={card.title} card={card} />
          ))}
        </div>
      </section>
      <FirstRunChecklist />
      <section aria-labelledby="recent-title" className="rr-home-panel">
        <h2 id="recent-title" className="rr-home-panel__title">
          Recent
        </h2>
        <DiagnosticView diagnostic={notAvailableDiagnostic("project.recent.list")} onFix={() => {}} />
      </section>
    </div>
  );
}
