/*
 * documentStore seeding gate: under a real Tauri webview the store starts EMPTY
 * (hydrated by backend `projection-updated`); in a plain browser / vitest it keeps
 * the `seedMockDocument()` demo so the mock-driven UI + suites render.
 *
 * The initial projection is chosen at module-init from `window.__TAURI_INTERNALS__`,
 * so each case resets modules and re-imports with the flag toggled.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import {
  documentStore,
  emptyDocument,
  nextDatumName,
  nextSketchName,
  seedMockDocument,
  selectGeometryCached,
} from "./documentStore";
import type { DatumMeta, SketchMeta } from "./documentStore";

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

/*
 * H3 — the rollback cursor an edit RESULT has to derive (results carry no cursor).
 * The gate that reads it refuses to inline-edit a row at or past the bar, so both
 * directions matter: a fresh commit must not look like a draft, and a genuinely
 * rolled-back timeline must not silently become fully applied.
 */
describe("nextAppliedOps", () => {
  it("a fully-applied timeline stays fully applied across an append or a delete", async () => {
    const { nextAppliedOps } = await import("./documentStore");
    expect(nextAppliedOps(5, 5, 6)).toBe(6); // commit a new op
    expect(nextAppliedOps(5, 5, 4)).toBe(4); // delete one
    expect(nextAppliedOps(0, 0, 1)).toBe(1); // first op in an empty document
  });

  it("a ROLLED-BACK timeline keeps its cursor, clamped into the new length", async () => {
    const { nextAppliedOps } = await import("./documentStore");
    expect(nextAppliedOps(2, 5, 6)).toBe(2);
    expect(nextAppliedOps(2, 5, 1)).toBe(1); // clamped — never past the end
  });
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
      datums: {},
      features: [],
      appliedOps: 0,
      // Nothing open ⇒ nothing to paint, and so no geometry-provenance chip.
      geometrySource: "none",
    });
  });
});

// ── Datum planes (DATUM W1) ──────────────────────────────────────────────────

/** Build a datums record from names (the frame is irrelevant to naming). */
function datumsNamed(...names: string[]): Record<string, DatumMeta> {
  const out: Record<string, DatumMeta> = {};
  names.forEach((name, i) => {
    out[`d${i}`] = {
      id: `d${i}`,
      name,
      basePlaneId: "XY",
      offset: 10,
      plane: { kind: "custom", origin: [0, 0, 10], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
      resolvedValid: true,
    };
  });
  return out;
}

describe("nextDatumName", () => {
  it("starts at Datum 1 and steps past the highest existing index", () => {
    expect(nextDatumName({})).toBe("Datum 1");
    expect(nextDatumName(datumsNamed("Datum 1"))).toBe("Datum 2");
    // Non-contiguous: one PAST the max, never the first free slot (monotonic).
    expect(nextDatumName(datumsNamed("Datum 1", "Datum 7"))).toBe("Datum 8");
  });

  it("ignores names that do not match the pattern", () => {
    expect(nextDatumName(datumsNamed("Top face", "Datum 2", "Datum X"))).toBe("Datum 3");
  });
});

describe("documentStore datum registry", () => {
  afterEach(() => {
    documentStore.getState().applySnapshot(seedMockDocument());
  });

  it("adds, replaces and removes datums without touching sketches/bodies", () => {
    const store = documentStore.getState();
    const before = store.sketches;
    const [d0] = Object.values(datumsNamed("Datum 1"));
    store.addDatum(d0);
    expect(documentStore.getState().datums.d0.name).toBe("Datum 1");
    expect(documentStore.getState().sketches).toBe(before);

    store.addDatum({ ...d0, name: "Renamed" });
    expect(documentStore.getState().datums.d0.name).toBe("Renamed");

    store.removeDatum("d0");
    expect(documentStore.getState().datums).toEqual({});
    // Idempotent — removing an absent datum is a no-op, not a throw.
    expect(() => store.removeDatum("nope")).not.toThrow();
  });

  it("the seeded demo document and the empty projection both start with no datums", () => {
    expect(seedMockDocument().datums).toEqual({});
    expect(emptyDocument().datums).toEqual({});
  });
});

// ── geometrySource (persisted-cache lane) ─────────────────────────────────────

describe("geometrySource", () => {
  afterEach(() => documentStore.getState().applySnapshot(seedMockDocument()));

  it("the empty projection has nothing to paint; the demo seed is genuinely live", () => {
    // The seed synthesizes its meshes locally — it never came out of a container,
    // so it must not put the mock lane (or e2e) under a "Last saved geometry" chip.
    expect(emptyDocument().geometrySource).toBe("none");
    expect(seedMockDocument().geometrySource).toBe("live");
  });

  it("selectGeometryCached is true for 'cached' ONLY", () => {
    const store = documentStore.getState();
    store.applySnapshot({ ...seedMockDocument(), geometrySource: "cached" });
    expect(selectGeometryCached(documentStore.getState())).toBe(true);

    for (const source of ["live", "none"] as const) {
      store.applySnapshot({ ...seedMockDocument(), geometrySource: source });
      expect(selectGeometryCached(documentStore.getState())).toBe(false);
    }
  });

  it("applyChange can flip it on its own (a regen publishes without a full snapshot)", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), geometrySource: "cached" });
    documentStore.getState().applyChange({ geometrySource: "live" });
    expect(selectGeometryCached(documentStore.getState())).toBe(false);
  });
});
