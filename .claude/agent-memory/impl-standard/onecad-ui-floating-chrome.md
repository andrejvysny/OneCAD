---
name: onecad-ui-floating-chrome
description: onecad-ui floating-chrome (step 4a) wiring gotchas: popups, cube host rect, egui 0.37 test input
metadata:
  type: project
---

- Menus: use `egui::Popup::menu(&resp).frame(menu_frame(ctx))` each frame (state in egui memory); rect-anchored menus via `Popup::new(.., rect, LayerId::background()).kind(Menu).open_bool(..)` (display_menu.rs).
- ViewCluster hosted with `.cube(0.0)` + 18px leading space so its Area starts below the 164px nav-cube region and never steals cube input; cube rect comes from `viewport_widgets::cube_host` (layout anchor).
- `consume_key` matches modifiers from the Event, not RawInput (no `raw.modifiers` in 0.37); `ctx.egui_wants_keyboard_input()` = text focus guard.
- Pill centre: inspector open -64, else explorer open +86, else 0 (design mocks override README).
- Sketch Finish/Discard now live in session_strip.rs (old sketch toolbar removed).
