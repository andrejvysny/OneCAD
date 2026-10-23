#!/usr/bin/env bash
# Content provenance shared by the embedded worker builder and bounded tests.
# Callers set the resolved platform/build variables before collecting inputs.

provenance_sha256() {
    local result
    result="$(shasum -a 256 "$1")" || return 1
    [[ "$result" =~ ^[a-f0-9]{64}[[:space:]] ]] || return 1
    printf '%s\n' "${result:0:64}"
}

provenance_tree() (
    set -o pipefail
    local path="$1"
    [[ -e "$path" ]] || { echo "provenance: missing input ${path}" >&2; return 1; }
    printf 'path=%s\n' "$path" || return 1
    if [[ -d "$path" ]]; then
        find -L "$path" -type f -exec shasum -a 256 {} + | LC_ALL=C sort || return 1
    else
        shasum -a 256 "$path" || return 1
    fi
)

provenance_inputs() {
    local path prefix tool
    local prefix_paths=()
    printf 'format=1\nplatform=%s\nconfig=%s\nhooks=%s\narch=arm64\ndeployment=%s\n' \
        "$PLATFORM" "$CONFIG" "$TEST_HOOKS" "$DEPLOYMENT_TARGET" || return 1
    printf 'sdk=%s\nsdkVersion=%s\nocctVersion=%s\n' "$SDK_PATH" "$SDK_VERSION" "$OCCT_VERSION" || return 1
    for path in worker/src worker/cmake worker/third_party worker/tools/link_probe \
        worker/CMakeLists.txt protocol crates/onecad-protocol crates/onecad-ios/include \
        scripts/build-ios-worker.sh scripts/ios-worker-provenance.sh \
        scripts/build-ios-occt.sh scripts/build-pinned-occt.sh scripts/check-worker-stdout-hygiene.sh; do
        provenance_tree "${ROOT_DIR}/${path}" || return 1
    done
    provenance_tree "$OCCT_ROOT" || return 1
    local prefixes="${ONECAD_HEADER_PREFIXES:-/opt/homebrew/opt/eigen;/opt/homebrew/opt/boost}"
    IFS=';' read -r -a prefix_paths <<<"$prefixes" || return 1
    for prefix in "${prefix_paths[@]}"; do provenance_tree "$prefix" || return 1; done
    provenance_tree "${SDK_PATH}/SDKSettings.json" || return 1
    for path in clang++ ld libtool; do
        tool="$(xcrun --sdk "$PLATFORM" --find "$path")" || return 1
        provenance_tree "$tool" || return 1
    done
    xcrun --sdk "$PLATFORM" clang++ --version || return 1
    xcrun --sdk "$PLATFORM" --show-sdk-build-version || return 1
    for tool in cmake ninja; do command -v "$tool" || return 1; done
    cmake --version || return 1
    ninja --version || return 1
    # These are passed explicitly to CMake; inherited caches cannot override them.
    printf 'CXXFLAGS=%s\nLDFLAGS=%s\n' "${CXXFLAGS:-}" "${LDFLAGS:-}"
}

provenance_collect() {
    local snapshot="$1"
    provenance_inputs >"$snapshot" || return 1
    provenance_sha256 "$snapshot"
}

provenance_outputs() {
    local path digest
    for path in libonecad_worker_bundle.a onecad-embedded-kernel.json \
        onecad-embedded-link.txt onecad_worker.h audit.txt; do
        [[ -s "${OUT_DIR}/${path}" ]] || return 1
        digest="$(provenance_sha256 "${OUT_DIR}/${path}")" || return 1
        printf '%s=%s\n' "$path" "$digest" || return 1
    done
}

provenance_record() {
    printf 'inputSha256=%s\n' "$1" || return 1
    provenance_outputs
}

provenance_matches() {
    local expected="$1" actual recorded
    [[ -s "${OUT_DIR}/onecad-worker-provenance.txt" ]] || return 1
    actual="$(provenance_record "$expected")" || return 1
    recorded="$(cat "${OUT_DIR}/onecad-worker-provenance.txt")" || return 1
    [[ "$recorded" == "$actual" ]]
}
