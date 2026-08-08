#!/usr/bin/env bash
# Bundle the OneCAD worker sidecar's non-system dylib closure into a .app so the
# packaged app runs on a clean Mac with no Homebrew present.
#
# Usage: scripts/bundle-dylibs.sh <path-to.app>
#
# Locates the worker sidecar in Contents/MacOS/, walks its transitive Homebrew /
# /usr/local dylib dependency closure (skipping the /usr/lib + /System system
# libraries the OS always ships), copies each dependency into Contents/Frameworks/,
# rewrites every install name to @rpath, adds an @executable_path/../Frameworks
# rpath to the worker, and ad-hoc re-signs every Mach-O it touched (install_name_tool
# invalidates code signatures). Idempotent: re-running against an already-bundled
# .app is a no-op because the worker's dependencies then resolve via @rpath and no
# longer look bundleable.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "bundle-dylibs.sh: macOS-only (uname=$(uname)); refusing to run." >&2
    exit 1
fi

if [[ $# -ne 1 ]]; then
    echo "usage: bundle-dylibs.sh <path-to.app>" >&2
    exit 2
fi

APP="$1"
if [[ ! -d "$APP" ]]; then
    echo "bundle-dylibs.sh: not an app bundle directory: $APP" >&2
    exit 2
fi
APP="$(cd "$APP" && pwd -P)"

MACOS_DIR="$APP/Contents/MacOS"
FRAMEWORKS_DIR="$APP/Contents/Frameworks"

# Locate the worker sidecar: onecad-worker, or onecad-worker-<triple> if the
# target-triple suffix survived bundling.
WORKER=""
for cand in "$MACOS_DIR"/onecad-worker "$MACOS_DIR"/onecad-worker-*; do
    if [[ -f "$cand" ]]; then
        WORKER="$cand"
        break
    fi
done
if [[ -z "$WORKER" ]]; then
    echo "bundle-dylibs.sh: no onecad-worker binary under $MACOS_DIR" >&2
    exit 1
fi

mkdir -p "$FRAMEWORKS_DIR"

# Bundle every resolved absolute non-system dependency. Pinned CI artifacts live
# under $RUNNER_TEMP, so limiting this to Homebrew would ship a worker whose OCCT
# dylibs disappear with the runner.
is_bundleable() {
    case "$1" in
        "$FRAMEWORKS_DIR"/*) return 1 ;;
        /System/* | /usr/lib/* | @*) return 1 ;;
        /*) [[ -f "$1" ]] ;;
        *) return 1 ;;
    esac
}

# Print the (non-header) dependency paths of a Mach-O, one per line. `otool -L`
# emits the binary path as its first line, then one indented dep per line; the
# path is the first whitespace-delimited field of each dep line.
deps_of() {
    otool -L "$1" | tail -n +2 | awk '{print $1}'
}

rpaths_of() {
    otool -l "$1" | awk '
        $1 == "cmd" && $2 == "LC_RPATH" { in_rpath = 1; next }
        in_rpath && $1 == "path" { print $2; in_rpath = 0 }
    '
}

expand_path_token() {
    local value="$1"
    local loader="$2"
    value="${value//@loader_path/$(dirname "$loader")}"
    value="${value//@executable_path/$MACOS_DIR}"
    printf '%s\n' "$value"
}

# Source-built OCCT uses @rpath install names. Resolve them through the current
# Mach-O's LC_RPATH entries so the original dylibs can be copied into the bundle.
resolve_dep() {
    local owner="$1"
    local dep="$2"
    if [[ "$dep" != @rpath/* ]]; then
        expand_path_token "$dep" "$owner"
        return
    fi
    local suffix="${dep#@rpath/}"
    local rpath candidate
    while IFS= read -r rpath; do
        [[ -z "$rpath" ]] && continue
        candidate="$(expand_path_token "$rpath" "$owner")/$suffix"
        if [[ -f "$candidate" ]]; then
            printf '%s/%s\n' "$(cd "$(dirname "$candidate")" && pwd -P)" "$(basename "$candidate")"
            return
        fi
    done < <(rpaths_of "$owner")
    printf '%s\n' "$dep"
}

# Breadth-first transitive closure of bundleable dependencies, discovered against
# the original Homebrew copies (which still carry absolute install names). Stored
# both as an array (CLOSURE_LIST, for the copy/rewrite phases) and as a
# newline-delimited membership string (SEEN, for O(1)-ish dedup).
CLOSURE_LIST=()
SEEN=""
WORKLIST=("$WORKER")
idx=0
while [[ $idx -lt ${#WORKLIST[@]} ]]; do
    current="${WORKLIST[$idx]}"
    idx=$((idx + 1))
    while IFS= read -r dep_ref; do
        [[ -z "$dep_ref" ]] && continue
        dep="$(resolve_dep "$current" "$dep_ref")"
        is_bundleable "$dep" || continue
        if printf '%s\n' "$SEEN" | grep -qxF "$dep"; then
            continue
        fi
        SEEN="${SEEN}${dep}"$'\n'
        CLOSURE_LIST+=("$dep")
        WORKLIST+=("$dep")
    done < <(deps_of "$current")
done

if [[ ${#CLOSURE_LIST[@]} -eq 0 ]]; then
    echo "bundle-dylibs.sh: no bundleable dylibs for $(basename "$WORKER") (already bundled?); nothing to do."
    exit 0
fi

# Copy each dependency into Frameworks/ and set its own id to @rpath/<name>.
for dep in "${CLOSURE_LIST[@]}"; do
    name="$(basename "$dep")"
    dest="$FRAMEWORKS_DIR/$name"
    cp -f "$dep" "$dest"
    chmod u+w "$dest"
    install_name_tool -id "@rpath/$name" "$dest"
done

# Rewrite every reference to a closure dependency to @rpath on the worker and on
# each copied dylib. install_name_tool -change is a no-op when the reference is
# absent, so applying the full closure to every binary is safe.
rewrite_refs() {
    local target="$1"
    for dep in "${CLOSURE_LIST[@]}"; do
        install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$target"
    done
}

rewrite_refs "$WORKER"
for dep in "${CLOSURE_LIST[@]}"; do
    rewrite_refs "$FRAMEWORKS_DIR/$(basename "$dep")"
done

strip_local_rpaths() {
    local target="$1"
    local rpath
    while IFS= read -r rpath; do
        if [[ "$rpath" == /* ]]; then
            install_name_tool -delete_rpath "$rpath" "$target"
        fi
    done < <(rpaths_of "$target")
}

strip_local_rpaths "$WORKER"
for dep in "${CLOSURE_LIST[@]}"; do
    strip_local_rpaths "$FRAMEWORKS_DIR/$(basename "$dep")"
done

# Add the Frameworks rpath to the worker; tolerate it already being present.
if ! install_name_tool -add_rpath "@executable_path/../Frameworks" "$WORKER" 2>/dev/null; then
    echo "bundle-dylibs.sh: @executable_path/../Frameworks rpath already present on worker (ok)."
fi

# install_name_tool invalidates signatures — ad-hoc re-sign every Mach-O touched.
codesign --force --sign - "$WORKER"
for dep in "${CLOSURE_LIST[@]}"; do
    codesign --force --sign - "$FRAMEWORKS_DIR/$(basename "$dep")"
done

echo "bundle-dylibs.sh: bundled ${#CLOSURE_LIST[@]} dylib(s) into $FRAMEWORKS_DIR and re-signed."
