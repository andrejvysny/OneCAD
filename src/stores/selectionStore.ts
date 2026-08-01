/*
 * Selection store (F-WP3).
 *
 * Frontend selection is a list of opaque entity refs plus a transient hover.
 * V1 shell only needs body / sketch / feature refs (the tree + inspector);
 * face / edge / vertex are included in the union for the picking WP that will
 * promote (snapshot, BodyId, TopoKey) tokens into real element refs later.
 */
import { createStore, useStore } from "zustand";

export type EntityKind =
  | "body"
  | "sketch"
  | "sketchRegion"
  /** A datum (reference) plane — a tree row, a viewport quad, a sketch host. */
  | "datum"
  | "feature"
  | "face"
  | "edge"
  | "vertex";

/**
 * Where a face/edge pick landed. Captured on click for a future
 * `AcquireElementIds` promotion (SCHEMA §7.5: frontend sends {topoKey, anchor},
 * Rust mints the ElementId).
 */
export interface SelectionAnchor {
  worldPoint: [number, number, number];
  surfaceUv?: [number, number];
}

interface EntityRefBase {
  /** Stable id. For face/edge this is the composite `${bodyId}#${topoKey}`. */
  id: string;
  /** Owning body (face/edge picks). */
  bodyId?: string;
  /** Snapshot-scoped TopoKey (`"f:22"` / `"e:5"`) for face/edge picks. */
  topoKey?: string;
  /** Persistent ElementId when already minted (else promoted on demand). */
  elementId?: string;
  /** Pick anchor, for AcquireElementIds promotion. */
  anchor?: SelectionAnchor;
}

export interface SketchRegionRef extends EntityRefBase {
  kind: "sketchRegion";
  /** Owning sketch. Kept explicit: callers must never recover it by parsing `id`. */
  sketchId: string;
  /** Exact worker-authored planar-cell identity. */
  regionId: string;
}

interface NonRegionEntityRef extends EntityRefBase {
  kind: Exclude<EntityKind, "sketchRegion">;
}

export type EntityRef = SketchRegionRef | NonRegionEntityRef;

export function sameRef(a: EntityRef, b: EntityRef): boolean {
  return a.kind === b.kind && a.id === b.id;
}

/** Collision-free local identity only; consumers use the explicit sketch/region fields. */
export function sketchRegionRefId(sketchId: string, regionId: string): string {
  return JSON.stringify([sketchId, regionId]);
}

export function sketchRegionRef(sketchId: string, regionId: string): SketchRegionRef {
  return {
    kind: "sketchRegion",
    id: sketchRegionRefId(sketchId, regionId),
    sketchId,
    regionId,
  };
}

/** Composite id for a face/edge ref: `${bodyId}#${topoKey}`. */
export function topoRefId(bodyId: string, topoKey: string): string {
  return `${bodyId}#${topoKey}`;
}

/**
 * The bodies a selection refers to, de-duplicated in pick order.
 *
 * A `body` ref IS its body; a face/edge/vertex pick carries its owning `bodyId`.
 * Sketch / sketchRegion / feature / datum refs have NO body and are skipped —
 * they must never fall back to "all bodies", because the callers (zoom-to-
 * selection, isolate) distinguish "nothing body-shaped is selected" from "these
 * bodies" and decide the fallback themselves.
 */
export function selectedBodyIds(refs: readonly EntityRef[]): string[] {
  const out: string[] = [];
  for (const ref of refs) {
    const id =
      ref.kind === "body"
        ? ref.id
        : ref.kind === "face" || ref.kind === "edge" || ref.kind === "vertex"
          ? ref.bodyId
          : undefined;
    if (id && !out.includes(id)) out.push(id);
  }
  return out;
}

export interface SelectionState {
  hover: EntityRef | null;
  selected: EntityRef[];
  set(refs: EntityRef[]): void;
  toggle(ref: EntityRef): void;
  clear(): void;
  setHover(ref: EntityRef | null): void;
}

export const selectionStore = createStore<SelectionState>()((set) => ({
  // Prototype 1c opens with Sketch 2 selected.
  selected: [{ kind: "sketch", id: "sketch2" }],
  hover: null,

  set(refs) {
    set({ selected: refs });
  },

  toggle(ref) {
    set((s) => {
      const has = s.selected.some((r) => sameRef(r, ref));
      return {
        selected: has
          ? s.selected.filter((r) => !sameRef(r, ref))
          : [...s.selected, ref],
      };
    });
  },

  clear() {
    set({ selected: [] });
  },

  setHover(ref) {
    set({ hover: ref });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useSelectionStore<T>(selector: (s: SelectionState) => T): T {
  return useStore(selectionStore, selector);
}

/** Primary (single) selection — the inspector reads this. */
export function primarySelection(s: SelectionState): EntityRef | null {
  return s.selected[0] ?? null;
}
