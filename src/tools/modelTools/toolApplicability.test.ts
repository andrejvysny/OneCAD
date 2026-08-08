import { describe, it, expect } from "vitest";
import { getToolApplicability, resolveTargetSketchId, type ToolApplicabilityContext } from "./toolApplicability";
import { sketchRegionRef, type EntityRef } from "@/stores/selectionStore";

const edge: EntityRef = { kind: "edge", id: "e1", bodyId: "body1" };
const face = (id: string, bodyId?: string): EntityRef => ({ kind: "face", id, bodyId });
const body = (id: string): EntityRef => ({ kind: "body", id });
const vertex: EntityRef = { kind: "vertex", id: "v1", bodyId: "body1" };
const sketch = (id: string): EntityRef => ({ kind: "sketch", id });

const NO_SKETCHES: ToolApplicabilityContext = { sketches: {} };
const ONE_VISIBLE: ToolApplicabilityContext = {
  sketches: { sketch1: { id: "sketch1", visible: true } },
};
const TWO_VISIBLE: ToolApplicabilityContext = {
  sketches: {
    sketch1: { id: "sketch1", visible: true },
    sketch2: { id: "sketch2", visible: true },
  },
};

describe("resolveTargetSketchId", () => {
  it("no sketch anywhere ⇒ null", () => {
    expect(resolveTargetSketchId([], NO_SKETCHES)).toBeNull();
  });

  it("exactly one visible sketch, none selected ⇒ that id", () => {
    expect(resolveTargetSketchId([], ONE_VISIBLE)).toBe("sketch1");
  });

  it("an explicit sketch selection wins over the sole-visible fallback", () => {
    expect(resolveTargetSketchId([sketch("sketch2")], ONE_VISIBLE)).toBe("sketch2");
  });

  it("2+ visible sketches, none selected ⇒ null (ambiguous)", () => {
    expect(resolveTargetSketchId([], TWO_VISIBLE)).toBeNull();
  });
});

describe("getToolApplicability — extrude", () => {
  it("empty selection, no sketch ⇒ disabled", () => {
    const v = getToolApplicability("extrude", [], NO_SKETCHES);
    expect(v).toEqual({ enabled: false, reason: "Select a sketch region (or a sketch) to extrude" });
  });

  it("wrong-kind selection (an edge) ⇒ disabled, same reason as empty", () => {
    const v = getToolApplicability("extrude", [edge], NO_SKETCHES);
    expect(v.enabled).toBe(false);
    expect(v.reason).toBe("Select a sketch region (or a sketch) to extrude");
  });

  it("a sketchRegion selected ⇒ enabled", () => {
    const v = getToolApplicability("extrude", [sketchRegionRef("sketch1", "r1")], NO_SKETCHES);
    expect(v).toEqual({ enabled: true });
  });

  it("no region selected, but a sketch is ⇒ enabled (tool-first fallback)", () => {
    const v = getToolApplicability("extrude", [sketch("sketch1")], NO_SKETCHES);
    expect(v).toEqual({ enabled: true });
  });

  it("no region selected, sole visible sketch ⇒ enabled (document-level fallback)", () => {
    const v = getToolApplicability("extrude", [], ONE_VISIBLE);
    expect(v).toEqual({ enabled: true });
  });

  it("regions selected span >1 sketch ⇒ disabled, error severity", () => {
    const v = getToolApplicability(
      "extrude",
      [sketchRegionRef("sketch1", "r1"), sketchRegionRef("sketch2", "r2")],
      NO_SKETCHES,
    );
    expect(v).toEqual({
      enabled: false,
      reason: "Extrude takes regions from one sketch — deselect the others",
      severity: "error",
    });
  });
});

describe("getToolApplicability — revolve", () => {
  it("empty selection, no sketch ⇒ disabled, revolve's own (non-region) wording", () => {
    const v = getToolApplicability("revolve", [], NO_SKETCHES);
    expect(v).toEqual({ enabled: false, reason: "Select a sketch to revolve" });
  });

  it("a sketchRegion selected ⇒ enabled", () => {
    const v = getToolApplicability("revolve", [sketchRegionRef("sketch1", "r1")], NO_SKETCHES);
    expect(v).toEqual({ enabled: true });
  });

  it("regions selected span >1 sketch ⇒ disabled, error severity", () => {
    const v = getToolApplicability(
      "revolve",
      [sketchRegionRef("sketch1", "r1"), sketchRegionRef("sketch2", "r2")],
      NO_SKETCHES,
    );
    expect(v).toEqual({
      enabled: false,
      reason: "Revolve takes regions from one sketch — deselect the others",
      severity: "error",
    });
  });
});

describe("getToolApplicability — fillet", () => {
  it("empty selection ⇒ disabled", () => {
    expect(getToolApplicability("fillet", [], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select edges, then Fillet",
    });
  });

  it("wrong-kind selection (a face) ⇒ disabled, same reason", () => {
    const v = getToolApplicability("fillet", [face("f1", "body1")], NO_SKETCHES);
    expect(v).toEqual({ enabled: false, reason: "Select edges, then Fillet" });
  });

  it("an edge selected ⇒ enabled", () => {
    expect(getToolApplicability("fillet", [edge], NO_SKETCHES)).toEqual({ enabled: true });
  });
});

describe("getToolApplicability — boolean", () => {
  it("empty selection ⇒ disabled", () => {
    expect(getToolApplicability("boolean", [], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select the target body, then pick the tool body",
    });
  });

  it("a body selected ⇒ enabled", () => {
    expect(getToolApplicability("boolean", [body("body1")], NO_SKETCHES)).toEqual({ enabled: true });
  });
});

describe("getToolApplicability — shell", () => {
  it("empty selection ⇒ disabled", () => {
    expect(getToolApplicability("shell", [], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select faces to remove, then Shell",
    });
  });

  it("a face selected ⇒ enabled", () => {
    expect(getToolApplicability("shell", [face("f1", "body1")], NO_SKETCHES)).toEqual({ enabled: true });
  });
});

describe("getToolApplicability — offsetFace", () => {
  it("empty selection ⇒ disabled", () => {
    expect(getToolApplicability("offsetFace", [], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select faces to offset, then Offset face",
    });
  });

  it("a single-body face selected ⇒ enabled", () => {
    expect(getToolApplicability("offsetFace", [face("f1", "body1")], NO_SKETCHES)).toEqual({ enabled: true });
  });

  it("faces spanning 2 bodies ⇒ disabled, error severity", () => {
    const v = getToolApplicability("offsetFace", [face("f1", "body1"), face("f2", "body2")], NO_SKETCHES);
    expect(v).toEqual({
      enabled: false,
      reason: "Offset face: every selected face must belong to the same body",
      severity: "error",
    });
  });

  it("a face with no body ⇒ disabled, error severity", () => {
    const v = getToolApplicability("offsetFace", [face("f1")], NO_SKETCHES);
    expect(v).toEqual({
      enabled: false,
      reason: "Offset face: that selection has no body",
      severity: "error",
    });
  });
});

describe.each(["linearPattern", "circularPattern"] as const)("getToolApplicability — %s", (tool) => {
  it("empty selection ⇒ disabled", () => {
    expect(getToolApplicability(tool, [], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select a body to pattern",
    });
  });

  it("a body selected ⇒ enabled", () => {
    expect(getToolApplicability(tool, [body("body1")], NO_SKETCHES)).toEqual({ enabled: true });
  });

  it("wrong-kind selection (a vertex) ⇒ disabled", () => {
    expect(getToolApplicability(tool, [vertex], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select a body to pattern",
    });
  });
});

describe("getToolApplicability — mirror", () => {
  it("empty selection ⇒ disabled", () => {
    expect(getToolApplicability("mirror", [], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select a body to mirror",
    });
  });

  it("a body selected ⇒ enabled", () => {
    expect(getToolApplicability("mirror", [body("body1")], NO_SKETCHES)).toEqual({ enabled: true });
  });
});

describe("getToolApplicability — transform", () => {
  it("empty selection ⇒ disabled", () => {
    expect(getToolApplicability("transform", [], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select a body to move",
    });
  });

  it("a body selected ⇒ enabled", () => {
    expect(getToolApplicability("transform", [body("body1")], NO_SKETCHES)).toEqual({ enabled: true });
  });

  it("a face selected (owning-body resolvable) is still disabled — the narrow `kind===\"body\"` rule, not the broader owning-body helper", () => {
    // ModelToolController's private `selectedBodyIds()` (what actually arms
    // Transform) only matches whole-body refs, unlike the store's exported
    // `selectedBodyIds(refs)` which resolves a face/edge/vertex to its owning
    // body. The applicability check must replicate the narrower rule so the
    // toolbar and the click never disagree.
    expect(getToolApplicability("transform", [face("f1", "body1")], NO_SKETCHES)).toEqual({
      enabled: false,
      reason: "Select a body to move",
    });
  });
});

describe("getToolApplicability — always-enabled tools", () => {
  it.each(["select", "sketch", "datum", "hole", "measure"] as const)(
    "%s is enabled regardless of selection",
    (tool) => {
      expect(getToolApplicability(tool, [], NO_SKETCHES)).toEqual({ enabled: true });
      expect(getToolApplicability(tool, [edge], NO_SKETCHES)).toEqual({ enabled: true });
    },
  );

  it("a SketchTool id (out of scope, never actually called with these) also falls through to enabled", () => {
    expect(getToolApplicability("line", [], NO_SKETCHES)).toEqual({ enabled: true });
  });
});
