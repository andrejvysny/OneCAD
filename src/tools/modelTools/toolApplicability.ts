/*
 * toolApplicability — PURE precondition matrix for the model-tool floating
 * toolbar (F-WP toolbar-gating). Single source of truth for two consumers:
 * `FloatingToolbar` (gray out + tooltip a tool before it's clicked) and
 * `ModelToolController`'s `arm*FromSelection` guards (the same rule, reused
 * for the post-click gate + status-hint text) — so the button's enabled state
 * and the click's actual outcome can never disagree.
 *
 * Structural precedent: src/tools/sketch/constraintApplicability.ts. Same
 * discipline here — no store/engine/ipc imports, only types, so this stays
 * exhaustively unit-testable as a plain function table.
 *
 * Every rule below is a port of an existing `arm*FromSelection` guard in
 * ModelToolController.ts, not a new design — reason strings are reused
 * verbatim because they double as the disabled-button tooltip text.
 */
import type { EntityRef, SketchRegionRef } from "@/stores/selectionStore";
import type { Tool } from "@/stores/toolStore";

export type ToolApplicabilitySeverity = "info" | "error";

export interface ToolApplicability {
  enabled: boolean;
  /** Present only when `enabled` is false. Reused verbatim as both the
   *  disabled-button tooltip and the setStatusHint message. */
  reason?: string;
  /** Mirrors the severity the equivalent setStatusHint call passes today;
   *  omitted ⇒ "info" (setStatusHint's own default). */
  severity?: ToolApplicabilitySeverity;
}

/** Structural subset of `SketchMeta` / `BodyMeta` (documentStore.ts) — kept local
 *  so this module never imports a store, only the fields the document-level
 *  fallbacks need. `visible` is REQUIRED, not defaulted: Hole and Measure are
 *  gated on a body the user can actually click, and a silent "assume visible"
 *  would re-enable them on exactly the empty document C6 is about. */
export interface ToolApplicabilityContext {
  sketches: Record<string, { id: string; visible: boolean }>;
  bodies?: Record<string, { visible: boolean; health?: "healthy" | "quarantined" }>;
}

const ENABLED: ToolApplicability = { enabled: true };
const BODY_MODELING_TOOLS = new Set<Tool>([
  "fillet",
  "boolean",
  "shell",
  "offsetFace",
  "linearPattern",
  "circularPattern",
  "mirror",
  "transform",
  "hole",
]);

/**
 * Pure port of `ModelToolController.pickTargetSketchId`: an explicitly
 * selected sketch wins; else the document's SOLE visible sketch; else null.
 * Exported so the controller's own method becomes a one-line delegation —
 * single source of truth for both the applicability check and the actual
 * arm target.
 */
export function resolveTargetSketchId(
  selected: readonly EntityRef[],
  ctx: ToolApplicabilityContext,
): string | null {
  const selectedSketch = selected.find((r) => r.kind === "sketch");
  if (selectedSketch) return selectedSketch.id;
  const visible = Object.values(ctx.sketches).filter((s) => s.visible);
  return visible.length === 1 ? visible[0].id : null;
}

function regionApplicability(
  toolLabel: "Extrude" | "Revolve",
  selected: readonly EntityRef[],
  ctx: ToolApplicabilityContext,
  noneSelectedReason: string,
): ToolApplicability {
  const picked = selected.filter(
    (ref): ref is SketchRegionRef => ref.kind === "sketchRegion",
  );
  if (picked.length > 0) {
    const sketchId = picked[0].sketchId;
    if (picked.some((ref) => ref.sketchId !== sketchId)) {
      return {
        enabled: false,
        reason: `${toolLabel} takes regions from one sketch — deselect the others`,
        severity: "error",
      };
    }
    return ENABLED;
  }
  return resolveTargetSketchId(selected, ctx)
    ? ENABLED
    : { enabled: false, reason: noneSelectedReason };
}

function offsetFaceApplicability(selected: readonly EntityRef[]): ToolApplicability {
  const faces = selected.filter((r) => r.kind === "face");
  if (faces.length === 0) {
    return { enabled: false, reason: "Select faces to offset, then Offset face" };
  }
  const bodies = [...new Set(faces.map((f) => f.bodyId ?? ""))];
  if (bodies.length > 1) {
    return {
      enabled: false,
      reason: "Offset face: every selected face must belong to the same body",
      severity: "error",
    };
  }
  if (!bodies[0]) {
    return { enabled: false, reason: "Offset face: that selection has no body", severity: "error" };
  }
  return ENABLED;
}

function requireKind(
  selected: readonly EntityRef[],
  kind: EntityRef["kind"],
  reason: string,
): ToolApplicability {
  return selected.some((r) => r.kind === kind) ? ENABLED : { enabled: false, reason };
}

/** How many bodies the document holds — all of them, or only the visible ones. */
function bodyCount(ctx: ToolApplicabilityContext, visibleOnly: boolean): number {
  const bodies = Object.values(ctx.bodies ?? {});
  return visibleOnly ? bodies.filter((b) => b.visible).length : bodies.length;
}

function selectedBodyIds(selected: readonly EntityRef[]): string[] {
  const ids = new Set<string>();
  for (const ref of selected) {
    if (ref.kind === "body") ids.add(ref.id);
    else if ("bodyId" in ref && typeof ref.bodyId === "string") ids.add(ref.bodyId);
  }
  return [...ids];
}

/**
 * Applicability of `tool` given the current selection (and, for the
 * extrude/revolve document-level fallback, which sketches exist). Called
 * with `Tool` (not just `ModelTool`) for call-site ergonomics — every id this
 * module has no rule for (select/sketch/datum/gear, and every SketchTool id)
 * falls through to always-enabled.
 */
export function getToolApplicability(
  tool: Tool,
  selected: readonly EntityRef[],
  ctx: ToolApplicabilityContext,
): ToolApplicability {
  if (BODY_MODELING_TOOLS.has(tool)) {
    const quarantined = selectedBodyIds(selected).find(
      (id) => ctx.bodies?.[id]?.health === "quarantined",
    );
    if (quarantined) {
      return {
        enabled: false,
        reason: "Quarantined imported geometry is view/export-only until repaired",
        severity: "error",
      };
    }
  }
  switch (tool) {
    case "extrude":
      return regionApplicability(
        "Extrude",
        selected,
        ctx,
        "Select a sketch region (or a sketch) to extrude",
      );
    case "revolve":
      return regionApplicability("Revolve", selected, ctx, "Select a sketch to revolve");
    case "fillet":
      // C7 / D9: a FACE selection arms too — the controller expands it to that
      // face's boundary edges. Most CAD lets you fillet every edge of a face,
      // and a disabled button with an edge that cannot be hit was the whole of
      // T8's dead end. A BODY selection stays disabled: "fillet this body"
      // has no defensible edge set, so it would be a guess.
      if (selected.some((r) => r.kind === "edge" || r.kind === "face")) return ENABLED;
      return {
        enabled: false,
        // A BODY is the one selection that LOOKS like it should work, so it gets
        // the corrective wording; every other state (including none) gets the
        // full "…, then Fillet" call to action.
        reason: selected.some((r) => r.kind === "body")
          ? "Select edges or a face"
          : "Select edges or a face, then Fillet",
      };
    case "boolean":
      // C6: Combine takes a target and a tool body, so one body in the document
      // can never satisfy it however it is selected.
      if (bodyCount(ctx, false) < 2) {
        return { enabled: false, reason: "Two bodies are needed to combine" };
      }
      return requireKind(
        selected,
        "body",
        "Select the target body, then pick the tool body",
      );
    // C6: both were falling through to `default: ENABLED`, so an empty document
    // offered a Hole to place on nothing and a Measure with nothing to read.
    // VISIBLE bodies: both tools start by clicking geometry on screen.
    case "hole":
      return bodyCount(ctx, true) > 0
        ? ENABLED
        : { enabled: false, reason: "Add a body first" };
    case "measure":
      return bodyCount(ctx, true) > 0
        ? ENABLED
        : { enabled: false, reason: "Nothing to measure" };
    case "shell":
      return requireKind(selected, "face", "Select faces to remove, then Shell");
    case "offsetFace":
      return offsetFaceApplicability(selected);
    case "linearPattern":
    case "circularPattern":
      return requireKind(
        selected,
        "body",
        "Select a body to pattern — feature patterns are not supported yet",
      );
    case "mirror":
      return requireKind(selected, "body", "Select a body to mirror");
    case "transform":
      return requireKind(selected, "body", "Select a body to move");
    default:
      return ENABLED;
  }
}
