import { describe, expect, it } from "vitest";
import {
  sameRef,
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
