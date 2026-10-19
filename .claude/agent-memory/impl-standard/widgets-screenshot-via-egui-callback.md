---
name: widgets-screenshot-via-egui-callback
description: Reliable gallery screenshots via egui ScreenshotCallback (no screencapture), plus egui layout gotchas hit in onecad-widgets
metadata:
  type: project
---

Screenshot without OS capture: temp-patch examples/gallery/main.rs (back up first) with env ONECAD_SHOT_IDX (screen index into SCREENS, 0=01a..8=07a), ONECAD_SHOT_OUT, ONECAD_SHOT_DARK; at frame 12 send `ViewportCommand::Screenshot(egui::ScreenshotCallback::new(|img| write PPM; exit))` (egui 0.37 has no Event::Screenshot/UserData); convert with `magick x.ppm -resize 1340x x.png`. Window inner_size 1340x1000. macOS has no `timeout`.

Gotchas: `allocate_ui_with_layout(size, ..)` shrinks to content, so fixed label columns must use `allocate_exact_size` + painter galley or fields misalign. `cargo fmt --all` can fail while another agent has an unfinished `mod` in onecad-ui; use `cargo fmt -p onecad-widgets`. rustfmt rewraps edited lines, so re-read before follow-up string-replace edits.

Small chrome glyphs (close, chevron, search, eye, check, plus, degree) are vector strokes in src/glyph.rs (pub(crate)); baked SVG icons at 10-14px render too faint. `glyph::by_name` maps baked names to the vector twin.
