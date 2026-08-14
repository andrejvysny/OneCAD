/*
 * Render-mode registry integrity.
 *
 * The table is the only place a mode is defined, so the invariants worth pinning
 * are structural: every id has a self-consistent descriptor, the cycle order
 * enumerates the table exactly once, and an unknown id can never escape as one.
 */
import { describe, it, expect } from "vitest";
import {
  coerceRenderMode,
  DEFAULT_RENDER_MODE,
  MATERIAL_KIND_VERTEX_COLORS,
  RENDER_MODES,
  RENDER_MODE_ORDER,
  vertexColorKind,
  type MaterialKind,
  type RenderModeId,
} from "./renderModes";

describe("RENDER_MODES", () => {
  it("keys its own descriptors — def.id always matches the table key", () => {
    for (const [key, def] of Object.entries(RENDER_MODES)) {
      expect(def.id).toBe(key);
    }
  });

  it("carries the exact display-mode button labels", () => {
    expect(RENDER_MODES.shaded.label).toBe("Shaded");
    expect(RENDER_MODES.shadedEdges.label).toBe("Shaded + edges");
    expect(RENDER_MODES.wireframe.label).toBe("Wireframe");
    expect(RENDER_MODES.assemblyColors.label).toBe("Assembly colors");
  });

  it("face/edge visibility: shaded = faces · shadedEdges = both · wireframe = edges · assemblyColors = both", () => {
    const vis = (id: RenderModeId) => [RENDER_MODES[id].faceVisible, RENDER_MODES[id].edgeVisible];
    expect(vis("shaded")).toEqual([true, false]);
    expect(vis("shadedEdges")).toEqual([true, true]);
    expect(vis("wireframe")).toEqual([false, true]);
    expect(vis("assemblyColors")).toEqual([true, true]);
  });
});

describe("vertexColorKind (per-body FACE_COLORS substitution)", () => {
  const KINDS: MaterialKind[] = ["standard", "shadedVertex", "assemblyColor"];

  it("maps every kind to a vertex-colored kind, and is idempotent", () => {
    for (const kind of KINDS) {
      const swapped = vertexColorKind(kind);
      expect(MATERIAL_KIND_VERTEX_COLORS[swapped]).toBe(true);
      expect(vertexColorKind(swapped)).toBe(swapped);
    }
  });

  it("no MODE names a vertex-colored kind — it is a per-body substitution only", () => {
    for (const def of Object.values(RENDER_MODES)) {
      expect(MATERIAL_KIND_VERTEX_COLORS[def.materialKind]).toBe(false);
    }
  });

  it("assemblyColor maps to the vertex-colored variant so imported face colors survive", () => {
    expect(vertexColorKind("assemblyColor")).toBe("shadedVertex");
    expect(MATERIAL_KIND_VERTEX_COLORS["assemblyColor"]).toBe(false);
  });

  it("covers every kind (a new kind with no entry would be undefined at runtime)", () => {
    expect(Object.keys(MATERIAL_KIND_VERTEX_COLORS).sort()).toEqual([...KINDS].sort());
    for (const kind of KINDS) expect(vertexColorKind(kind)).toBeDefined();
  });
});

describe("RENDER_MODE_ORDER", () => {
  it("covers every mode exactly once", () => {
    const keys = Object.keys(RENDER_MODES).sort();
    expect([...RENDER_MODE_ORDER].sort()).toEqual(keys);
    expect(new Set(RENDER_MODE_ORDER).size).toBe(RENDER_MODE_ORDER.length);
  });

  it("contains the default mode", () => {
    expect(RENDER_MODE_ORDER).toContain(DEFAULT_RENDER_MODE);
  });
});

describe("coerceRenderMode", () => {
  it("passes a valid id through unchanged", () => {
    for (const id of RENDER_MODE_ORDER) expect(coerceRenderMode(id)).toBe(id);
  });

  it("falls back to the default for anything unknown", () => {
    for (const bad of [undefined, null, "xray", 42, "", {}, "toString"]) {
      expect(coerceRenderMode(bad)).toBe(DEFAULT_RENDER_MODE);
    }
  });
});
