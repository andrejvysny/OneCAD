/*
 * documentStore seeding gate: under a real Tauri webview the store starts EMPTY
 * (hydrated by backend `projection-updated`); in a plain browser / vitest it keeps
 * the `seedMockDocument()` demo so the mock-driven UI + suites render.
 *
 * The initial projection is chosen at module-init from `window.__TAURI_INTERNALS__`,
 * so each case resets modules and re-imports with the flag toggled.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { documentStore, nextSketchName, seedMockDocument } from "./documentStore";
import type { SketchMeta } from "./documentStore";

const TAURI = "__TAURI_INTERNALS__";

/** Build a sketches record from names (other SketchMeta fields are irrelevant here). */
function sketchesNamed(...names: string[]): Record<string, SketchMeta> {
  const out: Record<string, SketchMeta> = {};
  names.forEach((name, i) => {
    out[`s${i}`] = {
      id: `s${i}`,
      name,
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: `geometry-${i}`,
    };
  });
  return out;
}

describe("nextSketchName", () => {
  it("starts at Sketch 1 when there are no sketches", () => {
    expect(nextSketchName({})).toBe("Sketch 1");
  });

  it("returns one past the highest existing index (2 / 4 / 5 → 6)", () => {
    expect(nextSketchName(sketchesNamed("Sketch 2", "Sketch 4", "Sketch 5"))).toBe("Sketch 6");
  });

  it("ignores names that don't match the `Sketch <n>` pattern", () => {
    expect(nextSketchName(sketchesNamed("Base plate", "Sketch 3", "Profile"))).toBe("Sketch 4");
    // No matching names at all → Sketch 1.
    expect(nextSketchName(sketchesNamed("Base plate", "Profile"))).toBe("Sketch 1");
  });
});

describe("documentStore.removeSketch", () => {
  it("removes a sketch and is idempotent (no-op when absent)", () => {
    documentStore.setState(seedMockDocument());
    expect(documentStore.getState().sketches["sketch2"]).toBeDefined();

    documentStore.getState().removeSketch("sketch2");
    expect(documentStore.getState().sketches["sketch2"]).toBeUndefined();
    expect(documentStore.getState().sketches["sketch4"]).toBeDefined(); // others untouched

    // Idempotent: removing an already-absent id (or an unknown one) is a no-op —
    // the sketches record keeps the SAME reference (no spurious set).
    const before = documentStore.getState().sketches;
    documentStore.getState().removeSketch("sketch2");
    documentStore.getState().removeSketch("does-not-exist");
    expect(documentStore.getState().sketches).toBe(before);
  });
});

afterEach(() => {
  delete (window as unknown as Record<string, unknown>)[TAURI];
  vi.resetModules();
});

describe("documentStore seeding gate", () => {
  it("seeds the mock document when NOT under Tauri (browser / vitest)", async () => {
    delete (window as unknown as Record<string, unknown>)[TAURI];
    vi.resetModules();
    const { documentStore } = await import("./documentStore");
    const s = documentStore.getState();
    expect(s.status).toBe("ready");
    expect(s.title).toBe("Bracket v2");
    expect(Object.keys(s.bodies)).toContain("body1");
  });

  it("starts EMPTY under a Tauri webview (awaits backend hydration)", async () => {
    (window as unknown as Record<string, unknown>)[TAURI] = {};
    vi.resetModules();
    const { documentStore } = await import("./documentStore");
    const s = documentStore.getState();
    expect(s.status).toBe("empty");
    expect(s.title).toBe("");
    expect(Object.keys(s.bodies)).toHaveLength(0);
    expect(s.features).toHaveLength(0);
  });

  it("emptyDocument() is the no-document projection", async () => {
    const { emptyDocument } = await import("./documentStore");
    expect(emptyDocument()).toEqual({
      status: "empty",
      revision: 0,
      title: "",
      dirty: false,
      bodies: {},
      sketches: {},
      features: [],
    });
  });
});
