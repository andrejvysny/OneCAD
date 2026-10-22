# Native extension UI and validation

The production extension UI uses `native_extensions::render` and real registry
snapshots. The reference is screens 16–22 of the supplied OneCAD design. The old
`extension_pages` catalog is a design fixture, not an installation source or a
runtime registry. Its example publishers, inventory, versions and permissions
must not appear as real installed addons.

## Surfaces

| Reference | Production behavior |
|---|---|
| 16 Manager | Installed counts, search, local-source indicator, Discover/Installed/Updates tabs, responsive cards, compact installed rows, enable controls, loading and empty states. Discover shows local packages and explicitly states that online discovery is unavailable. |
| 17 Detail | Breadcrumb navigation, package identity/version/status, Overview/Settings/Changelog tabs, actual command locations and permissions, command availability, update and uninstall actions. No invented screenshots, publisher verification or changelog. |
| 18 Consent | Host-owned Adds/Can sections, actual application destinations, version and content fingerprint, cancel and explicit enable. Native execution remains unsupported. |
| 19 Editor | Export contributions, generic menu contributions and searchable commands. Unsupported workspace, tool-pill, inspector and viewport contributions remain outside WIT v1; see ADR-0018. |
| 20 Settings | Per-package declared application paths, preserved unsaved drafts, validation errors and explicit reset with a retained backup. Reset is also reachable from an empty manager. |
| 21 Updates/failures | Local Update from folder, new permission review, stopped-package diagnostics, unavailable commands and recovery actions. No online update feed is configured. |
| 22 Developer | Install from folder, refresh installed contents, real paths/fingerprints and diagnostics. Source-folder watching and live reload are unavailable. |

## Registration and update invariants

A successful install or local update explicitly requests review of that package
in its resulting snapshot. UI code must not infer success from unrelated packages
appearing or changing during a failed action. Updating validates the replacement
before changing active files, fences the selected package fingerprint, rejects
collisions, revokes old approval and restores the prior package on reported
replacement/persistence failures. This is not a crash-recovery journal.

Both native menu events and queued service requests carry identity fences. Menu
registration tokens retire when contributions change; execution requests capture
the package fingerprint at invocation. A queued click cannot execute a newly
approved replacement merely because it has the same command ID. Ordinary document
unavailability and stale exports do not mark a healthy guest as crashed.

Settings and approval writes respect their readers' byte limits. Failed writes do
not publish unsaved executable paths. Corrupt application settings cannot prevent
disable or uninstall, and management errors return fresh registry state whenever
it is available. Unknown service failures retire advertised commands until refresh.
Permission, palette and reset modals own input, including Escape.
Only failures from an extension request may retire its pending UI state; an
ordinary file-operation failure in another document must not cancel that state.

## Verification

Build the worker and addons normally, then run the required native adapter tests:

```sh
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
ONECAD_TEST_ADDON_COMPONENT="$PWD/packaging/binaries/addons/dev.onecad.bambu-studio/extension.wasm" \
cargo test --locked -p onecad-ui native_extensions --lib
```

Linux needs `--features eframe/x11`. These tests include real component dispatch,
real worker geometry, the native execution adapter, a controlled process launcher,
registration/update fencing, settings recovery, and production-renderer pointer,
keyboard and layout checks. The controlled launcher records argv; it does not prove
Bambu Studio imported or sliced a model.

The optional `capture_native_extension_surfaces` test renders the same production
page function through egui-wgpu. It writes diagnostic PNGs, never approved goldens:

```sh
ONECAD_EXTENSION_CAPTURE_DIR=/tmp/onecad-extension-captures \
cargo test --locked -p onecad-ui \
  native_extensions::visual_tests::capture_native_extension_surfaces --lib \
  -- --ignored --nocapture
```

On Linux add `--features eframe/x11,wgpu/vulkan` and provide a Vulkan adapter;
software Vulkan is sufficient for 2D layout inspection. Captures cover light/dark
manager, detail, settings, permission and reset dialogs, failure, loading, search,
long-content and narrow-window states. They use the real Bambu package with
explicit presentation fixtures for errors and long text. The captured area is the
Home page content to the right of the sidebar, not a screenshot of a running
native window. Linux uses fallback fonts; macOS system-font metrics, native menu
presentation, Metal rendering and actual Bambu Studio import need macOS validation.
