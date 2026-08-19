/*
 * Pins the e2e evidence-retention rules that `e2e/outputDir.ts` implements.
 *
 * It lives under `src/test/` because vitest's `include` is `src/**` while
 * Playwright collects `e2e/**`; a test file inside `e2e/` would be picked up by
 * BOTH runners. The module under test has no browser or Playwright dependency,
 * so importing it across the root boundary costs nothing.
 *
 * What is worth pinning here is not the string format — it is the two properties
 * that were found by running the lane and getting them wrong:
 *
 *  1. one run resolves to ONE directory even though the config is loaded again
 *     by every worker process (the first attempt produced a directory per worker
 *     and scattered a single run's evidence), and
 *  2. pruning never proposes anything that is not a run directory, because
 *     `.last-run.json` shares that root and deleting it would break
 *     `--last-failed`.
 */
import { describe, expect, it } from "vitest";

import {
  DEFAULT_KEEP_RUNS,
  RUN_DIR_PREFIX,
  resolveRunId,
  runDirsToPrune,
  runOutputDir,
  runStamp,
} from "../../e2e/outputDir";

describe("runStamp", () => {
  it("is filesystem-safe and sorts lexicographically in time order", () => {
    const earlier = runStamp(new Date("2026-08-19T10:49:34.121Z"));
    const later = runStamp(new Date("2026-08-19T10:49:34.122Z"));

    expect(earlier).toBe("2026-08-19T10-49-34-121Z");
    // No ':' or '.' — both are legal on POSIX but ':' is not on Windows, and the
    // prune order is a plain string sort, so lexical order must track time.
    expect(earlier).not.toMatch(/[:.]/);
    expect(earlier < later).toBe(true);
  });
});

describe("resolveRunId", () => {
  it("stamps the id into the environment so worker processes inherit it", () => {
    const env: NodeJS.ProcessEnv = {};
    const first = resolveRunId(env, new Date("2026-08-19T10:49:34.121Z"));

    expect(first.fresh).toBe(true);
    expect(env.E2E_RUN_ID).toBe(first.id);
  });

  it("re-uses an inherited id, and reports that it is not the runner", () => {
    // The worker case: same env, a LATER clock. A second stamp here is what
    // scattered one run's artifacts across several directories.
    const env: NodeJS.ProcessEnv = {};
    const runner = resolveRunId(env, new Date("2026-08-19T10:49:34.121Z"));
    const worker = resolveRunId(env, new Date("2026-08-19T10:59:00.000Z"));

    expect(worker.id).toBe(runner.id);
    expect(worker.fresh).toBe(false);
  });

  it("gives one output directory per run, not per config load", () => {
    const env: NodeJS.ProcessEnv = {};
    const runner = runOutputDir(env, new Date("2026-08-19T10:49:34.121Z"));
    const worker = runOutputDir(env, new Date("2026-08-19T10:59:00.000Z"));

    expect(worker).toBe(runner);
    expect(runner).toContain(RUN_DIR_PREFIX);
  });

  it("lets E2E_OUTPUT_DIR override the whole layout", () => {
    const env: NodeJS.ProcessEnv = { E2E_OUTPUT_DIR: "somewhere/else" };
    expect(runOutputDir(env, new Date())).toBe("somewhere/else");
  });
});

describe("runDirsToPrune", () => {
  const runs = (...stamps: string[]) => stamps.map((s) => `${RUN_DIR_PREFIX}${s}`);

  it("keeps the newest N and proposes the rest", () => {
    const entries = runs("a", "b", "c", "d");
    expect(runDirsToPrune(entries, 2)).toEqual(runs("a", "b"));
  });

  it("proposes nothing while the directory count is within the budget", () => {
    expect(runDirsToPrune(runs("a", "b"), 2)).toEqual([]);
    expect(runDirsToPrune([], DEFAULT_KEEP_RUNS)).toEqual([]);
  });

  it("never proposes an entry it does not own", () => {
    // `.last-run.json` lives in the same root and is pinned there deliberately —
    // pruning it would silently break `--last-failed`.
    const entries = [".last-run.json", "notes.md", ...runs("a", "b", "c")];
    expect(runDirsToPrune(entries, 1)).toEqual(runs("a", "b"));
  });
});
