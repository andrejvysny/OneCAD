#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
    ARM64) expected=arm64 ;;
    X64) expected=x86_64 ;;
    *) echo "unsupported macOS runner architecture: ${1:-missing}" >&2; exit 1 ;;
esac
test "$(uname -s)" = Darwin
test "$(uname -m)" = "$expected"
if [[ $# -eq 2 ]]; then
    test "$(lipo -archs "$2")" = "$expected"
fi
