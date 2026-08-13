/*
 * WP0 red test — repair store snapshot ordering.
 *
 * Same-revision events must be compared lexicographically by
 * (revision, snapshotId). An older snapshotId must never overwrite a newer one.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { repairStore } from "./repairStore";

describe("repairStore WP0", () => {
  beforeEach(() => {
    repairStore.getState().reset();
  });

  it("rejects a same-revision older snapshot (lexicographic ordering)", () => {
    repairStore.getState().applyEvent({
      revision: 5,
      snapshotId: 10,
      items: [{ opId: "op1", refId: "ref1", bodyId: "body1", reason: "latest", candidateCount: 0 }],
    });

    repairStore.getState().applyEvent({
      revision: 5,
      snapshotId: 8,
      items: [{ opId: "op1", refId: "ref1", bodyId: "body1", reason: "stale", candidateCount: 0 }],
    });

    expect(repairStore.getState().items[0]?.reason).toBe("latest");
    expect(repairStore.getState().snapshotId).toBe(10);
  });
});
