/*
 * mockClient — body-edge projection into a sketch (SCHEMA §7.6
 * `ProjectToSketchPlane`, WP-P). Honest mock: only the seed BOX (`body1`)
 * resolves (see `mockClient.ts`'s "MOCK LIMIT" note by `sketchProjections`);
 * anything else — the cylinder's curved wall included — refuses rather than
 * fabricating geometry.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { mockClient, resetMockSketches } from "./mockClient";

describe("mockClient — projectToSketch / updateProjection / detachProjection (WP-P)", () => {
  beforeEach(() => resetMockSketches());

  it("projectToSketch(faceOutline) on a mock box face yields 4 locked line entities + 4 projection rows", async () => {
    await mockClient.enterSketch("sk");
    const result = await mockClient.projectToSketch({
      snapshotId: 1,
      sketchId: "sk",
      mode: "faceOutline",
      sources: [{ bodyId: "body1", topoKey: "f:4" }],
    });
    expect(result.refusals).toEqual([]);
    expect(result.sketchId).toBe("sk");
    expect(result.entities).toHaveLength(4);
    expect(result.entities.every((e) => e.type === "Line")).toBe(true);
    expect(result.entities.every((e) => e.sourceBodyId === "body1")).toBe(true);
    expect(result.entities.every((e) => e.sourceElementId.match(/^el_[0-9a-f]{8}$/))).toBe(true);
    expect(new Set(result.entities.map((e) => e.entityId)).size).toBe(4);

    const session = await mockClient.getSketch("sk");
    expect(Object.keys(session.projections ?? {})).toHaveLength(4);
    const projectedIds = new Set(result.entities.map((e) => e.entityId));
    const locked = session.entities.filter((e) => projectedIds.has(e.id));
    expect(locked).toHaveLength(4);
    expect(locked.every((e) => e.referenceLocked === true)).toBe(true);
  });

  it("projectToSketch on the mock cylinder's curved wall refuses unsupportedCurve, adding no entities", async () => {
    await mockClient.enterSketch("sk");
    const result = await mockClient.projectToSketch({
      snapshotId: 1,
      sketchId: "sk",
      mode: "faceOutline",
      sources: [{ bodyId: "body2", topoKey: "f:0" }],
    });
    expect(result.entities).toEqual([]);
    expect(result.refusals).toHaveLength(1);
    expect(result.refusals[0].code).toBe("unsupportedCurve");
    expect(result.refusals[0].bodyId).toBe("body2");
    expect(result.refusals[0].topoKey).toBe("f:0");

    const session = await mockClient.getSketch("sk");
    expect(session.projections ?? {}).toEqual({});
  });

  it("projectToSketch(edges) on a mock box edge yields one locked line", async () => {
    await mockClient.enterSketch("sk");
    const result = await mockClient.projectToSketch({
      snapshotId: 1,
      sketchId: "sk",
      mode: "edges",
      sources: [{ bodyId: "body1", topoKey: "e:0" }],
    });
    expect(result.refusals).toEqual([]);
    expect(result.entities).toHaveLength(1);
    expect(result.entities[0].type).toBe("Line");
    expect(result.entities[0].sourceElementId).toMatch(/^el_[0-9a-f]{8}$/);
  });

  it("detachProjection clears the projection rows and unlocks the entities", async () => {
    await mockClient.enterSketch("sk");
    const projected = await mockClient.projectToSketch({
      snapshotId: 1,
      sketchId: "sk",
      mode: "faceOutline",
      sources: [{ bodyId: "body1", topoKey: "f:4" }],
    });
    const detach = await mockClient.detachProjection("sk");
    expect(detach.entityIds.slice().sort()).toEqual(projected.entities.map((e) => e.entityId).sort());
    expect(detach.remaining).toBe(0);
    expect(detach.releasedConstraints).toBeGreaterThan(0);

    const session = await mockClient.getSketch("sk");
    expect(session.projections ?? {}).toEqual({});
    const projectedIds = new Set(projected.entities.map((e) => e.entityId));
    const stillLocked = session.entities.filter((e) => projectedIds.has(e.id) && e.referenceLocked);
    expect(stillLocked).toEqual([]);
  });

  it("detachProjection(subset) only clears the named entities", async () => {
    await mockClient.enterSketch("sk");
    const projected = await mockClient.projectToSketch({
      snapshotId: 1,
      sketchId: "sk",
      mode: "faceOutline",
      sources: [{ bodyId: "body1", topoKey: "f:4" }],
    });
    const [first, ...rest] = projected.entities.map((e) => e.entityId);
    const detach = await mockClient.detachProjection("sk", [first]);
    expect(detach.entityIds).toEqual([first]);
    expect(detach.remaining).toBe(rest.length);

    const session = await mockClient.getSketch("sk");
    expect(Object.keys(session.projections ?? {})).toHaveLength(rest.length);
  });

  it("updateProjection echoes the current rows unchanged (mock bodies never move)", async () => {
    await mockClient.enterSketch("sk");
    const projected = await mockClient.projectToSketch({
      snapshotId: 1,
      sketchId: "sk",
      mode: "faceOutline",
      sources: [{ bodyId: "body1", topoKey: "f:4" }],
    });
    const updated = await mockClient.updateProjection("sk");
    expect(updated.entities.map((e) => e.entityId).sort()).toEqual(
      projected.entities.map((e) => e.entityId).sort(),
    );
    expect(updated.entities.map((e) => e.projectedHash).sort()).toEqual(
      projected.entities.map((e) => e.projectedHash).sort(),
    );
    expect(updated.refusals).toEqual([]);
  });
});
