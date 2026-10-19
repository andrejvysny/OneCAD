---
name: onecad-ui-4b-integration
description: onecad-ui step 4b wiring gotchas: FeatureCard fixed labels, TitleBar no home-active, NumberField focus, app screenshot driving
metadata:
  type: project
---

- FeatureCard has `ok_label` ("Create ↵"/"Apply ↵") and `cancel_enabled` builders; host still gates dispatch (phase==Editing, can_apply).
- NumberField `keep_focus_on_enter(true)` mirrors the old `return_key(None)`; `step_keys(!app.composing)` gates arrow nudges during IME. No lost_focus workaround needed.
- Constraints menu closes when the pill button is absent (SketchEntity selection group); anchor = button rect from `pill::draw_groups`.
- `ctx.system_theme()` drives Appearance::System; `theme::install` already applies visuals, so no separate apply call.
- App screenshots: `nohup` the binary (a plain `( ... &)` dies), find window id with a CGWindowList swift snippet, `screencapture -l <id>`; cliclick clicks did not reach the unfocused window on a secondary display, so verify only static screens (temporarily flip a UiState default, revert after).
