#!/usr/bin/env bash
# Grep gate: the worker's stdout carries OCW1 frames ONLY (protocol/SCHEMA.md
# §2). Anything else writing to stdout — a stray printf, std::cout, or
# fprintf(stdout, ...) — corrupts the frame stream and is a hard bug. This
# script forbids those calls anywhere under worker/src.
#
# Usage: scripts/check-worker-stdout-hygiene.sh
# Exit 0 = clean. Exit 1 = violation(s) found, printed to stderr.
#
# worker/src legitimately uses std::snprintf (buffer formatting, no stream)
# and fprintf(stderr, ...) (Log.h's own sink) all over the place — the pattern
# below is scoped to avoid both: the `[^[:alnum:]_]` boundary before
# `(std::)?printf` stops it from matching inside `snprintf`, and the stdout
# alternative for fprintf only fires when the destination stream is
# literally `stdout`.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKER_SRC="${ROOT_DIR}/worker/src"

PATTERN='(^|[^[:alnum:]_])(std::)?(printf|puts|putchar)[[:space:]]*\(|fprintf[[:space:]]*\([[:space:]]*stdout|std::cout'

# Frame.cpp is the sole file that writes worker stdout, and it does so via the
# raw `::write()` syscall on an fd — never printf/cout — so no exclusion is
# needed for it (verified: `grep -nE 'printf|puts\(|putchar|std::cout|fprintf'
# worker/src/protocol/Frame.cpp` is empty).

HITS="$(grep -rnE "${PATTERN}" "${WORKER_SRC}" --include='*.cpp' --include='*.h' \
    | grep -vE ':[[:space:]]*//' || true)"

if [ -n "${HITS}" ]; then
    echo "check-worker-stdout-hygiene: forbidden stdout write(s) found:" >&2
    echo "${HITS}" >&2
    echo "" >&2
    echo "worker/src stdout carries OCW1 frames ONLY — route diagnostics through WLOG_* (util/Log.h) instead." >&2
    exit 1
fi

echo "==> worker stdout hygiene: clean"
