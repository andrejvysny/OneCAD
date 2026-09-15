#!/usr/bin/env bash
# The ONE mapping from a Rust target triple to everything the assistant sidecar
# is built and looked up under: its Bun compile target, its executable suffix,
# and its staged file name.
#
# WHY IT IS ITS OWN FILE. The same three answers are needed in three places —
# `scripts/build-assistant-host.sh` when it compiles, `src-tauri/src/assistant/
# mod.rs` when it looks the binary up at runtime, and the tests of both. They
# used to be spelled out separately, and they disagreed: the build script wrote
# `onecad-assistant-host-<triple>` with no extension while the Rust resolver
# looked for `onecad-assistant-host.exe` on Windows, so the Windows packaging
# seam named two different files. This file is the single answer; the Rust test
# `the_staged_name_agrees_with_the_build_script` runs it and compares.
#
# Source it for the functions, or run it for one answer:
#
#     scripts/lib/assistant-host-target.sh bun-target   x86_64-apple-darwin
#     scripts/lib/assistant-host-target.sh exe-suffix   x86_64-pc-windows-msvc
#     scripts/lib/assistant-host-target.sh staged-name  aarch64-apple-darwin
#     scripts/lib/assistant-host-target.sh host-triple
#
# An unknown triple is an ERROR, never a default. The previous script fell back
# to `aarch64-apple-darwin` when it could not determine a triple, which does not
# fail — it mislabels a binary, and a mislabelled binary is one that a bundle
# either cannot find or finds and cannot run.
set -euo pipefail

ONECAD_ASSISTANT_HOST_BASENAME="onecad-assistant-host"

# The Bun `--compile --target` value for a Rust target triple.
onecad_bun_target_for_triple() {
    case "$1" in
    x86_64-apple-darwin) echo "bun-darwin-x64" ;;
    aarch64-apple-darwin) echo "bun-darwin-arm64" ;;
    x86_64-unknown-linux-gnu) echo "bun-linux-x64" ;;
    aarch64-unknown-linux-gnu) echo "bun-linux-arm64" ;;
    x86_64-unknown-linux-musl) echo "bun-linux-x64-musl" ;;
    aarch64-unknown-linux-musl) echo "bun-linux-arm64-musl" ;;
    x86_64-pc-windows-msvc) echo "bun-windows-x64" ;;
    *)
        echo "assistant-host-target: no Bun compile target is known for '$1'." >&2
        echo "                       Add it here rather than guessing at the call site." >&2
        return 1
        ;;
    esac
}

# `.exe` for a Windows triple, nothing otherwise. Mirrors
# `assistant_host_exe_suffix` in src-tauri/src/assistant/mod.rs.
onecad_exe_suffix_for_triple() {
    case "$1" in
    *windows*) echo ".exe" ;;
    *) echo "" ;;
    esac
}

# `onecad-assistant-host-<triple>[.exe]`, the name bundle.externalBin expects.
onecad_staged_name_for_triple() {
    printf '%s-%s%s\n' \
        "${ONECAD_ASSISTANT_HOST_BASENAME}" "$1" "$(onecad_exe_suffix_for_triple "$1")"
}

# This machine's Rust host triple, or a failure. There is no fallback: a build
# that cannot name its target must not produce a binary claiming one.
onecad_host_triple() {
    if ! command -v rustc >/dev/null 2>&1; then
        echo "assistant-host-target: rustc is not on PATH, so the host triple is unknown." >&2
        echo "                       Pass --target <triple> or set ONECAD_ASSISTANT_TARGET." >&2
        return 1
    fi
    local triple
    triple="$(rustc -Vv | sed -n 's/^host: //p')"
    if [ -z "${triple}" ]; then
        echo "assistant-host-target: 'rustc -Vv' named no host triple." >&2
        return 1
    fi
    printf '%s\n' "${triple}"
}

# Run directly (not sourced) ⇒ answer one question and exit.
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    case "${1:-}" in
    bun-target) onecad_bun_target_for_triple "${2:?a target triple}" ;;
    exe-suffix) onecad_exe_suffix_for_triple "${2:?a target triple}" ;;
    staged-name) onecad_staged_name_for_triple "${2:?a target triple}" ;;
    host-triple) onecad_host_triple ;;
    *)
        echo "usage: $0 {bun-target|exe-suffix|staged-name} <rust-target-triple>" >&2
        echo "       $0 host-triple" >&2
        exit 2
        ;;
    esac
fi
