import { describe, expect, it } from "vitest";
import {
  sameRef,
  selectedBodyIds,
  sketchRegionRef,
  sketchRegionRefId,
} from "./selectionStore";

describe("sketchRegionRef", () => {
  it("keeps owning identities explicit and does not depend on parsing id", () => {
    expect(sketchRegionRef("sketch-a", "region-b")).toEqual({
      kind: "sketchRegion",
      id: sketchRegionRefId("sketch-a", "region-b"),
      sketchId: "sketch-a",
      regionId: "region-b",
    });
  });

  it("does not collide when either opaque identity contains a separator", () => {
    const first = sketchRegionRef("a#b", "c");
    const second = sketchRegionRef("a", "b#c");
    expect(first.id).not.toBe(second.id);
    expect(sameRef(first, second)).toBe(false);
  });
});

describe("selectedBodyIds", () => {
  it("takes a body ref's own id and a face/edge/vertex ref's owning bodyId", () => {
    expect(
      selectedBodyIds([
        { kind: "body", id: "bodyA" },
        { kind: "face", id: "bodyB#f:1", bodyId: "bodyB" },
        { kind: "edge", id: "bodyC#e:0", bodyId: "bodyC" },
        { kind: "vertex", id: "bodyD#v:9", bodyId: "bodyD" },
      ]),
    ).toEqual(["bodyA", "bodyB", "bodyC", "bodyD"]);
  });

  it("de-duplicates in pick order", () => {
    expect(
      selectedBodyIds([
        { kind: "face", id: "bodyB#f:1", bodyId: "bodyB" },
        { kind: "body", id: "bodyA" },
        { kind: "edge", id: "bodyB#e:3", bodyId: "bodyB" },
      ]),
    ).toEqual(["bodyB", "bodyA"]);
  });

  it("ignores refs that name no body rather than falling back to everything", () => {
    expect(
      selectedBodyIds([
        { kind: "sketch", id: "sketch2" },
        sketchRegionRef("sketch2", "r0"),
        { kind: "feature", id: "f3" },
        { kind: "datum", id: "datum1" },
        // A face pick that somehow lost its owner is dropped, not guessed.
        { kind: "face", id: "orphan#f:1" },
      ]),
    ).toEqual([]);
  });
});
