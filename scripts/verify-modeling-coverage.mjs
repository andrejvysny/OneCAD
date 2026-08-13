#!/usr/bin/env node
/*
 * The modeling-evidence manifest verifier (WP4.5).
 *
 * The manifest is the machine-readable claim about WHERE each supported operation
 * is proven. Until this pass it checked only shape: required fields present, a
 * valid `supportStatus`, and the corpus-case census. It did not open a single
 * cited file — so sixteen of them had rotted away (nine `src/tools/modelTools/*`
 * unit tests that were never written under those names, three e2e specs, an
 * `import_step.rs` that is really `step_import.rs`), and eleven rows pointed at a
 * CI job called `macos-full` that exists in no workflow. A manifest nobody can
 * falsify is not evidence; it is a comment with punctuation.
 *
 * What this now enforces, beyond the original shape checks:
 *   1. EVERY cited test path exists on disk (`;`-separated lists allowed).
 *   2. EVERY cited `ciJob` resolves to a real job id in `.github/workflows/*.yml`,
 *      and every non-empty evidence lane names the job that actually runs it —
 *      a row claiming a Playwright spec while naming only the `frontend` job is
 *      telling you the spec is gated when it is not.
 *   3. The five WP4.5 registry cross-checks: `KnownOperation`, worker dispatch,
 *      the SCHEMA §7.3 op catalogue, the frontend tool registrations, and the
 *      kernelbench operation families. A capability that exists in any of those
 *      and in no manifest row is untracked coverage, which is how a mode ships
 *      with nobody noticing it was never proven.
 *
 * Rows that are deliberately NOT `KnownOperation`s (a protocol verb, a timeline
 * lane) must be declared in the manifest's own `nonOperationRows`, so the
 * exception is data a reviewer sees rather than a special case buried here.
 */
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const manifestPath = resolve(process.env.MODELING_COVERAGE_MANIFEST ??
  resolve(root, "docs/qa/modeling-operation-coverage.json"));
const corpusDir = resolve(root, "corpus/cases");
const required = ["operation", "mode", "supportStatus", "cppTest", "rustRealWorkerTest",
  "frontendTest", "playwrightTest", "corpusCase", "kernelbenchSuite", "ciJob", "notes"];
const statuses = new Set(["supported", "deferred", "unsupported"]);
const classifications = new Set(["operation", "solver", "antiGoal", "unsupported"]);

/** Evidence lane → the CI job(s) that actually execute it. Any ONE satisfies. */
const laneJobs = {
  cppTest: ["worker-8-0-1", "linux-worker", "s3-worker"],
  rustRealWorkerTest: ["rust-8-0-1"],
  frontendTest: ["frontend"],
  playwrightTest: ["e2e-chromium", "e2e-webkit"],
  kernelbenchSuite: ["linux-kernelbench", "s4-kernelbench"],
};

/**
 * Frontend tool id → the operation(s) it authors. A tool is a GESTURE, not an op:
 * `fillet` authors both Fillet and Chamfer, `mirror` authors MirrorBody, and
 * `select`/`measure`/`datum`/`sketch` author no timeline operation of their own
 * (Sketch is authored by the sketch lane, which has its own rows).
 */
const toolOperations = {
  extrude: ["Extrude"],
  revolve: ["Revolve"],
  fillet: ["Fillet", "Chamfer"],
  boolean: ["Boolean"],
  shell: ["Shell"],
  offsetFace: ["OffsetFace"],
  hole: ["Hole"],
  linearPattern: ["LinearPattern"],
  circularPattern: ["CircularPattern"],
  mirror: ["MirrorBody"],
  transform: ["TransformBody"],
};

function fail(message) {
  process.stderr.write(`modeling coverage: ${message}\n`);
  process.exitCode = 1;
}

/** `a; b` → `["a", "b"]`; empty string → `[]`. */
function list(value) {
  return String(value).split(";").map((part) => part.trim()).filter(Boolean);
}

function read(path) {
  return readFileSync(resolve(root, path), "utf8");
}

/** Job ids of one workflow: the `^  <id>:` keys under the top-level `jobs:`. */
function workflowJobs(path) {
  const lines = read(path).split("\n");
  const start = lines.findIndex((line) => line === "jobs:");
  if (start < 0) return [];
  const ids = [];
  for (const line of lines.slice(start + 1)) {
    if (/^[A-Za-z]/.test(line)) break; // a new top-level key ends the jobs block
    const match = /^ {2}([A-Za-z0-9_-]+):\s*$/.exec(line);
    if (match) ids.push(match[1]);
  }
  return ids;
}

let manifest;
try {
  manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
} catch (error) {
  fail(`cannot parse manifest: ${error.message}`);
  process.exit();
}
if (manifest.version !== 1 || !Array.isArray(manifest.rows) ||
  !manifest.corpusCases || typeof manifest.corpusCases !== "object") {
  fail("expected version 1 rows array and corpusCases object");
}
if (!Array.isArray(manifest.nonOperationRows)) {
  fail("expected a nonOperationRows array (rows that are deliberately not KnownOperations)");
}
const nonOperation = new Set(manifest.nonOperationRows ?? []);

const jobs = new Set([
  ...workflowJobs(".github/workflows/ci.yml"),
  ...workflowJobs(".github/workflows/self-hosted.yml"),
]);
if (jobs.size === 0) {
  // A scan that finds nothing would make every `ciJob` check vacuously true.
  fail("no CI jobs parsed from .github/workflows — refusing to validate vacuously");
}

const corpus = new Set(readdirSync(corpusDir)
  .filter((name) => name.endsWith(".json"))
  .map((name) => JSON.parse(readFileSync(resolve(corpusDir, name), "utf8")).id));
const covered = new Map();
const operations = new Set();

for (const [index, row] of manifest.rows.entries()) {
  for (const field of required) {
    if (typeof row[field] !== "string") fail(`row ${index} missing string ${field}`);
  }
  if (!statuses.has(row.supportStatus)) fail(`row ${index} has invalid supportStatus`);
  if (!row.operation || !row.mode) fail(`row ${index} needs operation and mode`);
  operations.add(row.operation);
  const where = `${row.operation}/${row.mode}`;

  // 1. Every cited path exists. A dead path is worse than an empty field: the
  //    empty field says "unproven", the dead path says "proven" and is not.
  for (const field of ["cppTest", "rustRealWorkerTest", "frontendTest", "playwrightTest"]) {
    for (const path of list(row[field])) {
      if (!existsSync(resolve(root, path))) fail(`${where} ${field} names a missing file ${path}`);
    }
  }

  // 2. Every named job is real, and every lane with evidence names its job.
  const named = list(row.ciJob);
  for (const job of named) {
    if (!jobs.has(job)) fail(`${where} names ciJob ${job}, which is not a job in any workflow`);
  }
  for (const [lane, owners] of Object.entries(laneJobs)) {
    if (list(row[lane]).length === 0) continue;
    if (!named.some((job) => owners.includes(job))) {
      fail(`${where} cites ${lane} but names no job that runs it (expected one of ${owners.join(", ")})`);
    }
  }

  if (!row.corpusCase) continue;
  if (!corpus.has(row.corpusCase)) fail(`row ${index} names unknown corpus case ${row.corpusCase}`);
  covered.set(row.corpusCase, (covered.get(row.corpusCase) ?? 0) + 1);
}

for (const id of corpus) {
  if (covered.get(id) !== 1) fail(`corpus case ${id} needs exactly one manifest row`);
  if (!classifications.has(manifest.corpusCases[id])) {
    fail(`corpus case ${id} needs an explicit classification`);
  }
}
for (const id of Object.keys(manifest.corpusCases)) {
  if (!corpus.has(id)) fail(`classification names unknown corpus case ${id}`);
}

// ── 3. The five WP4.5 registry cross-checks ─────────────────────────────────
//
// Each scan asserts it found something first: a regex that silently stops
// matching would otherwise turn its whole cross-check green.
function crossCheck(label, names) {
  if (names.length === 0) {
    fail(`${label}: scanned zero entries — the source moved and this check went vacuous`);
    return;
  }
  for (const name of names) {
    if (!operations.has(name)) fail(`${label} knows ${name}, which has no manifest row`);
  }
}

const knownOperations = [...read("src-tauri/crates/onecad-core/src/document/record.rs")
  .matchAll(/^pub enum KnownOperation \{([\s\S]*?)^\}/gm)]
  .flatMap((block) => [...block[1].matchAll(/^\s{4}([A-Z][A-Za-z]*)\(/gm)].map((m) => m[1]));
crossCheck("KnownOperation", knownOperations);

const dispatched = [...read("worker/src/session/PlanExecutor.cpp")
  .matchAll(/op_type == "([A-Za-z]+)"/g)].map((m) => m[1]);
crossCheck("worker dispatch", [...new Set(dispatched)]);

const schemaOps = [...read("protocol/SCHEMA.md")
  .matchAll(/^\*\*([A-Za-z]+)\*\* \(`op\.[A-Za-z]+`\)/gm)].map((m) => m[1]);
crossCheck("SCHEMA §7.3 op catalogue", [...new Set(schemaOps)]);

const toolIds = [...read("src/modules/modeling/tools.ts")
  .matchAll(/\{ tool: "([A-Za-z]+)", scope: "model"/g)].map((m) => m[1]);
if (toolIds.length === 0) fail("frontend tool registrations: scanned zero tools");
crossCheck("frontend tool registrations",
  [...new Set(toolIds.flatMap((tool) => toolOperations[tool] ?? []))]);

const families = [...read("src-tauri/crates/onecad-kernelbench/src/case_v2.rs")
  .matchAll(/^pub enum OperationFamily \{([\s\S]*?)^\}/gm)]
  .flatMap((block) => [...block[1].matchAll(/^\s{4}([A-Z][A-Za-z]*),/gm)].map((m) => m[1]));
crossCheck("kernelbench families", families);
for (const family of families) {
  const suites = manifest.rows
    .filter((row) => row.operation === family)
    .flatMap((row) => list(row.kernelbenchSuite));
  if (suites.length === 0) {
    fail(`kernelbench family ${family} has manifest rows but none names a kernelbenchSuite`);
  }
}

// The reverse direction: a row naming an operation no registry knows is either a
// typo or a capability that quietly disappeared.
const registryNames = new Set([...knownOperations, ...dispatched, ...schemaOps]);
for (const operation of operations) {
  if (registryNames.has(operation) || nonOperation.has(operation)) continue;
  fail(`row operation ${operation} is in no registry and is not declared in nonOperationRows`);
}
for (const declared of nonOperation) {
  if (!operations.has(declared)) fail(`nonOperationRows declares ${declared}, which has no row`);
}

if (!process.exitCode) {
  process.stdout.write(
    `modeling coverage: ${manifest.rows.length} rows, ${corpus.size} corpus cases, ` +
    `${jobs.size} CI jobs, ${registryNames.size} registry operations\n`,
  );
}
