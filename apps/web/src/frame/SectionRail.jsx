// The section rail (design 3.1): Home, then the Studio and System groups. The current section is aria-current, and a
// count beside a section says how many of its jobs are running. Sections a version lacks are listed too, so the rail
// never changes shape; their page says they aren't available yet.

import { useStore } from "@rastai/plugin";

import { ACTIVE_JOB_STATES, HOME_SECTION_ID, JOB_KIND_SECTIONS, SECTION_GROUPS } from "../constants.js";
import { useShell } from "../app/shell-context.js";
import { sectionFragment } from "../routing/route.js";
import { sectionLabel } from "../state/mode.js";

export function SectionRail() {
  const shell = useShell();
  const display = useStore(shell.stores.display);
  const route = useStore(shell.stores.route);
  const jobs = /** @type {{ kind: string, state: string }[]} */ (useStore(shell.stores.jobs));
  const counts = runningCounts(jobs);
  return (
    <nav id="section-rail" className="rr-section-rail" aria-label="Sections" tabIndex={-1}>
      <ul className="rr-section-rail__list">
        <RailItem
          sectionId={HOME_SECTION_ID}
          label={sectionLabel(HOME_SECTION_ID, display.mode)}
          current={route.sectionId === HOME_SECTION_ID}
          count={0}
        />
      </ul>
      {SECTION_GROUPS.map((group) => (
        <div key={group.label} className="rr-section-rail__group">
          <p className="rr-section-rail__group-label">{group.label}</p>
          <ul className="rr-section-rail__list">
            {group.sectionIds.map((sectionId) => (
              <RailItem
                key={sectionId}
                sectionId={sectionId}
                label={sectionLabel(sectionId, display.mode)}
                current={route.sectionId === sectionId}
                count={counts[sectionId] ?? 0}
              />
            ))}
          </ul>
        </div>
      ))}
    </nav>
  );
}

/**
 * @param {object} props
 * @param {string} props.sectionId
 * @param {string} props.label
 * @param {boolean} props.current
 * @param {number} props.count
 */
function RailItem({ sectionId, label, current, count }) {
  return (
    <li>
      <a
        className={`rr-section-rail__link${current ? " rr-section-rail__link--current" : ""}`}
        href={sectionFragment(sectionId)}
        aria-current={current ? "page" : undefined}
      >
        <span>{label}</span>
        {count > 0 ? (
          <span className="rr-section-rail__count" aria-label={`${count} running`}>
            {count}
          </span>
        ) : null}
      </a>
    </li>
  );
}

/**
 * How many active jobs each section has, from the jobs' kinds.
 * @param {{ kind: string, state: string }[]} jobs
 * @returns {Object<string, number>}
 */
export function runningCounts(jobs) {
  /** @type {Object<string, number>} */
  const counts = {};
  for (const job of jobs) {
    const sectionId = JOB_KIND_SECTIONS[job.kind];
    if (sectionId !== undefined && ACTIVE_JOB_STATES.includes(job.state)) {
      counts[sectionId] = (counts[sectionId] ?? 0) + 1;
    }
  }
  return counts;
}
