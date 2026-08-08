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

/** Structural subset of `SketchMeta` (documentStore.ts) — kept local so this
 *  module never imports a store, only the two fields the sketch fallback needs. */
export interface ToolApplicabilityContext {
  sketches: Record<string, { id: string; visible: boolean }>;
}

const ENABLED: ToolApplicability = { enabled: true };

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

/**
 * Applicability of `tool` given the current selection (and, for the
 * extrude/revolve document-level fallback, which sketches exist). Called
 * with `Tool` (not just `ModelTool`) for call-site ergonomics — every id this
 * module has no rule for (select/sketch/datum/hole/measure, and every
 * SketchTool id) falls through to always-enabled.
 */
export function getToolApplicability(
  tool: Tool,
  selected: readonly EntityRef[],
  ctx: ToolApplicabilityContext,
): ToolApplicability {
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
      return requireKind(selected, "edge", "Select edges, then Fillet");
    case "boolean":
      return requireKind(
        selected,
        "body",
        "Select the target body, then pick the tool body",
      );
    case "shell":
      return requireKind(selected, "face", "Select faces to remove, then Shell");
    case "offsetFace":
      return offsetFaceApplicability(selected);
    case "linearPattern":
    case "circularPattern":
      return requireKind(selected, "body", "Select a body to pattern");
    case "mirror":
      return requireKind(selected, "body", "Select a body to mirror");
    case "transform":
      return requireKind(selected, "body", "Select a body to move");
    default:
      return ENABLED;
  }
}
