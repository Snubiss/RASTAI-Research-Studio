// A section whose page this version doesn't have yet (design 6, "honest" states): it says so, with the catalog code
// RRS-C0010, and shows no made-up data. Its section id is named, so a reader knows what is missing.

import { DiagnosticView } from "@rastai/ui";

import { notAvailableDiagnostic } from "./not-available.js";

/**
 * @param {object} props
 * @param {string} props.sectionId
 */
export function NotAvailableSection({ sectionId }) {
  const diagnostic = notAvailableDiagnostic(`section ${sectionId}`);
  return (
    <div className="rr-not-available">
      <p>This section isn't available in this version of Rastai yet.</p>
      <DiagnosticView diagnostic={diagnostic} />
    </div>
  );
}
