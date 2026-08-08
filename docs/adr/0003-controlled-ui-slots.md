# 0003 — UI contributions go into controlled slots, never raw DOM

- Status: Accepted
- Date: 2026-08-08

## Context

The editor currently mounts nineteen concrete feature components as siblings and
positions them absolutely. Order inside a z-index band is load-bearing — a past
defect had tool chips rendering under the side panels, making them unclickable.

Any extension mechanism has to preserve that kind of invariant while letting code
the shell has never heard of appear on screen. Plugin ecosystems that hand out
DOM access instead produce inconsistent spacing, icon sizes, keyboard behavior and
error presentation — and then cannot change their own layout without breaking
third parties.

## Decision

Contributions are **React components plus metadata**, placed into a fixed,
closed set of host-owned slots:

```
shell.top / shell.left / shell.right / shell.bottom
toolbar.primary / toolbar.contextual / toolbar.overflow
viewport.overlay / viewport.layer / viewport.context
inspector.section
tree.decorator / tree.context
status.section
```

The host owns the panel frame, resizing, focus, keyboard navigation, theme,
dragging, layout persistence and visibility. The contribution owns content only.

Explicitly not part of the API: `document.querySelector`, appending arbitrary DOM,
replacing the editor shell, absolute pixel placement.

Because placement is semantic rather than positional, the same contribution can be
laid out differently on desktop and iPad without the contributor doing anything.

## Cost

A contribution that genuinely needs a layout the slots do not offer cannot have
it, and must wait for a new host template. Accepted: arbitrary layout freedom is
the thing that makes a plugin UI feel foreign, and it cannot be taken back once
granted.
