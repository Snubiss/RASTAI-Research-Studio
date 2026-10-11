// The CI workflows against the plan and the repo's pins (M1-06). ci.yml must hold exactly the jobs branch protection
// requires (plan 8.4), each running one `just` recipe; nightly.yml must fuzz every cargo-fuzz target in crates/; both
// run only when started by hand while the repository is private (PROC-01); and tools/ci/install-dev-tools must
// install the versions the rest of the repo pins. The files are read as plain text,
// line by line: these few facts need no YAML parser (none is in spec §27). Lines split on "\r\n" too, so a Windows
// checkout with CRLF endings passes the same way.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { expect, test } from "vitest";
import packageFile from "../../package.json" with { type: "json" };

const REPO_ROOT_URL = new URL("../../", import.meta.url);

const CI_WORKFLOW = ".github/workflows/ci.yml";
const NIGHTLY_WORKFLOW = ".github/workflows/nightly.yml";
const PLAN = "docs/plan/implementation-plan.md";
const INSTALL_SCRIPT = "tools/ci/install-dev-tools";
const JUSTFILE = "justfile";
const PYTHON_WORKSPACE = "python/pyproject.toml";
const GIT_ATTRIBUTES = ".gitattributes";
const CRATES_FOLDER = "crates/";
const PYTHON_FOLDER = "python/";

// The jobs that run the Python tests without the posix_only ones (plan 8.7).
const PORTABLE_PYTHON_JOBS = ["test-python-macos", "test-python-windows"];
const PORTABLE_PYTHON_COMMAND = "just test-python-portable";
// The pure-compute packages whose tests those jobs run, once each package exists (plan 8.7).
const PURE_COMPUTE_PACKAGES = ["rastai_formats", "rastai_models", "rastai_data", "rastai_train"];

// The `checks` job runs `just check`, whose codegen-check and proto-check need buf and the Rust protoc plugins, and
// whose proto-check reads the merge base with origin/main, so it needs the history.
const CHECKS_JOB = "checks";
const CHECKS_TOOLS = ["just", "uv", "node", "cargo-deny", "buf", "protoc-gen-prost", "protoc-gen-tonic"];
const INSTALL_TOOLS_LINE = /^ *run: tools\/ci\/install-dev-tools (.*)$/;
const FULL_HISTORY_LINE = "fetch-depth: 0";
const PROTOC_PLUGIN_CACHE_LINES = ["~/.cargo/bin/protoc-gen-prost", "~/.cargo/bin/protoc-gen-tonic"];

// A job id: a key indented by exactly two spaces. `jobs:` is the last top-level key of ci.yml, so after it every
// such key is a job.
const JOB_LINE = /^ {2}([a-z0-9-]+):$/;
// A step that names an anchor (`- &install-just`) or reuses one (`- *install-just`).
const ANCHOR_LINE = /^( *)- &([a-z0-9-]+)$/;
const ALIAS_LINE = /^ *- \*([a-z0-9-]+)$/;
// A step's `run: just ...`, on its own line or on the step's first line (`- run: just ...`).
const JUST_RUN_LINE = /^ *(?:- )?run: (just .*)$/;
// plan 8.4 keeps the required checks on one line of its protection.json.
const CONTEXTS_LINE = /"contexts": (\[.*\])/;
// The .gitattributes line that turns off end-of-line conversion for every file.
const NO_EOL_CONVERSION_LINE = "* -text";
// One entry of nightly.yml's fuzz matrix.
const FUZZ_ENTRY_LINE = /^ *- \{ crate: ([a-z0-9-]+), target: ([a-z0-9_]+) \}$/;
// The name of a `[[bin]]` (a fuzz target) in a cargo-fuzz Cargo.toml.
const BIN_NAME_LINE = /^name = "([a-z0-9_]+)"$/;
// nightly.yml caches the cargo-fuzz binary and builds it only when that cache misses, so no job rebuilds it nightly.
const CARGO_FUZZ_CACHE_PATH_LINE = "path: ~/.cargo/bin/cargo-fuzz";
const CARGO_FUZZ_INSTALL_CONDITION_LINE = "if: steps.cargo-fuzz-cache.outputs.cache-hit != 'true'";
// A run that isn't a pull request's has a concurrency group of its own (its commit), so no run loses its result to a
// later one.
const CONCURRENCY_GROUP_LINE = /^ {2}group: (.*)$/;
const COMMIT_IN_GROUP = "github.sha";
// nightly.yml carries each target's fuzz corpus from one night to the next in the cache.
const FUZZ_CORPUS_CACHE_PATH_LINE = "path: crates/${{ matrix.crate }}/fuzz/corpus/${{ matrix.target }}";
// Both workflows run only when started by hand while the repository is private (owner decision of 2026-10-11,
// PROC-01; plan 3.6): the local pre-push hook and the verifier's fresh-clone run are the gate instead.
const WORKFLOWS = [CI_WORKFLOW, NIGHTLY_WORKFLOW];
const TRIGGERS_LINE = "on:";
const MANUAL_ONLY_TRIGGERS = ["workflow_dispatch:"];
// The cpu environment's interpreter has one spelling in the justfile, cpu_python, which is right on every OS.
const CPU_PYTHON_BY_HAND = "{{cpu_environment}}/bin/python";

/**
 * @param {string} path a path relative to the repository root
 * @returns {string[]}
 */
function readLines(path) {
  return readFileSync(new URL(path, REPO_ROOT_URL), "utf8").split(/\r?\n/);
}

/** @returns {string[]} the required status checks of plan 8.4 */
function requiredChecks() {
  for (const line of readLines(PLAN)) {
    const match = line.match(CONTEXTS_LINE);
    if (match) {
      return JSON.parse(match[1]);
    }
  }
  throw new Error(`no "contexts" line in ${PLAN}`);
}

/**
 * Splits a workflow into its jobs: each job id with the lines that follow it, up to the next job.
 * @param {string[]} lines
 * @returns {Map<string, string[]>}
 */
function jobBlocks(lines) {
  const jobs = new Map();
  const jobsStart = lines.indexOf("jobs:");
  /** @type {string[] | null} the lines of the job being read; null before the first job */
  let currentLines = null;
  for (const line of lines.slice(jobsStart + 1)) {
    const match = line.match(JOB_LINE);
    if (match) {
      currentLines = [];
      jobs.set(match[1], currentLines);
    } else if (currentLines) {
      currentLines.push(line);
    }
  }
  return jobs;
}

/**
 * The `just` command each anchored step runs, if any, so a job that reuses the step (`- *name`) counts it too.
 * An anchored step is the lines below its `- &name` that are indented deeper than the dash.
 * @param {string[]} lines
 * @returns {Map<string, string>}
 */
function anchoredJustCommands(lines) {
  const commands = new Map();
  lines.forEach((line, index) => {
    const anchor = line.match(ANCHOR_LINE);
    if (!anchor) {
      return;
    }
    const dashIndent = anchor[1].length;
    for (const stepLine of lines.slice(index + 1)) {
      const indent = stepLine.length - stepLine.trimStart().length;
      if (stepLine.trim() !== "" && indent <= dashIndent) {
        break;
      }
      const run = stepLine.match(JUST_RUN_LINE);
      if (run) {
        commands.set(anchor[2], run[1]);
      }
    }
  });
  return commands;
}

/**
 * Every `just` command a job runs, written in the job or reused from an anchored step.
 * @param {string[]} jobLines
 * @param {Map<string, string>} anchoredCommands
 * @returns {string[]}
 */
function justCommandsOfJob(jobLines, anchoredCommands) {
  const commands = [];
  for (const line of jobLines) {
    const run = line.match(JUST_RUN_LINE);
    const alias = line.match(ALIAS_LINE);
    const reusedCommand = alias ? anchoredCommands.get(alias[1]) : undefined;
    if (run) {
      commands.push(run[1]);
    } else if (reusedCommand) {
      commands.push(reusedCommand);
    }
  }
  return commands;
}

/** @returns {Map<string, string[]>} each ci.yml job id with the `just` commands it runs */
function ciJustCommands() {
  const lines = readLines(CI_WORKFLOW);
  const anchoredCommands = anchoredJustCommands(lines);
  const commands = new Map();
  for (const [job, jobLines] of jobBlocks(lines)) {
    commands.set(job, justCommandsOfJob(jobLines, anchoredCommands));
  }
  return commands;
}

/**
 * The lines of one ci.yml job.
 * @param {string} job
 * @returns {string[]}
 */
function ciJobLines(job) {
  const lines = jobBlocks(readLines(CI_WORKFLOW)).get(job);
  if (!lines) {
    throw new Error(`no job ${job} in ${CI_WORKFLOW}`);
  }
  return lines;
}

/**
 * The tools a job's `run: tools/ci/install-dev-tools ...` step names.
 * @param {string[]} jobLines
 * @returns {string[]}
 */
function installedTools(jobLines) {
  for (const line of jobLines) {
    const match = line.match(INSTALL_TOOLS_LINE);
    if (match) {
      return match[1].split(" ");
    }
  }
  throw new Error("the job never runs tools/ci/install-dev-tools");
}

/** @returns {string[]} "<crate>/<target>" for every [[bin]] in crates/<crate>/fuzz/Cargo.toml */
function fuzzTargetsInCrates() {
  const targets = [];
  for (const crate of readdirSync(new URL(CRATES_FOLDER, REPO_ROOT_URL))) {
    const fuzzManifest = `${CRATES_FOLDER}${crate}/fuzz/Cargo.toml`;
    if (!existsSync(new URL(fuzzManifest, REPO_ROOT_URL))) {
      continue;
    }
    let inBinSection = false;
    for (const line of readLines(fuzzManifest)) {
      if (line.startsWith("[")) {
        inBinSection = line === "[[bin]]";
      }
      const name = line.match(BIN_NAME_LINE);
      if (inBinSection && name) {
        targets.push(`${crate}/${name[1]}`);
      }
    }
  }
  return targets.sort();
}

/** @returns {string[]} "<crate>/<target>" for every entry of nightly.yml's fuzz matrix */
function fuzzTargetsInNightly() {
  const targets = [];
  for (const line of readLines(NIGHTLY_WORKFLOW)) {
    const entry = line.match(FUZZ_ENTRY_LINE);
    if (entry) {
      targets.push(`${entry[1]}/${entry[2]}`);
    }
  }
  return targets.sort();
}

/**
 * The triggers under a workflow's top-level `on:` key, one trimmed line each, comments left out. The block ends at
 * the next line that starts in the first column.
 * @param {string} path
 * @returns {string[]}
 */
function workflowTriggers(path) {
  const lines = readLines(path);
  const triggersStart = lines.indexOf(TRIGGERS_LINE);
  if (triggersStart < 0) {
    throw new Error(`no "${TRIGGERS_LINE}" line in ${path}`);
  }
  const triggers = [];
  for (const line of lines.slice(triggersStart + 1)) {
    const startsInFirstColumn = line !== "" && !line.startsWith(" ");
    if (startsInFirstColumn) {
      break;
    }
    const trimmed = line.trim();
    if (trimmed !== "" && !trimmed.startsWith("#")) {
      triggers.push(trimmed);
    }
  }
  return triggers;
}

/**
 * The value of the first line that matches `pattern`; its first group is the value.
 * @param {string} path
 * @param {RegExp} pattern
 * @returns {string}
 */
function findValue(path, pattern) {
  for (const line of readLines(path)) {
    const match = line.match(pattern);
    if (match) {
      return match[1];
    }
  }
  throw new Error(`no line in ${path} matches ${pattern}`);
}

/**
 * A constant at the top of the install script, written NAME="value".
 * @param {string} name
 * @returns {string}
 */
function installScriptConstant(name) {
  return findValue(INSTALL_SCRIPT, new RegExp(`^${name}="([^"]+)"$`));
}

/**
 * A justfile variable, written name := "value".
 * @param {string} name
 * @returns {string}
 */
function justfileVariable(name) {
  return findValue(JUSTFILE, new RegExp(`^${name} := "([^"]+)"$`));
}

test("ci.yml has exactly the jobs branch protection requires (plan 8.4)", () => {
  const jobs = [...ciJustCommands().keys()].sort();
  expect(jobs).toEqual(requiredChecks().sort());
});

test("every ci.yml job runs exactly one just recipe", () => {
  for (const [job, commands] of ciJustCommands()) {
    expect(commands, job).toHaveLength(1);
  }
});

test("the macOS and Windows Python jobs leave out the posix_only tests", () => {
  const commands = ciJustCommands();
  for (const job of PORTABLE_PYTHON_JOBS) {
    expect(commands.get(job), job).toEqual([PORTABLE_PYTHON_COMMAND]);
  }
  const recipe = readLines(JUSTFILE).find((line) => line.includes("-m pytest -m"));
  expect(recipe).toContain('-m "not posix_only"');
});

test("test-python-portable runs the tests of every pure-compute package that exists", () => {
  const portableFolders = justfileVariable("portable_test_folders").split(" ");
  for (const folder of portableFolders) {
    expect(existsSync(new URL(`${PYTHON_FOLDER}${folder}`, REPO_ROOT_URL)), folder).toBe(true);
  }
  for (const packageName of PURE_COMPUTE_PACKAGES) {
    const testsFolder = `${packageName}/tests`;
    if (existsSync(new URL(`${PYTHON_FOLDER}${testsFolder}`, REPO_ROOT_URL))) {
      expect(portableFolders).toContain(testsFolder);
    }
  }
});

test("the checks job installs every tool just check needs and fetches the history proto-check reads", () => {
  const jobLines = ciJobLines(CHECKS_JOB);
  expect(installedTools(jobLines)).toEqual(CHECKS_TOOLS);
  const trimmedLines = jobLines.map((line) => line.trim());
  expect(trimmedLines).toContain(FULL_HISTORY_LINE);
  for (const cacheLine of PROTOC_PLUGIN_CACHE_LINES) {
    expect(trimmedLines).toContain(cacheLine);
  }
});

test("nightly.yml fuzzes every cargo-fuzz target in crates/", () => {
  expect(fuzzTargetsInNightly()).toEqual(fuzzTargetsInCrates());
});

test("nightly.yml builds cargo-fuzz only when its cached binary is missing", () => {
  const lines = readLines(NIGHTLY_WORKFLOW).map((line) => line.trim());
  expect(lines).toContain(CARGO_FUZZ_CACHE_PATH_LINE);
  expect(lines).toContain(CARGO_FUZZ_INSTALL_CONDITION_LINE);
});

test("ci.yml and nightly.yml run only when started by hand (workflow_dispatch)", () => {
  for (const workflow of WORKFLOWS) {
    expect(workflowTriggers(workflow), workflow).toEqual(MANUAL_ONLY_TRIGGERS);
  }
});

test("ci.yml gives each run outside a pull request a concurrency group of its own commit", () => {
  const group = findValue(CI_WORKFLOW, CONCURRENCY_GROUP_LINE);
  expect(group).toContain(COMMIT_IN_GROUP);
});

test("nightly.yml keeps each target's fuzz corpus from one night to the next", () => {
  const lines = readLines(NIGHTLY_WORKFLOW).map((line) => line.trim());
  expect(lines).toContain(FUZZ_CORPUS_CACHE_PATH_LINE);
});

test("the justfile reaches the cpu environment's Python only through cpu_python", () => {
  const justfileText = readLines(JUSTFILE).join("\n");
  expect(justfileText).not.toContain(CPU_PYTHON_BY_HAND);
});

test("install-dev-tools installs the versions the repo pins elsewhere", () => {
  expect(installScriptConstant("NODE_VERSION")).toBe(packageFile.engines.node);
  expect(installScriptConstant("UV_VERSION")).toBe(findValue(PYTHON_WORKSPACE, /^required-version = "==([^"]+)"/));
  expect(installScriptConstant("CARGO_DENY_VERSION")).toBe(justfileVariable("cargo_deny_version"));
  expect(installScriptConstant("BUF_VERSION")).toBe(justfileVariable("buf_version"));
  expect(installScriptConstant("PROTOC_GEN_PROST_VERSION")).toBe(justfileVariable("protoc_gen_prost_version"));
  expect(installScriptConstant("PROTOC_GEN_TONIC_VERSION")).toBe(justfileVariable("protoc_gen_tonic_version"));
  expect(readLines(JUSTFILE)).toContain(`#   cargo install --locked just@${installScriptConstant("JUST_VERSION")}`);
});

test("install-dev-tools checks uv, Node.js and buf against the SHA-256 the justfile header gives", () => {
  const justfileText = readLines(JUSTFILE).join("\n");
  expect(justfileText).toContain(installScriptConstant("UV_SHA256_LINUX"));
  expect(justfileText).toContain(installScriptConstant("NODE_SHA256_LINUX"));
  expect(justfileText).toContain(installScriptConstant("BUF_SHA256_LINUX"));
});

test("every OS checks out the bytes git stores, so the Windows runners see the same snapshots", () => {
  expect(readLines(GIT_ATTRIBUTES)).toContain(NO_EOL_CONVERSION_LINE);
});
