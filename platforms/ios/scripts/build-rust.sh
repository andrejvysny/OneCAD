#!/usr/bin/env bash
# Xcode "Build Rust" phase for the OneCAD app target (runs before Compile Sources).
#
# Builds crates/onecad-ios for the active SDK and configuration and stages
# libonecad_ios.a plus the kernel lane's libonecad_worker_bundle.a into
# ONECAD_RUST_LIB_DIR, which the app target links by path. Contract:
# docs/ios/BUILD.md ("Build phase contract").
#
#   PLATFORM_NAME  iphonesimulator -> aarch64-apple-ios-sim
#                  iphoneos        -> aarch64-apple-ios
#   CONFIGURATION  Release -> --release; Debug/Acceptance -> dev profile;
#                  Acceptance additionally -> --features onecad-ios/test-hooks
#
# ONECAD_IOS_STUB_LIB_DIR (explicit opt-in, never set by the project) skips
# cargo and the worker and stages link-only stub archives from that directory.
# It exists to compile/link the Swift host without the Rust lane; a stub build
# proves nothing about runtime behavior.
set -euo pipefail

die() {
    # "error:" prefix makes Xcode surface the line in the issue navigator.
    echo "error: build-rust.sh: $*" >&2
    exit 1
}

warn() {
    echo "warning: build-rust.sh: $*"
}

: "${PLATFORM_NAME:?build-rust.sh must run from Xcode (PLATFORM_NAME unset)}"
: "${CONFIGURATION:?build-rust.sh must run from Xcode (CONFIGURATION unset)}"
: "${SRCROOT:?build-rust.sh must run from Xcode (SRCROOT unset)}"
: "${ONECAD_RUST_LIB_DIR:?ONECAD_RUST_LIB_DIR build setting is missing}"

REPO_ROOT="$(cd "${SRCROOT}/../.." && pwd)"
readonly REPO_ROOT
readonly APP_LIB="libonecad_ios.a"
readonly WORKER_LIB="libonecad_worker_bundle.a"

rust_target() {
    case "$PLATFORM_NAME" in
        iphonesimulator) echo "aarch64-apple-ios-sim" ;;
        iphoneos) echo "aarch64-apple-ios" ;;
        *) die "unsupported PLATFORM_NAME '${PLATFORM_NAME}': only iphonesimulator and iphoneos (arm64) are built" ;;
    esac
}

check_archs() {
    local arch
    # Xcode leaves ARCHS empty when every requested arch is excluded (e.g.
    # ARCHS=x86_64 with the project's EXCLUDED_ARCHS); treat that as refused.
    [[ -n "${ARCHS:-}" ]] || die "ARCHS is empty: no supported architecture left for ${PLATFORM_NAME}; OneCAD iPad builds arm64 only (no x86_64 simulator)"
    for arch in ${ARCHS}; do
        if [[ "$arch" != "arm64" ]]; then
            die "unsupported architecture '${arch}': OneCAD iPad builds arm64 only (no x86_64 simulator); use an Apple-silicon simulator or generic iOS device destination"
        fi
    done
}

# Prints "<cargo profile dir> <worker config>".
profile_for_configuration() {
    case "$CONFIGURATION" in
        Release) echo "release Release" ;;
        Debug | Acceptance) echo "debug Debug" ;;
        *) die "unsupported CONFIGURATION '${CONFIGURATION}': expected Debug, Release or Acceptance" ;;
    esac
}

worker_dir() {
    local worker_config="$1"
    local suffix=""
    [[ "$CONFIGURATION" == "Acceptance" ]] && suffix="-test-hooks"
    echo "${REPO_ROOT}/target/ios-worker/${PLATFORM_NAME}-${worker_config}${suffix}"
}

# The worker bundle is produced by the kernel lane's scripts/build-ios-worker.sh.
ensure_worker() {
    local dir="$1" worker_config="$2"
    if [[ -f "${dir}/${WORKER_LIB}" ]]; then
        # Only built when missing: kernel/OCCT changes need a manual rebuild.
        echo "note: using existing ${dir}/${WORKER_LIB}; delete it or rerun scripts/build-ios-worker.sh after kernel changes"
        return 0
    fi
    local builder="${REPO_ROOT}/scripts/build-ios-worker.sh"
    if [[ ! -x "$builder" ]]; then
        die "missing ${dir}/${WORKER_LIB} and ${builder} does not exist; build the embedded worker for ${PLATFORM_NAME}/${worker_config} first (see docs/ios/BUILD.md)"
    fi
    local args=(--platform "$PLATFORM_NAME" --config "$worker_config" --out "$dir")
    [[ "$CONFIGURATION" == "Acceptance" ]] && args+=(--test-hooks)
    echo "==> ${builder} ${args[*]}"
    "$builder" "${args[@]}" || die "${builder} failed for ${PLATFORM_NAME}/${worker_config}"
    [[ -f "${dir}/${WORKER_LIB}" ]] || die "${builder} succeeded but did not produce ${dir}/${WORKER_LIB}"
}

cargo_target_dir() {
    echo "${REPO_ROOT}/target/ios-xcode/${PLATFORM_NAME}-${CONFIGURATION}"
}

build_rust() {
    local target="$1" profile="$2" worker="$3"
    local target_dir
    target_dir="$(cargo_target_dir)"
    local args=(build --locked -p onecad-ios --target "$target")
    [[ "$profile" == "release" ]] && args+=(--release)
    [[ "$CONFIGURATION" == "Acceptance" ]] && args+=(--features onecad-ios/test-hooks)

    # Xcode runs phases with a minimal PATH; rustup's shims live in CARGO_HOME.
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:${PATH}"
    command -v cargo >/dev/null || die "cargo not found in ${CARGO_HOME:-$HOME/.cargo}/bin or PATH; install rustup (docs/ios/BUILD.md)"

    echo "==> cargo ${args[*]} (CARGO_TARGET_DIR=${target_dir})"
    # SDKROOT names the iOS SDK; leaving it set makes host build scripts link
    # against the wrong platform. cc/rustc locate the target SDK via xcrun.
    (cd "$REPO_ROOT" && env -u SDKROOT \
        CARGO_TARGET_DIR="$target_dir" \
        ONECAD_EMBEDDED_WORKER_DIR="$worker" \
        cargo "${args[@]}") || die "cargo build failed for ${target} (${CONFIGURATION})"
}

# Copy only on content change so an unchanged library keeps its mtime and the
# app is not relinked.
stage() {
    local src="$1" name="$2"
    [[ -f "$src" ]] || die "expected archive not found: ${src}"
    mkdir -p "$ONECAD_RUST_LIB_DIR"
    local dst="${ONECAD_RUST_LIB_DIR}/${name}"
    if ! cmp -s "$src" "$dst"; then
        cp -f "$src" "$dst"
        echo "==> staged ${name} from ${src}"
    fi
}

# Release and Debug must never carry acceptance hooks (ARCHITECTURE.md, C ABI).
# Checked on the source archive, before anything is staged for the linker.
check_test_hooks() {
    local lib="$1"
    local symbols hooks
    [[ -f "$lib" ]] || die "expected archive not found: ${lib}"
    # Rust's std objects embed bitcode from a newer LLVM than Xcode's nm can
    # parse; the Mach-O symbol table is what the linker uses anyway.
    symbols="$(xcrun nm -gjU --no-llvm-bc "$lib")" || die "nm could not read ${lib}"
    hooks="$(grep -c '^_onecad_ios_test_' <<<"$symbols" || true)"
    if [[ "$CONFIGURATION" != "Acceptance" && "$hooks" != "0" ]]; then
        die "${lib} exports ${hooks} onecad_ios_test_ symbols in ${CONFIGURATION}; test-hooks must only be linked in Acceptance"
    fi
    if [[ "$CONFIGURATION" == "Acceptance" && "$hooks" == "0" ]]; then
        die "${lib} exports no onecad_ios_test_ symbols; Acceptance requires the test-hooks feature"
    fi
}

main() {
    check_archs
    local target profiles profile worker_config
    target="$(rust_target)"
    profiles="$(profile_for_configuration)"
    profile="${profiles% *}"
    worker_config="${profiles#* }"

    if [[ -n "${ONECAD_IOS_STUB_LIB_DIR:-}" ]]; then
        warn "ONECAD_IOS_STUB_LIB_DIR=${ONECAD_IOS_STUB_LIB_DIR}: linking STUB archives, not the Rust app or kernel; runtime behavior is not representative"
        check_test_hooks "${ONECAD_IOS_STUB_LIB_DIR}/${APP_LIB}"
        stage "${ONECAD_IOS_STUB_LIB_DIR}/${APP_LIB}" "$APP_LIB"
        stage "${ONECAD_IOS_STUB_LIB_DIR}/${WORKER_LIB}" "$WORKER_LIB"
        return 0
    fi

    local worker
    worker="$(worker_dir "$worker_config")"
    ensure_worker "$worker" "$worker_config"
    build_rust "$target" "$profile" "$worker"
    local built
    built="$(cargo_target_dir)/${target}/${profile}/${APP_LIB}"
    check_test_hooks "$built"
    stage "$built" "$APP_LIB"
    stage "${worker}/${WORKER_LIB}" "$WORKER_LIB"
}

main "$@"
