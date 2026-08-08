#!/usr/bin/env bash
# Standalone test for scripts/occt-fingerprint.sh. No framework, no network, no
# OCCT build required: every case is a synthetic CMakeCache.txt.
#
# Run:  scripts/tests/occt-fingerprint.test.sh
#
# ── Fixture provenance ───────────────────────────────────────────────────────
# The 8.0.1 fixture is a VERBATIM CAPTURE of the fingerprinted lines from a real
# `CMakeCache.txt`, produced by running
#
#   scripts/build-pinned-occt.sh --version 8.0.1 --prefix … --build-dir …
#     --build-id onecad-occt-8.0.1-b8f597c67781-kp1
#
# on macOS 15 (Darwin 25.4.0) / CMake 4.x / Ninja, 2026-08-08. `USE_TK` really is
# absent there, and the `:UNINITIALIZED` types really are what a `-D` with no
# declared type produces. The 7.9.3 fixture is the same shape (see below).
#
# Every ABSENCE is justified by a specific guard in the pinned CMakeLists.txt:
#
#   USE_TK absent  ← 8.0.1 L558-565 / 7.9.3 (identical block):
#       if (CAN_USE_TK AND USE_TK) … else()
#         if (NOT CAN_USE_TK)
#           OCCT_CHECK_AND_UNSET ("USE_TK")   # unset(USE_TK CACHE)
#     with CAN_USE_TK from OCCT_IS_PRODUCT_REQUIRED (CSF_TclTkLibs CAN_USE_TK),
#     which scans BUILD_TOOLKITS — empty of Tcl/Tk consumers under
#     BUILD_MODULE_Draw=OFF.
#
#   USE_TCL present ← 8.0.1 L527 OCCT_IS_PRODUCT_REQUIRED (CSF_TclLibs USE_TCL)
#     assigns a *normal* variable (occt_macros.cmake L459-477, `set(… PARENT_SCOPE)`),
#     which shadows but never unsets the cache entry our `-DUSE_TCL=OFF` created.
#     A `-D` with no type is recorded as `:UNINITIALIZED`.
#
#   USE_VTK present ← guarded by `list (FIND BUILD_TOOLKITS TKIVtk…)`, and TKIVtk
#     lives in the Visualization module, which stays ON.
#
# The 7.9.3 fixture differs only in cosmetic cache lines: both CMakeLists handle
# every fingerprinted key identically (verified by diffing their
# USE_*/CAN_USE_*/OCCT_CHECK_AND_UNSET lines at the pinned commits).
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
LIB="${HERE}/../occt-fingerprint.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail=0

ok() { printf '  ok   %s\n' "$1"; pass=$((pass + 1)); }
no() { printf '  FAIL %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

# The policy string, restated here on purpose: if someone edits the library's
# policy, this literal must be updated too — the fingerprint never changes by
# accident on one side only.
EXPECTED="BUILD_LIBRARY_TYPE=Shared,BUILD_MODULE_Draw=OFF,BUILD_USE_PCH=OFF,CMAKE_BUILD_TYPE=Release,USE_DRACO=OFF,USE_FFMPEG=OFF,USE_FREEIMAGE=OFF,USE_FREETYPE=OFF,USE_OPENGL=OFF,USE_RAPIDJSON=OFF,USE_TBB=OFF,USE_TCL=OFF,USE_TK=OFF,USE_VTK=OFF"

# ── Fixtures ─────────────────────────────────────────────────────────────────

# `cmake -DUSE_TCL=OFF` with no declared type → UNINITIALIZED; OCCT's own
# `set(… CACHE BOOL)` declarations become BOOL.
cache_801() {
    cat >"$1" <<'EOF'
# This is the CMakeCache file.
BUILD_LIBRARY_TYPE:UNINITIALIZED=Shared
BUILD_MODULE_Draw:BOOL=OFF
BUILD_MODULE_Visualization:BOOL=ON
BUILD_USE_PCH:UNINITIALIZED=OFF
CMAKE_BUILD_TYPE:STRING=Release
CMAKE_INSTALL_PREFIX:PATH=/tmp/onecad-occt/8.0.1
INSTALL_DIR:PATH=/tmp/onecad-occt/8.0.1
USE_DRACO:BOOL=OFF
USE_FFMPEG:BOOL=OFF
USE_FREEIMAGE:BOOL=OFF
USE_FREETYPE:BOOL=OFF
USE_OPENGL:BOOL=OFF
USE_RAPIDJSON:BOOL=OFF
USE_TBB:BOOL=OFF
USE_TCL:UNINITIALIZED=OFF
USE_VTK:UNINITIALIZED=OFF
//ADVANCED property for variable: CMAKE_AR
CMAKE_AR-ADVANCED:INTERNAL=1
EOF
}

cache_793() {
    cat >"$1" <<'EOF'
CMAKE_BUILD_TYPE:STRING=Release
BUILD_LIBRARY_TYPE:STRING=Shared
BUILD_MODULE_Draw:BOOL=OFF
BUILD_USE_PCH:BOOL=OFF
USE_DRACO:BOOL=OFF
USE_FFMPEG:BOOL=OFF
USE_FREEIMAGE:BOOL=OFF
USE_FREETYPE:BOOL=OFF
USE_GLES2:BOOL=OFF
USE_OPENGL:BOOL=OFF
USE_OPENVR:BOOL=OFF
USE_RAPIDJSON:BOOL=OFF
USE_TBB:BOOL=OFF
USE_TCL:UNINITIALIZED=OFF
USE_VTK:BOOL=OFF
EOF
}

# ── Runner ───────────────────────────────────────────────────────────────────
#
# Every case runs in a SUBSHELL that sources the library fresh, so a case may
# pre-set policy globals to drive a negative path without leaking into the next.

# expect_ok <name> <cache-builder-or-file> [pre-source setup]
expect_ok() {
    local name="$1" file="$2" setup="${3:-}"
    local got rc
    got="$(
        # shellcheck disable=SC1090
        eval "$setup"
        . "$LIB"
        occt_normalized_options "$file" 2>&1
    )"
    rc=$?
    if [ $rc -ne 0 ]; then
        no "$name" "expected success, got rc=$rc: $got"
    elif [ "$got" != "$EXPECTED" ]; then
        no "$name" "normalized mismatch: $got"
    else
        ok "$name"
    fi
}

# expect_fail <name> <cache-file> <substring-of-stderr> [pre-source setup]
expect_fail() {
    local name="$1" file="$2" needle="$3" setup="${4:-}"
    local got rc
    got="$(
        # shellcheck disable=SC1090
        eval "$setup"
        . "$LIB"
        occt_normalized_options "$file" 2>&1
    )"
    rc=$?
    if [ $rc -eq 0 ]; then
        no "$name" "expected failure, succeeded with: $got"
    elif ! printf '%s' "$got" | grep -qF "$needle"; then
        no "$name" "wrong diagnostic; wanted '$needle', got: $got"
    else
        ok "$name"
    fi
}

echo "occt-fingerprint"

# 1 — the reported 8.0.1 break: USE_TK deleted by OCCT, everything else present.
c="$TMP/801.txt"; cache_801 "$c"
expect_ok "8.0.1 Draw=OFF (USE_TK unset by OCCT) normalizes to policy" "$c"

# 2 — 7.9.3 must produce the SAME fingerprint from its own cache shape.
c793="$TMP/793.txt"; cache_793 "$c793"
expect_ok "7.9.3 Draw=OFF normalizes to the identical policy string" "$c793"

# 3 — determinism: same input, same output, byte for byte.
a="$(. "$LIB"; occt_normalized_options "$c")"
b="$(. "$LIB"; occt_normalized_options "$c793")"
if [ "$a" = "$b" ]; then ok "7.9.3 and 8.0.1 fingerprints are byte-identical"
else no "7.9.3 and 8.0.1 fingerprints are byte-identical" "$a != $b"; fi

# 4 — a cache where OCCT kept every option (e.g. a Draw=ON-shaped build that
#     still configured all our features OFF) is equally valid.
call="$TMP/all-present.txt"; cache_801 "$call"; echo "USE_TK:BOOL=OFF" >>"$call"
expect_ok "every dependent option present is accepted" "$call"

# 5 — a present dependent option that DISAGREES with policy is still a failure:
#     normalization must not swallow a real difference.
cbad="$TMP/tk-on.txt"; cache_801 "$cbad"; echo "USE_TK:BOOL=ON" >>"$cbad"
got="$(. "$LIB"; occt_normalized_options "$cbad")"
if [ "$got" = "$EXPECTED" ]; then no "present-but-ON dependent option is not normalized away" "got policy string"
else ok "present-but-ON dependent option is not normalized away"; fi

# 6 — alternate CMake boolean spellings canonicalize (0/FALSE/no/NOTFOUND/empty).
cspell="$TMP/spellings.txt"
cat >"$cspell" <<'EOF'
CMAKE_BUILD_TYPE:STRING=Release
BUILD_LIBRARY_TYPE:STRING=Shared
BUILD_MODULE_Draw:BOOL=0
BUILD_USE_PCH:BOOL=FALSE
USE_DRACO:BOOL=no
USE_FFMPEG:BOOL=N
USE_FREEIMAGE:BOOL=NOTFOUND
USE_FREETYPE:BOOL=
USE_OPENGL:BOOL=Freetype-NOTFOUND
USE_RAPIDJSON:BOOL=IGNORE
USE_TBB:BOOL=off
USE_TCL:UNINITIALIZED=OFF
USE_VTK:BOOL=0
EOF
expect_ok "CMake boolean spellings canonicalize to ON/OFF" "$cspell"

# 7 — a value that is not a CMake boolean is fatal, never guessed.
cjunk="$TMP/junk.txt"; cache_801 "$cjunk"
sed -i.bak 's/^USE_TBB:BOOL=OFF/USE_TBB:BOOL=MAYBE/' "$cjunk"
expect_fail "a non-boolean value is fatal" "$cjunk" "not a CMake boolean: 'MAYBE'"

# 8 — a missing REQUIRED key is still fatal (the fingerprint is not weakened).
creq="$TMP/no-tcl.txt"; cache_801 "$creq"
sed -i.bak '/^USE_TCL:/d' "$creq"
expect_fail "a missing required key is fatal" "$creq" "omitted required option: USE_TCL"

creq2="$TMP/no-buildtype.txt"; cache_801 "$creq2"
sed -i.bak '/^CMAKE_BUILD_TYPE:/d' "$creq2"
expect_fail "a missing CMAKE_BUILD_TYPE is fatal" "$creq2" "omitted required option: CMAKE_BUILD_TYPE"

# 9 — THE safety property: a dependent option we requested ON must never be
#     normalized to OFF just because OCCT removed it. Driven by pre-setting the
#     policy before sourcing (see the note in the library).
POLICY_TK_ON="${EXPECTED/USE_TK=OFF/USE_TK=ON}"
expect_fail "a dependent option requested ON that OCCT dropped is fatal" "$c" \
    "was requested 'ON'" \
    "OCCT_OPTION_POLICY='$POLICY_TK_ON'"

# 10 — duplicate cache lines: the last wins, matching CMake's read order.
cdup="$TMP/dup.txt"; cache_801 "$cdup"
echo "USE_TBB:BOOL=ON" >>"$cdup"
echo "USE_TBB:BOOL=OFF" >>"$cdup"
expect_ok "duplicate cache entries resolve to the last occurrence" "$cdup"

# 11 — a missing cache file fails cleanly rather than emitting a fingerprint.
expect_fail "a missing cache file is fatal" "$TMP/does-not-exist.txt" "no such cache file"

# 12 — occt_verify_options agrees with the policy for a real-shape cache and
#      reports a diff otherwise.
if v="$(. "$LIB"; occt_verify_options "$c")" && [ "$v" = "$EXPECTED" ]; then
    ok "occt_verify_options accepts the policy-conformant cache"
else
    no "occt_verify_options accepts the policy-conformant cache" "got: ${v:-<fail>}"
fi
if (. "$LIB"; occt_verify_options "$cbad" >/dev/null 2>&1); then
    no "occt_verify_options rejects a non-conformant cache" "unexpectedly accepted"
else
    ok "occt_verify_options rejects a non-conformant cache"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
