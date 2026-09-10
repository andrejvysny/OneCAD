#!/usr/bin/env node
/*
 * The modeling-operation CONTRACT verifier.
 *
 * Most fields in `modeling-operation-contracts.json` are prose: a human sentence
 * about semantics that no script can check. Until this pass NONE of it was
 * checked against code — the verifier read field presence, enum membership and
 * duplicate keys, and nothing else. That is how the manifest came to claim
 * Extrude/NewBody and Revolve/NewBody publish at "Tier A" while
 * `ExtrudeOp.cpp:1259` and `RevolveOp.cpp:442` had asked for Tier B since WP-E.
 * The manifest was the stale side, and nothing could say so.
 *
 * Two fields ARE machine-checkable, and are now checked:
 *
 *   - `validationTier` — cross-read against the `PublicationTier::TierA/TierB`
 *     tokens in the worker source that owns that operation's publication.
 *   - `supportStatus` — cross-read against the worker's `op_type == "…"`
 *     dispatch arms in `PlanExecutor.cpp`.
 *
 * Everything else stays prose ON PURPOSE, and the file says so rather than
 * implying more than it proves. Both checks fail loudly if their source stops
 * matching — a check that silently scans zero tokens is worse than no check.
 */
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const contractsPath = resolve(process.env.MODELING_CONTRACTS_PATH ??
  resolve(root, "docs/qa/modeling-operation-contracts.json"));
const coveragePath = resolve(process.env.MODELING_COVERAGE_MANIFEST ??
  resolve(root, "docs/qa/modeling-operation-coverage.json"));

const required = [
  "operation",
  "mode",
  "supportStatus",
  "uiExposure",
  "inputTopLevelShapes",
  "requiresExactlyOneSolid",
  "emptyResultSemantics",
  "multiSolidResultSemantics",
  "bodyLifecycle",
  "bodyIdPolicy",
  "authoritativeHistory",
  "historyAbsenceBehavior",
  "validationTier",
  "toleranceEvidence",
  "cancellationPolicy",
  "previewFidelity",
  "persistenceBehavior",
  "userRecoveryBehavior",
];
const statuses = new Set(["supported", "deferred", "unsupported"]);
const uiExposures = new Set(["exposed", "hidden", "unsupported"]);

function fail(message) {
  process.stderr.write(`modeling contracts: ${message}\n`);
  process.exitCode = 1;
}

let contracts;
try {
  contracts = JSON.parse(readFileSync(contractsPath, "utf8"));
} catch (error) {
  fail(`cannot parse contracts: ${error.message}`);
  process.exit(1);
}
if (contracts.version !== 1 || !Array.isArray(contracts.rows)) {
  fail("expected version 1 rows array");
  process.exit(1);
}

const keys = new Set();
const operations = new Set();
for (const [index, row] of contracts.rows.entries()) {
  for (const field of required) {
    if (!(field in row)) fail(`row ${index} missing ${field}`);
  }
  if (typeof row.operation !== "string" || typeof row.mode !== "string") {
    fail(`row ${index} needs operation and mode strings`);
  }
  if (!statuses.has(row.supportStatus)) {
    fail(`row ${index} has invalid supportStatus ${row.supportStatus}`);
  }
  const key = `${row.operation}::${row.mode}`;
  if (keys.has(key)) fail(`row ${index} duplicates ${key}`);
  keys.add(key);
  operations.add(row.operation);
  if (!uiExposures.has(row.uiExposure)) {
    fail(`row ${index} has invalid uiExposure ${row.uiExposure}`);
  }
  if (row.supportStatus === "unsupported" && row.uiExposure !== "unsupported") {
    fail(`row ${index} unsupported operation must declare uiExposure unsupported`);
  }
  // Deferred lineage/history fields must be explicitly marked, not omitted.
  for (const field of ["authoritativeHistory", "historyAbsenceBehavior"]) {
    const value = row[field];
    if (typeof value === "string" && value.toLowerCase().includes("deferred") && !/owner|dependency/i.test(value)) {
      fail(`row ${index} ${field} is deferred but missing owner/dependency`);
    }
  }
}

let coverage;
try {
  coverage = JSON.parse(readFileSync(coveragePath, "utf8"));
} catch (error) {
  fail(`cannot parse coverage manifest: ${error.message}`);
  process.exit(1);
}
const coverageOps = new Set();
for (const row of coverage.rows) coverageOps.add(row.operation);
for (const op of operations) {
  if (!coverageOps.has(op)) fail(`contract operation ${op} is missing from coverage manifest`);
}

// ── Semantic check 1: `validationTier` against the worker's publication tier ──
//
// The worker source that owns an operation's RESULT publication. Fillet and
// Chamfer share one file; the two pattern ops share one; components share one.
// An operation absent from this table is prose-only for this check (Sketch has
// no op file; Loft and Sweep have no implementation at all).
const tierSources = {
  Extrude: "worker/src/ops/ExtrudeOp.cpp",
  Revolve: "worker/src/ops/RevolveOp.cpp",
  Fillet: "worker/src/ops/FilletChamferOp.cpp",
  Chamfer: "worker/src/ops/FilletChamferOp.cpp",
  Shell: "worker/src/ops/ShellOp.cpp",
  Boolean: "worker/src/ops/BooleanOp.cpp",
  LinearPattern: "worker/src/ops/PatternOp.cpp",
  CircularPattern: "worker/src/ops/PatternOp.cpp",
  MirrorBody: "worker/src/ops/MirrorOp.cpp",
  TransformBody: "worker/src/ops/TransformOp.cpp",
  Hole: "worker/src/ops/HoleOp.cpp",
  OffsetFace: "worker/src/ops/OffsetFaceOp.cpp",
  Gear: "worker/src/ops/GearOp.cpp",
  PlaceComponent: "worker/src/ops/ComponentOp.cpp",
  DetachComponent: "worker/src/ops/ComponentOp.cpp",
};

const tierCache = new Map();
function tiersRequestedBy(relPath) {
  if (tierCache.has(relPath)) return tierCache.get(relPath);
  const abs = resolve(root, relPath);
  let requested = null;
  if (!existsSync(abs)) {
    fail(`validationTier: ${relPath} does not exist — the source moved and this check went vacuous`);
  } else {
    const source = readFileSync(abs, "utf8");
    const tierA = /PublicationTier::TierA/.test(source);
    const tierB = /PublicationTier::TierB/.test(source);
    if (!tierA && !tierB) {
      fail(`validationTier: ${relPath} names no PublicationTier at all — this check went vacuous`);
    } else {
      requested = { tierA, tierB };
    }
  }
  tierCache.set(relPath, requested);
  return requested;
}

for (const row of contracts.rows) {
  const relPath = tierSources[row.operation];
  if (!relPath) continue;
  const requested = tiersRequestedBy(relPath);
  if (!requested) continue;
  const claim = String(row.validationTier ?? "");
  // The LEADING tier token is the claim; a later mention is commentary, not a
  // second claim. `"Tier A per result (P3 deferred full Tier B for performance)"`
  // claims A and defers B — reading both as claims made the check cry wolf on
  // exactly the rows that are honest about what they do not do yet.
  const leading = /\bTier (A|B)\b/.exec(claim);
  const claimsA = leading?.[1] === "A";
  const claimsB = leading?.[1] === "B";
  const where = `${row.operation}::${row.mode}`;
  // A row may legitimately claim A where the op file branches on fusing.
  if (claimsA && !claimsB && requested.tierB && !requested.tierA) {
    fail(`${where} validationTier claims Tier A, but ${relPath} only ever requests Tier B`);
  }
  if (claimsB && !requested.tierB) {
    fail(`${where} validationTier claims Tier B, but ${relPath} never requests it`);
  }
}

// ── Semantic check 2: `supportStatus` against the worker dispatch ─────────────
//
// Same regex the coverage verifier uses, deliberately: one reading of the
// dispatch, so the two manifests cannot disagree about what the worker executes.
// A row whose operation is not a `KnownOperation` is a protocol verb or a
// timeline lane, not something the worker dispatches by `op_type` — those are
// out of scope for the dispatch check rather than failures.
const recordRs = resolve(root, "src-tauri/crates/onecad-core/src/document/record.rs");
const knownContractOps = new Set();
if (!existsSync(recordRs)) {
  fail("supportStatus: record.rs does not exist — this check went vacuous");
} else {
  const blocks = [...readFileSync(recordRs, "utf8")
    .matchAll(/^pub enum KnownOperation \{([\s\S]*?)^\}/gm)];
  for (const block of blocks) {
    for (const m of block[1].matchAll(/^\s{4}([A-Z][A-Za-z]*)\(/gm)) knownContractOps.add(m[1]);
  }
  if (knownContractOps.size === 0) {
    fail("supportStatus: scanned zero KnownOperation variants — this check went vacuous");
  }
}

const planExecutor = resolve(root, "worker/src/session/PlanExecutor.cpp");
if (!existsSync(planExecutor)) {
  fail("supportStatus: PlanExecutor.cpp does not exist — this check went vacuous");
} else {
  const dispatched = new Set([...readFileSync(planExecutor, "utf8")
    .matchAll(/op_type == "([A-Za-z]+)"/g)].map((m) => m[1]));
  if (dispatched.size === 0) {
    fail("supportStatus: scanned zero dispatch arms — this check went vacuous");
  } else {
    for (const row of contracts.rows) {
      const where = `${row.operation}::${row.mode}`;
      // Only operations the worker could plausibly execute are in scope; a row
      // for a non-op lane is declared in the coverage manifest's nonOperationRows.
      if (row.supportStatus === "unsupported" && dispatched.has(row.operation)) {
        fail(`${where} is marked unsupported but the worker dispatches ${row.operation}`);
      }
      if (row.supportStatus === "supported" && !dispatched.has(row.operation) &&
          knownContractOps.has(row.operation)) {
        fail(`${where} is marked supported but the worker has no ${row.operation} dispatch arm`);
      }
    }
  }
}

if (!process.exitCode) {
  process.stdout.write(
    `modeling contracts: ${contracts.rows.length} rows, ${operations.size} operations, ` +
    `${Object.keys(tierSources).length} tier-checked operations\n`);
}
