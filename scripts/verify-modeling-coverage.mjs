#!/usr/bin/env node
import { readdirSync, readFileSync } from "node:fs";
import { basename, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const manifestPath = resolve(process.env.MODELING_COVERAGE_MANIFEST ??
  resolve(root, "docs/qa/modeling-operation-coverage.json"));
const corpusDir = resolve(root, "corpus/cases");
const required = ["operation", "mode", "supportStatus", "cppTest", "rustRealWorkerTest",
  "frontendTest", "playwrightTest", "corpusCase", "kernelbenchSuite", "ciJob", "notes"];
const statuses = new Set(["supported", "deferred", "unsupported"]);
const classifications = new Set(["operation", "solver", "antiGoal", "unsupported"]);

function fail(message) {
  process.stderr.write(`modeling coverage: ${message}\n`);
  process.exitCode = 1;
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

const corpus = new Set(readdirSync(corpusDir)
  .filter((name) => name.endsWith(".json"))
  .map((name) => JSON.parse(readFileSync(resolve(corpusDir, name), "utf8")).id));
const covered = new Map();
for (const [index, row] of manifest.rows.entries()) {
  for (const field of required) {
    if (typeof row[field] !== "string") fail(`row ${index} missing string ${field}`);
  }
  if (!statuses.has(row.supportStatus)) fail(`row ${index} has invalid supportStatus`);
  if (!row.operation || !row.mode) fail(`row ${index} needs operation and mode`);
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
if (!process.exitCode) process.stdout.write(`modeling coverage: ${manifest.rows.length} rows, ${corpus.size} corpus cases\n`);
