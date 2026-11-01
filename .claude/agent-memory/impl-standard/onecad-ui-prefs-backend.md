---
name: onecad-ui-prefs-backend
description: Where nav/shortcut/decimals prefs live and testing constraints in onecad-ui
metadata:
  type: project
---

Preferences backend: `navigation_prefs.rs` (also holds pure `drag_motion`/`wheel_motion`/`zoom_anchor` used by `viewport.rs`), `shortcut_prefs.rs`, lookup fns in `tool_registry.rs` take `&[ShortcutOverride]`.

Viewport tests that build an `App` (Host) are `#[ignore]`/need ONECAD_WORKER_PATH + GPU, so testable logic is extracted as pure helpers instead. Unused-until-Settings-UI APIs carry a module `#![allow(dead_code, reason=...)]`. Clippy `enum_variant_names` fires on `DragInput` (needs allow). `cargo test -p onecad-ui` has 2 env-only failures (datum_ui, sketch_services regression) without the worker.
