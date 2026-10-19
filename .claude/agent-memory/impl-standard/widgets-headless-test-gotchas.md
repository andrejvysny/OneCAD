---
name: widgets-headless-test-gotchas
description: onecad-widgets egui 0.37 headless tests + API gotchas (textures_delta, Id::new deprecated, galley colour)
metadata:
  type: project
---

- `ctx.run_ui(..)` output must get `out.textures_delta.clear()` or icon-loading tests panic in `TexturesDelta::drop`.
- `egui::Id::new`/`Ui::id` deprecated (-D warnings): use `Id::unique`, `IdSalt::new`/`impl AsIdSalt`, `ui.scope_id()`.
- `painter.galley(pos, g, c)` ignores `c` unless galley laid out with `Color32::PLACEHOLDER` (needed for 40% disabled alpha).
- Animated open/close: `animate_bool_with_time_and_easing(id, v, 0.16, cubic_out)`; first call snaps to value; advance with `RawInput.time`.
- BSD sed needs `sed -i ''`. Gallery screenshot trick: temp env filter in examples/gallery/main.rs (restore after).
- `metrics::shadow(kind, &Theme, dark)` now takes theme (tints from `t.shadow`); python `s.index("fn hud(")` style splices can hit `pub fn` inside impls first — anchor on `
fn `.
