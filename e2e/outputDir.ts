/*
 * Run-scoped Playwright output directory.
 *
 * ── The defect this exists to fix ────────────────────────────────────────────
 * Playwright wipes the ENTIRE `outputDir` in the `clear output` setup task of
 * every run (`createRemoveOutputDirsTask`, playwright/lib/runner). With the
 * default flat `test-results/`, the FIRST triage re-run destroys the evidence of
 * the run being triaged — the specs a sweep just failed are exactly the specs a
 * human re-runs next.
 *
 * That is measured, not theoretical. The 2026-08-17 close-out sweep failed one
 * test and `test-results/` was later found holding only `.last-run.json` — no
 * `console.log`, no `pageerror.log`, no `fe-logs.json`, though `fixtures.ts`
 * writes all three on failure. Only-`.last-run.json` is precisely what a clean
 * later run leaves behind. MC-R9's own rule is that a browser-lane
 * nondeterminism closes on measured evidence and never on a clean re-run, so a
 * lane that deletes its own evidence can never close it.
 *
 * ── The fix ──────────────────────────────────────────────────────────────────
 * Every run gets `test-results/run-<stamp>`. The wipe then targets a directory
 * that does not exist yet, and earlier runs' traces, screenshots and attachments
 * survive untouched.
 *
 * `.last-run.json` is pinned back to the stable root via
 * `PLAYWRIGHT_LAST_RUN_OUTPUT_FILE` (playwright/lib/runner `LastRunReporter`),
 * because it lives in `outputDir` by default and `--last-failed` would otherwise
 * read a fresh, empty directory on every run and never filter anything.
 *
 * Retention is bounded here rather than left to grow: traces are megabytes, and
 * a lane nobody prunes is a lane somebody disables.
 */
import fs from "node:fs";
import path from "node:path";

/** Where all run directories live. Also holds the pinned `.last-run.json`. */
export const RESULTS_ROOT = "test-results";

/** Prefix for a single run's directory. `pruneRunDirs` only ever touches these. */
export const RUN_DIR_PREFIX = "run-";

/** How many run directories survive a prune, newest first. */
export const DEFAULT_KEEP_RUNS = 10;

/**
 * A filesystem-safe, lexicographically sortable stamp: `2026-08-19T10-49-34-121Z`.
 * Sortable matters — `pruneRunDirs` orders by name, so it needs no `stat` call
 * and cannot be confused by a directory whose mtime moved after the run.
 */
export function runStamp(now: Date = new Date()): string {
  return now.toISOString().replace(/[:.]/g, "-");
}

/**
 * Resolve this run's id, memoized THROUGH THE ENVIRONMENT.
 *
 * Load-bearing, and found by running it: Playwright loads the config in the
 * runner process AND again in every worker process — and spawns a fresh worker
 * after a test dies. A stamp computed per load therefore produced a DIFFERENT
 * directory per worker, scattering one run's evidence across several. Worker
 * processes inherit `process.env`, so stamping the id into the environment on
 * the first load is what makes every later load agree.
 *
 * `fresh` is true only for that first load, which is how the caller knows it is
 * the runner and may prune.
 */
export function resolveRunId(
  env: NodeJS.ProcessEnv = process.env,
  now: Date = new Date(),
): { id: string; fresh: boolean } {
  const existing = env.E2E_RUN_ID?.trim();
  if (existing) return { id: existing, fresh: false };
  const id = runStamp(now);
  env.E2E_RUN_ID = id;
  return { id, fresh: true };
}

/**
 * The output directory for THIS run.
 *
 * `E2E_OUTPUT_DIR` overrides it wholesale (CI, or a human who wants a named
 * directory); `E2E_RUN_ID` names just the run without changing the layout, and
 * is also how the runner hands its stamp to its workers.
 */
export function runOutputDir(env: NodeJS.ProcessEnv = process.env, now: Date = new Date()): string {
  const override = env.E2E_OUTPUT_DIR?.trim();
  if (override) return override;
  return path.join(RESULTS_ROOT, `${RUN_DIR_PREFIX}${resolveRunId(env, now).id}`);
}

/** The stable path `.last-run.json` is pinned to, outside any run directory. */
export function lastRunFile(): string {
  return path.join(RESULTS_ROOT, ".last-run.json");
}

/**
 * Names of the run directories to DELETE, given every entry in the results root.
 *
 * Pure so it can be asserted without a filesystem. Non-`run-` entries are never
 * returned — `.last-run.json` and anything a human parked there is not ours to
 * remove.
 */
export function runDirsToPrune(entries: readonly string[], keep: number = DEFAULT_KEEP_RUNS): string[] {
  const runs = entries.filter((e) => e.startsWith(RUN_DIR_PREFIX)).sort();
  if (keep <= 0) return runs;
  return runs.slice(0, Math.max(0, runs.length - keep));
}

/**
 * Delete all but the newest `keep` run directories.
 *
 * Best-effort by design: a prune failure must never fail a test run, so every
 * error is swallowed. It runs at config load, i.e. BEFORE this run's directory
 * exists, so the current run is never a prune candidate.
 *
 * Call it ONLY from the runner — `resolveRunId().fresh` is the discriminator. A
 * worker re-loading the config must not delete directories the run it belongs to
 * is still writing into.
 */
export function pruneRunDirs(root: string = RESULTS_ROOT, keep: number = DEFAULT_KEEP_RUNS): void {
  let entries: string[];
  try {
    entries = fs.readdirSync(root);
  } catch {
    return; // No results root yet — nothing to prune.
  }
  for (const name of runDirsToPrune(entries, keep)) {
    try {
      fs.rmSync(path.join(root, name), { recursive: true, force: true });
    } catch {
      // Ignore: a locked or already-removed directory is not this lane's problem.
    }
  }
}
