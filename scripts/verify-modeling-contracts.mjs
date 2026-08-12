#!/usr/bin/env node
import { readFileSync } from "node:fs";
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
  if ("uiExposure" in row && !uiExposures.has(row.uiExposure)) {
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

if (!process.exitCode) {
  process.stdout.write(`modeling contracts: ${contracts.rows.length} rows, ${operations.size} operations\n`);
}
