/*
 * refFromModelHits — arbitrates a body face/edge PickHit against a coplanar
 * sketch-fill SketchStaticHit under the same pointer (W0.4 pick-through).
 *
 * Default (no Alt): a numerically coplanar sketch fill wins the tie
 * (`secondaryHitWins`), pinned by Picker.test.ts:76 and unchanged here.
 * `pickThrough` (Alt held) suppresses that tie-break so the body face/edge
 * underneath the fill becomes selectable — the sketch is only used when
 * there is no body hit at all.
 */
import { describe, it, expect } from "vitest";
import * as THREE from "three";
import { refFromModelHits } from "./ViewportRoot";
import type { PickHit } from "./engine/Picker";
import type { SketchStaticHit } from "./engine/SketchStaticLayer";

function bodyHit(distance = 100): PickHit {
  return {
    bodyId: "body1",
    kind: "face",
    topoKey: "f:0",
    distance,
    worldPos: new THREE.Vector3(0, 0, 0),
  };
}

function sketchHit(distance = 100.0005): SketchStaticHit {
  return { kind: "sketchRegion", sketchId: "sketch1", regionId: "r0", distance };
}

describe("refFromModelHits — pick-through arbitration (W0.4)", () => {
  it("tie + no Alt: the coplanar sketch fill wins (current behavior pinned)", () => {
    const ref = refFromModelHits(bodyHit(), sketchHit(), false);
    expect(ref?.kind).toBe("sketchRegion");
  });

  it("tie + Alt (pickThrough): the body face wins outright", () => {
    const ref = refFromModelHits(bodyHit(), sketchHit(), true);
    expect(ref?.kind).toBe("face");
    expect(ref?.bodyId).toBe("body1");
  });

  it("Alt with only a sketch hit (no body under the pointer): sketch ref", () => {
    const ref = refFromModelHits(null, sketchHit(), true);
    expect(ref?.kind).toBe("sketchRegion");
  });

  it("Alt with only a body hit: body ref", () => {
    const ref = refFromModelHits(bodyHit(), null, true);
    expect(ref?.kind).toBe("face");
    expect(ref?.bodyId).toBe("body1");
  });

  it("no hits at all: null regardless of Alt", () => {
    expect(refFromModelHits(null, null, false)).toBeNull();
    expect(refFromModelHits(null, null, true)).toBeNull();
  });
});
