import { describe, it, expect } from "vitest";
import { parseRenderModuleState, serializeRenderModuleState } from "./serialize";
import { createEmptyRenderState, type RenderModuleState } from "./state";
import { createMaterial, mintMaterialId } from "./material";

const MAT_A = "mat_a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
const MAT_B = "mat_b2b2b2b2-b2b2-4b2b-8b2b-b2b2b2b2b2b2";

function fullState(): RenderModuleState {
  const state = createEmptyRenderState();

  const glass = createMaterial("Glass", {
    base: { base_color: [0.9, 0.95, 1], base_metalness: 0 },
    specular: { specular_weight: 1, specular_roughness: 0.05 },
    transmission: { transmission_weight: 0.95, transmission_color: [1, 1, 1] },
    coat: { coat_weight: 0.3, coat_ior: 1.5 },
  });
  const steel = createMaterial("Brushed Steel", {
    base: { base_color: [0.6, 0.6, 0.63], base_metalness: 1, base_diffuse_roughness: 0.4 },
    fuzz: { fuzz_weight: 0.1 },
  });

  state.library[glass.id] = glass;
  state.library[steel.id] = steel;
  state.assignments.bodies["body-1"] = glass.id;
  state.assignments.bodies["body-2"] = steel.id;
  state.assignments.faces["el_face-7"] = steel.id;

  return state;
}

describe("serialize round trip", () => {
  it("deep-equals the original state through parse(serialize(x)) for a full multi-layer state", () => {
    const state = fullState();
    const wire = serializeRenderModuleState(state);
    const { state: parsed, warnings } = parseRenderModuleState(wire);
    expect(warnings).toEqual([]);
    expect(parsed).toEqual(state);
  });

  it("round-trips the empty state", () => {
    const state = createEmptyRenderState();
    const { state: parsed, warnings } = parseRenderModuleState(serializeRenderModuleState(state));
    expect(warnings).toEqual([]);
    expect(parsed).toEqual(state);
  });

  it("serializes to a plain JSON-safe value (no live references back into state)", () => {
    const state = fullState();
    const wire = serializeRenderModuleState(state) as RenderModuleState;
    wire.library = {};
    expect(Object.keys(state.library).length).toBeGreaterThan(0);
  });
});

describe("unknown-key preservation", () => {
  it("preserves an unknown root-level key across a round trip", () => {
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {},
      assignments: { bodies: {}, faces: {} },
      futureRootField: { nested: true, value: 42 },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(warnings).toEqual([]);
    expect((state as unknown as Record<string, unknown>).futureRootField).toEqual({
      nested: true,
      value: 42,
    });

    const roundTripped = parseRenderModuleState(serializeRenderModuleState(state!));
    expect((roundTripped.state as unknown as Record<string, unknown>).futureRootField).toEqual({
      nested: true,
      value: 42,
    });
  });

  it("preserves an unknown material-level key across a round trip", () => {
    const id = mintMaterialId();
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {
        [id]: { id, name: "Mystery", base: {}, futureMaterialField: "keep-me" },
      },
      assignments: { bodies: {}, faces: {} },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(warnings).toEqual([]);
    expect((state!.library[id] as unknown as Record<string, unknown>).futureMaterialField).toBe(
      "keep-me",
    );

    const roundTripped = parseRenderModuleState(serializeRenderModuleState(state!));
    expect(
      (roundTripped.state!.library[id] as unknown as Record<string, unknown>).futureMaterialField,
    ).toBe("keep-me");
  });

  it("preserves an unknown layer-level key across a round trip", () => {
    const id = mintMaterialId();
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {
        [id]: {
          id,
          name: "Mystery Layer",
          base: { base_metalness: 0.4, futureBaseField: "kept" },
          specular: { specular_weight: 1, futureSpecularField: 7 },
        },
      },
      assignments: { bodies: {}, faces: {} },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(warnings).toEqual([]);
    const mat = state!.library[id] as unknown as {
      base: Record<string, unknown>;
      specular: Record<string, unknown>;
    };
    expect(mat.base.futureBaseField).toBe("kept");
    expect(mat.specular.futureSpecularField).toBe(7);

    const roundTripped = parseRenderModuleState(serializeRenderModuleState(state!));
    const rtMat = roundTripped.state!.library[id] as unknown as {
      base: Record<string, unknown>;
      specular: Record<string, unknown>;
    };
    expect(rtMat.base.futureBaseField).toBe("kept");
    expect(rtMat.specular.futureSpecularField).toBe(7);
  });
});

describe("schemaVersion gate", () => {
  it("refuses schemaVersion 2 with state null and a warning, touching nothing", () => {
    const raw = { schemaVersion: 2, library: { x: { garbage: true } } };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(state).toBeNull();
    expect(warnings.length).toBeGreaterThan(0);
    expect(warnings[0]).toMatch(/schemaVersion/);
  });

  it("refuses a missing schemaVersion", () => {
    const { state, warnings } = parseRenderModuleState({ library: {}, assignments: {} });
    expect(state).toBeNull();
    expect(warnings.length).toBeGreaterThan(0);
  });

  it("refuses a non-object root", () => {
    expect(parseRenderModuleState("not-an-object").state).toBeNull();
    expect(parseRenderModuleState(null).state).toBeNull();
    expect(parseRenderModuleState([1, 2, 3]).state).toBeNull();
  });
});

describe("tolerant material/assignment validation", () => {
  it("drops a structurally broken material (no id) with a warning, keeping the rest of the library intact", () => {
    const goodId = mintMaterialId();
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {
        broken: { name: "No Id", base: {} },
        [goodId]: { id: goodId, name: "Good", base: { base_metalness: 0.5 } },
      },
      assignments: { bodies: {}, faces: {} },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(state).not.toBeNull();
    expect(Object.keys(state!.library)).toEqual([goodId]);
    expect(state!.library[goodId].name).toBe("Good");
    expect(warnings.some((w) => w.includes("broken"))).toBe(true);
  });

  it("drops a material missing its base layer, and one with an invalid name", () => {
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {
        noBase: { id: mintMaterialId(), name: "No Base" },
        badName: { id: mintMaterialId(), name: 42, base: {} },
      },
      assignments: { bodies: {}, faces: {} },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(Object.keys(state!.library)).toEqual([]);
    expect(warnings).toHaveLength(2);
  });

  it("drops an invalid known field within a layer but keeps the rest of the material", () => {
    const id = mintMaterialId();
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {
        [id]: {
          id,
          name: "Partially Bad",
          base: { base_metalness: "not-a-number", base_diffuse_roughness: 0.2 },
        },
      },
      assignments: { bodies: {}, faces: {} },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(state!.library[id]).toBeDefined();
    expect(state!.library[id].base.base_metalness).toBeUndefined();
    expect(state!.library[id].base.base_diffuse_roughness).toBe(0.2);
    expect(warnings.some((w) => w.includes("base_metalness"))).toBe(true);
  });

  it("drops a non-object optional layer but keeps the material", () => {
    const id = mintMaterialId();
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {
        [id]: { id, name: "Weird Coat", base: {}, coat: "not-an-object" },
      },
      assignments: { bodies: {}, faces: {} },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(state!.library[id].coat).toBeUndefined();
    expect(warnings.some((w) => w.includes("coat"))).toBe(true);
  });

  it("drops an invalid Color3 field", () => {
    const id = mintMaterialId();
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {
        [id]: { id, name: "Bad Color", base: { base_color: [1, 2] } },
      },
      assignments: { bodies: {}, faces: {} },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(state!.library[id].base.base_color).toBeUndefined();
    expect(warnings.some((w) => w.includes("base_color"))).toBe(true);
  });

  it("drops a broken assignment value but keeps the rest", () => {
    const raw = {
      schemaVersion: 1,
      openPbrRevision: "1.1.1",
      library: {},
      assignments: {
        bodies: { "body-1": MAT_A, "body-2": 12345 },
        faces: { "el_1": "not-a-material-id", "el_2": MAT_B },
      },
    };
    const { state, warnings } = parseRenderModuleState(raw);
    expect(state!.assignments.bodies).toEqual({ "body-1": MAT_A });
    expect(state!.assignments.faces).toEqual({ "el_2": MAT_B });
    expect(warnings.some((w) => w.includes("body-2"))).toBe(true);
    expect(warnings.some((w) => w.includes("el_1"))).toBe(true);
  });

  it("treats a missing library/assignments as empty, without warnings", () => {
    const { state, warnings } = parseRenderModuleState({ schemaVersion: 1, openPbrRevision: "1.1.1" });
    expect(state!.library).toEqual({});
    expect(state!.assignments).toEqual({ bodies: {}, faces: {} });
    expect(warnings).toEqual([]);
  });
});
