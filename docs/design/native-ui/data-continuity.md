# Native UI migration: user-data continuity spec (P0.5)

Status: Phase 0 spec, written 2026-10-17 against `d76857c2`. Plane: ONECAD-260 (program ONECAD-255).
Scope: **macOS desktop app only.** The iPad target is out of scope by user direction (2026-10-17) and nothing here specifies iPad behavior. Covered: what an existing Tauri-app user has on disk, what the native (Rust UI + wgpu) app must read and keep byte-compatible, the Phase 1b settings write-through, the native first-launch migration, and the tests that prove it. Open decisions are tagged **[OPEN-n]**.

All `file:line` citations are against the working tree at the revision above. Observations marked *(observed)* came from inspecting this developer machine's real data read-only. For SQLite, copies were made into a scratch dir and queried there.

Path shorthand: a bare `lib.rs`, `autosave.rs`, `recents.rs`, `library.rs` or `api/mod.rs` means the file under `src-tauri/src/`. Crate paths such as `tauri-2.11.5/…`, `tauri-utils-2.9.3/…`, `dirs-6.0.0/…` and `dirs-sys-0.5.0/…` are under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`; those versions match `src-tauri/Cargo.lock`.

## 1. Inventory: what exists today

### 1.1 Bundle identifier

- `src-tauri/tauri.conf.json:5` — `"identifier": "com.andrejvysny.onecad"` (`productName` `onecad` at :3, version `0.1.0` at :4).
- QA variant: `src-tauri/tauri.native-qa.conf.json:3` — `com.andrejvysny.onecad.nativeqa`, plus an explicit WKWebView `data_store_identifier` set in `src-tauri/src/lib.rs:391-397` (rationale in `docs/qa/native-modeling-acceptance.md:18-25`). Its data is deliberately isolated and is **not** a migration source.
- The `"identifier": "tauri-e2e"` in `src-tauri/tauri.e2e.conf.json:11` and `tauri.agent.conf.json:12` is a capability id, not a bundle id.

### 1.2 How paths resolve

Tauri 2.11.5's `PathResolver` (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-2.11.5/src/path/desktop.rs:238-251`):

- `app_config_dir() = dirs::config_dir()/<identifier>`
- `app_data_dir() = dirs::data_dir()/<identifier>`

On macOS, `dirs` 6.0.0 (`dirs-6.0.0/src/mac.rs:7,10,12`; selected at `lib.rs:24,26`) maps **both** `config_dir` and `data_dir` to `~/Library/Application Support`. So on Mac every Rust-owned file lives in one directory:

```
~/Library/Application Support/com.andrejvysny.onecad/
```

*(observed)* That directory holds `autosave/`, `library/` and `recents.json`, which confirms the config and data dirs coincide.

Note: Tauri takes the identifier from config, not from the running bundle. So `tauri dev` (an unbundled `target/debug/onecad`) uses the **same** app-data dir as the installed app, but a **different** WebKit store (see 1.6).

### 1.3 Every `app.path()` / app-dir use (grep, non-test)

| Site | Dir API | What |
|---|---|---|
| `src-tauri/src/autosave.rs:115-117` (`autosave_root`) | `app_data_dir()` | Root for autosave and recovery |
| `src-tauri/src/library.rs:72-79` (`library_root`) | `ONECAD_LIBRARY_ROOT` env, else `app_data_dir()/library` | Component library |
| `src-tauri/src/recents.rs:51-56` (`recents_file`) | `app_config_dir()/recents.json` | Start-screen recents |

Callers: `lib.rs:419` (blur flush), `lib.rs:482` (library seed and reindex at setup), `lib.rs:515` (autosave driver), `api/mod.rs:960-972` (`check_recovery`), `api/mod.rs:4140` (`list_recents`). Logging is **not** app-data: `logging.rs:62-71` uses `ONECAD_LOG_DIR`, else `<repo>/logs` in debug and none in release. Nothing to migrate there.

### 1.4 What lives under the app-data root

**Autosave and recovery.** The layout is owned by `src-tauri/crates/onecad-core/src/io/recovery.rs:8-9,70-86`:

```
autosave/<documentId>.onecad          autosave container (CONTAINER_EXT, :66)
autosave/<documentId>.session.json    SessionMarker, pretty JSON (:142-146)
```

- `SessionMarker` (`recovery.rs:91-114`, camelCase) has these fields:
  - `documentId`
  - `pid`
  - `openedPath?` (skipped when absent)
  - `lastAutosave` (RFC3339; skipped when empty)
  - `title?`
- `scan_recoverable` (`:192`) is keyed to containers. A marker whose pid is live means skip. A dead pid or a missing marker means offer. Anything older than `ORPHAN_TTL` = 30 days (`:63`) is swept.
- Scheduling lives in the app layer: `AUTOSAVE_DEBOUNCE` 30 s (`autosave.rs:78`), `AUTOSAVE_MAX_AGE` = `AUTOSAVE_INTERVAL_SECS` 120 s (`recovery.rs:53`), the dirty gate, and the blur flush (`lib.rs:412-431`).

**Recents.** `recents.json` is a JSON array, newest first, capped at `MAX_RECENTS = 10` (`recents.rs:35`). Each element is `{path, name, lastOpenedMs}` (`recents.rs:38-47`, camelCase). Writes go through `onecad_core::io::durable_write` (`crates/onecad-core/src/io/mod.rs:204`: tmp, fsync, rename, parent fsync). A corrupt file is quarantined to `recents.json.corrupt-<secs>[-n]` (`recents.rs:84-105`).

**Library.** `library/` is the root of `onecad-library`. It contains:

- `library.json` index (`crates/onecad-library/src/index.rs:13`)
- `blobs/` (`blob.rs:16`)
- `templates/` (`template.rs:27`)
- per-package dirs containing `component.toml` (`package.rs:19`)
- the seeder's `.seed-version` and `.seed-ledger.json` (`src-tauri/src/library_seed.rs:69,73`)

The seeder's rule is that the user's copy always wins (`library_seed.rs:19-25`). *(observed)* All of these are present on this machine.

### 1.5 Settings: `src/stores/settingsStore.ts`

Settings are the only user state outside Rust's reach: zustand `persist` with the default JSON storage, i.e. `localStorage`.

- Key `onecad.settings` (`:207`), `version: 11` (`:305`). There is no `partialize`, so the whole state is persisted except the functions. The envelope is `{"state":{…},"version":11}`.
- Schema v11 (`:144-205`; defaults at `:218-237`, `:263-275`):

| Field | Type | Default |
|---|---|---|
| `snapTo.grid` | bool | true |
| `snapTo.sketchGuideLines` | bool | true |
| `snapTo.sketchGuidePoints` | bool | true |
| `snapTo.quadrant` | bool | true |
| `snapTo.intersection` | bool | true |
| `snapTo.onCurve` | bool | true |
| `snapTo.dimensionRound` | bool | true |
| `snapTo.polarTracking` | bool | true |
| `snapTo.guidePoints3d` | bool (reserved, unread) | true |
| `snapTo.distantEdges` | bool (reserved, unread) | false |
| `show.guidePoints` | bool | true |
| `show.snappingHints` | bool | true |
| `show.liveDimensions` | bool | true |
| `show.constraintChips` | bool | true |
| `show.coincidentBadges` | bool | false |
| `experimentalWebGpu` | bool | false |
| `navigation.inputDevice` | `"auto"\|"mouse"\|"trackpad"` (`viewport/engine/navInput.ts:25`) | `"auto"` |
| `displayMode` | `"shaded"\|"shadedEdges"\|"wireframe"\|"assemblyColors"` (`viewport/engine/renderModes.ts:58`) | `"shadedEdges"` (`:125`) |
| `theme` | `"light"\|"dark"\|"system"` (`theme/themes.ts:19`) | `"system"` (`:43`) |
| `displayUnit` | `"mm"\|"cm"\|"m"\|"in"` (`units/lengthUnits.ts:18`) | `"mm"` (`:60`) |
| `snapRadius` | `"s"\|"m"\|"l"` = 5/8/12 px (`tools/sketch/snapRadius.ts:18,25`) | `"m"` (`:38`) |
| `autoConstrainMode` | `"off"\|"minimal"\|"standard"` (`settingsStore.ts:36`) | `"standard"` (`:49`) |

**Migrations** (`:331-377`):

| Step | Change |
|---|---|
| v1→2 | Backfills quadrant, intersection and onCurve as true |
| v2→3 | Adds navigation |
| v3→4 | Adds displayMode |
| v4→5 | Adds theme |
| v5→6 | Adds displayUnit |
| v6→7 | Adds dimensionRound and liveDimensions as true |
| v7→8 | Adds polarTracking true and snapRadius |
| v8→9 | Adds constraintChips true |
| v9→10 | Adds coincidentBadges **false** |
| v10→11 | Adds autoConstrainMode |

Every backfilled value equals the current default, and each step uses `{default, ...existing}` so it never overrides an existing key.

**Merge** (`:389-404`) runs on every hydration:

- It deep-merges `snapTo` and `show` per key against the defaults (`mergeBooleanRecord`, `:252-261`). A non-boolean falls back to the default, and an explicit `false` survives.
- It coerces `displayMode`, `theme`, `displayUnit`, `snapRadius` and `autoConstrainMode`.
- It does **not** coerce `navigation.inputDevice` (`:394-397`, plain spread).
- Unknown top-level keys survive, because `{...current, ...blob}` writes them back.

Consequence: **migrate followed by merge is the same as "coerce every field against defaults"** for any input version. The Rust side does not need to replay migration steps. It needs one coerce function plus a version stamp.

### 1.6 The no-FOUC theme script

In `index.html:15-34`, the inline script runs before the module bundle:

- It does `JSON.parse(localStorage.getItem("onecad.settings")).state.theme` (`:23-24`).
- A value other than light or dark becomes `matchMedia("(prefers-color-scheme: dark)")` (`:25-27`).
- It stamps `<html data-theme>` and `style.colorScheme` (`:28-29`). On any exception it falls back to `light` (`:31`).
- The viewport reads `dataset.theme` (`src/viewport/engine/ViewportEngine.ts:236`, `palette.ts:166`).

It duplicates `STORAGE_KEY` and the resolve rule on purpose (`index.html:18-20`; `themes.ts:59` `resolveTheme`).

### 1.7 WKWebView localStorage on disk (macOS)

The Tauri app uses the default `WKWebsiteDataStore`. On macOS, WebKit stores it under `~/Library/WebKit/<CFBundleIdentifier or process name>/WebsiteData/`. Tauri's `dataDirectory` is unsupported on macOS, and `data_store_identifier` requires macOS 14+ (`tauri-utils-2.9.3/src/config.rs:2239-2242`; `docs/qa/native-modeling-acceptance.md:20-25`).

*(observed)* layout:

```
~/Library/WebKit/com.andrejvysny.onecad/WebsiteData/
  Default/salt                                   8-byte salt
  Default/<H(top)>/<H(client)>/origin            serialized origin pair
  Default/<H(top)>/<H(client)>/LocalStorage/localstorage.sqlite3 (+ -wal, -shm)
  LocalStorage/                                  legacy per-origin dir (empty here)
```

How WebKit builds it:

- The directory names are base64url(SHA-256(origin string ‖ salt)), computed separately for the top origin and the client origin, with the salt read from `Default/salt`. Source: WebKit `Source/WebKit/NetworkProcess/storage/NetworkStorageManager.cpp`, `encode()` / `originDirectoryPath()` / the salt in its constructor.
- The file is `<root>/LocalStorage/localstorage.sqlite3`. Source: `OriginStorageManager.cpp` `resolvedLocalStoragePath()` and `LocalStorageManager.cpp` `s_fileName = "localstorage.sqlite3"`.
- The older layout used `<databaseIdentifier>.localstorage` (`LocalStorageManager.cpp` `originToFileName`).
- Links: https://github.com/WebKit/WebKit/blob/main/Source/WebKit/NetworkProcess/storage/NetworkStorageManager.cpp , https://github.com/WebKit/WebKit/blob/main/Source/WebKit/NetworkProcess/storage/LocalStorageManager.cpp .

*(observed)* specifics this spec relies on:

- **Schema.** `CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB NOT NULL ON CONFLICT FAIL);`
- **Value encoding.** The value is **UTF-16LE** without a BOM: `hex(substr(value,1,8)) = 7B00220073007400`, which is `{"st`. The 1050-byte blob decodes with `iconv -f UTF-16LE` to the v11 envelope (`"theme":"light"`).
- **`origin` file format** (little-endian):
  - u32 length, u8 flag (1 = 8-bit), then the bytes of the scheme
  - the same encoding for the host
  - u8 has-port, then a u16 port when present
  - the whole thing repeated for the client origin
- **Production origin.** It is `tauri://localhost` with no port (bytes `05000000 01 "tauri" 09000000 01 "localhost" 00`, twice).
- **Dev store.** `tauri dev` runs unbundled, so its store is under `~/Library/WebKit/onecad/` with origin `http://localhost:1420` (port bytes `01 8c05`). That is separate settings from the installed app.

The hash dir names cannot be predicted without the salt. An importer must scan `Default/*/*/origin` and match the origin, not compute a path.

## 2. What the native app must keep identical

1. **Bundle id `com.andrejvysny.onecad`** for the release Mac app (`CFBundleIdentifier` in the native Info.plist), with the same value baked in as a Rust const. The native app is not Tauri. It must resolve its root as `dirs::data_dir()/com.andrejvysny.onecad`, not from `NSBundle`, so that unbundled dev and test runs agree with Tauri's rule (1.2). An env override `ONECAD_APP_DATA_DIR` follows the `ONECAD_LIBRARY_ROOT` precedent at `library.rs:72-78`. It is required for the tests in section 7, and for QA isolation.
2. **Paths, unchanged:**
   - `autosave/` with the `onecad-core` layout
   - `library/` (with the `ONECAD_LIBRARY_ROOT` override)
   - `recents.json`
   - new: `settings.json`

   All of these sit directly under the root above.
3. **Formats, unchanged.** Same structs (moved per plan into `onecad-app`), no re-serialization on read:
   - `.onecad` container and `SessionMarker` JSON via the same `onecad-core` crate
   - `recents.json` via the same `RecentEntry` serde (camelCase), cap, newest-first order, quarantine behavior and `durable_write`
   - library tree via `onecad-library` plus `library_seed`, with the same `SEED_VERSION` semantics
4. **Recovery semantics, unchanged.** The native app calls `scan_recoverable(root, pid_alive, now)` and `write_marker` / `remove_marker` from the same module, uses the same debounce and max-age constants, the same dirty gate and the same blur flush (the `HostLifecycle` port, plan Phase 1b). Pid liveness makes concurrent Tauri and native runs safe: each skips containers the other's live pid owns.
5. **Settings semantics.** It must keep the v11 field set, the defaults, the coercion rules (1.5) and the theme resolve rule (`themes.ts:59`) exactly.

## 3. Phase 1b: settings write-through (Tauri app, before any native UI)

Goal: by the time the native app ships, every active user's settings already exist in a Rust-owned file, and the Rust module owns schema and versioning.

- **Rust module `settings`** (moves with the engine to `onecad-app`):
  - `struct SettingsV11` mirrors the table in 1.5 (serde camelCase, nested `snapTo`, `show` and `navigation`), plus `#[serde(flatten)] extra: Map<String, Value>` so that unknown top-level keys survive a round trip, matching the TS merge. **[OPEN-1]**: keep `extra`, or drop unknown keys.
  - The on-disk envelope is identical to zustand's, `{"state": SettingsV11, "version": 11}`. One parser then serves both the Rust file and the localStorage import.
  - `fn coerce(raw: serde_json::Value) -> SettingsV11` handles input of any version: each field gets its value if it is valid and the default otherwise. Booleans are taken only when they are JSON booleans, never parsed from strings. This is a superset of TS migrate followed by merge (1.5).
  - `CURRENT_VERSION = 11`. A future schema change bumps it in Rust first, and the TS `version` follows.
  - `navigation.inputDevice` is coerced to `"auto"` when it is not valid. TS does not do this, so it is a tightening. **[OPEN-2]**: confirm, or patch TS to match.
- **Path:** `<app_config_dir>/settings.json`, which is the same directory as `recents.json` on Mac. It is written with `onecad_core::io::durable_write`. A corrupt file is quarantined as `settings.json.corrupt-<secs>[-n]`, reusing the `recents::quarantine_corrupt` pattern.
- **Commands** (registered next to `list_recents`):
  - `write_settings(blob: serde_json::Value) -> Result<(), ApiError>`. It runs `coerce`, writes the envelope, and is idempotent. A Rust write failure is logged, and the command returns `ApiError::Io`, which React logs but never surfaces as fatal.
  - `read_settings() -> Option<SettingsEnvelope>`. It exists for reconciliation (below) and for tests.
- **React side:**
  - A new `src/stores/settingsSync.ts` (owned by Phase 1b, not this task) subscribes to `settingsStore`. It debounces (~250 ms), then calls `invoke("write_settings", { blob: { state: <persisted subset>, version: 11 } })`.
  - The persisted subset is exactly what zustand would write: `JSON.parse(JSON.stringify(state))`, with the functions dropped.
  - It also writes once right after hydration, which seeds the file for users who never change a setting.
  - localStorage persistence **stays**, because `index.html:23` still reads it.
- **Authority while both exist** **[OPEN-3]**:
  - Option A (simple, recommended for 1b): localStorage stays authoritative for the Tauri app, and the Rust file is a mirror. The cost is that a change made in the native app during dual-run is not seen by the Tauri app.
  - Option B: at Tauri startup, `read_settings` and, if it differs, rehydrate the store and rewrite localStorage. Settings then flow both ways, but this adds an async rehydrate and one possible frame of stale non-theme settings.

  Decide before Phase 3 dual-run starts.
- **Exit flush** **[OPEN-4]**: a debounced write can be pending when `confirm_exit` runs. Either flush synchronously in the exit path, or accept that up to 250 ms of setting changes can be lost. Recommendation: have the React exit handler flush before it calls `confirm_exit`.

## 4. Native app first launch: settings resolution

This runs synchronously in `main`, before the event loop and the window exist (the file is about 1 KB):

1. Read `<root>/settings.json`. If it exists and parses, use `coerce(state)` and stop. If it exists but does not parse, quarantine it and continue to step 2.
2. **Best-effort WKWebView import** (Mac only, once):
   - Locate `~/Library/WebKit/com.andrejvysny.onecad/WebsiteData/Default/*/*/origin`.
   - Parse each one and pick the directory whose top and client origins are both `tauri://localhost` with no port.
   - Copy `localstorage.sqlite3`, `-wal` and `-shm` to a temp dir. Never open WebKit's live files: the Tauri app may be running, and opening could checkpoint or lock WebKit's WAL.
   - Open the copy read-only (rusqlite) and run `SELECT value FROM ItemTable WHERE key='onecad.settings'`. Decode the value as UTF-16LE, then parse the envelope with `coerce`.
   - Also check the legacy `WebsiteData/LocalStorage/tauri_localhost_0.localstorage` (same schema). This name is an assumption from `originToFileName` and was **not** observed. **[OPEN-5]**: keep the legacy probe, or drop it.
   - Any failure at any step falls through to defaults. Each step is logged at info level, and never as an error dialog.
   - The dev store `~/Library/WebKit/onecad/` (`http://localhost:1420`) is **not** imported. **[OPEN-6]**: confirm.
3. Otherwise use the defaults from section 1.5.
4. Write the result to `settings.json` immediately (`durable_write`). The import therefore runs at most once, and the WebKit store is never touched again. The import never deletes or modifies the WebKit data, so a rollback to the Tauri app is loss-free.

The dependency cost is rusqlite with bundled SQLite (the app is macOS-only, so no platform gating is needed beyond `cfg(target_os = "macos")` on the importer module). **[OPEN-7]**: rusqlite vs a minimal read-only reader; rusqlite is recommended.

## 5. Theme before window creation (replaces `index.html:15-34`)

- After step 4 the app has `theme: ThemePref`. Resolve it with the `themes.ts:59` rule:
  - `light` or `dark` is used as is.
  - `system` reads the OS appearance (`NSApp.effectiveAppearance`, or the `AppleInterfaceStyle` user default, read before the window exists).
- Apply it before the first frame:
  - the window theme hint (winit `WindowAttributes::with_theme`)
  - the first wgpu clear color and the egui or native style palette
  - the window background color, so the first composited frame is never the wrong color
- That is the native equivalent of stamping `data-theme` before paint.
- Fallback: an unreadable file or an unknown value resolves as `system`. The HTML script falls back to `light` only on an exception. **[OPEN-8]**: match that `light` fallback exactly, or use `system`; `system` is recommended.
- With `system`, follow live OS appearance changes at runtime. The resolved value is never persisted, as `settingsStore.ts:162-169` already requires.

## 6. Recovery and autosave compatibility

- **Same root, same crate, same constants** (section 2, items 2-4). A crash in the Tauri app is offered by the next native launch and the reverse, because the container is the source of truth and the pid decides ownership.
- **Container version skew.** If the native app is built from a newer `onecad-core` than the Tauri app it replaces, an older Tauri build may refuse a newer container. `recover_document` already keeps a failed restore retryable (`api/mod.rs:984-989`), so nothing is lost. Rule: only the newer app may write a newer container version; no native-only container format before cutover.
- **Concurrent dual-run** (Tauri and native both open during Phases 3-5), with the same bundle id:
  - LaunchServices treats them as one app: same Dock identity, `open` routing and TCC grants.
  - Both write `recents.json` with last-writer-wins and no lock. Concurrent writes can lose one entry, but cannot corrupt the file, thanks to `durable_write`.
  - **[OPEN-9]**: for dual-run QA, native dev/QA builds use a distinct id (the `.nativeqa` precedent) or `ONECAD_APP_DATA_DIR`, and only the release native build uses `com.andrejvysny.onecad`. Recommended.
- **Not migrated:** import temp workspaces (OS temp dir, swept by `imports::sweep_stale_workspaces`, `lib.rs:471-475`), logs and WebKit caches.

## 7. Migration tests

Fixtures go under `tests/fixtures/data-continuity/` in the crate that owns `settings`. Every test runs with `ONECAD_APP_DATA_DIR=<tempdir>` and `HOME` pointing at a temp dir, so nothing touches real `~/Library`. That is the same sandboxing reason `recents.rs:14-18` gives for its `*_at` twins.

**Fixtures**

- F1 `settings-v11-light.json`: the exact observed envelope (1.7) with `theme:"light"`.
- F2 `settings-v1.json`: `{"state":{"snapTo":{"grid":false}},"version":1}`.
- F3 `settings-garbage.json`: a v11 envelope with:
  - `displayMode:"x"`, `theme:7`, `snapRadius:"xl"`, `autoConstrainMode:null`, `navigation:{inputDevice:"pen"}`
  - `snapTo.grid:"false"`, `show.coincidentBadges:true`
  - an unknown top-level key `futureField:1`
- F4 `settings-corrupt.json`: `{"state":` (truncated).
- F5 `webkit-store/`: a synthetic `WebsiteData/Default/<a>/<b>/{origin,LocalStorage/localstorage.sqlite3}`. It is generated by a test helper using the observed `ItemTable` schema, a UTF-16LE value of F1, and an origin file for `tauri://localhost`. A decoy sibling dir holds origin `http://localhost:1420` with `theme:"dark"`, and a third holds an `inspector-resource` origin.
- F6 `autosave/`: an autosave pair (`<id>.onecad` plus `<id>.session.json`) written by the current `onecad-core`, with a dead pid (for example `u32::MAX`), and a second container with no marker.
- F7 `recents.json`: three entries pointing at temp `.onecad` files (one missing), plus a `library/` tree holding one user-edited seed package.

**Rust unit tests (`settings` module)**

- T1: `coerce(F1)` reproduces F1 field for field, and serializing and reparsing gives equal JSON.
- T2: `coerce(F2)` keeps `snapTo.grid == false`; every other field equals the default; the output version is 11.
- T3: `coerce(F3)` returns:
  - defaults for `displayMode`, `theme`, `snapRadius`, `autoConstrainMode` and `inputDevice`
  - `snapTo.grid == true`, because a string is not a boolean
  - `show.coincidentBadges == true`, because explicit values survive
  - `futureField` preserved in `extra` (only if [OPEN-1] keeps it)
- T4, parity: run a fixed corpus (F1-F3 plus one blob per version v1..v10) through TS (`settingsStore` migrate followed by merge in vitest, dumping JSON) and through Rust `coerce`. The two outputs must be byte-equal after canonical key sorting. The vitest dump is checked in as a golden file, and Rust compares against it. This is the guard against the two implementations drifting.
- T5: `write_settings` followed by `read_settings` round-trips, writes through `durable_write` (no temp sibling is left behind), and a corrupt existing file (F4) is quarantined, never overwritten.

**Native first-launch tests (Rust integration, Mac)**

- T6: with an empty root and an empty HOME, the result is the defaults and `settings.json` is created. The import attempt logs "no webkit store".
- T7: with an empty root and F5 at `$HOME/Library/WebKit/com.andrejvysny.onecad/…`, the theme is `light` (the decoy's `dark` is ignored), `settings.json` equals F1, and the F5 file bytes and mtimes are unchanged after the run.
- T8: with an existing `settings.json` (`theme:"dark"`) and F5, the result is `dark` and no SQLite file is opened. Assert this through the import-skipped log line or an injected opener.
- T9: with F4 as `settings.json` and F5 present, `settings.json` is quarantined and the import result is used.
- T10: an F5 with a locked or truncated SQLite file still gives defaults, with no panic.
- T11: theme pre-window, as a pure function test: `(pref, os_dark)` maps to the resolved value across all six combinations, plus fallback on a corrupt file. These are the same cases as `src/theme/themes.test.ts`.

**Continuity tests (shared crate, run once against the Tauri build and once against the native build's path resolver)**

- T12: root resolution. With `ONECAD_APP_DATA_DIR` unset and `HOME=<tmp>` (`dirs-sys-0.5.0/src/lib.rs:33-37` reads `$HOME` first, so this sandboxes the resolver), the native resolver yields `<tmp>/Library/Application Support/com.andrejvysny.onecad`, equal to Tauri's `app_data_dir()` and `app_config_dir()` for the same identifier.
- T13: over F6, `scan_recoverable` returns two offers (one labelled, one orphan). Recovering through the native path keeps `openedPath` and `title`. Discarding removes both files.
- T14: a marker owned by a live pid (the test's own) is not offered. This covers the dual-run case.
- T15: over F7, the native recents list equals the Tauri `list_at` output (same order, missing file handling per `recents.rs:161`). One record call keeps the cap at 10 and the camelCase on-disk format.
- T16: running the native seeder over F7's library leaves the user-edited package byte-identical (`library_seed.rs:19-25` rule) and keeps `.seed-version` and `.seed-ledger.json` semantics.

**Manual, user-run (tracked as owed validation in Plane)**

- M1: an installed Tauri app with non-default settings (dark, inches, wireframe, snap radius L), a recent project and one unsaved crash (kill -9 while dirty). Install and launch the native app. The first frame is dark with no white flash, settings match, the recent project is listed, and the crash is offered.
- M2: relaunch the Tauri app afterwards. Its settings and library are unchanged, which proves the native app did not touch WebKit or library data destructively.

## 8. Open decisions

| # | Decision | Recommendation |
|---|---|---|
| OPEN-1 | Preserve unknown top-level settings keys (`extra`) | Preserve, to keep TS round-trip parity and rollback safety |
| OPEN-2 | Coerce `navigation.inputDevice` in Rust (TS does not) | Coerce, and patch TS `merge` to match in 1b |
| OPEN-3 | Settings authority during dual-run (mirror vs bidirectional) | Mirror in 1b; revisit before Phase 3 |
| OPEN-4 | Flushing a pending write-through on exit | Flush in the React exit handler before `confirm_exit` |
| OPEN-5 | Probe the legacy `.localstorage` file | Keep; it is cheap, and its absence is harmless |
| OPEN-6 | Import the dev-build WebKit store | No |
| OPEN-7 | SQLite reader for the import | rusqlite (bundled), Mac only |
| OPEN-8 | Theme fallback on unreadable settings | `system` |
| OPEN-9 | Bundle id for native dev/QA builds | Distinct id or `ONECAD_APP_DATA_DIR`; release only uses `com.andrejvysny.onecad` |
| OPEN-10 | When the WKWebView import code is deleted | Keep for at least one release after the Tauri app is retired, since users may skip versions |

## 9. How this spec was checked

- Every repo `path:line` citation was resolved by a script against `git ls-files`: 68 resolved and 9 were flagged. Six flags were the bare `lib.rs` shorthand, which the script matched to the wrong crate; each was checked by hand against `src-tauri/src/lib.rs`. The other three were the external crate paths, also checked by hand with `sed -n`.
- On-disk facts in 1.2 and 1.7 were checked with `ls`, `find`, `xxd` and `sqlite3 -readonly` on copies. The decisive outputs were `onecad.settings|1050` in the bundled store, origin bytes `0500 0000 0174 6175 7269 …` (`tauri://localhost`), an 8-byte `Default/salt`, and an empty legacy `WebsiteData/LocalStorage/`.
- Nothing was built or run. All tests in section 7 are **NOT DONE**: they are a specification for Phase 1b and Phase 3.
