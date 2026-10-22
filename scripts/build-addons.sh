#!/usr/bin/env bash
# Compile source addons into capability-restricted Wasm components and stage them.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if [[ $# -gt 1 ]]; then
    echo "usage: scripts/build-addons.sh [addon-directory]" >&2
    exit 2
fi
if ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    echo "Install the guest target with: rustup target add wasm32-unknown-unknown" >&2
    exit 1
fi
STAGING="${ONECAD_ADDON_STAGING_DIR:-$ROOT/packaging/binaries/addons}"
MESSAGES="$(mktemp "${TMPDIR:-/tmp}/onecad-addon-build.XXXXXX")"
trap 'rm -f "$MESSAGES"' EXIT
ADDONS=("$ROOT"/addons/*)
if [[ $# -eq 1 ]]; then
    ADDONS=("$1")
fi
BUILT=0
for addon in "${ADDONS[@]}"; do
    [[ -f "$addon/Cargo.toml" && -f "$addon/manifest.json" ]] || continue
    cargo build --locked --release --target wasm32-unknown-unknown \
        --manifest-path "$addon/Cargo.toml" --message-format=json-render-diagnostics > "$MESSAGES"
    cargo run --locked --quiet --target-dir "${CARGO_TARGET_DIR:-$ROOT/target}/addon-build-tools" \
        -p onecad-extension-sdk --features build-tools \
        --bin onecad-addon-build -- "$MESSAGES" "$addon/manifest.json" "$STAGING"
    BUILT=$((BUILT + 1))
done
if [[ "$BUILT" -eq 0 ]]; then
    echo "No addon source packages found" >&2
    exit 1
fi
