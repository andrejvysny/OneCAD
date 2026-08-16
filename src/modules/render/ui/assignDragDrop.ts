/*
 * Dragging a material out of the library panel and dropping it on the model.
 *
 * WHERE THE LISTENERS LIVE, and why not in `ViewportRoot`. They are
 * capture-phase `dragover`/`drop` on `window`, installed by this module and
 * disposed with it — the same pattern `modules/library/attachmentPicker.ts`
 * uses for its viewport pick gesture, and for the same reason: a viewport
 * affordance owned by a non-modeling module goes through the engine's EXISTING
 * public surfaces, so neither the shell nor the viewport learns that materials
 * exist (ADR-0003). The alternative — an `onDrop` prop on the viewport's own
 * div — would put a render import in `ViewportRoot`, i.e. the editor shell
 * knowing a feature, which is exactly the coupling the platform refactor
 * removes.
 *
 * A window-wide listener has to be certain the drop landed ON the canvas, and
 * `probePick` alone cannot say so: its raycast is not clipped to the canvas
 * rect, so a point over the sidebar still projects a ray and can hit a body
 * behind it. `raycastFromClient` IS clipped (it returns null off-canvas), so it
 * is used as the containment test before the pick is even attempted. Nothing
 * outside the canvas is ever accepted.
 *
 * ROUTING (Fusion's convention):
 *   body hit                  → body assignment, through the override policy
 *   face hit, Alt held        → a face override
 *   face hit, no Alt          → the face's BODY, through the override policy
 * A face whose ElementId has not been promoted yet cannot carry an override, so
 * Alt on one falls back to the body rather than dropping the gesture silently.
 */
import { getViewportEngine } from "@/viewport/engineBridge";

import type { MaterialId } from "../model/material";
import { renderStore } from "../store/renderStore";
import { assignBodyWithOverridePolicy } from "./overridePolicy";

/** The `dataTransfer` type a material drag carries. Private to this module. */
export const MATERIAL_DND_TYPE = "application/x-onecad-material";

/**
 * The slice of `DataTransfer` this module uses. Narrowed on purpose: jsdom has
 * no `DataTransfer`, so a test supplies this shape directly.
 */
export interface MaterialDataTransfer {
  readonly types?: readonly string[];
  getData(format: string): string;
  setData(format: string, data: string): void;
  effectAllowed?: string;
  dropEffect?: string;
}

/** Stamp a drag with the material it carries. Also sets the copy effect, which
 *  is what makes the cursor say "this adds, it does not move". */
export function encodeMaterialDrag(dt: MaterialDataTransfer, materialId: MaterialId): void {
  dt.setData(MATERIAL_DND_TYPE, materialId);
  dt.effectAllowed = "copy";
}

/**
 * Whether a drag carries a material — readable during `dragover`, where the
 * browser's protected mode makes `getData` return `""` for every type. Only the
 * type LIST is legible there, which is why this is separate from decoding.
 */
export function carriesMaterial(dt: MaterialDataTransfer | null | undefined): boolean {
  if (!dt) return false;
  const types = dt.types;
  if (types === undefined) return false;
  return Array.from(types).includes(MATERIAL_DND_TYPE);
}

/** The material id a drop carries, or `null` when it carries none. */
export function decodeMaterialDrag(
  dt: MaterialDataTransfer | null | undefined,
): MaterialId | null {
  if (!dt) return null;
  const raw = dt.getData(MATERIAL_DND_TYPE);
  return raw ? raw : null;
}

// ── Routing ──────────────────────────────────────────────────────────────────

/** The part of a `PickHit` this module needs (`viewport/engine/Picker.ts`). */
export interface MaterialDropHit {
  readonly bodyId: string;
  readonly kind: "face" | "edge";
  readonly elementId?: string;
}

export type MaterialDropRoute =
  | { readonly kind: "body"; readonly bodyId: string }
  | { readonly kind: "face"; readonly bodyId: string; readonly elementId: string }
  | { readonly kind: "none" };

/**
 * PURE. What a hit + modifier means, with no store, engine or document in
 * sight — the whole decision this file makes, testable on its own.
 *
 * An EDGE hit routes to its body: an edge wears no material of its own, and
 * refusing the drop because the pointer was a few pixels off a face boundary
 * would read as a broken gesture.
 */
export function routeMaterialDrop(hit: MaterialDropHit | null, alt: boolean): MaterialDropRoute {
  if (hit === null) return { kind: "none" };
  if (alt && hit.kind === "face" && hit.elementId) {
    return { kind: "face", bodyId: hit.bodyId, elementId: hit.elementId };
  }
  return { kind: "body", bodyId: hit.bodyId };
}

export interface MaterialDropDeps {
  /** True when the point is over the viewport canvas. */
  onCanvas(clientX: number, clientY: number): boolean;
  /** One-shot pick at a client point (`ViewportEngine.probePick`). */
  pick(clientX: number, clientY: number): MaterialDropHit | null;
  assignBody(bodyId: string, materialId: MaterialId): Promise<unknown>;
  assignFace(faceElementId: string, materialId: MaterialId): Promise<unknown>;
}

export interface MaterialDropEvent {
  readonly clientX: number;
  readonly clientY: number;
  readonly altKey: boolean;
  readonly materialId: MaterialId;
}

/** Resolve a drop and apply it. Returns what it did, for tests and diagnostics. */
export async function applyMaterialDrop(
  event: MaterialDropEvent,
  deps: MaterialDropDeps,
): Promise<MaterialDropRoute> {
  if (!deps.onCanvas(event.clientX, event.clientY)) return { kind: "none" };
  const route = routeMaterialDrop(deps.pick(event.clientX, event.clientY), event.altKey);
  if (route.kind === "body") await deps.assignBody(route.bodyId, event.materialId);
  else if (route.kind === "face") await deps.assignFace(route.elementId, event.materialId);
  return route;
}

// ── Wiring ───────────────────────────────────────────────────────────────────

/**
 * The live dependencies. The engine is read through `engineBridge` on every
 * call, never captured: it is re-created on remount (React 19 StrictMode
 * double-invokes mount effects) and a captured handle would keep picking
 * against a disposed scene.
 *
 * `engineBridge` itself is a cheap import — its only reference to the engine is
 * type-only — so this costs no WebGL-shaped module graph.
 */
export function liveMaterialDropDeps(): MaterialDropDeps {
  return {
    onCanvas: (x, y) => getViewportEngine()?.raycastFromClient(x, y) != null,
    pick: (x, y) => getViewportEngine()?.probePick(x, y) ?? null,
    assignBody: (bodyId, materialId) => assignBodyWithOverridePolicy(bodyId, materialId),
    assignFace: (elementId, materialId) =>
      renderStore.getState().assignFace(elementId, materialId),
  };
}

/**
 * Install the viewport drop target. Returns the teardown.
 *
 * `dragover` must `preventDefault()` for a drop to be delivered at all, and it
 * does so ONLY for a drag carrying our own type over the canvas — every other
 * drag on the page, and this drag anywhere else on it, behaves exactly as it
 * did before this existed.
 */
export function installMaterialDropTarget(
  target: Pick<Window, "addEventListener" | "removeEventListener"> = window,
  deps: () => MaterialDropDeps = liveMaterialDropDeps,
): () => void {
  const onDragOver = (e: DragEvent): void => {
    if (!carriesMaterial(e.dataTransfer)) return;
    if (!deps().onCanvas(e.clientX, e.clientY)) return;
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    e.preventDefault();
  };

  const onDrop = (e: DragEvent): void => {
    const materialId = decodeMaterialDrag(e.dataTransfer);
    if (materialId === null) return;
    const d = deps();
    if (!d.onCanvas(e.clientX, e.clientY)) return;
    e.preventDefault();
    void applyMaterialDrop(
      { clientX: e.clientX, clientY: e.clientY, altKey: e.altKey, materialId },
      d,
    );
  };

  target.addEventListener("dragover", onDragOver as EventListener, true);
  target.addEventListener("drop", onDrop as EventListener, true);
  return () => {
    target.removeEventListener("dragover", onDragOver as EventListener, true);
    target.removeEventListener("drop", onDrop as EventListener, true);
  };
}
