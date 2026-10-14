# Isolated native modeling acceptance on macOS

Use this procedure for the local ONECAD modeling milestone. Results and remaining
acceptance obligations belong in the existing Plane tasks. This procedure is not
a claim that native acceptance passed.

## Build and isolation

Record `git rev-parse HEAD`, `git status --short`, macOS/toolchain versions and
the staged worker manifest. Coordinate builds with the implementation owner.

```sh
ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" scripts/build-worker.sh Release
bun run tauri build --debug --bundles app --config src-tauri/tauri.native-qa.conf.json
```

The resulting app is `src-tauri/target/debug/bundle/macos/OneCAD QA.app`.
The configuration uses bundle identifier `com.andrejvysny.onecad.nativeqa`:
app data, autosaves, recent-file history and the default library live under
`~/Library/Application Support/com.andrejvysny.onecad.nativeqa`.
For this bundle identifier, app initialization sets an explicit WKWebView
`data_store_identifier` to separate frontend storage. It is assigned after
`generate_context!`: tauri-utils 2.9.3 incorrectly generates a `Vec<u8>` for
the configuration field's `[u8; 16]` type. macOS does not support `dataDirectory`.
Inspect these locations after launch to confirm isolation on the actual build.
The QA Info.plist supplies `ONECAD_LOG_DIR=/tmp/onecad-native-qa/logs` for
LaunchServices launches, separating the session log from the checkout's
`logs/dev.jsonl`. Verify the environment and actual log destination after launch.
Archive the QA log before restarting: the app truncates its log on every start.
Reuse of this configuration retains previous QA data; it does not imply a fresh
profile. Record retained state or use a new identifier and data-store UUID for a
fresh qualification run. Never clear the normal application's storage.

## Foreground execution

Announce the foreground interval before taking mouse/keyboard input. Verify
Accessibility and Screen Recording permissions and identify the QA app's process
and window. Do not attach to an original document or stop unrelated processes.
Use new QA documents saved to a dedicated temporary directory; copy supplied
fixtures before opening them. Record hashes of any original fixtures and verify
they remain unchanged. Stop input on focus/ownership loss and release held keys.

Draw/constrain/change a rectangle, drag and release a corner, finish its selected
profile, Extrude, edit upstream dimensions, undo/redo, save, quit, reopen and edit
again. Record dimensions and authoritative geometry as well as visible status.
Retain exact input values, selections, timestamps, screenshots, bounded logs,
source revision, app/worker hashes and the action journal.

Automated frontend tests, real-worker integration tests, accessibility actions
and foreground pointer/keyboard acceptance are separate evidence classes.
Permission denial, unavailable capture and skipped checks are blockers, not
passes. Stop only test-owned processes when the session ends; retain QA files
and evidence for review.
