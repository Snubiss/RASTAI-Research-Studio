/// <reference types="vite/client" />
// The web app's constants (layout frontend-and-plugins.md 8.1). Numbers and names the other files use come from here,
// so a change to a timing or a rail group is one edit.

// The version this build was made from. vite.config.js writes it; the daemon reports its own on /api/health, and a
// mismatch is RRS-U0004 (catalog). Vite fills import.meta.env in the build; the test runner doesn't, and then there is
// no version to compare, so no check runs.
export const APP_VERSION = typeof import.meta.env === "undefined" ? "" : (import.meta.env.RASTAI_APP_VERSION ?? "");

// Home is the first screen and is not in a rail group. Section ids are contracts 15.6 names.
export const HOME_SECTION_ID = "home";

// The section rail, in its groups (design 3.1). Chat is the Basic label of "inference" and Inference its Advanced
// label (design decision 9); the rail reads the effective mode.
export const SECTION_GROUPS = [
  { label: "Studio", sectionIds: ["inference", "training", "designer", "data", "gym"] },
  { label: "System", sectionIds: ["remote", "plugins", "settings"] },
];
/** @type {Object<string, { basic: string, advanced: string }>} */
export const SECTION_LABELS = {
  home: { basic: "Home", advanced: "Home" },
  inference: { basic: "Chat", advanced: "Inference" },
  training: { basic: "Training", advanced: "Training" },
  designer: { basic: "Designer", advanced: "Designer" },
  data: { basic: "Data", advanced: "Data" },
  gym: { basic: "Agent Gym", advanced: "Agent Gym" },
  remote: { basic: "Remote", advanced: "Remote" },
  plugins: { basic: "Plugins", advanced: "Plugins" },
  settings: { basic: "Settings", advanced: "Settings" },
};

// The mode a window has before ui.mode is read (contracts 11.2: advanced in the prototype).
export const DEFAULT_MODE = "advanced";
// The theme and display settings before settings.get answers (contracts 11.2).
export const DEFAULT_THEME_SETTING = "system";
export const DEFAULT_DENSITY = "comfortable";

// The event hub (layout 8.4). Durable types every window needs, whatever its listeners asked for.
export const DURABLE_EVENT_TYPES = [
  "command.finished",
  "plugin.state_changed",
  "conversation.changed",
  "job.state_changed",
  "document.changed",
];
// Changes to the union of listened-to types reopen the stream after this quiet time, so a burst of subscribes costs
// one reopen (layout 8.4: debounced 100 ms).
export const RESUBSCRIBE_DEBOUNCE_MS = 100;
// A hidden window pauses its stream after this long, and resumes it with the last sequence when shown again.
export const HIDDEN_PAUSE_MS = 60_000;

// The egress badge stays on this long after the last egress.active event (layout 8.4: two report intervals).
export const EGRESS_BADGE_HOLD_MS = 10_000;

// Job states the status bar counts as running (contracts 9.1; the daemon's JobSummary.state words).
/** @type {string[]} */
export const ACTIVE_JOB_STATES = ["queued", "starting", "running", "paused", "stopping"];

// The catalog codes the shell raises itself (catalog/errors). The fix for each is in the catalog.
export const CODE_VERSION_DIFFERS = "RRS-U0004";
export const CODE_DAEMON_UNREACHABLE = "RRS-U0001";
export const CODE_PANEL_CRASHED = "RRS-U1004";

// The keyboard: "G" then one of these letters goes to a section (design 3.7). The prefix expires after this long.
export const GO_PREFIX_TIMEOUT_MS = 1500;
/** @type {Object<string, string>} */
export const GO_KEYS = {
  h: "home",
  c: "inference",
  t: "training",
  d: "designer",
  p: "psychology",
  s: "settings",
};

// The regions F6 moves between, in order (design 3.7).
export const FOCUS_REGIONS = ["section-rail", "main", "status-bar"];

// The job kinds the rail counts beside a section (design 3.1: "Training 1"): a job's kind word maps to its section.
/** @type {Object<string, string>} */
export const JOB_KIND_SECTIONS = { train: "training" };
