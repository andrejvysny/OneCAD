/*
 * WP0 red test — repair rebind must use the authoritative candidate body.
 *
 * `ResolveRefs` echoes `{revision, snapshotId, refId, bodyId}` for each candidate,
 * where `bodyId` is the body the candidate belongs to. The current
 * `rebindCandidate()` ignores this body and promotes against `deriveOperatedBody(item)`,
 * which can be a different body — a silent wrong bind.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { rebindCandidate, repairInputPath } from "./historyActions";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { repairStore } from "@/stores/repairStore";
import { resetStores } from "@/test/resetStores";

/** What the mocked `getOperationParams` answers — set per test. */
const storedParams = vi.hoisted(() => ({
  value: {} as Record<string, unknown>,
  fail: false,
}));

/** The face-centre query WP-F's reference-face repair must read the anchor from. */
const elementInfoMock = vi.hoisted(() =>
  vi.fn((bodyId: string, elementId: string, topoKey?: string) =>
    Promise.resolve({ bodyId, elementId, topoKey, center: [11, 22, 33] }),
  ),
);
const applyEditCommandMock = vi.hoisted(() =>
  vi.fn(() =>
    Promise.resolve({ revision: 1, features: [], changedBodies: [], removedBodies: [] }),
  ),
);

vi.mock("@/ipc/client", () => ({
  createClient: () => ({
    resolveRefs: vi.fn(),
    getOperationParams: vi.fn(() =>
      storedParams.fail
        ? Promise.reject(new Error("unavailable"))
        : Promise.resolve(storedParams.value),
    ),
    elementInfo: elementInfoMock,
    applyEditCommand: applyEditCommandMock,
  }),
}));

vi.mock("@/ipc/promote", () => ({
  promoteOne: vi.fn(() =>
    Promise.resolve({
      bodyId: "bodyA",
      elementId: "el1",
    }),
  ),
}));

import { promoteOne } from "@/ipc/promote";

describe("historyActions rebindCandidate WP0", () => {
  beforeEach(() => {
    resetStores();
    elementInfoMock.mockClear();
    applyEditCommandMock.mockClear();
    documentStore.setState({
      bodies: {
        bodyA: { id: "bodyA", name: "Body A", visible: true },
        bodyB: { id: "bodyB", name: "Body B", visible: true },
      },
      // A rebindable feature for opId "op1" (Fillet: unconditional non-null
      // InputPath), so the repair reaches the promoteOne call under test.
      features: [
        { id: "op1", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
      ],
    });
    selectionStore.setState({ selected: [] });
    repairStore.getState().reset();
  });

  it("promotes against the candidate's authoritative bodyId, not the operated body", async () => {
    const item = {
      opId: "op1",
      refId: "ref1",
      bodyId: "bodyB", // the feature historically operated on bodyB
      reason: "face missing",
      candidateCount: 0,
    };

    const candidate = {
      topoKey: "f:5",
      worldPos: [1, 2, 3] as [number, number, number],
      score: 0.95,
      margin: 0.1,
      summary: "candidate A",
      // Denormalized from ResolveRefResult.bodyId by the panel — see
      // ResolveCandidate.bodyId. Deliberately NOT bodyB (item.bodyId).
      bodyId: "bodyA",
    };

    await rebindCandidate(item, candidate);

    // The authoritative candidate body is bodyA. The fix must promote against bodyA.
    expect(promoteOne).toHaveBeenCalledWith(
      expect.anything(),
      "bodyA",
      expect.objectContaining({
        topoKey: "f:5",
        anchor: { worldPoint: [1, 2, 3] },
      }),
      undefined,
    );
  });
});

describe("repairInputPath — the Chamfer reference-face slot (SCHEMA §7.3/§9, WP-F)", () => {
  const chamferFeature = {
    id: "op1",
    kind: "fillet" as const,
    opType: "Chamfer",
    label: "Chamfer",
    valueText: "4.0 mm",
    status: "ok" as const,
  };

  beforeEach(() => {
    elementInfoMock.mockClear();
    applyEditCommandMock.mockClear();
    storedParams.fail = false;
    storedParams.value = {};
    documentStore.setState({ features: [chamferFeature] });
  });

  it("fetches the stored params for a Chamfer so N is known, not guessed", async () => {
    storedParams.value = { edgeIds: ["el_e1", "el_e2"], referenceFaces: [] };
    // Slot 1 is still an EDGE (N = 2)…
    expect(await repairInputPath({ opId: "op1", refId: "op1.input1", reason: "ambiguous", candidateCount: 1 }))
      .toEqual({ path: "filletEdges", index: 1 });
    // …and slot 2 is the first trailing reference-FACE slot.
    expect(await repairInputPath({ opId: "op1", refId: "op1.input2", reason: "ambiguous", candidateCount: 1 }))
      .toEqual({ path: "chamferReferenceFace", index: 0 });
  });

  it("echoes a `legacyReferenceFace` item's seedEdgeId back as the pair's key", async () => {
    // SCHEMA §9: the item names an EMPTY slot, so the CREATE must be told which
    // contour edge to key the pair by — core refuses one that is not in `edgeIds`.
    storedParams.value = { edgeIds: ["el_e1"], referenceFaces: [] };
    expect(
      await repairInputPath({
        opId: "op1",
        refId: "op1.input1",
        reason: "legacyReferenceFace",
        seedEdgeId: "el_e1",
        candidateCount: 2,
      }),
    ).toEqual({ path: "chamferReferenceFace", index: 0, edgeId: "el_e1" });
  });

  it("refuses when the params cannot be fetched rather than mis-repairing an edge", async () => {
    storedParams.fail = true;
    expect(
      await repairInputPath({ opId: "op1", refId: "op1.input2", reason: "ambiguous", candidateCount: 1 }),
    ).toBeNull();
  });
});

describe("rebindCandidate — the chamfer reference face's ANCHOR (SCHEMA §7.3, WP-F)", () => {
  beforeEach(() => {
    elementInfoMock.mockClear();
    applyEditCommandMock.mockClear();
    storedParams.fail = false;
    // Slot 1 is the first trailing reference-FACE slot (N = 1).
    storedParams.value = { edgeIds: ["el_e1"], referenceFaces: [] };
    documentStore.setState({
      bodies: { bodyA: { id: "bodyA", name: "Body A", visible: true } },
      features: [
        { id: "op1", kind: "fillet", opType: "Chamfer", label: "Chamfer", valueText: "4.0 mm", status: "ok" },
      ],
    });
  });

  const legacyItem = {
    opId: "op1",
    refId: "op1.input1",
    reason: "legacyReferenceFace",
    seedEdgeId: "el_e1",
    candidateCount: 2,
  };

  it("anchors the created pair at the FACE CENTRE, not the seed edge's world point", async () => {
    // A `legacyReferenceFace` candidate's `worldPos` is the SEED EDGE's anchor, and
    // that point lies on BOTH adjacent faces — it would tie the resolver's anchor
    // rung between them on every replay, the exact ambiguity §7.3 makes the anchor
    // non-optional to prevent.
    await rebindCandidate(
      legacyItem,
      {
        topoKey: "f:3",
        worldPos: [1, 2, 3],
        score: 0.5,
        margin: 0,
        summary: "",
        bodyId: "bodyA",
      },
      700,
    );

    expect(elementInfoMock).toHaveBeenCalledWith("bodyA", "el1", "f:3");
    const calls = applyEditCommandMock.mock.calls as unknown as Array<
      [{ path: { path: string; index: number; edgeId?: string }; reference: { element: { anchor?: { worldPoint: number[] } } } }]
    >;
    expect(calls).toHaveLength(1);
    // The CREATE is keyed by the item's own seed edge (core refuses one that is not
    // in `edgeIds`), and the anchor is the face's interior point.
    expect(calls[0][0].path).toEqual({ path: "chamferReferenceFace", index: 0, edgeId: "el_e1" });
    expect(calls[0][0].reference.element.anchor).toEqual({ worldPoint: [11, 22, 33] });
  });

  it("falls back to the candidate's worldPos when the face query cannot answer", async () => {
    // An ANCHORLESS ref is refused by core outright, so a present-but-coarse anchor
    // is the better failure.
    elementInfoMock.mockRejectedValueOnce(new Error("no such element"));
    await rebindCandidate(
      legacyItem,
      { topoKey: "f:3", worldPos: [1, 2, 3], score: 0.5, margin: 0, summary: "", bodyId: "bodyA" },
      700,
    );
    const calls = applyEditCommandMock.mock.calls as unknown as Array<
      [{ reference: { element: { anchor?: { worldPoint: number[] } } } }]
    >;
    expect(calls[0][0].reference.element.anchor).toEqual({ worldPoint: [1, 2, 3] });
  });

  it("leaves a NON-chamfer slot's anchor exactly as it was (no extra round trip)", async () => {
    documentStore.setState({
      features: [
        { id: "op1", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
      ],
    });
    await rebindCandidate(
      { opId: "op1", refId: "op1.input0", reason: "ambiguous", candidateCount: 1 },
      { topoKey: "e:4", worldPos: [1, 2, 3], score: 0.9, margin: 0.2, summary: "", bodyId: "bodyA" },
    );
    expect(elementInfoMock).not.toHaveBeenCalled();
    const calls = applyEditCommandMock.mock.calls as unknown as Array<
      [{ reference: { element: { anchor?: { worldPoint: number[] } } } }]
    >;
    expect(calls[0][0].reference.element.anchor).toEqual({ worldPoint: [1, 2, 3] });
  });
});
