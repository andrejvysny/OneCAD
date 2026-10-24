#!/usr/bin/env bash
# Shared signing policy; source this file before modifying a bundle.

onecad_signing_init() {
    ONECAD_CODESIGN_ARGS=(--force --sign "${ONECAD_SIGNING_IDENTITY:--}")
    ONECAD_DEVELOPER_ID=0
    [[ "${ONECAD_SIGNING_IDENTITY:--}" == - ]] && return 0

    local identity
    identity="$(security find-identity -v -p codesigning | awk -v requested="$ONECAD_SIGNING_IDENTITY" '
        /"Developer ID Application: / {
            fingerprint = $2
            name = $0
            sub(/^[^"]*"/, "", name)
            sub(/".*$/, "", name)
            if (requested == fingerprint || requested == name) {
                selected = fingerprint
                matches++
            }
        }
        END { if (matches == 1) print selected; else exit 1 }
    ')" || {
        echo "macOS signing: requested valid Developer ID Application identity not found or ambiguous" >&2
        echo "Inspect unlocked keychains with: security find-identity -v -p codesigning" >&2
        return 1
    }
    ONECAD_CODESIGN_ARGS=(--force --sign "$identity" --options runtime --timestamp)
    ONECAD_DEVELOPER_ID=1
}

onecad_sign_target() {
    local target="$1"
    local entitlements="${2:-}"
    if [[ "$ONECAD_DEVELOPER_ID" == 1 && -n "$entitlements" ]]; then
        codesign "${ONECAD_CODESIGN_ARGS[@]}" --entitlements "$entitlements" "$target"
    else
        codesign "${ONECAD_CODESIGN_ARGS[@]}" "$target"
    fi
    codesign --verify --strict --verbose=2 "$target"
}
