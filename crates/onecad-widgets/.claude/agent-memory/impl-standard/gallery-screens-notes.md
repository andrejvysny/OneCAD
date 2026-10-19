---
name: gallery-screens-notes
description: Gotchas for the onecad-widgets gallery Screens tab (examples/gallery/screens) and screenshotting it
metadata:
  type: project
---

Gallery screens are drawn with child Uis in a 1280x800 stage (no egui Areas); layout maths lives in `screens/layout.rs` (tests run via `cargo test -p onecad-widgets --example gallery`).

Screenshot recipe: temporarily patch `main.rs` to send `ViewportCommand::Screenshot(egui::ScreenshotCallback::new(..))` after ~25 passes, write a PPM, convert with `magick`; set tab/dark/`Id::unique("screens-selected")` from an env var. Restore main.rs afterwards. `file://` is blocked in playwright-cli; serve the design folder with `python3 -m http.server` and screenshot `[id="02a"]`.

Design mocks vs README: pill centre is +86 px right of viewport centre when only the Explorer is open (README says only -64 with Inspector); start sidebar is 240 px wide in the mock (README says 220).
