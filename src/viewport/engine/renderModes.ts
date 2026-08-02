/*
 * Render-mode registry — the single descriptor table every body render mode is
 * driven from.
 *
 * A mode is DATA, not a switch statement: `faceVisible` / `edgeVisible` say
 * which of a BodyObject's two children draw, and `materialKind` says which
 * shared material set they draw with. Adding a mode (xray, ghost, …) is a new
 * entry here plus, if it needs different shading, a new {@link MaterialKind}
 * that BodyMaterialLibrary knows how to build — no per-mode branching in the
 * BodyObject, the ingest controller, or the store.
 *
 * `label` lives here too so the chrome (display-mode button) and the engine can
 * never disagree about what a mode is called.
 */

/**
 * Which shared material set a mode draws with. One kind today; future modes
 * add a kind rather than mutating the standard set (mutation would leak across
 * every body sharing it).
 */
export type MaterialKind = "standard";

export type RenderModeId = "shaded" | "shadedEdges" | "wireframe";

export interface RenderModeDef {
  id: RenderModeId;
  label: string;
  faceVisible: boolean;
  edgeVisible: boolean;
  materialKind: MaterialKind;
}

export const RENDER_MODES: Record<RenderModeId, RenderModeDef> = {
  shaded: {
    id: "shaded",
    label: "Shaded",
    faceVisible: true,
    edgeVisible: false,
    materialKind: "standard",
  },
  shadedEdges: {
    id: "shadedEdges",
    label: "Shaded + edges",
    faceVisible: true,
    edgeVisible: true,
    materialKind: "standard",
  },
  wireframe: {
    id: "wireframe",
    label: "Wireframe",
    faceVisible: false,
    edgeVisible: true,
    materialKind: "standard",
  },
};

/** Cycle order for the display-mode button (also the canonical id enumeration). */
export const RENDER_MODE_ORDER: readonly RenderModeId[] = ["shaded", "shadedEdges", "wireframe"];

/**
 * `shadedEdges` is what the renderer has always actually drawn (a BodyObject
 * carries a face Mesh AND an edge LineSegments), so it is the honest default —
 * and it makes the first click on the display button visibly change something.
 */
export const DEFAULT_RENDER_MODE: RenderModeId = "shadedEdges";

/**
 * Narrow an untrusted value (persisted setting, URL param, older document) to a
 * known mode id, falling back to the default rather than throwing — an unknown
 * mode must never leave the viewport with no way to draw a body.
 */
export function coerceRenderMode(v: unknown): RenderModeId {
  if (typeof v === "string" && Object.prototype.hasOwnProperty.call(RENDER_MODES, v)) {
    return v as RenderModeId;
  }
  return DEFAULT_RENDER_MODE;
}
