---
name: onecad-ui-settings-pages
description: Settings page gotchas in onecad-ui - no App in unit tests, disabled-tooltip path, formatter hook, egui fork Id API
metadata:
  type: project
---

onecad-ui `App` needs a wgpu RenderState, so unit tests cannot construct one; keep page code on `&mut UiState`/`&mut Ui` (see `settings_pages::content`, `ShortcutsUi::keys`) and extract pure helpers (`start_screen::parent`, `tool_keys::blocked`) for anything that would take `&mut App`.
`Response::on_disabled_hover_text` never fires for widgets drawn with `Sense::hover` (response still `enabled()`); use `tooltip::attach(resp, Tip::new(name).reason(Some(..)))`. SegmentedControl has no `enabled`: wrap in `ui.add_enabled_ui(false, ..)`.
This egui fork deprecates `Id::new`/`Ui::id`; use `egui::Id::unique((..))`. A PostToolUse hook rustfmts written files; still run `rustfmt --edition 2024 --check` through `bash -c` (zsh does not word-split `$F`).
Segmented control cannot take a mono font (decimals row uses default font).
