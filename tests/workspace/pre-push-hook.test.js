// The pre-push hook (PROC-01, plan 3.6): git runs tools/hooks/pre-push before every push, and the hook refuses the
// push unless `just check` passes. Each test runs the real hook under sh in a scratch git repository, with a stand-in
// `just` first on PATH that records its arguments and exits with a chosen status, so the test needs neither the full
// check nor a network. The hook is POSIX sh, so these tests run where vitest runs sh: Linux and macOS.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { spawnSync } from "node:child_process";
// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { tmpdir } from "node:os";
import { afterEach, expect, test } from "vitest";

const HOOK_PATH = decodeURIComponent(new URL("../../tools/hooks/pre-push", import.meta.url).pathname);
const JUSTFILE_URL = new URL("../../justfile", import.meta.url);

// The first line of every refusal, and the recipe the hook must run.
const REFUSED_PREFIX = "pre-push: push refused:";
const CHECK_ARGUMENTS = "check";
const FAILED_CHECK_TEXT = "`just check` failed";
const UNCOMMITTED_TEXT = "uncommitted changes or untracked files";
const NOT_CHECKED_OUT_TEXT = "is not the checked-out commit";

// The install-hooks recipe and the one line it must run: repo-local config, never --global.
const INSTALL_HOOKS_RECIPE_LINE = "install-hooks:";
const INSTALL_HOOKS_COMMAND_LINE = "    git config core.hooksPath tools/hooks";

// Any executable bit: owner, group or others.
const EXECUTABLE_BITS = 0o111;
// A commit identity for the scratch repository, so the test never depends on the machine's git config.
const SCRATCH_GIT_IDENTITY = ["-c", "user.name=Hook Test", "-c", "user.email=hook-test@example.com"];
// Where the stand-in `just` writes the arguments it was called with, and the GIT_DIR it saw.
const ARGUMENTS_FILE_NAME = "just-arguments";
const GIT_DIR_FILE_NAME = "just-git-dir";
// What the stand-in writes when GIT_DIR is unset or empty.
const NO_GIT_DIR = "none";
const TRACKED_FILE_NAME = "notes.txt";
// A file the scratch repository never adds, like a new module someone forgot to commit.
const UNTRACKED_FILE_NAME = "forgotten.txt";
// What git writes on the hook's stdin: the branch pushed, and the all-zero name it uses for "no object".
const PUSHED_REF = "refs/heads/main";
const ZERO_OBJECT_NAME = "0".repeat(40);
const DELETION_LOCAL_REF = "(delete)";

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
const INHERITED_PATH = process.env.PATH;

/** @type {string[]} scratch folders to remove after each test */
const scratchFolders = [];

afterEach(() => {
  for (const folder of scratchFolders.splice(0)) {
    rmSync(folder, { recursive: true, force: true });
  }
});

/**
 * @typedef {object} CommandResult
 * @property {number} status the exit status
 * @property {string} stdout
 * @property {string} stderr
 */

/**
 * @param {string} command
 * @param {string[]} commandArguments
 * @param {string} folder the working folder
 * @param {string} path the PATH the command sees
 * @param {Record<string, string>} [extraEnvironment] more variables beside PATH
 * @param {string} [input] what the command reads on stdin
 * @returns {CommandResult}
 */
function runCommand(command, commandArguments, folder, path, extraEnvironment = {}, input = "") {
  const environment = { ...extraEnvironment, PATH: path };
  const options = { cwd: folder, env: environment, encoding: "utf8", input };
  const result = spawnSync(command, commandArguments, options);
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
}

/** @returns {string} a new empty folder that afterEach removes */
function makeScratchFolder() {
  const folder = mkdtempSync(`${tmpdir()}/rastai-pre-push-`);
  scratchFolders.push(folder);
  return folder;
}

/**
 * Commits `text` as the tracked file's new content.
 * @param {string} folder
 * @param {string} text
 */
function commitTrackedFile(folder, text) {
  writeFileSync(`${folder}/${TRACKED_FILE_NAME}`, text);
  runCommand("git", ["add", TRACKED_FILE_NAME], folder, INHERITED_PATH);
  runCommand("git", [...SCRATCH_GIT_IDENTITY, "commit", "--quiet", "-m", text], folder, INHERITED_PATH);
}

/**
 * @param {string} folder
 * @param {string} revision
 * @returns {string} the commit's full object name
 */
function objectNameOf(folder, revision) {
  return runCommand("git", ["rev-parse", revision], folder, INHERITED_PATH).stdout.trim();
}

/**
 * A git repository with two commits, so its tree starts clean and HEAD~1 is a commit that is not checked out.
 * @param {string} folder
 */
function makeScratchRepository(folder) {
  runCommand("git", ["init", "--quiet"], folder, INHERITED_PATH);
  commitTrackedFile(folder, "first\n");
  commitTrackedFile(folder, "second\n");
}

/**
 * The line git writes on the hook's stdin for one ref.
 * @param {string} folder the scratch repository
 * @param {PushedCommit} pushedCommit
 * @returns {string}
 */
function pushLine(folder, pushedCommit) {
  if (pushedCommit === "delete") {
    return `${DELETION_LOCAL_REF} ${ZERO_OBJECT_NAME} ${PUSHED_REF} ${objectNameOf(folder, "HEAD")}\n`;
  }
  const revision = pushedCommit === "head" ? "HEAD" : "HEAD~1";
  return `${PUSHED_REF} ${objectNameOf(folder, revision)} ${PUSHED_REF} ${ZERO_OBJECT_NAME}\n`;
}

/**
 * Leaves the scratch tree as the test asks: clean, with a changed tracked file, or with a new untracked file.
 * @param {string} folder
 * @param {TreeChange} treeChange
 */
function changeTree(folder, treeChange) {
  if (treeChange === "tracked") {
    writeFileSync(`${folder}/${TRACKED_FILE_NAME}`, "changed\n");
  }
  if (treeChange === "untracked") {
    writeFileSync(`${folder}/${UNTRACKED_FILE_NAME}`, "never added\n");
  }
}

/**
 * Writes a `just` that records its arguments and exits with `exitStatus`.
 * @param {string} folder the folder that goes first on PATH
 * @param {number} exitStatus
 */
function writeStandInJust(folder, exitStatus) {
  const script = [
    "#!/bin/sh",
    `echo "$@" > "${folder}/${ARGUMENTS_FILE_NAME}"`,
    `echo "\${GIT_DIR:-${NO_GIT_DIR}}" > "${folder}/${GIT_DIR_FILE_NAME}"`,
    `exit ${exitStatus}`,
    "",
  ].join("\n");
  writeFileSync(`${folder}/just`, script);
  chmodSync(`${folder}/just`, 0o755);
}

/**
 * @param {string} path
 * @returns {string | null} the file's text without the final newline, or null when there is no such file
 */
function readIfPresent(path) {
  return existsSync(path) ? readFileSync(path, "utf8").trim() : null;
}

/**
 * @typedef {object} HookRun
 * @property {CommandResult} result what the hook did
 * @property {string | null} justArguments what `just` was called with (null when the hook never called it)
 * @property {string | null} justGitDir the GIT_DIR `just` saw, or NO_GIT_DIR
 */

/** @typedef {"none" | "tracked" | "untracked"} TreeChange what is left uncommitted in the tree */
/** @typedef {"head" | "other" | "delete"} PushedCommit the checked-out commit, HEAD~1, or a branch deletion */

/**
 * @typedef {object} HookOptions
 * @property {number} justExitStatus what the stand-in `just` exits with
 * @property {TreeChange} [treeChange] defaults to "none"
 * @property {PushedCommit} [pushedCommit] defaults to "head"
 * @property {boolean} [gitDirSet] whether GIT_DIR is set, as git does in a linked worktree
 */

/**
 * Runs the hook in a fresh scratch repository with a stand-in `just`, feeding it one pushed ref on stdin.
 * @param {HookOptions} options
 * @returns {HookRun}
 */
function runHook(options) {
  const { justExitStatus, treeChange = "none", pushedCommit = "head", gitDirSet = false } = options;
  const repository = makeScratchFolder();
  const standInFolder = makeScratchFolder();
  makeScratchRepository(repository);
  writeStandInJust(standInFolder, justExitStatus);
  changeTree(repository, treeChange);
  /** @type {Record<string, string>} */
  const extraEnvironment = {};
  if (gitDirSet) {
    extraEnvironment.GIT_DIR = `${repository}/.git`;
  }
  const hookPath = `${standInFolder}:${INHERITED_PATH}`;
  const input = pushLine(repository, pushedCommit);
  const result = runCommand("sh", [HOOK_PATH], repository, hookPath, extraEnvironment, input);
  return {
    result,
    justArguments: readIfPresent(`${standInFolder}/${ARGUMENTS_FILE_NAME}`),
    justGitDir: readIfPresent(`${standInFolder}/${GIT_DIR_FILE_NAME}`),
  };
}

test("the hook refuses the push when just check fails, and says why", () => {
  const { result, justArguments } = runHook({ justExitStatus: 1 });
  expect(justArguments).toBe(CHECK_ARGUMENTS);
  expect(result.status).not.toBe(0);
  expect(result.stderr).toContain(REFUSED_PREFIX);
  expect(result.stderr).toContain(FAILED_CHECK_TEXT);
});

test("the hook lets the push through when just check passes", () => {
  const { result, justArguments } = runHook({ justExitStatus: 0 });
  expect(justArguments).toBe(CHECK_ARGUMENTS);
  expect(result.status).toBe(0);
  expect(result.stderr).not.toContain(REFUSED_PREFIX);
});

test("the hook refuses a tree with uncommitted changes before running the check", () => {
  const { result, justArguments } = runHook({ justExitStatus: 0, treeChange: "tracked" });
  expect(justArguments).toBeNull();
  expect(result.status).not.toBe(0);
  expect(result.stderr).toContain(REFUSED_PREFIX);
  expect(result.stderr).toContain(UNCOMMITTED_TEXT);
});

test("the hook refuses a tree with an untracked file before running the check", () => {
  const { result, justArguments } = runHook({ justExitStatus: 0, treeChange: "untracked" });
  expect(justArguments).toBeNull();
  expect(result.status).not.toBe(0);
  expect(result.stderr).toContain(REFUSED_PREFIX);
  expect(result.stderr).toContain(UNCOMMITTED_TEXT);
});

test("the hook refuses to push a commit that is not checked out, without running the check", () => {
  const { result, justArguments } = runHook({ justExitStatus: 0, pushedCommit: "other" });
  expect(justArguments).toBeNull();
  expect(result.status).not.toBe(0);
  expect(result.stderr).toContain(REFUSED_PREFIX);
  expect(result.stderr).toContain(NOT_CHECKED_OUT_TEXT);
});

test("the hook lets a branch deletion through without running the check", () => {
  const { result, justArguments } = runHook({ justExitStatus: 1, pushedCommit: "delete" });
  expect(justArguments).toBeNull();
  expect(result.status).toBe(0);
  expect(result.stderr).not.toContain(REFUSED_PREFIX);
});

test("the check runs without the GIT_DIR git sets in a linked worktree", () => {
  const { result, justGitDir } = runHook({ justExitStatus: 0, gitDirSet: true });
  expect(result.status).toBe(0);
  expect(justGitDir).toBe(NO_GIT_DIR);
});

test("the hook is executable, so git runs it", () => {
  expect(statSync(HOOK_PATH).mode & EXECUTABLE_BITS).not.toBe(0);
});

test("just install-hooks points core.hooksPath at tools/hooks in this clone's config only", () => {
  const justfileLines = readFileSync(JUSTFILE_URL, "utf8").split(/\r?\n/);
  const recipeStart = justfileLines.indexOf(INSTALL_HOOKS_RECIPE_LINE);
  expect(recipeStart).toBeGreaterThanOrEqual(0);
  expect(justfileLines[recipeStart + 1]).toBe(INSTALL_HOOKS_COMMAND_LINE);
});
