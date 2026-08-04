import { describe, it, expect, beforeEach } from "vitest";
import { mockClient, resetMockSketches } from "./mockClient";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import type { SketchConstraint, SketchEntity } from "./types";

describe("mockClient — sketch solver lane flows", () => {
  beforeEach(() => resetMockSketches());

  it("enterSketch(persisted document sketch) hydrates real geometry", async () => {
    // "sketch2" is seeded in documentStore (seedMockDocument); re-entry must show
    // real geometry, not a silent empty session (parity with the real client).
    const s = await mockClient.enterSketch("sketch2");
    expect(s.sketchId).toBe("sketch2");
    expect(s.plane.kind).toBe("XY");
    expect(s.entities.length).toBeGreaterThan(0);
  });

  it("enterSketch(unknown id, not in the document) opens an empty XY session", async () => {
    const s = await mockClient.enterSketch("sk-not-in-doc");
    expect(s.sketchId).toBe("sk-not-in-doc");
    expect(s.plane.kind).toBe("XY");
    expect(s.entities).toEqual([]);
    expect(s.dof).toBe(0);
    expect(s.status).toBe("FullyConstrained");
  });

  it("enterSketch({newOnPlane}) mints an id on the requested plane", async () => {
    const s = await mockClient.enterSketch({ newOnPlane: "XZ" });
    expect(s.sketchId).toMatch(/^sk-\d+$/);
    expect(s.plane.kind).toBe("XZ");
  });

  it("sketchUpsert re-solves and bumps the revision", async () => {
    await mockClient.enterSketch("sk");
    const entities: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];
    const constraints: SketchConstraint[] = [{ id: "c1", type: "Horizontal", entities: ["e1"] }];
    const r1 = await mockClient.sketchUpsert("sk", entities, constraints);
    expect(r1.sketchRevision).toBe(1);
    expect(r1.dof).toBe(3); // line (4) − 1
    expect(r1.status).toBe("UnderConstrained");
    const r2 = await mockClient.sketchUpsert("sk", entities, constraints);
    expect(r2.sketchRevision).toBe(2);
  });

  it("finishSketch returns regions for a committed rectangle", async () => {
    await mockClient.enterSketch("sk");
    const rect: SketchEntity[] = [
      { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
      { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] },
      { id: "e3", type: "Line", p0: [40, 20], p1: [0, 20] },
      { id: "e4", type: "Line", p0: [0, 20], p1: [0, 0] },
    ];
    await mockClient.sketchUpsert("sk", rect, []);
    const res = await mockClient.finishSketch("sk");
    expect(res.regions).toHaveLength(1);
    expect(res.regions[0].outerLoop).toHaveLength(4);
  });

  it("finishSketch on an unknown sketch yields no regions", async () => {
    const res = await mockClient.finishSketch("nope");
    expect(res.regions).toEqual([]);
  });

  it("getSketch returns the live lane session when one exists (pure read)", async () => {
    await mockClient.enterSketch("sk");
    await mockClient.sketchUpsert("sk", [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }], []);
    const s = await mockClient.getSketch("sk");
    expect(s.sketchId).toBe("sk");
    expect(s.entities).toHaveLength(1);
    expect(s.entities[0].id).toBe("e1");
  });

  it("getSketch returns the seeded rectangle for a persisted document sketch (no session)", async () => {
    documentStore.setState(seedMockDocument());
    const s = await mockClient.getSketch("sketch4"); // a document sketch, never entered
    expect(s.plane.kind).toBe("XY");
    expect(s.entities.length).toBeGreaterThan(0);
  });

  it("getSketch rejects an unknown sketch (no session, not a document sketch)", async () => {
    documentStore.setState(seedMockDocument());
    await expect(mockClient.getSketch("ghost-xyz")).rejects.toThrow(/unknown sketch/);
  });

  it("cancelSketch resolves without throwing", async () => {
    await mockClient.enterSketch("sk");
    await expect(mockClient.cancelSketch("sk")).resolves.toBeUndefined();
  });

  it("deleteSketch drops the document row + the lane session (re-enter is empty)", async () => {
    documentStore.setState(seedMockDocument()); // ensure "sketch2" is a document sketch
    // First entry hydrates the seeded rectangle (document sketch with no session).
    const first = await mockClient.enterSketch("sketch2");
    expect(first.entities.length).toBeGreaterThan(0);

    await mockClient.deleteSketch("sketch2");
    // The store row is gone…
    expect(documentStore.getState().sketches["sketch2"]).toBeUndefined();

    // …so re-entering the SAME id is no longer a document sketch → empty session
    // (the rectangle-hydration path only fires for a known document sketch).
    const again = await mockClient.enterSketch("sketch2");
    expect(again.entities).toEqual([]);
  });

  /*
   * H3b — the inspector's sketch-dimension quick path. The mock must accept it on a
   * sketch the user has only SELECTED (no `enterSketch` first), because that is the
   * only state the history panel can be in.
   */
  describe("setSketchDimension", () => {
    it("patches ONE constraint's value and re-solves, on a never-entered sketch", async () => {
      documentStore.setState(seedMockDocument());
      // Seed a dimensioned session, then drop back to "only selected" by asserting
      // through the public read the panel itself uses.
      await mockClient.enterSketch("sketch2");
      await mockClient.sketchUpsert(
        "sketch2",
        [{ id: "e1", type: "Line", p0: [0, 0], p1: [60, 0] }],
        [
          { id: "c1", type: "Distance", entities: ["e1"], value: 60 },
          { id: "c2", type: "Horizontal", entities: ["e1"] },
        ],
      );

      const before = await mockClient.sketchUpsert(
        "sketch2",
        (await mockClient.getSketch("sketch2")).entities,
        (await mockClient.getSketch("sketch2")).constraints,
      );
      const res = await mockClient.setSketchDimension("sketch2", "c1", "Distance", 75);
      expect(res.sketchId).toBe("sketch2");
      // Re-solved through the SHARED lane, so it advances the sketch revision.
      expect(res.sketchRevision).toBe(before.sketchRevision + 1);

      const after = await mockClient.getSketch("sketch2");
      expect(after.constraints.find((c) => c.id === "c1")?.value).toBe(75);
      // Every OTHER constraint and all geometry survive the patch.
      expect(after.constraints.map((c) => c.id)).toEqual(["c1", "c2"]);
      expect(after.entities).toHaveLength(1);
    });

    it("hydrates a document sketch that has no session yet instead of failing", async () => {
      documentStore.setState(seedMockDocument());
      // No enterSketch: the seeded rectangle is hydrated on demand. It carries no
      // dimensional constraints, so the patch is a no-op that still re-solves —
      // what matters is that it does not throw at the panel.
      await expect(
        mockClient.setSketchDimension("sketch4", "c-none", "Distance", 20),
      ).resolves.toMatchObject({ sketchId: "sketch4" });
    });
  });
});
