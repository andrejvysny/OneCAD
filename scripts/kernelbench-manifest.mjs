#!/usr/bin/env node
/*
 * Kernelbench baselines — digests (per platform) and semantics (across them).
 *
 * Why this exists: the campaign's own replay check runs a case twice in the SAME
 * process tree and compares the two, so it proves determinism WITHIN a build. It
 * cannot answer the question a refactor actually raises — "did this change move a
 * digest?" — because there is nothing retained from before. The report's aggregate
 * ("136/136, gatingFailures 0") cannot answer it either: a case can change shape
 * and still pass.
 *
 * TWO baselines, because they answer different questions.
 *
 * 1. DIGESTS are per platform, keyed
 *      `(suite, case, backend, variant, platform) -> {inputDigest, normalizedDigest}`.
 *
 *    Measured 2026-08-10 by running T0 on macOS/arm64/AppleClang and on
 *    Linux/x86_64/gcc 14 against the same pinned OCCT 8.0.1 (identical build id
 *    AND identical 16-hex kernel fingerprint), 182 of 272 digest values differ.
 *    The pattern is exact and is not a bug:
 *      - `translated` inputDigest: 0 of 32 changed — translation is exact in FP;
 *      - `rotated`    inputDigest: 32 of 32 changed — the transform uses trig;
 *      - `base`       inputDigest: 20 of 72 changed — precisely the trig-built
 *        shapes (all 8 valence4, overflow-02/-03). Every box and valence3 case
 *        is bit-identical;
 *      - normalizedDigest: 130 of 136 changed — OCCT's own arithmetic diverges.
 *
 *    The digest quantizes to 1e-9, and that does NOT make it portable: rounding
 *    to a grid narrows but never closes boundary straddles, and with thousands of
 *    quantized values per record a straddle is near-certain. So a digest is a
 *    SAME-HOST regression tripwire. Comparing one across hosts would report a
 *    kernel regression every time and is meaningless. Hence the platform key.
 *
 * 2. SEMANTICS is one row per suite, NOT platform-keyed, because it is the
 *    portability claim: two hosts running the same pinned kernel must agree on
 *    every verdict, domain, failure class, differential classification, replay
 *    stability and metamorph outcome. In the same measurement they agreed on all
 *    of it. Timing and tolerance distributions are excluded — those are host
 *    properties, not kernel behaviour.
 *
 * Usage:
 *   node scripts/kernelbench-manifest.mjs record          <out-dir> <suite> <digests.json>
 *   node scripts/kernelbench-manifest.mjs compare         <out-dir> <suite> <digests.json>
 *   node scripts/kernelbench-manifest.mjs semantic-record <out-dir> <suite> <semantics.json>
 *   node scripts/kernelbench-manifest.mjs semantic-compare <out-dir> <suite> <semantics.json>
 *
 * `record` MERGES by (suite, platform), so several suites and several platforms
 * share one file. `compare` checks only the rows for the named suite on the
 * CURRENT platform and exits non-zero on any difference, naming every changed,
 * missing and unexpected row.
 *
 * Platform is `${process.platform}-${process.arch}` (darwin-arm64, linux-x64),
 * overridable with ONECAD_BENCH_PLATFORM so a baseline can be recorded from an
 * artifact produced elsewhere.
 *
 * Exit: 0 match · 1 difference · 2 usage · 3 missing input.
 */
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const MODES = ["record", "compare", "semantic-record", "semantic-compare"];
const [, , mode, outDir, suite, manifestPath] = process.argv;

if (!MODES.includes(mode) || !outDir || !suite || !manifestPath) {
  console.error(
    `usage: kernelbench-manifest.mjs <${MODES.join("|")}> <out-dir> <suite> <manifest.json>`,
  );
  process.exit(2);
}

const platform =
  process.env.ONECAD_BENCH_PLATFORM || `${process.platform}-${process.arch}`;

function readJson(path, what) {
  if (!existsSync(path)) {
    console.error(`no ${what} at ${path} — run the suite first`);
    process.exit(3);
  }
  return JSON.parse(readFileSync(path, "utf8"));
}

function loadBaseline(path) {
  return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : {};
}

function writeSorted(path, obj, label) {
  const sorted = Object.fromEntries(Object.keys(obj).sort().map((k) => [k, obj[k]]));
  writeFileSync(path, `${JSON.stringify(sorted, null, 2)}\n`);
  console.log(label);
}

/* ---------------------------------------------------------------- digests -- */

/** `(suite, caseId, backend, variant, platform) -> {inputDigest, normalizedDigest}`. */
function readDigestRows(dir, suiteName) {
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
    rows[`${suiteName}|${r.caseId}|${r.backend}|${variant}|${platform}`] = {
      inputDigest: r.inputDigest ?? null,
      normalizedDigest: r.normalizedDigest ?? null,
    };
  }
  return rows;
}

/** Rows for this suite AND this platform. A foreign platform's rows are not ours. */
function scopeDigests(baseline, suiteName) {
  return Object.fromEntries(
    Object.entries(baseline).filter(
      ([k]) => k.startsWith(`${suiteName}|`) && k.endsWith(`|${platform}`),
    ),
  );
}

/* -------------------------------------------------------------- semantics -- */

/**
 * The platform-INDEPENDENT projection of a run. Timing and tolerance
 * distributions are deliberately absent: they describe the host, not the kernel,
 * and pinning them would make this gate fail on a busy machine.
 */
function readSemantics(dir) {
  const s = readJson(`${dir}/summary.json`, "summary");
  return {
    schemaVersion: s.schemaVersion ?? null,
    records: s.records ?? null,
    gatingFailures: s.gatingFailures ?? null,
    counts: s.counts ?? null,
    differential: s.differential ?? null,
    replay: s.replay ?? null,
    metamorph: s.metamorph ?? null,
    criticalTransitions: s.criticalTransitions ?? null,
  };
}

/** Deep, order-independent difference, reported as dotted paths. */
function diff(want, got, path = "", out = []) {
  const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
  if (isObj(want) && isObj(got)) {
    for (const k of new Set([...Object.keys(want), ...Object.keys(got)]).values()) {
      diff(want[k], got[k], path ? `${path}.${k}` : k, out);
    }
  } else if (JSON.stringify(want) !== JSON.stringify(got)) {
    out.push(`${path}: expected ${JSON.stringify(want)}, got ${JSON.stringify(got)}`);
  }
  return out;
}

/* ------------------------------------------------------------------- main -- */

if (mode === "record") {
  const observed = readDigestRows(outDir, suite);
  const existing = loadBaseline(manifestPath);
  // Drop this suite's old rows FOR THIS PLATFORM so a removed case does not
  // linger — and so recording on one host never deletes another host's rows.
  for (const key of Object.keys(scopeDigests(existing, suite))) delete existing[key];
  writeSorted(
    manifestPath,
    { ...existing, ...observed },
    `recorded ${Object.keys(observed).length} rows for ${suite} on ${platform} → ${manifestPath}`,
  );
  process.exit(0);
}

if (mode === "semantic-record") {
  const observed = readSemantics(outDir);
  const existing = loadBaseline(manifestPath);
  existing[suite] = observed;
  writeSorted(manifestPath, existing, `recorded semantics for ${suite} → ${manifestPath}`);
  process.exit(0);
}

if (mode === "semantic-compare") {
  const baseline = readJson(manifestPath, "semantics baseline");
  const want = baseline[suite];
  if (!want) {
    console.error(`no semantics baseline for ${suite} — semantic-record one first`);
    process.exit(3);
  }
  const problems = diff(want, readSemantics(outDir));
  if (problems.length) {
    console.error(`kernelbench SEMANTIC mismatch for ${suite} on ${platform} (${problems.length}):`);
    for (const p of problems) console.error(`  ${p}`);
    process.exit(1);
  }
  console.log(`kernelbench semantics OK for ${suite} on ${platform}`);
  process.exit(0);
}

// mode === "compare"
if (!existsSync(manifestPath)) {
  console.error(`no manifest at ${manifestPath} — record one first`);
  process.exit(3);
}
const observed = readDigestRows(outDir, suite);
const expected = scopeDigests(JSON.parse(readFileSync(manifestPath, "utf8")), suite);
if (!Object.keys(expected).length) {
  console.error(
    `no digest rows for ${suite} on ${platform} in ${manifestPath} — record this platform first`,
  );
  process.exit(3);
}

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
  console.error(`kernelbench manifest MISMATCH for ${suite} on ${platform} (${problems.length}):`);
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
console.log(
  `kernelbench manifest OK for ${suite} on ${platform}: ${Object.keys(expected).length} rows unchanged`,
);
