#!/usr/bin/env node
/*
 * Kernelbench digest MANIFEST — record and compare, across builds.
 *
 * Why this exists: the campaign's own replay check runs a case twice in the SAME
 * process tree and compares the two, so it proves determinism WITHIN a build. It
 * cannot answer the question a refactor actually raises — "did this change move a
 * digest?" — because there is nothing retained from before. The report's aggregate
 * ("136/136, gatingFailures 0") cannot answer it either: a case can change shape
 * and still pass.
 *
 * So: freeze `(suite, case, backend, variant) -> {inputDigest, normalizedDigest}`
 * into a committed file, and diff against it after the change.
 *
 * Usage:
 *   node scripts/kernelbench-manifest.mjs record <out-dir> <suite> <manifest.json>
 *   node scripts/kernelbench-manifest.mjs compare <out-dir> <suite> <manifest.json>
 *
 * `record` MERGES by suite, so several suites share one manifest file. `compare`
 * checks only the rows for the named suite and exits non-zero on any difference,
 * naming every changed, missing and unexpected row.
 */
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const [, , mode, outDir, suite, manifestPath] = process.argv;

if (!["record", "compare"].includes(mode) || !outDir || !suite || !manifestPath) {
  console.error(
    "usage: kernelbench-manifest.mjs <record|compare> <out-dir> <suite> <manifest.json>",
  );
  process.exit(2);
}

/** `(suite, caseId, backend, variant) -> {inputDigest, normalizedDigest}`, sorted. */
function readRows(dir, suiteName) {
  const path = `${dir}/results.jsonl`;
  if (!existsSync(path)) {
    console.error(`no results at ${path} — run the suite first`);
    process.exit(3);
  }
  const rows = {};
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const text = line.trim();
    if (!text) continue;
    const r = JSON.parse(text);
    // A record that never produced a digest (crash, timeout) is recorded as such
    // rather than skipped — "this case stopped producing a digest" is exactly the
    // kind of regression the manifest is here to catch.
    // `campaignVariant` is `{name}` — the metamorph variant (base / translated /
    // rotated). Take the NAME, not the object: stringifying the object collapses
    // every variant of a case to one key and would hide a moved digest.
    const variant = r.campaignVariant?.name ?? "base";
    rows[`${suiteName}|${r.caseId}|${r.backend}|${variant}`] = {
      inputDigest: r.inputDigest ?? null,
      normalizedDigest: r.normalizedDigest ?? null,
    };
  }
  return rows;
}

const observed = readRows(outDir, suite);

if (mode === "record") {
  const existing = existsSync(manifestPath)
    ? JSON.parse(readFileSync(manifestPath, "utf8"))
    : {};
  // Drop this suite's old rows so a removed case does not linger.
  for (const key of Object.keys(existing)) {
    if (key.startsWith(`${suite}|`)) delete existing[key];
  }
  const merged = { ...existing, ...observed };
  const sorted = Object.fromEntries(Object.keys(merged).sort().map((k) => [k, merged[k]]));
  writeFileSync(manifestPath, `${JSON.stringify(sorted, null, 2)}\n`);
  console.log(`recorded ${Object.keys(observed).length} rows for ${suite} → ${manifestPath}`);
  process.exit(0);
}

if (!existsSync(manifestPath)) {
  console.error(`no manifest at ${manifestPath} — record one first`);
  process.exit(3);
}
const baseline = JSON.parse(readFileSync(manifestPath, "utf8"));
const expected = Object.fromEntries(
  Object.entries(baseline).filter(([k]) => k.startsWith(`${suite}|`)),
);

const problems = [];
for (const [key, want] of Object.entries(expected)) {
  const got = observed[key];
  if (!got) {
    problems.push(`MISSING  ${key}`);
    continue;
  }
  for (const field of ["inputDigest", "normalizedDigest"]) {
    if (got[field] !== want[field]) {
      problems.push(`CHANGED  ${key} ${field}\n           was ${want[field]}\n           now ${got[field]}`);
    }
  }
}
for (const key of Object.keys(observed)) {
  if (!expected[key]) problems.push(`UNEXPECTED ${key}`);
}

if (problems.length) {
  console.error(`kernelbench manifest MISMATCH for ${suite} (${problems.length}):`);
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
console.log(`kernelbench manifest OK for ${suite}: ${Object.keys(expected).length} rows unchanged`);
