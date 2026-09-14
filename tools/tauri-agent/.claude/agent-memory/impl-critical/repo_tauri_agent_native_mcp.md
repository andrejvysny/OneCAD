---
name: repo-tauri-agent-native-mcp
description: tauri-agent MCP native surface — why every AX walk invalidates held refs, where the AX pool lives, the two settle lanes, and the TargetSchema arm that crashes the ui_* tools
metadata:
  type: project
---

Facts from wiring the accessibility layer into the MCP tools of `tools/tauri-agent`
(`src/mcp/tools/native.ts`, `{axRef}` in `TargetSchema`, `settleNative`). Companions:
[[repo-tauri-agent-ax]] (Swift), [[repo-tauri-agent-native-gates]] (the two point gates).

- **EVERY AX walk bumps the helper's ref generation and drops the previous map**
  (`AxRefs.shared.begin()` in `main.swift`, called by `ax_snapshot` AND `ax_find`). So the native
  settle, which hashes a walk, invalidates every AX ref the caller is holding. That is inherent to
  hashing the subtree, not a bug: the pipeline adopts each walk into `session.axRefs` so the local
  pool never lags the helper, and the new generation is reported in `data.nativeSettle.generation`.
  The workflow is therefore snapshot → act → snapshot. A `{ref}` webview click needs no such
  re-snapshot, because the DOM settle reads the idle tuple rather than re-walking.
- **The AX ref pool is `PipelineSession.axRefs`, an OPTIONAL member.** `SessionOrchestrator` does
  not declare it; the tools write it through the structural `PipelineSession` seam and the property
  lands on the orchestrator instance at runtime (nothing freezes it). Required members cannot be
  added to `PipelineSession` without editing the orchestrator, which is often owned by another
  package — optional + structural is the seam that works.
- **The AX ref generation is helper-side, so a `reconnectBridge()` does NOT invalidate it.**
  `session.refs` is cleared there; `axRefs` deliberately is not.
- **The node fingerprint the native settle hashes must EXCLUDE `ref`.** Every walk mints new refs,
  so hashing them means nothing ever goes quiet. Bounds are rounded to whole points, or sub-pixel
  jitter reads as movement.
- **An offset on an `{axRef}` target has to be re-gated.** The resolver gates the element's CENTRE;
  adding an offset afterwards would be a way around `checkGlobalPoint`. `offsetPoint()` re-reads the
  owned windows only when an offset was actually passed.
- **Known wart: `TargetSchema` now has an `{axRef}` arm, and the `ui_*` tools share that schema.**
  `ui_find`/`ui_inspect`/`ui_snapshot {root}` therefore accept it and die in the semantic resolver
  with `TypeError: undefined is not an object (evaluating 'point.space')` → INTERNAL (`ui_find`
  with `all:true` is clean: INVALID_TARGET). The fix is one guard in `semantic/resolve.ts#locate`
  or in `ui.ts`; neither was in the package's allowed files.
- **`pointer_up` / `pointer_down` with NO target still gate through `checkPoint`**, so releasing at
  a cursor that sits in a native panel is refused. Pass the same `{axRef}` to `pointer_up`, or use
  `keyboard_release_all`.
- Native queries deliberately skip `ensureCalibrated()`: AX coordinates are global display points
  and owe nothing to the window geometry, and the case they exist for (a panel in front) is exactly
  when the WebView bridge is least trustworthy.

**Why:** each of these is a silent-wrong-answer hazard — a settle that never goes quiet, a pool a
generation behind, a gate bypassed by an offset.
**How to apply:** when touching the native MCP tools, the AX pool, or either settle lane.
