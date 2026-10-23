# Manual native release procedures

Project outcomes and unfinished acceptance work belong in the existing
[ONECAD Plane project](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/issues/).
Use its [documentation index](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/pages/0bfe867d-215a-4cfb-a501-9f628976bc9d)
to locate canonical QA records. This document describes procedures, not completed
gates. Retired web-editor results do not qualify the native host. A missing
capability is a gap under its current owner, not permission to implement it as
part of cleanup.

## Preparation and evidence

Use the pinned toolchain, a fresh production sidecar and an isolated temporary
profile. From the repository root:

```sh
make build
make run QA_ROOT=/private/tmp/onecad-release-check
```

`make build` builds its worker under `worker/build-native` by default and generates
a constrained-plate QA fixture when needed. `make run` only launches those existing
outputs; use `make dev` to build before launch. See the [native host instructions](../../crates/onecad-ui/README.md)
for blank-document startup, profile isolation, host arguments and input tracing.
Use a fresh profile and cold process for startup and theme checks. Restarting a
process is distinct from reopening a document in the running host.

Record the checked Git revision, OS/display/input setup, actual worker identity,
profile path, commands, observations and required evidence. Record failures as
`GATE <section>/<item> — expected vs observed`, with the relevant profile
`logs/dev.jsonl` excerpt. `ONECAD_INPUT_TRACE=1` enables bounded native input
records. Screenshots and saved-document inspection establish visible and
persisted results; GPU completion logs alone do not prove OS presentation.

## Visual checks

Inspect the current native host in Light, Dark and System appearance, using the
same fixture and camera where comparing images:

- Check Wireframe, Shaded, ShadedEdges and supported Xray output across a full
  orbit. Check edge contrast, highlights, grid and shadow behavior.
- Check sketch dim/restore, selected geometry, constraint labels, origin markers
  and repair-candidate overlays where supported. Compare observed behavior with
  canonical acceptance requirements; do not infer parity from a retired capture.
- Check native titlebar/traffic-light alignment, chrome spacing and text/icon
  readability. Switch the OS appearance while System is selected and relaunch
  to check persisted appearance without a startup flash.
- Check Home/recents using a project saved in the isolated profile; reopen it
  and verify visible geometry and document identity.

Approved renderer goldens and declared tolerances remain unchanged. Manual
review supplements required Metal tests; it does not replace them.

## Native input and lifecycle checks

Use physical mouse, keyboard and trackpad hardware. Synthetic host tests do not
establish these outcomes:

- Check menu and keyboard commands with document focus and text-field focus,
  including Undo/Redo and clipboard routing. Check Japanese IME composition,
  candidate placement and input-source switching.
- Check dirty-document close through the titlebar, Cmd+W and Cmd+Q. Verify the
  save/discard/cancel result and clean process teardown separately. A save or
  worker shutdown acknowledgement does not prove AppKit teardown.
- Check pan, diagonal pan, orbit, pinch and modifier changes during momentum
  against the existing input contract. Check cursor anchoring and absence of
  unintended whole-window magnification.
- Check mouse hot-plug and any supported explicit device override. Record
  unsupported behavior as a gap rather than assuming a legacy implementation.
- Check sleep/wake and GPU/display changes: verify camera and shading retention,
  or the documented controlled unavailable state with document recovery intact.

## Autosave and crash recovery

Use disposable QA documents only. Save the original fixture before each crash
scenario; keep the isolated profile and logs for inspection.

1. Open a saved document, make no changes, terminate the host abruptly and
   relaunch. Verify that no recovery offer appears for untouched work.
2. Edit continuously for 3 minutes without pauses longer than 30 s, terminate
   abruptly and relaunch. Check the recovery offer and inspect its last edit.
3. Restore that offer, terminate again while unsaved and relaunch. Verify that
   recovery remains available.
4. After a crash with unsaved edits to a saved project, check recovery discovery
   and the Recent path. Verify that opening a saved version explicitly discards
   the intended recovery copy and does not discard other protected offers.
5. Verify recovered document name, timestamp and identity. Save and reopen the
   recovered document; inspect geometry and persisted edits.

Do not assume every scenario passes. Required application regressions, current
native recovery behavior and physical lifecycle acceptance are distinct evidence.

## Automated prerequisites and packaging

Run the affected Rust workspace, CTest, xtask and strict native CI lanes using
their current repository commands. Required worker and GPU lanes use
`ONECAD_REQUIRE_WORKER=1` and `ONECAD_REQUIRE_GPU=1` respectively. Missing
prerequisites or skipped required tests do not qualify a lane. Preserve explicit
test inventories, portable artifact names, manifests, relative paths, hashes and
negative controls.

See [packaging](../PACKAGING.md) for local bundle, source/relink materials,
signing and notarization procedures. Validate the freshly packaged application,
worker identity, required assets and self-tests independently of development
startup. Notarization requires its actual credentials and evidence; an ad-hoc
local bundle does not establish distribution acceptance. Cleanup does not close
ONECAD-269 or establish native product/release acceptance.

## Reliability acceptance and hardware handoff

This procedure qualifies save, crash recovery and reopen on macOS and iPadOS.
It is a runbook, not a result record. Keep outcomes and remaining work in the
canonical Plane QA Page. Shared Rust checks, native Mac/Metal checks, iPad
simulator checks and physical iPad memory/jetsam checks are separate lanes. A
lane without its required machine is **not run**; Linux or Simulator success
cannot qualify native notification delivery, physical memory use or jetsam.

### Shared checks on the exact source tree

Finish source edits before starting. Use the repository toolchain and the
freshly built/staged pinned production worker; set `ONECAD_WORKER_PATH` to its
absolute path. Build addons as in `.github/workflows/ci.yml` before the full
workspace suite. Do not substitute a stub, change a fingerprint, or remove a
required flag to turn a missing prerequisite into a pass.

The following Bash block uses existing Cargo/xtask gates. It records all tracked
and nonignored untracked files, including new Rust modules; a commit SHA alone
cannot identify an uncommitted tree. Evidence lives outside the checkout. Keep
these manifests, diff, commands, toolchain, worker hash/manifest and logs together.
Ignored generated inputs (the worker, embedded kernel bundles and addons) need
their own build logs and hashes; they are not covered by the source manifest.

```sh
set -euo pipefail
: "${ONECAD_WORKER_PATH:?set an absolute production worker path}"
test -x "$ONECAD_WORKER_PATH"
export ONECAD_REQUIRE_WORKER=1
QA_EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/onecad-reliability.XXXXXX")"
export QA_EVIDENCE
cat > "$QA_EVIDENCE/source-snapshot.py" <<'PY'
import hashlib, json, os, pathlib, stat, subprocess, sys

def git(*args):
    return subprocess.check_output(["git", *args])

paths = set(git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0"))
files = []
for raw in sorted(paths - {b""}):
    path = pathlib.Path(os.fsdecode(raw))
    entry = {"path": os.fsdecode(raw)}
    try:
        mode = path.lstat().st_mode
        entry["mode"] = stat.S_IMODE(mode)
        if stat.S_ISLNK(mode):
            entry["symlink"] = os.readlink(path)
        elif stat.S_ISREG(mode):
            entry["sha256"] = hashlib.file_digest(path.open("rb"), "sha256").hexdigest()
        else:
            raise RuntimeError(f"unexpected source type: {path}")
    except FileNotFoundError:
        entry["deleted"] = True
    files.append(entry)
json.dump({"head": git("rev-parse", "HEAD").decode().strip(),
           "status": git("status", "--porcelain=v1", "--untracked-files=all").decode(),
           "files": files}, sys.stdout, sort_keys=True, indent=2)
PY
python3 "$QA_EVIDENCE/source-snapshot.py" > "$QA_EVIDENCE/source-before.json"
git diff --binary HEAD > "$QA_EVIDENCE/source.diff"
record() {
  local label="$1"
  shift
  printf '%q ' "$@" >> "$QA_EVIDENCE/commands.txt"
  printf '\n' >> "$QA_EVIDENCE/commands.txt"
  "$@" 2>&1 | tee "$QA_EVIDENCE/$label.log"
}
record rustc rustc -Vv
record cargo cargo -V
record worker "$ONECAD_WORKER_PATH" --selftest
python3 - "$ONECAD_WORKER_PATH" <<'PY' > "$QA_EVIDENCE/worker-sha256.txt"
import hashlib, pathlib, sys
p = pathlib.Path(sys.argv[1])
print(hashlib.file_digest(p.open("rb"), "sha256").hexdigest(), p)
PY
cp packaging/binaries/onecad-worker-manifest.json "$QA_EVIDENCE/worker-manifest.json"
record fmt cargo fmt --all --check
record clippy cargo clippy --workspace --all-targets -- -D warnings
record stub cargo build -p onecad-worker-stub
record persistence cargo test -p onecad-app --test autosave_recovery \
  --test recovery_hardening --test persistence_lane -- --nocapture
record app cargo test -p onecad-app --lib -- --nocapture
record ui cargo test -p onecad-ui --lib -- --nocapture
record document-paths cargo test -p onecad-app --lib document_paths -- --nocapture
record recents-relocation cargo test -p onecad-app --lib recents::relocation_tests -- --nocapture
record recovery-relocation cargo test -p onecad-ui --lib startup_recovery::tests::relocation -- --nocapture
record ios cargo test -p onecad-ios --lib -- --nocapture
record tooling cargo test -p xtask
record layers cargo xtask check-rust-layers
record coverage cargo xtask verify-modeling-coverage
record contracts cargo xtask verify-modeling-contracts
record hygiene cargo xtask check-hygiene
record workspace cargo test --workspace -- --nocapture
python3 "$QA_EVIDENCE/source-snapshot.py" > "$QA_EVIDENCE/source-after.json"
cmp "$QA_EVIDENCE/source-before.json" "$QA_EVIDENCE/source-after.json"
printf 'Shared check logs: %s\n' "$QA_EVIDENCE"
```

Python 3.11 or later supplies `hashlib.file_digest`. A command failure or changed
manifest invalidates the combined run; after fixing source, rerun the affected
checks and the final combined gate on the finished tree. Preserve failed logs.
The stub build supplies the explicit protocol/chaos test fixture; the production
worker path remains unchanged. Review test summaries and `skip:` diagnostics: zero selected tests or skipped
required worker/reliability tests fail acceptance. Ignored Metal/host tests are
not executed by the shared suite and remain required in their own lane. Optional
embedded-kernel tests without a linked bundle must be reported as not run;
they cannot qualify the iPad kernel. The explicit persistence invocation must
execute every nonignored test in all three named suites with no skips. Keep the
application save-ownership, recovery, reservation, UI budget and iOS lifecycle
unit-test names/results from the same run; a green test filter alone is not
proof that an intended case executed.

### Fresh fixtures and semantic checks

Use a new profile and new disposable documents for each scenario, keeping the
same profile only where the steps explicitly require relaunch/recovery. On Mac,
generate three independently authored fixtures (three file copies would retain
one document UUID):

```sh
for fixture in A B C; do
  ONECAD_NATIVE_PLATE_FIXTURE="$QA_EVIDENCE/$fixture.onecad" \
    cargo test -p onecad-app --test native_plate_workflow \
    synthetic_plate_native_workflow -- --exact --nocapture \
    2>&1 | tee "$QA_EVIDENCE/fixture-$fixture.log"
done
make run QA_ROOT="$QA_EVIDENCE/mac-profile" DOCUMENT="$QA_EVIDENCE/A.onecad"
```

The generated fixture is a constrained 80×50 mm **profile**, with no solid yet;
extrude it to 6 mm, then edit the extrusion to the scenario's recorded depth.
On iPad, use New part three times and create the reference constrained
60×40×4 mm plate with a Ø6 mm through hole using the existing
[workflow procedure](../ios/TESTING.md#required-workflow-requiredworkflowtests). Name each with
the run ID and A/B/C suffix. Each must have its own UUID and working file.
Uninstall the app only before the first launch of a fresh simulator run, never
between the crash and recovery steps. Preserve the physical app container before
any device reset or reinstall.

Before a fault, record UUID, working path, saved revision, dirty state, feature
order/IDs, sketch definitions/constraints, dimensions, body count, camera and
selection for every open document. Distinguish committed edits from an unaccepted
preview. After Restore and after Save/reopen, compare those same semantics with
the expected checkpoint, including import-blob identities where used. A title,
a successful future, a timestamp or a nonempty file alone does not prove the edit
survived. `document.json` in the `.onecad` container is authoritative; retain it
along with `manifest.json` for comparison, and reopen through the application to
exercise its hash/identity validation and regeneration. The simulator gate's
existing geometry oracle checks its declared artifacts against
`fixtures/ios/acceptance/expectations.json`; retain those reports rather than
claiming arbitrary manual files received the same oracle check. For a modified
reference plate, independently verify the new dimensions, live regenerated
geometry, healthy solid and expected volume `width * height * depth - pi * 9 * depth`.

### Mac native host and Metal

On a Mac with actual Metal, run the existing strict
[native-host gate](../native-host-ci-gate.md) and renderer lane in
`.github/workflows/native-renderer.yml` with `ONECAD_REQUIRE_WORKER=1` and
`ONECAD_REQUIRE_GPU=1`. Use fresh output directories. The authoritative host and
service lists are `xtask/src/evidence/host_names.rs`; new ignored delivery cases
must be added there. Capture verification failures and prerequisite negative
controls. Also execute the scenarios below with the real native host. Eframe
test frames and direct `relieve_memory_pressure` calls cannot establish OS
notification delivery or native window teardown.

To observe the Mac route, record `sw_vers`, hardware/RAM, adapter, process ID,
actual pressure command and its exit/output. After checking the installed
`man memory_pressure`, use its supported warning/critical simulation, commonly
`/usr/bin/memory_pressure -S -l warn` and `-S -l critical`. This is an OS-delivered
synthetic notification, not evidence of actual memory exhaustion. The host log
must contain `macOS memory-pressure notification released display caches` with
`released_bytes`
and `lowered_scale`; absence of the callback is a failure of this scenario,
even if an explicit relief call works. Repeat while inactive/parked scenes and a
pending scene delivery exist, return to every tab, then close the app and confirm
clean teardown. Run the native source lifecycle check too:

```sh
cargo test -p onecad-ui --lib memory_pressure -- --nocapture
```

This must include
`memory_pressure::native::tests::native_source_registers_and_releases_callback_after_cancel_without_a_gpu`
on macOS. The four portable mailbox tests cover signal/coalescing behavior; the
native case covers real source registration/cancellation and callback-context
release, without claiming that an OS pressure notification was delivered.

### iPad simulator

On a Mac with Xcode, run the existing strict gate from a clean checkout of the
reviewed tree (the gate deliberately rejects dirty trees):

```sh
ONECAD_REQUIRE_WORKER=1 \
  cargo xtask ios-simulator-gate /private/tmp/onecad-reliability-simulator-NEW
cargo xtask ios-simulator-gate --verify-only \
  /private/tmp/onecad-reliability-simulator-NEW
```

Follow [SIMULATOR_GATE.md](../ios/SIMULATOR_GATE.md) for the worker, run ID,
embedded-kernel, XCTest, artifact and Mac-oracle prerequisites. The checked
inventory is `platforms/ios/acceptance-inventory.json`; add new tests there instead
of treating an unselected XCTest as acceptance. Every expected test must run
once, with no unapproved skips. Preserve `.xcresult`, run IDs, container artifacts,
semantic JSON, provenance sidecars and `acceptance.json`. XCUITest `terminate()`
is process termination; it is neither a real crash nor jetsam. Simulator memory
warnings exercise routing; they do not establish physical-device memory bounds.
Run the remaining manual scenarios below separately if they have no inventoried
XCTest, and label their evidence accordingly.

### Required save/recovery/pressure scenarios

Run each applicable case on both native Mac and iPad, including the fresh-process
reopen. Use test-only held writes/admission and fault controls for deterministic
shared tests; use actual UI/OS observations in the native lanes. A fault that
cannot be induced on the available machine is not run.

| Scenario | Required observation and failure criteria |
|---|---|
| Clean kill | Open a saved fixture, make no edits, terminate/relaunch: no recovery offer. Any fabricated dirty offer fails. |
| Continuous edits | Edit for 3 minutes, with no pause longer than 30 s; confirm a durable checkpoint outcome, then terminate/relaunch. Restore includes the last **completed** checkpoint's committed edits; pending/expired/cancelled background waiting alone is no durability claim. Missing or regressed checkpoint content fails. |
| Restore, second crash | Restore the unsaved checkpoint, do not Save, terminate/relaunch again. The same committed content and identity remain recoverable; Save then fresh-process reopen preserves them. |
| Three independently dirty documents | Give A/B/C different dimensions/edits, history, selection and cameras. Flush/background, terminate/relaunch, restore each offer without overwriting the others (relaunch as needed for deferred offers). Save/reopen each and compare its own UUID/content. One document's save/discard/close must leave the other two protected. |
| Save caller cancelled/closed/switched | Hold a save after admission, cancel/drop its caller or close/replace/switch its originating document, then let writing finish. The admitted transaction retains its lane and path/identity ownership until disk work ends; it writes only its original snapshot and never clears a newer runtime's dirty/recovery state. A later save succeeds. Cancellation before admission must write nothing. Save As into the reserved recovery directory, including path aliases, must fail before writing; an alias of another open document must remain reserved. A UI that refuses a busy switch is not evidence for cancellation after admission; the deterministic shared regression must execute too. |
| Open racing shutdown | Hold an open before runtime publication, complete host shutdown, then release the open. It must be refused without publishing a runtime, worker or claims into the closed host. |
| Autosave caller cancelled | Cancel a queued background request before admission: no write. Cancel waiting after admission: its owned transaction completes with a coherent marker/container pair and retained memory lease; a later request cannot race cleanup/rename. Inspect checkpoint content after the second request, not just returned status. |
| Interrupted/failed overwrite | Preserve the saved file's SHA-256, inject write/rename/marker failure, and verify the prior file remains readable and byte-identical, recovery remains coherent and the document stays dirty. Remove the fault, Save and reopen successfully. Retained rollback backups are evidence requiring reconciliation, not permission to delete protected data. |
| Pressure with pending and adopted scenes | While A has an unconsumed pending display reply and B/C have adopted caches, deliver pressure, then activate all tabs. Evict disposable thumbnails/preparation caches first; no stale reply may resurrect evicted caches or permanently blank the viewport. Geometry regenerates and dirty/history/selection/camera, import bytes, saved file and recovery remain intact. Observe both the released-byte result and eventual semantic content. |
| Bounded refusal | Hold enough owned work to exhaust admission, request more capture/preview work and observe the documented bounded refusal (current CPU admission deadline 3 s; allow scheduling overhead and record actual elapsed time). No infinite repaint/retry loop or second accepted CAD edit. Release pressure/work, then Save and reopen the committed document successfully; display refusal must not disable persistence. |
| Recovery-path relocation | Exercise a saved app-container path after its container root changes, plus a legacy Inbox recent/recovery entry, including UUID collision and an unavailable saved source. Restore loads checkpoint semantics, and only a validated matching working copy can replace the saved reference. A missing legacy UUID or a missing/corrupt/mismatched candidate retains its unresolved reference; do not guess identity from a filename. Recent may persist a validated container rebase during listing; Inbox migration updates it after successful open/recovery. Restore of a valid checkpoint with a corrupt/conflicting saved copy uses a fresh claimed destination and leaves the original untouched; Open saved retains strict identity validation. Open saved/Discard affects only the chosen offer; failed adoption preserves retryable recovery and other offers. |

### Physical iPad memory and jetsam

Use a real supported iPad with the Release-equivalent build and record model,
RAM, iPadOS version, build configuration/signing, app/kernel identity and fixture
hashes. A simulator, injected ABI `MEMORY_WARNING`, debugger memory-warning menu,
background task expiry or `terminate()` cannot satisfy jetsam acceptance.

1. Run the three-document scenario with progressively larger disposable geometry
   and import blobs. Record workload sizes, resident/physical footprint and high
   water over time using Instruments/Xcode or the device's diagnostics. Keep
   renderer/CPU admission snapshots, real UIKit memory-warning timestamps and
   the `lifecycle: memory warning released display caches` results. Partial
   ledgers and reservation limits are not measured RSS or a process-memory cap.
2. Observe a genuine device memory warning under a recorded workload. Confirm
   cache release before new allocations, document preservation and eventual
   redraw. Repeat with pending display and persistence work. A genuine warning
   that never reaches the callback, corrupts content or loses Save usability fails.
3. After a confirmed checkpoint, obtain an actual OS memory-pressure termination
   using the disposable workload, then export the matching device jetsam report
   (process, timestamp, reason and footprint). Relaunch without clearing the
   container, Restore, Save and fresh-process reopen every protected document.
   Compare checkpoint semantics and record any edits accepted after the last
   completed checkpoint separately. No jetsam report means jetsam was not run.
4. Export the app container, native logs, checkpoint/saved files, semantic
   comparisons, device trace and jetsam report under the same run ID. Hash the
   artifacts and record their source device/container paths and acquisition
   times. Keep failed artifacts. Mark each scenario pass/fail/not run with the
   exact command or UI action and expected versus observed content.

Retain source manifests and final-tree comparison alongside every lane's native
OS/toolchain, worker/kernel identities, commands, exit codes, test inventories,
fixture hashes and artifacts. Each lane's claim ends at what it actually ran;
shared checks plus a prepared handoff leave the unrun hardware lanes open.
