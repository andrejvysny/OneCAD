import { describe, it, expect, beforeEach } from "vitest";
import { sketchSelectionStore, sameSketchSel, type SketchSel } from "./sketchSelectionStore";

const S = sketchSelectionStore.getState;

beforeEach(() => {
  sketchSelectionStore.setState({
    selected: [],
    hover: null,
    constraintHover: null,
    selectedConstraintId: null,
  });
});

describe("sketchSelectionStore.set", () => {
  it("replaces the whole selection", () => {
    S().set([{ entityId: "e1" }, { entityId: "e2", point: "Start" }]);
    expect(S().selected).toEqual([{ entityId: "e1" }, { entityId: "e2", point: "Start" }]);
    S().set([{ entityId: "e3" }]);
    expect(S().selected).toEqual([{ entityId: "e3" }]);
  });
});

describe("sketchSelectionStore.toggle (identity = entityId + point)", () => {
  it("adds a pick not already selected", () => {
    S().toggle({ entityId: "e1" });
    expect(S().selected).toEqual([{ entityId: "e1" }]);
  });

  it("removes a pick that is already selected", () => {
    S().set([{ entityId: "e1", point: "End" }]);
    S().toggle({ entityId: "e1", point: "End" });
    expect(S().selected).toEqual([]);
  });

  it("the SAME entity's body and a vertex are distinct identities", () => {
    S().toggle({ entityId: "e1" }); // body pick
    S().toggle({ entityId: "e1", point: "Start" }); // vertex pick — distinct
    expect(S().selected).toEqual([{ entityId: "e1" }, { entityId: "e1", point: "Start" }]);
    // Toggling the body off leaves the vertex.
    S().toggle({ entityId: "e1" });
    expect(S().selected).toEqual([{ entityId: "e1", point: "Start" }]);
  });

  it("different point on the same entity is a distinct identity", () => {
    S().toggle({ entityId: "e1", point: "Start" });
    S().toggle({ entityId: "e1", point: "End" });
    expect(S().selected).toEqual([
      { entityId: "e1", point: "Start" },
      { entityId: "e1", point: "End" },
    ]);
  });
});

describe("sketchSelectionStore.setHover / setConstraintHover", () => {
  it("sets and clears the hover pick", () => {
    S().setHover({ entityId: "e2", point: "Center" });
    expect(S().hover).toEqual({ entityId: "e2", point: "Center" });
    S().setHover(null);
    expect(S().hover).toBeNull();
  });

  it("sets and clears the constraint hover id", () => {
    S().setConstraintHover("k7");
    expect(S().constraintHover).toBe("k7");
    S().setConstraintHover(null);
    expect(S().constraintHover).toBeNull();
  });
});

describe("sketchSelectionStore.setSelectedConstraint (plan item 5b)", () => {
  it("selects a constraint and CLEARS the entity selection in one write", () => {
    S().set([{ entityId: "e1" }, { entityId: "e2" }]);
    S().setSelectedConstraint("c1");
    expect(S().selectedConstraintId).toBe("c1");
    expect(S().selected).toEqual([]);
  });

  it("clearing the constraint leaves the entity selection alone", () => {
    S().setSelectedConstraint("c1");
    S().setSelectedConstraint(null);
    expect(S().selectedConstraintId).toBeNull();
  });

  it("selecting an entity clears the constraint pick (set AND toggle)", () => {
    S().setSelectedConstraint("c1");
    S().set([{ entityId: "e1" }]);
    expect(S().selectedConstraintId).toBeNull();

    S().setSelectedConstraint("c2");
    S().toggle({ entityId: "e9" });
    expect(S().selectedConstraintId).toBeNull();
  });
});

describe("sketchSelectionStore.clear", () => {
  it("resets selected, hover, constraintHover and selectedConstraintId", () => {
    S().set([{ entityId: "e1" }]);
    S().setHover({ entityId: "e2" });
    S().setConstraintHover("k1");
    S().setSelectedConstraint("k2");
    S().clear();
    expect(S().selected).toEqual([]);
    expect(S().hover).toBeNull();
    expect(S().constraintHover).toBeNull();
    expect(S().selectedConstraintId).toBeNull();
  });
});

describe("sameSketchSel", () => {
  it("matches on entityId AND point", () => {
    const a: SketchSel = { entityId: "e1", point: "Start" };
    expect(sameSketchSel(a, { entityId: "e1", point: "Start" })).toBe(true);
    expect(sameSketchSel(a, { entityId: "e1", point: "End" })).toBe(false);
    expect(sameSketchSel(a, { entityId: "e2", point: "Start" })).toBe(false);
    expect(sameSketchSel({ entityId: "e1" }, { entityId: "e1" })).toBe(true);
    expect(sameSketchSel({ entityId: "e1" }, { entityId: "e1", point: "Start" })).toBe(false);
  });
});
