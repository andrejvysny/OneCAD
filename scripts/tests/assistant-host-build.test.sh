#!/usr/bin/env bash
# Negative controls for the assistant sidecar's build and bootstrap scripts.
#
# Both scripts exist to REFUSE: an unknown target, a missing compiler, a cached
# artifact that no longer matches its inputs, a pin that is not the pin, a
# prefix this project did not create. Every one of those was previously a silent
# success — the build script fell back to `aarch64-apple-darwin` when it could
# not name a target, reused any artifact that happened to exist, and `rm -rf`'d
# whatever path `AGENTKIT_PREFIX` pointed at. A check that cannot go red is not
# a check.
#
# The cache controls mutate the STAGED artifact and its manifest, then rebuild,
# so the tree is left with a correct staged sidecar. The bootstrap controls run
# against throwaway prefixes in a temp directory and never touch `.agentkit-src`.
# Nothing here reaches the network.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
build="$root/scripts/build-assistant-host.sh"
bootstrap="$root/scripts/bootstrap-agentkit.sh"
mapping="$root/scripts/lib/assistant-host-target.sh"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pass() { echo "  ✓ $1"; }
fail() {
    echo "  ✗ $1" >&2
    exit 1
}

# Requires `$1` (a command line, run through bash -c) to fail, and its combined
# output to contain `$2`.
expect_failure() {
    local command="$1" expected="$2" why="$3" output status
    set +e
    output="$(bash -c "$command" 2>&1)"
    status=$?
    set -e
    [ "$status" -ne 0 ] || fail "$why — it SUCCEEDED"
    case "$output" in
    *"$expected"*) pass "$why" ;;
    *) fail "$why — output did not mention '$expected':
$output" ;;
    esac
}

# Requires `$1` to produce output containing `$2`, whatever its exit status.
expect_output() {
    local command="$1" expected="$2" why="$3" output
    set +e
    output="$(bash -c "$command" 2>&1)"
    set -e
    case "$output" in
    *"$expected"*) pass "$why" ;;
    *) fail "$why — output did not mention '$expected':
$output" ;;
    esac
}

echo "target mapping:"
[ "$(bash "$mapping" staged-name x86_64-pc-windows-msvc)" = \
    "onecad-assistant-host-x86_64-pc-windows-msvc.exe" ] ||
    fail "a Windows staged name must carry .exe"
pass "a Windows staged name carries .exe"
[ "$(bash "$mapping" staged-name aarch64-apple-darwin)" = \
    "onecad-assistant-host-aarch64-apple-darwin" ] ||
    fail "a macOS staged name carries no extension"
pass "a macOS staged name carries no extension"
[ "$(bash "$mapping" bun-target x86_64-unknown-linux-gnu)" = "bun-linux-x64" ] ||
    fail "the Linux Bun target is bun-linux-x64"
pass "each supported triple maps to one Bun target"

echo "build script refusals:"
expect_failure "bash '$build' --target sparc64-unknown-nothing" \
    "no Bun compile target is known" \
    "an unknown target fails instead of falling back to Apple Silicon"
expect_failure "bash '$build' --target x86_64-pc-windows-msvc-typo" \
    "no Bun compile target is known" \
    "a mistyped target fails instead of being built under the wrong name"
expect_failure "bash '$build' --wat" "unknown argument" \
    "an unknown argument is refused"
# PATH without bun: the script must say so rather than reuse whatever is staged.
bun_free="$work/bun-free-path"
mkdir -p "$bun_free"
for tool in bash sh env dirname sed cut find git date ls chmod mkdir cat sha256sum shasum; do
    command -v "$tool" >/dev/null 2>&1 && ln -sf "$(command -v "$tool")" "$bun_free/$tool"
done
expect_failure "PATH='$bun_free' bash '$build'" "bun is required but not on PATH" \
    "a missing Bun compiler fails loudly"

echo "build cache verification:"
triple="$(bash "$mapping" host-triple)"
staged="$root/src-tauri/binaries/$(bash "$mapping" staged-name "$triple")"
manifest="$staged.manifest.json"
bash "$build" >/dev/null
[ -f "$staged" ] && [ -f "$manifest" ] || fail "the build must stage a binary and a manifest"
expect_output "bash '$build'" "Verified staged sidecar, nothing to rebuild" \
    "an unchanged input reuses the verified artifact"

# A stale artifact: the file on disk is no longer the one the manifest recorded.
printf 'x' >>"$staged"
expect_output "bash '$build'" "the staged binary is not the one the manifest recorded" \
    "a modified artifact is rebuilt, not shipped"

# A changed source or lockfile.
sed -i.bak 's/"sourceSha256": "[^"]*"/"sourceSha256": "0000"/' "$manifest" && rm -f "$manifest.bak"
expect_output "bash '$build'" "the sidecar source or lockfile changed" \
    "a changed source digest is rebuilt"

# A different AgentKit pin.
sed -i.bak 's/"agentkitCommit": "[^"]*"/"agentkitCommit": "deadbeef"/' "$manifest" && rm -f "$manifest.bak"
expect_output "bash '$build'" "the AgentKit pin changed" \
    "a changed AgentKit pin is rebuilt"

# A binary built for another target.
sed -i.bak 's/"bunTarget": "[^"]*"/"bunTarget": "bun-windows-x64"/' "$manifest" && rm -f "$manifest.bak"
expect_output "bash '$build'" "it was built for bun-windows-x64" \
    "an artifact built for another target is rebuilt"

# A binary built by another Bun.
sed -i.bak 's/"bunVersion": "[^"]*"/"bunVersion": "0.0.1"/' "$manifest" && rm -f "$manifest.bak"
expect_output "bash '$build'" "a different Bun built it (0.0.1)" \
    "an artifact built by another Bun is rebuilt"

# Leave the tree with a verified artifact.
bash "$build" >/dev/null
expect_output "bash '$build'" "Verified staged sidecar" "the tree is left verified"

echo "bootstrap safety and provenance:"
# THE destructive-cleanup control: a prefix this script did not create must be
# refused, and must still be there afterwards.
foreign="$work/someones-directory"
mkdir -p "$foreign"
echo "important" >"$foreign/notes.txt"
expect_failure "AGENTKIT_PREFIX='$foreign' bash '$bootstrap'" "Refusing to delete it" \
    "a prefix without the ownership marker is never deleted"
[ -f "$foreign/notes.txt" ] || fail "the refused prefix was deleted anyway"
pass "the refused prefix is still on disk"

# A fake pinned checkout, so the cache decisions can be exercised without the
# network and without touching the real .agentkit-src.
fake="$work/fake-prefix"
mkdir -p "$fake/packages/agentkit/dist"
git -C "$fake" init -q
git -C "$fake" config user.email t@example.com
git -C "$fake" config user.name test
echo '{"name":"agentkit"}' >"$fake/packages/agentkit/dist/index.js"
echo 'lock' >"$fake/bun.lock"
git -C "$fake" add -A
git -C "$fake" commit -qm pinned
pin="$(git -C "$fake" rev-parse HEAD)"
dist_sha="$(sha256sum "$fake/packages/agentkit/dist/index.js" 2>/dev/null | cut -d' ' -f1 ||
    shasum -a 256 "$fake/packages/agentkit/dist/index.js" | cut -d' ' -f1)"
lock_sha="$(sha256sum "$fake/bun.lock" 2>/dev/null | cut -d' ' -f1 ||
    shasum -a 256 "$fake/bun.lock" | cut -d' ' -f1)"
stamp="$fake/.onecad-agentkit-build.json"
write_stamp() {
    cat >"$stamp" <<JSON
{
  "commit": "$1",
  "lockfileSha256": "$2",
  "distSha256": "$3",
  "bunVersion": "1.3.11"
}
JSON
}

# The honest cache hit.
write_stamp "$pin" "$lock_sha" "$dist_sha"
expect_output "AGENTKIT_PREFIX='$fake' AGENTKIT_COMMIT='$pin' bash '$bootstrap'" \
    "verified cached build" "a stamp that matches every input is reused"

# No stamp at all — the old early exit, which proved only that a file existed.
rm -f "$stamp"
expect_output "AGENTKIT_PREFIX='$fake' AGENTKIT_COMMIT='$pin' bash '$bootstrap'" \
    "no build stamp" "a dist with no stamp is not evidence of anything"

# A dist that was built from another commit.
write_stamp "0000000000000000000000000000000000000000" "$lock_sha" "$dist_sha"
expect_output "AGENTKIT_PREFIX='$fake' AGENTKIT_COMMIT='$pin' bash '$bootstrap'" \
    "the dist was built from" "a dist from another commit is rebuilt"

# A dist that changed after it was built.
write_stamp "$pin" "$lock_sha" "0000"
expect_output "AGENTKIT_PREFIX='$fake' AGENTKIT_COMMIT='$pin' bash '$bootstrap'" \
    "the dist has changed since it was built" "a mutated dist is rebuilt"

# A lockfile that changed after it was built.
write_stamp "$pin" "0000" "$dist_sha"
expect_output "AGENTKIT_PREFIX='$fake' AGENTKIT_COMMIT='$pin' bash '$bootstrap'" \
    "the lockfile has changed" "a changed lockfile is rebuilt"

# The wrong pin: the prefix is at one commit and the script wants another. The
# refetch it then attempts has no reachable remote here, which is why this
# asserts the decision rather than the outcome.
write_stamp "$pin" "$lock_sha" "$dist_sha"
expect_output "AGENTKIT_PREFIX='$fake' AGENTKIT_COMMIT='1111111111111111111111111111111111111111' bash '$bootstrap'" \
    "want 1111111111111111111111111111111111111111" "a prefix at the wrong pin is refetched"

# Locally modified pinned source: refused, so an edit is never silently
# discarded and never silently shipped.
echo "edited" >>"$fake/packages/agentkit/dist/index.js"
expect_failure "AGENTKIT_PREFIX='$fake' AGENTKIT_COMMIT='$pin' bash '$bootstrap'" \
    "has local modifications" "a dirty pinned checkout is refused"

echo "assistant-host build controls: all passed"
