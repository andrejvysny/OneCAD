---
name: widgets-gallery-screenshot-recipe
description: How to screenshot onecad-widgets gallery groups headlessly-ish on macOS
metadata:
  type: project
---

Temp-patch examples/gallery/main.rs (back it up, restore after): env GALLERY_GROUP filters GROUPS by title prefix, GALLERY_DARK sets dark, ViewportBuilder inner_size [900,1300] + position [0,30], ScrollArea vertical_scroll_offset from GALLERY_SCROLL. Run the built binary in background, sleep 5, `screencapture -x -R0,30,900,1300 out.png`, kill. OS clamps window to ~1300pt tall; use scroll offset for lower cards. Avoid zsh `set -- $str` loops; use nested for.

`stage()` in g_shell.rs: the note below it visually overlaps the stage bottom edge; size stages with ~20px slack. Dark stage (canvas_model) is near-invisible against bg.
