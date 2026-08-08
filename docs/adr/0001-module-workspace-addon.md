# 0001 — Module, Workspace and Addon are three different things

- Status: Accepted
- Date: 2026-08-08

## Context

FreeCAD's `Gui::Workbench` composes command sets, toolbars, menus and dock UI in
one abstraction. It is genuinely powerful, and OneCAD should borrow the
composition ideas. But it collapses three concerns into one:

```
Module == Workspace == UI implementation
```

The result is isolated environments: users learn "switch to workbench X to find
feature Y", and capabilities cannot easily be combined across domains.

OneCAD's product ambition is the opposite — one application, context-aware
commands, minimal visible mode switching.

## Decision

Three separate concepts, with separate lifecycles:

- **Module** — owns a domain capability, its state and its contributions.
  `onecad.modeling`. A module is not automatically a workspace.
- **Workspace** — a presentation configuration: which commands, panels and tool
  groups are prominent for a kind of work. It decides *how capabilities are
  presented*, never *which capabilities exist*.
- **Addon** — the installable/distributable package. It may contribute modules,
  workspaces, or contributions directly, or contain only resources.

Consequences of the split:

- a workspace may combine capabilities from several modules;
- a module may participate in several workspaces;
- an addon may simply extend the default workspace instead of creating one;
- **module active ≠ workspace active** — modeling stays loaded and its geometry
  stays available while some other workspace is in front.

## Cost

Three registries and three lifecycles instead of one, and contributors must learn
which of the three they are writing. That cost is paid once; a collapsed model
would be paid at every future domain.
