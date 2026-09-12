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
import { describe, it, expect, beforeEach, vi } from "vitest";

vi.mock("@/ipc/promote", () => ({
  promoteOne: vi.fn(),
  stalePickHint: vi.fn(),
  STALE_PICK_HINT: "Selection is out of date — pick again",
}));

import * as THREE from "three";
import { refFromModelHits, promotePick, viewportGeometryChip } from "./ViewportRoot";
import { promoteOne } from "@/ipc/promote";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import type { PromotedElement } from "@/ipc/types";
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

  it("tie + no Alt against a body EDGE: the edge wins (sketch traces the boundary)", () => {
    const edge: PickHit = { ...bodyHit(), kind: "edge", topoKey: "e:3" };
    const ref = refFromModelHits(edge, sketchHit(), false);
    expect(ref?.kind).toBe("edge");
    expect(ref?.bodyId).toBe("body1");
  });

  it("sketch clearly in FRONT of a body edge: the sketch still wins", () => {
    const edge: PickHit = { ...bodyHit(200), kind: "edge", topoKey: "e:3" };
    const ref = refFromModelHits(edge, sketchHit(100), false);
    expect(ref?.kind).toBe("sketchRegion");
  });

  it("no hits at all: null regardless of Alt", () => {
    expect(refFromModelHits(null, null, false)).toBeNull();
    expect(refFromModelHits(null, null, true)).toBeNull();
  });
});

/*
 * viewportGeometryChip — which of the two mutually exclusive geometry-status
 * chips the viewport overlays. One slot, two independent facts, so the
 * precedence is worth pinning rather than re-deriving from JSX conditions.
 */
describe("viewportGeometryChip", () => {
  it("shows nothing until the engine is ready, whatever the state says", () => {
    expect(viewportGeometryChip(false, true, false, true)).toBeNull();
    expect(viewportGeometryChip(false, false, false, true)).toBeNull();
  });

  it("pending wins over cached — meshes are mid-flight, so nothing is 'last saved'", () => {
    expect(viewportGeometryChip(true, true, false, true)).toBe("pending");
    expect(viewportGeometryChip(true, true, false, false)).toBe("pending");
  });

  it("a sticky error hint suppresses pending, and does NOT promote cached in its place", () => {
    expect(viewportGeometryChip(true, true, true, true)).toBeNull();
  });

  it("cached shows ALONGSIDE an error hint — a failed regen over stale geometry is exactly the case to label", () => {
    // Suppressing it here would leave last-saved geometry looking authoritative
    // at the very moment the rebuild that would have replaced it failed.
    expect(viewportGeometryChip(true, false, true, true)).toBe("cached");
  });

  it("live geometry shows no chip at all", () => {
    expect(viewportGeometryChip(true, false, false, false)).toBeNull();
  });
});

// ── promotePick write-back (WP-U4 / D-5, Astra break F3) ────────────────────

describe("promotePick", () => {
  const faceRef = (topoKey: string, marker?: string): EntityRef => ({
    kind: "face",
    id: `body1#${topoKey}`,
    bodyId: "body1",
    topoKey,
    ...(marker ? { anchor: { worldPoint: [1, 2, 3] as [number, number, number] } } : {}),
  });

  beforeEach(() => {
    vi.mocked(promoteOne).mockReset();
    selectionStore.getState().set([]);
  });

  const client = {} as Parameters<typeof promotePick>[0];

  it("writes the minted ElementId back and LEAVES the pick-time label alone", async () => {
    // The mesh label the user clicked is what this publication draws through;
    // the promotion answers with the backend's own key for the same element.
    // Overwriting `topoKey` with it makes the highlight depend on a key the
    // displayed table may not carry at all.
    const ref = faceRef("f:4");
    selectionStore.getState().set([ref]);
    vi.mocked(promoteOne).mockResolvedValue({
      topoKey: "f:9",
      elementId: "el_a",
      kind: "face",
      bodyId: "body1",
    });

    promotePick(client, ref);
    await new Promise((r) => setTimeout(r, 0));

    expect(selectionStore.getState().selected[0].elementId).toBe("el_a");
    expect(selectionStore.getState().selected[0].topoKey).toBe("f:4");
    expect(selectionStore.getState().selected[0].id).toBe("body1#f:4");
  });

  it("leaves a FRESH pick that reuses the promoted ref's id untouched", async () => {
    // `EntityRef.id` is reusable: deselect and pick the same label again and the
    // new ref carries the same string. The reply belongs to the OBJECT it was
    // asked about, not to whatever currently holds its id.
    const stale = faceRef("f:4");
    selectionStore.getState().set([stale]);
    let release!: (v: PromotedElement) => void;
    vi.mocked(promoteOne).mockReturnValue(
      new Promise<PromotedElement>((r) => {
        release = r;
      }),
    );

    promotePick(client, stale);
    const fresh = faceRef("f:4", "fresh"); // same id, a DIFFERENT object
    selectionStore.getState().set([fresh]);
    release({ topoKey: "f:9", elementId: "el_a", kind: "face", bodyId: "body1" });
    await new Promise((r) => setTimeout(r, 0));

    expect(selectionStore.getState().selected).toEqual([fresh]);
  });
});
