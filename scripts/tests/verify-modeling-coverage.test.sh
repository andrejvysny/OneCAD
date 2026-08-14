#!/usr/bin/env bash
# Negative controls for the modeling-evidence verifiers.
#
# Both verifiers exist to FAIL. Every assertion they make is of the form "this
# claim is still true", and a check that cannot go red is indistinguishable from
# no check at all — which is exactly what happened: sixteen manifest paths rotted
# away and eleven rows named a CI job that does not exist, under a green verifier.
#
# So each control here breaks ONE thing and requires the verifier to notice. They
# operate on a TEMP COPY of the manifest (`MODELING_COVERAGE_MANIFEST` /
# `MODELING_CONTRACTS_PATH`); the repo's own files are never touched.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
temp="$(mktemp)"
contracts_temp="$(mktemp)"
trap 'rm -f "$temp" "$contracts_temp"' EXIT

coverage="$root/docs/qa/modeling-operation-coverage.json"
contracts="$root/docs/qa/modeling-operation-contracts.json"

# Runs the coverage verifier over a manifest mutated by `node -e "$1"` and requires
# a NON-ZERO exit. `$2` is what the mutation is meant to prove.
expect_coverage_failure() {
  cp "$coverage" "$temp"
  node -e "$1" "$temp"
  if MODELING_COVERAGE_MANIFEST="$temp" node "$root/scripts/verify-modeling-coverage.mjs" >/dev/null 2>&1; then
    echo "verify-modeling-coverage did NOT fail on: $2" >&2
    exit 1
  fi
  echo "  ✓ $2"
}

echo "verify-modeling-coverage negative controls:"

# The original control: a corpus case claimed by two rows.
expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));x.rows.push({...x.rows[0]});fs.writeFileSync(p,JSON.stringify(x))' \
  "duplicate corpus classification"

# THE acceptance test the roadmap asks for: rename a cited spec, CI goes red.
expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));const r=x.rows.find(r=>r.playwrightTest);r.playwrightTest=r.playwrightTest.replace(/\.spec\.ts$/,".renamed.spec.ts");fs.writeFileSync(p,JSON.stringify(x))' \
  "a cited Playwright spec that no longer exists"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));const r=x.rows.find(r=>r.cppTest);r.cppTest="worker/tests/test_ghost.cpp";fs.writeFileSync(p,JSON.stringify(x))' \
  "a cited C++ test that no longer exists"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));const r=x.rows.find(r=>r.ciJob);r.ciJob="macos-full";fs.writeFileSync(p,JSON.stringify(x))' \
  "a ciJob that is not a job in any workflow"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));const r=x.rows.find(r=>r.playwrightTest&&r.ciJob.includes("e2e"));r.ciJob=r.ciJob.split(";").map(s=>s.trim()).filter(j=>!j.startsWith("e2e")).join("; ");fs.writeFileSync(p,JSON.stringify(x))' \
  "an e2e spec cited by a row that names no e2e job"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));x.rows=x.rows.filter(r=>r.operation!=="Hole");fs.writeFileSync(p,JSON.stringify(x))' \
  "a KnownOperation with no manifest row at all"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));x.rows[0]={...x.rows[0],operation:"Bevel",mode:"invented",corpusCase:""};fs.writeFileSync(p,JSON.stringify(x))' \
  "a row naming an operation no registry knows"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));x.nonOperationRows=[];fs.writeFileSync(p,JSON.stringify(x))' \
  "an undeclared non-operation row"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));delete x.rows[0].uiExposure;fs.writeFileSync(p,JSON.stringify(x))' \
  "a row missing uiExposure"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));const r=x.rows.find(r=>r.uiExposure==="exposed"&&r.supportStatus==="supported");r.playwrightTest="";fs.writeFileSync(p,JSON.stringify(x))' \
  "an exposed supported row missing its browser lane"

expect_coverage_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));const r=x.rows.find(r=>r.uiExposure==="hidden");r.playwrightTest="e2e/boolean-preview.spec.ts";fs.writeFileSync(p,JSON.stringify(x))' \
  "a hidden row claiming browser exposure"

echo "verify-modeling-contracts negative controls:"

expect_contracts_failure() {
  cp "$contracts" "$contracts_temp"
  node -e "$1" "$contracts_temp"
  if MODELING_CONTRACTS_PATH="$contracts_temp" node "$root/scripts/verify-modeling-contracts.mjs" >/dev/null 2>&1; then
    echo "verify-modeling-contracts did NOT fail on: $2" >&2
    exit 1
  fi
  echo "  ✓ $2"
}

expect_contracts_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));delete x.rows[0].validationTier;fs.writeFileSync(p,JSON.stringify(x))' \
  "a row missing a required field"

expect_contracts_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));x.rows[0].uiExposure="maybe";fs.writeFileSync(p,JSON.stringify(x))' \
  "an invalid uiExposure value"

expect_contracts_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));x.rows.push({...x.rows[0]});fs.writeFileSync(p,JSON.stringify(x))' \
  "a duplicate operation::mode pair"

expect_contracts_failure \
  'const fs=require("node:fs"),p=process.argv[1],x=JSON.parse(fs.readFileSync(p));x.rows[0].operation="Bevel";fs.writeFileSync(p,JSON.stringify(x))' \
  "an operation absent from the coverage manifest"

echo "modeling verifier negative controls: OK"
