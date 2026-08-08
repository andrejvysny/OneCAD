#!/usr/bin/env bash
# OCCT build-option fingerprinting — the EFFECTIVE-CONFIGURATION normalizer.
#
# Sourceable library, no side effects on source. `build-pinned-occt.sh` uses it;
# `scripts/tests/occt-fingerprint.test.sh` tests it standalone.
#
# ── Why this is not a plain "every -D must appear in CMakeCache.txt" check ────
# OCCT's top-level CMakeLists REMOVES a 3rd-party option from the cache when no
# toolkit in the built set requires the corresponding CSF_* product:
#
#     macro (OCCT_CHECK_AND_UNSET VARNAME)
#       if (DEFINED ${VARNAME})
#         unset (${VARNAME} CACHE)     # ← gone from CMakeCache.txt entirely
#       endif()
#     endmacro()
#
# and the capability flags are computed by scanning the toolkit set:
#
#     OCCT_IS_PRODUCT_REQUIRED (CSF_TclTkLibs CAN_USE_TK)
#
# So with `BUILD_MODULE_Draw=OFF` nothing requires CSF_TclTkLibs, `CAN_USE_TK` is
# OFF, and CMakeLists.txt (8.0.1 L558-565; 7.9.3 identical) runs
#
#     if (CAN_USE_TK AND USE_TK) … else() if (NOT CAN_USE_TK) OCCT_CHECK_AND_UNSET ("USE_TK")
#
# — our `-DUSE_TK=OFF` is deleted from the cache. A literal presence check then
# fails the build on a configuration that is exactly what we asked for.
#
# ── What "effective configuration" means here ────────────────────────────────
# An ABSENT dependent option is not unknown: OCCT deleted it precisely because
# the feature is not part of this build. Its effective value is therefore OFF.
# That substitution is accepted ONLY when the policy requested OFF. If the policy
# requested ON and the key vanished, the feature we asked for was silently
# dropped — a real mismatch, and still a hard failure.
#
# This is a normalization, NOT a relaxation:
#   · REQUIRED keys must still be materialized; absence is fatal.
#   · A DEPENDENT key that IS present must still match the policy value.
#   · Boolean spellings are canonicalized against CMake's own truth constants;
#     any other spelling is fatal rather than guessed.
#
# Provenance: the OCCT CMakeLists.txt at both pinned commits —
#   8.0.1 b8f597c677811d1f9f4d8a97f5ae2825c0353a42
#   7.9.3 a016080bf6738d6aeae020badee4e888ad1540a5
# — were diffed over their USE_*/CAN_USE_*/OCCT_CHECK_AND_UNSET lines and handle
# every key below identically, so one policy covers both versions.
#
# Bash 3.2 compatible on purpose (macOS ships 3.2 as /bin/bash): no associative
# arrays, no `mapfile`, no `${var^^}`.

# ── Policy ───────────────────────────────────────────────────────────────────
# These four are THE fingerprint contract. They are deliberately not `readonly`
# so the test can drive negative cases; no production caller may reassign them.

# Every fingerprinted key, in the emitted (LC_ALL=C sorted) order.
OCCT_OPTION_KEYS=(
    BUILD_LIBRARY_TYPE
    BUILD_MODULE_Draw
    BUILD_USE_PCH
    CMAKE_BUILD_TYPE
    USE_DRACO
    USE_FFMPEG
    USE_FREEIMAGE
    USE_FREETYPE
    USE_OPENGL
    USE_RAPIDJSON
    USE_TBB
    USE_TCL
    USE_TK
    USE_VTK
)

# Keys OCCT may delete from the cache when the build needs no such product.
# Each is annotated with the CMakeLists guard that can unset it.
#   BUILD_USE_PCH   CMake < 3.16
#   USE_DRACO       NOT CAN_USE_DRACO
#   USE_FFMPEG      NOT CAN_USE_FFMPEG
#   USE_FREEIMAGE   NOT CAN_USE_FREEIMAGE
#   USE_FREETYPE    NOT CAN_USE_FREETYPE
#   USE_OPENGL      NOT CAN_USE_OPENGL
#   USE_RAPIDJSON   NOT CAN_USE_RAPIDJSON
#   USE_TBB         NOT CAN_USE_TBB
#   USE_TK          NOT CAN_USE_TK        ← the observed Draw=OFF case
#   USE_VTK         TKIVtk/TKIVtkDraw not in BUILD_TOOLKITS
# Everything else is REQUIRED: BUILD_LIBRARY_TYPE / BUILD_MODULE_Draw /
# CMAKE_BUILD_TYPE are plain cache variables, and USE_TCL is only ever shadowed
# by a *normal* variable from OCCT_IS_PRODUCT_REQUIRED — never unset.
OCCT_DEPENDENT_OPTION_KEYS=(
    BUILD_USE_PCH USE_DRACO USE_FFMPEG USE_FREEIMAGE USE_FREETYPE
    USE_OPENGL USE_RAPIDJSON USE_TBB USE_TK USE_VTK
)

# Keys whose value is a boolean, and therefore canonicalized to ON/OFF. Every
# other key is compared verbatim (BUILD_LIBRARY_TYPE=Shared, CMAKE_BUILD_TYPE=Release).
OCCT_BOOLEAN_OPTION_KEYS=(
    BUILD_MODULE_Draw BUILD_USE_PCH USE_DRACO USE_FFMPEG USE_FREEIMAGE
    USE_FREETYPE USE_OPENGL USE_RAPIDJSON USE_TBB USE_TCL USE_TK USE_VTK
)

# The effective configuration this script guarantees, in OCCT_OPTION_KEYS order.
#
# `:=` so the test can pre-set a policy to exercise the "requested ON but OCCT
# dropped it" path. That seam is why `build-pinned-occt.sh` re-asserts this
# string against its own literal before configuring: a pre-set (or edited)
# policy can never quietly become the one a real build is fingerprinted against.
: "${OCCT_OPTION_POLICY:=BUILD_LIBRARY_TYPE=Shared,BUILD_MODULE_Draw=OFF,BUILD_USE_PCH=OFF,CMAKE_BUILD_TYPE=Release,USE_DRACO=OFF,USE_FFMPEG=OFF,USE_FREEIMAGE=OFF,USE_FREETYPE=OFF,USE_OPENGL=OFF,USE_RAPIDJSON=OFF,USE_TBB=OFF,USE_TCL=OFF,USE_TK=OFF,USE_VTK=OFF}"

# ── Helpers ──────────────────────────────────────────────────────────────────

_occt_contains() {
    local needle="$1"; shift
    local item
    for item in "$@"; do [ "$item" = "$needle" ] && return 0; done
    return 1
}

# Prints the policy value for a key. Fails when the key has no policy entry.
# Walks the comma-joined policy by parameter expansion rather than IFS word
# splitting — splitting behaves differently under zsh, and this file gets sourced
# ad hoc during debugging.
occt_policy_value() {
    local key="$1" rest="$OCCT_OPTION_POLICY" pair
    while [ -n "$rest" ]; do
        pair="${rest%%,*}"
        if [ "$pair" = "$rest" ]; then rest=""; else rest="${rest#*,}"; fi
        if [ "${pair%%=*}" = "$key" ]; then
            printf '%s' "${pair#*=}"
            return 0
        fi
    done
    echo "occt-fingerprint: no policy entry for key: $key" >&2
    return 1
}

# Canonicalizes a CMake boolean to ON/OFF. Only CMake's documented truth
# constants are accepted (case-insensitive, per cmake-language(7) "Condition
# Syntax"); anything else is a hard failure, so an unexpected value is never
# silently folded into a passing fingerprint.
occt_canonical_bool() {
    local upper
    upper="$(printf '%s' "$1" | tr '[:lower:]' '[:upper:]')"
    case "$upper" in
        ON | 1 | TRUE | Y | YES) printf 'ON' ;;
        OFF | 0 | FALSE | N | NO | IGNORE | NOTFOUND | "") printf 'OFF' ;;
        *-NOTFOUND) printf 'OFF' ;;
        *)
            echo "occt-fingerprint: not a CMake boolean: '$1'" >&2
            return 1
            ;;
    esac
}

# Prints one raw cache value; returns 1 when the key is absent from the cache.
# CMakeCache.txt lines are `KEY:TYPE=VALUE`; a `-D` with no type yields
# `KEY:UNINITIALIZED=VALUE`, so the type is matched loosely. The last occurrence
# wins, matching CMake's own read order. An empty value is a real value, so
# presence is decided by grep, not by emptiness.
occt_cache_lookup() {
    local cache_file="$1" key="$2"
    grep -q "^${key}:[^=]*=" "$cache_file" || return 1
    sed -n "s/^${key}:[^=]*=//p" "$cache_file" | tail -n 1
}

# ── The normalizer ───────────────────────────────────────────────────────────
#
#   occt_normalized_options <CMakeCache.txt>
#
# Prints the effective, comma-joined configuration in OCCT_OPTION_KEYS order.
# Returns non-zero (reason on stderr) when the configured build cannot be
# represented by the policy.
occt_normalized_options() {
    local cache_file="$1"
    if [ ! -f "$cache_file" ]; then
        echo "occt-fingerprint: no such cache file: $cache_file" >&2
        return 1
    fi

    local out="" key raw value expected
    for key in "${OCCT_OPTION_KEYS[@]}"; do
        expected="$(occt_policy_value "$key")" || return 1

        if raw="$(occt_cache_lookup "$cache_file" "$key")"; then
            if _occt_contains "$key" "${OCCT_BOOLEAN_OPTION_KEYS[@]}"; then
                value="$(occt_canonical_bool "$raw")" || {
                    echo "occt-fingerprint: option $key" >&2
                    return 1
                }
            else
                value="$raw"
            fi
        elif ! _occt_contains "$key" "${OCCT_DEPENDENT_OPTION_KEYS[@]}"; then
            # A non-dependent key must always be materialized.
            echo "occt-fingerprint: configured OCCT cache omitted required option: $key" >&2
            return 1
        elif [ "$expected" != "OFF" ]; then
            # We asked for the feature and OCCT dropped it — the build is NOT what
            # the policy describes. Never normalize this away.
            echo "occt-fingerprint: dependent option $key was requested '$expected' but" \
                "OCCT removed it from the cache (feature not built)" >&2
            return 1
        else
            # Absent + requested OFF ⇒ OCCT excluded the feature. Effective OFF.
            value="OFF"
        fi

        [ -n "$out" ] && out="${out},"
        out="${out}${key}=${value}"
    done
    printf '%s' "$out"
}

#   occt_verify_options <CMakeCache.txt>
#
# Normalizes and compares against the policy. Prints the normalized string on
# success; diagnoses the difference and returns 1 otherwise.
occt_verify_options() {
    local cache_file="$1" actual
    actual="$(occt_normalized_options "$cache_file")" || return 1
    if [ "$actual" != "$OCCT_OPTION_POLICY" ]; then
        echo "configured OCCT options differ from fingerprint policy" >&2
        echo "expected: $OCCT_OPTION_POLICY" >&2
        echo "actual:   $actual" >&2
        return 1
    fi
    printf '%s' "$actual"
}
