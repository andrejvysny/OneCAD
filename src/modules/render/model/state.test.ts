import { describe, it, expect } from "vitest";
import {
  CURRENT_RENDER_SCHEMA_VERSION,
  PINNED_OPENPBR_REVISION,
  createEmptyRenderState,
  isEmptyRenderState,
} from "./state";
import { createMaterial } from "./material";

describe("createEmptyRenderState", () => {
  it("carries the pinned schema version and OpenPBR revision", () => {
    const state = createEmptyRenderState();
    expect(state.schemaVersion).toBe(CURRENT_RENDER_SCHEMA_VERSION);
    expect(state.openPbrRevision).toBe(PINNED_OPENPBR_REVISION);
    expect(state.library).toEqual({});
    expect(state.assignments).toEqual({ bodies: {}, faces: {} });
  });
});

describe("isEmptyRenderState", () => {
  it("is true for a freshly created state", () => {
    expect(isEmptyRenderState(createEmptyRenderState())).toBe(true);
  });

  it("is false once a material is in the library", () => {
    const state = createEmptyRenderState();
    const mat = createMaterial("Steel");
    state.library[mat.id] = mat;
    expect(isEmptyRenderState(state)).toBe(false);
  });

  it("is false with a body assignment but an empty library", () => {
    const state = createEmptyRenderState();
    state.assignments.bodies["body-1"] = "mat_a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
    expect(isEmptyRenderState(state)).toBe(false);
  });

  it("is false with a face assignment but an empty library", () => {
    const state = createEmptyRenderState();
    state.assignments.faces["el_1"] = "mat_a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
    expect(isEmptyRenderState(state)).toBe(false);
  });
});
