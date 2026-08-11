#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
temp="$(mktemp)"
trap 'rm -f "$temp"' EXIT

cp "$root/docs/qa/modeling-operation-coverage.json" "$temp"
node -e 'const fs=require("node:fs"); const p=process.argv[1]; const x=JSON.parse(fs.readFileSync(p)); x.rows.push({...x.rows[0]}); fs.writeFileSync(p, JSON.stringify(x));' "$temp"
if MODELING_COVERAGE_MANIFEST="$temp" node "$root/scripts/verify-modeling-coverage.mjs"; then
  echo "expected duplicate corpus classification to fail" >&2
  exit 1
fi
echo "verify-modeling-coverage negative control: OK"
