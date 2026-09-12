#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FEATURES="$(cd "$ROOT/src-tauri" && cargo tree -e features -i tauri-runtime-wry)"

if grep -q 'tauri-runtime-wry feature "tracing"' <<<"$FEATURES"; then
  echo "FAIL: Tauri tracing makes background event eval synchronous and can deadlock IPC" >&2
  exit 1
fi

echo "PASS: tauri-runtime-wry tracing feature is disabled"
