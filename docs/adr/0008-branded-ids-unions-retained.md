# 0008 — Namespaced branded IDs, with the internal unions retained

- Status: Accepted
- Date: 2026-08-08

## Context

Tools are identified today by closed TypeScript unions — `ModelTool` (15
variants), `SketchTool` (17) — and those literals flow through the tool store,
the toolbar, the keymap, the tool controller's per-op switches, persisted
settings, e2e selectors and debug surfaces.

Registries need open, namespaced identity (`onecad.modeling.tool.extrude`) so a
module the shell has never heard of can contribute a tool. The obvious move —
`type ToolId = string` — buys that openness by deleting every exhaustiveness check
in the modeling controller, in a codebase whose largest correctness wins came from
the compiler refusing to let a case be forgotten.

## Decision

Both, at different layers.

- **Registry boundary**: branded, namespaced ids. `ToolId`, `CommandId`,
  `PanelId`, … are branded strings; modeling exposes const maps
  (`ModelingTools.Extrude = "onecad.modeling.tool.extrude"`) so internal callers
  stay strongly typed while the registry stays open.
- **Inside modeling**: `ModelTool` / `SketchTool` unions are unchanged. The tool
  store keeps its short literals, so existing exhaustive switches, persisted
  settings values, e2e selectors and debug output all keep working.
- Translation happens only at the boundary, through explicit bidirectional maps.

A `switch` over modeling tools **inside modeling** is fine. A switch over modeling
tools inside platform or shell code is the thing this architecture removes.

## Cost

Two names for the same tool, plus a mapping table that must stay complete —
enforced by a test asserting the map covers every union member in both directions.
The alternative costs exhaustiveness checking across the largest and most
defect-sensitive controller in the frontend.
