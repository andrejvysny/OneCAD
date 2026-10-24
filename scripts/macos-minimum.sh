#!/usr/bin/env bash
# Derive the bundle's launch floor from the code it actually ships.
set -euo pipefail

fail() {
    printf 'macOS minimum: %s\n' "$*" >&2
    exit 1
}

minimum_for() {
    local path="$1" metadata
    xcrun lipo "$path" -verify_arch arm64 >/dev/null || fail "missing arm64 slice: $path"
    metadata="$(xcrun vtool -arch arm64 -show-build "$path")" || fail "cannot inspect: $path"
    printf '%s\n' "$metadata" | awk '
        $1 == "cmd" {
            active = ($2 == "LC_BUILD_VERSION"); commands += active
            if (active && NF != 2) bad = 1
            next
        }
        active && $1 == "platform" { platforms++; platform = $2; if (NF != 2) bad = 1 }
        active && $1 == "minos" { minima++; version = $2; if (NF != 2) bad = 1 }
        END {
            if (bad || commands != 1 || platforms != 1 || minima != 1 ||
                (platform != "MACOS" && platform != "1") ||
                version !~ /^[0-9]+[.][0-9]+([.][0-9]+)?$/) exit 1
            split(version, parts, ".")
            if (parts[1] < 1 || parts[1] > 65535 || parts[2] > 255 || parts[3] > 255) exit 1
            printf "%d.%d.%d\n", parts[1], parts[2], parts[3]
        }
    ' || fail "invalid or ambiguous macOS build version: $path"
}

maximum_of() {
    awk -v current="$1" -v candidate="$2" 'BEGIN {
        split(current, a, "."); split(candidate, b, ".")
        for (i = 1; i <= 3; i++) {
            if (a[i] + 0 > b[i] + 0) { print current; exit }
            if (a[i] + 0 < b[i] + 0) { print candidate; exit }
        }
        print current
    }'
}

scan_files() {
    local path kind minimum maximum=0.0.0 count=0
    while IFS= read -r -d '' path; do
        kind="$(file -b "$path")" || fail "cannot identify: $path"
        case "$kind" in
            *Mach-O*) ;;
            *)
                case "$path" in
                    */Contents/MacOS/* | */Contents/Frameworks/*.dylib)
                        fail "expected Mach-O code: $path" ;;
                esac
                continue ;;
        esac
        minimum="$(minimum_for "$path")" || return 1
        maximum="$(maximum_of "$maximum" "$minimum")"
        count=$((count + 1))
    done
    [[ "$count" -gt 0 ]] || fail 'bundle contains no Mach-O code'
    printf '%s\n' "$maximum"
}

[[ $# -eq 1 ]] || fail 'usage: scripts/macos-minimum.sh APP'
APP="$1"
[[ -d "$APP/Contents" ]] || fail "not an app bundle: $APP"
for required in onecad onecad-worker; do
    [[ -f "$APP/Contents/MacOS/$required" ]] || fail "missing $required"
done
# Unresolved aliases must not bypass code inspection or refer outside the bundle.
links="$(find "$APP/Contents" -type l -print -quit)" || fail 'cannot enumerate bundle aliases'
[[ -z "$links" ]] || fail "unsupported bundle symlink: $links"
find "$APP/Contents" -type f -print0 | scan_files
