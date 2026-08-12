/*
 * WP0 red test — repair rebind must use the authoritative candidate body.
 *
 * `ResolveRefs` echoes `{revision, snapshotId, refId, bodyId}` for each candidate,
 * where `bodyId` is the body the candidate belongs to. The current
 * `rebindCandidate()` ignores this body and promotes against `deriveOperatedBody(item)`,
 * which can be a different body — a silent wrong bind.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { rebindCandidate } from "./historyActions";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { repairStore } from "@/stores/repairStore";
import { resetStores } from "@/test/resetStores";

vi.mock("@/ipc/client", () => ({
  createClient: () => ({
    resolveRefs: vi.fn(),
    applyEditCommand: vi.fn(() =>
      Promise.resolve({
        revision: 1,
        features: [],
        changedBodies: [],
        removedBodies: [],
      }),
    ),
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
