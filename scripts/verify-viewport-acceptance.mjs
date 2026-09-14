#!/usr/bin/env node
/*
 * VP-HARDENING acceptance-matrix verifier (04-ACCEPTANCE-AND-TESTS.md §1).
 *
 * The acceptance plan defines 127 stable case IDs (`TEST-LINE-01` …). Each ID
 * must (a) exist in the matrix, (b) name the lanes the plan assigns it, and
 * (c) once claimed `implemented`, be discoverable in at least one test source
 * file by its literal ID token. A discovered test is NOT a passing test — the
 * `state` column records the last measured outcome and is updated only from a
 * recorded run under `docs/qa/viewport-hardening/runs/`.
 *
 * Checks enforced:
 *   1. The matrix and the plan document agree on the exact ID set and lanes.
 *   2. Every row whose `implementedIn` is non-empty cites files that exist and
 *      that contain the ID token literally (title or metadata).
 *   3. Every row with state `focused-pass`/`integrated-pass`/`native-accepted`
 *      has a non-empty `evidence` pointing at an existing run artifact.
 *   4. No row is `native-accepted` unless one of its lanes is N or P.
 *
 * Usage: node scripts/verify-viewport-acceptance.mjs [--regen]
 *   --regen rewrites the matrix ID/lane columns from the plan document while
 *   preserving every hand-maintained column (state, implementedIn, evidence, notes).
 */
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const planPath = resolve(root, "docs/viewport-hardening/04-ACCEPTANCE-AND-TESTS.md");
const matrixPath = resolve(root, "docs/qa/viewport-hardening/acceptance-matrix.json");

const STATES = new Set([
  "not-started", "reproducing", "implementing", "focused-pass",
  "integrated-pass", "native-accepted", "blocked-native", "blocked-device", "failed",
]);

function fail(message) {
  process.stderr.write(`viewport acceptance: ${message}\n`);
  process.exitCode = 1;
}

/** Parse `| TEST-X-NN | procedure | result | lanes |` rows from the plan. */
function readPlan() {
  const text = readFileSync(planPath, "utf8");
  const out = new Map();
  for (const line of text.split("\n")) {
    const m = /^\| (TEST-[A-Z]+-\d{2}) \| (.*) \| (.*) \| ([A-Z,]+) \|$/.exec(line);
    if (!m) continue;
    out.set(m[1], { id: m[1], lanes: m[4].split(","), procedure: m[2].trim(), required: m[3].trim() });
  }
  return out;
}

function readMatrix() {
  if (!existsSync(matrixPath)) return { version: 1, cases: [] };
  return JSON.parse(readFileSync(matrixPath, "utf8"));
}

const plan = readPlan();
if (plan.size !== 127) fail(`plan defines ${plan.size} IDs, expected 127`);

if (process.argv.includes("--regen")) {
  const prev = new Map(readMatrix().cases.map((c) => [c.id, c]));
  const cases = [...plan.values()].map((p) => ({
    id: p.id,
    lanes: p.lanes,
    state: prev.get(p.id)?.state ?? "not-started",
    workPackage: prev.get(p.id)?.workPackage ?? "",
    implementedIn: prev.get(p.id)?.implementedIn ?? [],
    evidence: prev.get(p.id)?.evidence ?? "",
    notes: prev.get(p.id)?.notes ?? "",
  }));
  writeFileSync(matrixPath, JSON.stringify({ version: 1, cases }, null, 2) + "\n");
  process.stdout.write(`viewport acceptance: regenerated ${cases.length} rows\n`);
}

const matrix = readMatrix();
const seen = new Set();
for (const c of matrix.cases) {
  if (seen.has(c.id)) fail(`${c.id}: duplicate row`);
  seen.add(c.id);
  const p = plan.get(c.id);
  if (!p) { fail(`${c.id}: not defined in the plan`); continue; }
  if (p.lanes.join(",") !== (c.lanes ?? []).join(",")) {
    fail(`${c.id}: lanes ${JSON.stringify(c.lanes)} differ from plan ${JSON.stringify(p.lanes)}`);
  }
  if (!STATES.has(c.state)) fail(`${c.id}: unknown state ${c.state}`);
  for (const file of c.implementedIn ?? []) {
    const abs = resolve(root, file);
    if (!existsSync(abs)) { fail(`${c.id}: cited test file missing: ${file}`); continue; }
    if (!readFileSync(abs, "utf8").includes(c.id)) fail(`${c.id}: ${file} does not contain the ID token`);
  }
  const passing = ["focused-pass", "integrated-pass", "native-accepted"].includes(c.state);
  if (passing && (c.implementedIn ?? []).length === 0) fail(`${c.id}: ${c.state} without an implementation citation`);
  if (passing && !c.evidence) fail(`${c.id}: ${c.state} without an evidence path`);
  if (passing && c.evidence) {
    const runsPrefix = "docs/qa/viewport-hardening/runs/";
    if (c.evidence.endsWith(".log") || !c.evidence.endsWith("manifest.json") || !c.evidence.startsWith(runsPrefix)) {
      fail(`${c.id}: evidence must be a runs/<id>/manifest.json, not a log`);
    } else {
      const abs = resolve(root, c.evidence);
      if (!existsSync(abs)) {
        fail(`${c.id}: evidence path missing: ${c.evidence}`);
      } else {
        let parsed;
        try {
          parsed = JSON.parse(readFileSync(abs, "utf8"));
        } catch {
          fail(`${c.id}: evidence manifest does not parse as JSON: ${c.evidence}`);
        }
        if (parsed) {
          const commitOk = typeof parsed.commit === "string" && /^[0-9a-f]{40}$/.test(parsed.commit);
          if (!commitOk) fail(`${c.id}: evidence manifest missing a valid 40-hex commit`);
          const lanes = parsed.lanes && typeof parsed.lanes === "object" ? Object.values(parsed.lanes) : [];
          if (lanes.length === 0 || !lanes.some((l) => l && l.exit === 0)) {
            fail(`${c.id}: evidence manifest has no lane with exit 0`);
          }
        }
      }
    }
  }
  if (c.state === "native-accepted" && !c.lanes.some((l) => l === "N" || l === "P")) {
    fail(`${c.id}: native-accepted but has no N/P lane`);
  }
}
for (const id of plan.keys()) if (!seen.has(id)) fail(`${id}: missing from matrix`);

if (process.exitCode) process.exit(process.exitCode);
const byState = {};
for (const c of matrix.cases) byState[c.state] = (byState[c.state] ?? 0) + 1;
process.stdout.write(`viewport acceptance: ${matrix.cases.length} cases ok · ${JSON.stringify(byState)}\n`);
