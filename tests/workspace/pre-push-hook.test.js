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
const UNCOMMITTED_TEXT = "uncommitted changes";

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
 * @property {string} stderr
 */

/**
 * @param {string} command
 * @param {string[]} commandArguments
 * @param {string} folder the working folder
 * @param {string} path the PATH the command sees
 * @param {Record<string, string>} [extraEnvironment] more variables beside PATH
 * @returns {CommandResult}
 */
function runCommand(command, commandArguments, folder, path, extraEnvironment = {}) {
  const environment = { ...extraEnvironment, PATH: path };
  const result = spawnSync(command, commandArguments, { cwd: folder, env: environment, encoding: "utf8" });
  return { status: result.status, stderr: result.stderr };
}

/** @returns {string} a new empty folder that afterEach removes */
function makeScratchFolder() {
  const folder = mkdtempSync(`${tmpdir()}/rastai-pre-push-`);
  scratchFolders.push(folder);
  return folder;
}

/**
 * A git repository with one committed file, so its tree starts clean.
 * @param {string} folder
 */
function makeScratchRepository(folder) {
  runCommand("git", ["init", "--quiet"], folder, INHERITED_PATH);
  writeFileSync(`${folder}/${TRACKED_FILE_NAME}`, "first\n");
  runCommand("git", ["add", TRACKED_FILE_NAME], folder, INHERITED_PATH);
  runCommand("git", [...SCRATCH_GIT_IDENTITY, "commit", "--quiet", "-m", "first"], folder, INHERITED_PATH);
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

/**
 * Runs the hook in a fresh scratch repository with a stand-in `just` that exits with `justExitStatus`.
 * @param {number} justExitStatus
 * @param {boolean} dirtyTree whether to leave an uncommitted change to a tracked file
 * @param {boolean} [gitDirSet] whether to run the hook with GIT_DIR set, as git does in a linked worktree
 * @returns {HookRun}
 */
function runHook(justExitStatus, dirtyTree, gitDirSet = false) {
  const repository = makeScratchFolder();
  const standInFolder = makeScratchFolder();
  makeScratchRepository(repository);
  writeStandInJust(standInFolder, justExitStatus);
  if (dirtyTree) {
    writeFileSync(`${repository}/${TRACKED_FILE_NAME}`, "changed\n");
  }
  /** @type {Record<string, string>} */
  const extraEnvironment = {};
  if (gitDirSet) {
    extraEnvironment.GIT_DIR = `${repository}/.git`;
  }
  const hookPath = `${standInFolder}:${INHERITED_PATH}`;
  const result = runCommand("sh", [HOOK_PATH], repository, hookPath, extraEnvironment);
  return {
    result,
    justArguments: readIfPresent(`${standInFolder}/${ARGUMENTS_FILE_NAME}`),
    justGitDir: readIfPresent(`${standInFolder}/${GIT_DIR_FILE_NAME}`),
  };
}

test("the hook refuses the push when just check fails, and says why", () => {
  const { result, justArguments } = runHook(1, false);
  expect(justArguments).toBe(CHECK_ARGUMENTS);
  expect(result.status).not.toBe(0);
  expect(result.stderr).toContain(REFUSED_PREFIX);
  expect(result.stderr).toContain(FAILED_CHECK_TEXT);
});

test("the hook lets the push through when just check passes", () => {
  const { result, justArguments } = runHook(0, false);
  expect(justArguments).toBe(CHECK_ARGUMENTS);
  expect(result.status).toBe(0);
  expect(result.stderr).not.toContain(REFUSED_PREFIX);
});

test("the hook refuses a tree with uncommitted changes before running the check", () => {
  const { result, justArguments } = runHook(0, true);
  expect(justArguments).toBeNull();
  expect(result.status).not.toBe(0);
  expect(result.stderr).toContain(REFUSED_PREFIX);
  expect(result.stderr).toContain(UNCOMMITTED_TEXT);
});

test("the check runs without the GIT_DIR git sets in a linked worktree", () => {
  const { result, justGitDir } = runHook(0, false, true);
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
