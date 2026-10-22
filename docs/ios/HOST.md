# iPadOS host bridge (`crates/onecad-ios`)

How the Rust side of the UIKit host runs the shared editor. The C contract is
[`onecad_ios.h`](../../crates/onecad-ios/include/onecad_ios.h); boundaries and
ownership are in [ARCHITECTURE.md](ARCHITECTURE.md). This page describes code
behaviour and asserts no test result.

## Editor without eframe

`onecad-ui` keeps eframe, `rfd`, the AppKit menu bar (`muda`), `arboard` and
`run()` behind its default `desktop` feature. `onecad-ios` depends on it with
`default-features = false` and drives `onecad_ui::host::Editor`, a thin facade
over the same `App` that eframe runs on macOS:

- `Editor::new(ctx, render_state, QaPaths::sandbox(layout), dialogs)` is
  `App::new_with`; desktop `App::new(cc, paths)` builds the same way with the
  eframe creation context, `NativeDialogs` and the AppKit menu.
- `Editor::frame(ctx, raw)` is `App::frame`: `raw_input_hook` (routing, then the
  CAD fence) and `ctx.run_ui(raw, |ui| { logic; ui })`, eframe's order.
- `QaPaths::sandbox` takes the container directories (`HostLayout`): documents
  in the app's Documents directory; service data and settings under
  `<Application Support>/data` and `/config`. No QA marker, lock or launch
  environment applies. The caches directory is created but nothing writes it yet.
- Without a menu bar the title bar shows the existing "File ▾" control
  (`TitleBar::tab_file`) with a menu of New, Open, Save, Save As, Save All,
  Close Tab, Import, Insert STEP, Export STEP, Send a Copy, Undo and Redo.
  Rows queue the same `menu::Action`s the AppKit menu sends; `input::route`
  drains them next frame with the enablement it publishes (computed as if this
  menu's own popup were closed). Save All and Send a Copy follow Save's
  enablement; Save All has no keyboard shortcut on iPad. The title bar has no traffic-light inset
  and does not drag a window.
- On iOS, ⌘ sets `command` and `mac_cmd`, orbit direction matches macOS, and the
  CoreText font lookup runs as on macOS. `onecad-ui` enables egui's
  `default_fonts`, so egui's bundled fonts stay behind SF as the fallback for
  missing glyphs and for a system font file the sandbox cannot read (device
  readability is unverified).

## Threads

| Thread | Work |
|---|---|
| UIKit main | Every `onecad_ios_*` call (checked; others get `ERR_WRONG_THREAD`), egui pass, Metal acquire/encode/submit/present, a11y flattening. |
| `onecad-native-services` (Tokio, 4) | Document services and bridge actors, unchanged from macOS. |
| Any | `wake` via egui's repaint callback; never calls back into the library. |

The wake callback goes through a gate. Requests made on the main thread inside
`onecad_ios_frame` do not wake (the frame result carries them); all others do.
`destroy` closes the gate under its lock, so no wake runs after it returns; a
failed `create` closes it too. Destroy then drops the editor and waits up to 3 s
for the service runtime thread to exit. If it is still exiting, destroy returns
anyway (it can no longer reach the host) and the next `create` waits up to 3 s
for it before failing with `ERR_STATE`, so two runtimes never share the
app-support data.

Every export runs under `catch_unwind`. A contained panic returns `ERR_PANIC`,
poisons the app (later calls return `ERR_TERMINAL`) and only `destroy` proceeds.
A re-entrant call returns `ERR_STATE`. `onecad_ios_last_error` returns this
thread's last failure text, never NULL.

## Frame loop

`onecad_ios_frame(time_s)` on a display link, paused until `wake`:

1. Refused (`ERR_STATE`) in the background, after terminate or without a
   surface. Only the leading `struct_size` of the result is read.
2. Before the pass: a cancelled contact's press is forgotten
   (`onecad_ui::host::cancel_contact`), and files from `open_path` are opened.
3. `RawInput`: screen rect in points (`px / scale`), root viewport
   `native_pixels_per_point = scale`, safe-area insets (bottom is the larger of
   the safe area and the keyboard overlap), `time`, `focused` (scene active),
   `system_theme` (trait events), `max_texture_side` (device limit), and the
   queued events.
4. `Editor::frame`: routing fences the previous frame's CAD sampling, then logic
   and ui; the viewport encodes and submits the CAD pass.
5. Texture uploads → tessellation → drawable acquire → egui pass (load-clear,
   gamma-space `Bgra8Unorm` surface) → one `queue.submit` → present → texture
   frees. Without a drawable the uploads are still submitted.
6. Result flags: `REDRAW_REQUESTED` and `repaint_after_s` from the root viewport's
   repaint delay (`-1` = none; `0` when a drawable could not be acquired), `PRESENTED`, `TEXT_INPUT_ACTIVE` with the cursor
   rect and `NUMERIC_EXPRESSION` while a dimension/numeric editor is open,
   `A11Y_CHANGED`, `HOST_REQUESTS`. `KERNEL_TERMINAL` is not produced yet.

Surface: one Metal device (`required_device_features` as on desktop; limits the
adapter cannot meet, such as the Simulator's 15 inter-stage variables, are
lowered to the adapter's) exists from `create`; `surface_attach` creates the `wgpu` surface over the retained
`CAMetalLayer`. `Outdated` reconfigures, `Lost` recreates the surface from the
same layer, `Suboptimal` presents then reconfigures, timeouts skip the frame.
`surface_detach` waits (at most 1 s) for submitted work before releasing the
surface. A drawable is acquired only after the editor pass, never twice per frame.

## Input mapping

Samples are copied at submit and routed at once through
`onecad_interaction::touch` (`docs/ios/INPUT.md`): the normalizer (rules 1-7)
and the router (one owner per contact) use the route context of the last
completed frame: `over_control` is `onecad_ui::host::over_control` (anything
but the bare CAD canvas drawn that pass), `drag_authoring` is
`Editor::drag_authoring` (a sketch creation tool, a sketch handle drag, or a
model tool with canvas handles), and `pencil_only_drawing` comes from
`capabilities_set`. Test-hook replay uses the same path. Up to 4096 samples per
batch and 16 384 queued events; overflow coalesces the oldest intermediate
movement and returns `ERR_OVERFLOW`; presses, releases, cancellations, keys and
text are never dropped, and navigation and orbit moves between frames merge.
Input calls return `ERR_STATE` in the background and after terminate
(`cancel_all` is always accepted).

| Routed event | egui events |
|---|---|
| `Press` (`Author`, `Control`, `Tap`) | `PointerMoved` + primary press; it holds the single pointer. The frame sets `host::set_pointer_kind` (finger: 22 pt edge pick aperture; Pencil/pointer: 8 pt) and `host::set_pencil_double_tap` (a Pencil press with `tap_count` ≥ 2 picks like a double click, face to body, even when egui's click timing does not count it) |
| `Move` of the pointer contact | `PointerMoved` per changed position (disposable under overflow) |
| `Release` | `PointerMoved` + release; after a finger or a tap, `PointerGone` one frame later |
| `Tap` (finger within 8 pt / 0.5 s) | press and release at the begin position in one frame (double tap counted by egui's timing) |
| `Orbit` (canvas finger past the slop, no drag authoring) | `PointerMoved(position)` (only while no pointer is held) + `MouseWheel{Trackpad, Point, Start/Move/End/Cancel}` with Shift, which the editor treats as orbit; `PointerGone` after the end |
| `MultiTouch` | nothing: native pan/pinch drive navigation |
| `Activate` (a finger tapped a control while another contact held the pointer) | no pointer event: the next frame sends `AccessKitActionRequest(Click)` to the last-flattened enabled clickable accessibility element under the begin position, from the previous frame's tree; nothing if none |
| `Cancel` (UIKit cancel, `cancel_all`, generation change, Pencil precedence, multi-touch, document switch) | no release: the contact's queued events are dropped; if egui saw its press, the next frame first forgets it (`onecad_ui::host::cancel_contact`) and carries `PointerGone`. No click or drag commit can result. |
| `Hover` / `HoverExit` (Pencil or pointer, no pointer held) | `PointerMoved` / deferred `PointerGone` |
| `Overlay::Prediction` | latest points kept per authoring contact (transient; never an event, not drawn yet) |
| Navigation `BEGAN/MOVED/ENDED/CANCELLED` | `BEGAN` hands canvas fingers to it (`Router::native_navigation`): an authoring finger is cancelled, an orbit ends in place; ignored while a Pencil or pointer authors. Then `PointerMoved(anchor)` (no pointer held), `MouseWheel{Trackpad, Point}` pan delta, `Zoom(scale_delta)`; deferred `PointerGone` after the end. Rotation is not mapped. |
| Key (HID usage, modifiers) | `ModifiersChanged` on change, `Key{key, physical_key, pressed, repeat}`; ⌘ = `command` + `mac_cmd` |
| `text_insert` | `Text`; `"\n"` is an Enter tap; while composing, `Ime(Commit)` |
| `text_delete_backward` | Backspace tap |
| `text_set_marked` | `Ime(Preedit)`; empty ends composition without commit |
| Pencil barrel tap / squeeze (`pencil_event_submit`) | INPUT.md accessory policy on the event's `preferred` action, applied at the start of the next frame: `Editor::toggle_previous_sketch_tool` or `Editor::open_tool_palette` at the hover pose (only with `CAP_HOVER` and `HAS_LOCATION`), else the last Pencil position, else the canvas centre. A squeeze acts on `ENDED`; a barrel tap must be `ENDED`; other phases are `ERR_INVALID_ARGUMENT`. Visible equivalents: the tool pill (and the palette is the pill's tools). |

A deferred `PointerGone` never shares a frame with the release or navigation it
follows: `sketch_gestures::route_raw` cancels on any `PointerGone` in a frame
and egui drops hover-gated navigation. Opening a system-provided file cancels
live contacts (`DOCUMENT_SWITCH`) before the frame that switches documents.
A frame whose editor pass changed the active document by any other path (tab
strip, ⌃Tab, a New or Open that got its own tab) cancels every live contact
with `DOCUMENT_SWITCH` right after the pass (`Editor::active_tab` compared
across frames); the next frame forgets egui's press and carries
`PointerGone`, so a stroke or drag begun on one document never releases on
another.

### Hardware keyboard and text

While the host is the `UIKeyInput` first responder, UIKit reports a hardware
Return or Backspace both as a `UIPress` and as `insertText`/`deleteBackward`.
Forward each keystroke once: text, Return and Backspace through `text_insert` /
`text_delete_backward`, and only the other keys (shortcuts with ⌘/⌃/⌥, arrows,
Escape, Tab) through `key_submit`. Without an active text input every key goes
through `key_submit`.

## Accessibility

`ctx.enable_accesskit()`; each frame's `accesskit_update` is merged into a node
map, pruned to what the root reaches, and flattened depth-first (back to front)
into `onecad_ios_a11y_node`s. Hidden subtrees and text runs are skipped; a node
with bounds of non-zero area is exported when it has a label or value, or when
it is clickable/focusable and its role is not `other`. A static text's value
(egui keeps a label widget's text there) doubles as its label. Bounds are
logical points (the root's points→pixels transform is not applied).

Identifier: `author_id` when set, otherwise `<role>.<label>` with the role name
(`button`, `text_field`, `static_text`, `tab`, `checkbox`, `image`, `canvas`,
`menu_item`, `slider`, `group`, `other`) and the label lowercased with each run
of other characters replaced by `-` (`"Export STEP…"` → `menu_item.export-step`).
Repeats in z-order get `#2`, `#3`. Labels are the editor's English strings.

Author ids set today: the touch File menu (`onecad.menu.file` title bar button,
`onecad.menu.file.new`, `.open`, `.save`, `.saveAs`, `.saveAll`, `.closeTab`,
`.import`, `.insertStep`, `.export`, `.sendCopy`, `onecad.menu.edit.undo`,
`.redo`) and the editor
catalogue in [TESTING.md](TESTING.md#accessibility-identifiers) (tools,
operation fields, confirm/cancel, tabs, history, planes, viewport, multi-select).
Flags: disabled, selected/toggled, focused. `A11Y_CHANGED` is set when the
flattened list differs; the snapshot generation increments with it.

`a11y_activate(id)` sends AccessKit `Click` for clickable nodes, else a
synthesized tap at the node centre.

## Host requests

Bounded queue of 64 (pending requests plus dialogs popped and not yet
completed; a dialog counts once). A dialog dropped with its document frees its
slot, and its request is withdrawn if the host has not popped it.
`FRAME_HOST_REQUESTS` reports pending ones; `host_request_next` pops one
(strings valid until the next call) and returns `ERR_STATE` when empty.

| Source | Request | Completion |
|---|---|---|
| Open, Import, Insert STEP dialog | `OPEN_DOCUMENT` (label, extensions, multiple) | `host_request_complete(id, paths)`; empty = cancelled. The host copies picked files into the sandbox; opening copies non-STEP files into Documents. |
| Save As, Export dialog | `SAVE_DOCUMENT` (label, extension, `suggested_name`) | One path inside Documents (other paths are refused by the services). |
| Finished export; Send a Copy | `EXPORT_FILE` (`path`, `suggested_name`) | None needed; the host presents an export picker. Send a Copy names the document's saved `.onecad` and is refused (status line) when the document has no file or unsaved changes. |
| egui copy / open URL | `COPY_TEXT` / `OPEN_URL` (`text`) | None. |

A full or closed queue cancels the dialog. `WILL_TERMINATE` and `destroy` close
the queue and cancel every waiting dialog first (an open picker holds its
document's actor); completing such a request afterwards returns
`ERR_INVALID_ARGUMENT`. `onecad_ios_open_path` queues a system-provided file
(at most 16) that the next frame opens like Open Recent; a `.step`/`.stp`
file is imported into a new document instead (`Request::Import(Some(path))`,
the File › Import path without its dialog).

## Files and documents

Managed-copy policy: `LSSupportsOpeningDocumentsInPlace` is `NO`, so the app
never edits a file where another provider keeps it. Rust's document services
write only inside the sandbox (path claims, atomic container writes, crash
recovery, recents and autosave all assume a local, always-reachable file with
POSIX rename semantics), and a provider file can vanish, be evicted or be
replaced behind the app (iCloud, third-party providers, revoked bookmarks).
`UIFileSharingEnabled` exposes `Documents` to Finder file sharing.

| Path | Behaviour |
|---|---|
| In: Open, Import, Insert STEP pickers | `UIDocumentPickerViewController(forOpeningContentTypes:asCopy: true)`; the picked files are copied into `Documents/Inbox` (`DocumentInbox.importCopies`, off the main thread) and only sandbox paths reach Rust. A failed copy shows `onecad.import.failed`; Cancel completes the request empty and the editor reports `File action cancelled`. |
| In: Files "Open in" | The system copies the file into `Documents/Inbox`; one that arrives elsewhere is copied there. Then `onecad_ios_open_path`. |
| Security scope | `SecurityScope.withAccess` starts the source URL's security-scoped access, runs the whole coordinated read (`NSFileCoordinator`, `.withoutChanges`) and copy synchronously inside it, and stops it when that returns or throws; an ungranted scope is never stopped. Each URL of a batch gets its own scope. |
| Save / Save As | The native name prompt answers `SAVE_DOCUMENT` with `Documents/<name>.onecad`. The container writer creates an exclusive same-directory temp file, writes and `fsync`s it, renames it over the target and `fsync`s the directory (`onecad-core` `ContainerWriter::save_inner`, `OwnedTempFile`); a failure removes the temp and leaves the previous file untouched, the document dirty and bound to its previous path. |
| Out: Export STEP, Send a Copy | Rust writes (STEP) or names (saved `.onecad`) a sandbox file and queues `EXPORT_FILE`; the host presents `UIDocumentPickerViewController(forExporting:asCopy: true)`, which performs the coordinated write at the destination. The document keeps its sandbox path. A save that runs meanwhile replaces the source only by rename, so the picker reads one complete container. |
| Recovery | `DID_ENTER_BACKGROUND` flushes every open document's autosave (the host holds a 5 s background task); `WILL_TERMINATE` and `destroy` await it. A dirty document closed by shutdown keeps its recovery files; the next launch offers them (Recover unsaved work: Restore, Open saved, Discard recovery). A process killed after the flush is offered the flushed revision; edits after it are lost. |
| Tabs | One `UIScene`, at most three document kernels (KERNEL.md run cap). Each tab keeps its own document host, history, selection, camera and dirty state; the strip ignores a tab tap while the active document is busy (file action, command, decision), so a save or import finishes in the document that started it. |

Physical-only (ONECAD-469; the simulator offers only local storage): iCloud
Drive download/eviction, third-party File Provider extensions, a provider
revoking access or going offline mid-copy, and security-scoped bookmarks of
real provider URLs.

## Lifecycle

| Event | Effect |
|---|---|
| `WILL_RESIGN_ACTIVE` | cancel contacts, `WindowFocused(false)`, unfocused frames |
| `DID_ENTER_BACKGROUND` | as above, request an autosave flush for every open document (not awaited; a document whose actor waits on an open picker flushes after it), refuse frames and input |
| `WILL_ENTER_FOREGROUND` | frames allowed (inactive) |
| `DID_BECOME_ACTIVE` | `WindowFocused(true)`, focused frames |
| `MEMORY_WARNING` | release parked tabs' switcher thumbnails |
| `WILL_TERMINATE` | cancel contacts, close the request queue (cancelling dialogs), await the autosave flush of every open document, stop the document hosts (flush and stop share 3 s); then frames, input, `a11y_activate`, `open_path` and lifecycle events other than `MEMORY_WARNING` return `ERR_STATE` |
| `TRAIT_DARK` / `TRAIT_LIGHT` | `system_theme` (used when Appearance is System) |

`destroy` runs the terminate sequence (unless `WILL_TERMINATE` already did),
closes the wake gate and the request queue, drops the editor, releases the
surface and waits for the service runtime as described under Threads.

## Test hooks

With the `test-hooks` feature only. `onecad_ios_test_command` takes one JSON
object and returns `{"ok":true,...}` or `{"ok":false,"error":"..."}` (free it
with `onecad_ios_test_string_free`). Implemented commands (the catalogue
`onecad_ios_test.h` points at; [TESTING.md](TESTING.md) links here):

| Command | Request | Response fields |
|---|---|---|
| `ping` | `{"cmd":"ping"}` | `abi` |
| `replay` | `{"cmd":"replay","samples":[<sample>],"generation"?}`; a sample is the trace fixture sample of `INPUT.md` (appendix): `contact`, `phase`, `t`, `x`, `y`, optional `kind` (Pencil), `provenance` (actual), `tap_count`, pose and estimation fields, names or ABI numbers; at most 4096 samples; `ERR_STATE`-equivalent error in the background | `synthetic: true`, `accepted`, `overflowed` |
| `prediction` | `{"cmd":"prediction","contact"}` | `synthetic: true`, `prediction` (`[x, y]` or null) |

Replay goes through the same normalizer, router and route context as
`onecad_ios_input_submit` (`IosApp::submit_touches`); each batch is logged with
`synthetic = true`. Samples also accept `tap_count` (default 1). The egui
events it produces are not marked further. Semantic queries (run and kernel
identity, mass properties, history, constraints), acceptance artifacts and
fault injection are catalogued in
[TESTING.md](TESTING.md#test-hook-commands-onecad_ios_testh).
