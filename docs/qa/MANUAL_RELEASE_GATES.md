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
ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" scripts/build-worker.sh Release
make run QA_ROOT=/private/tmp/onecad-release-check
```

`make run` builds its worker under `worker/build-native` by default and generates
a constrained-plate QA fixture when needed. See the [native host instructions](../../crates/onecad-ui/README.md)
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
