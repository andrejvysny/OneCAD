/*
 * sketchService.trimEntity — parametric trim commit path (real mock lane):
 *   - a click that trims a segment replaces the target with its surviving pieces,
 *     cascade-drops constraints referencing the target, and pushes ONE "trim" undo;
 *   - a target with no qualifying crossings falls back to whole-entity delete;
 *   - no-ops on an unknown id / no session.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { trimEntity } from "./sketchService";
import { mockClient, resetMockSketches } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { sketchStore } from "@/stores/sketchStore";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";

function seed(entities: SketchEntity[], constraints: SketchConstraint[]): void {
  sketchStore.getState().setSession({
    sketchId: "sk-trim",
    plane: planeFor("XY"),
    entities,
    constraints,
    dof: 0,
    status: "UnderConstrained",
  });
}

describe("trimEntity — parametric trim commit", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("trims the clicked span into pieces, cascade-drops constraints, pushes ONE 'trim' undo", async () => {
    // b (horizontal target) crossed by a (x=5) and c (x=15); click between them.
    seed(
      [
        { id: "a", type: "Line", p0: [5, -50], p1: [5, 50] },
        { id: "b", type: "Line", p0: [-50, 0], p1: [50, 0] },
        { id: "c", type: "Line", p0: [15, -50], p1: [15, 50] },
      ],
      [
        { id: "k1", type: "Vertical", entities: ["b"] }, // refs the target (dropped)
        { id: "k2", type: "Vertical", entities: ["a"] }, // unrelated (kept)
      ],
    );
    await trimEntity(mockClient, "b", [10, 0], { minSize: 4 });

    const s = sketchStore.getState().session!;
    const ids = s.entities.map((e) => e.id);
    expect(ids).not.toContain("b"); // target trimmed away
    expect(ids).toContain("a");
    expect(ids).toContain("c");
    expect(s.entities).toHaveLength(4); // a, c + 2 surviving pieces
    expect(s.entities.filter((e) => e.id !== "a" && e.id !== "c").every((e) => e.type === "Line")).toBe(true);
    // Cascade: k1 (refs b) dropped; k2 survives.
    expect(s.constraints.map((c) => c.id)).toEqual(["k2"]);
    // Exactly one undo entry, tagged "trim".
    expect(sketchStore.getState().undoStack).toHaveLength(1);
    expect(sketchStore.getState().lastUndoPush?.kind).toBe("trim");
  });

  it("falls back to whole-entity delete when there are no crossings", async () => {
    // b and d are parallel (no crossing) ⇒ trimPieces null ⇒ whole delete.
    seed(
      [
        { id: "b", type: "Line", p0: [-50, 0], p1: [50, 0] },
        { id: "d", type: "Line", p0: [-50, 20], p1: [50, 20] },
      ],
      [{ id: "k1", type: "Horizontal", entities: ["b"] }],
    );
    await trimEntity(mockClient, "b", [10, 0], { minSize: 4 });

    const s = sketchStore.getState().session!;
    expect(s.entities.map((e) => e.id)).toEqual(["d"]); // b removed, no pieces added
    expect(s.constraints).toEqual([]); // k1 (refs b) cascaded away
    expect(sketchStore.getState().lastUndoPush?.kind).toBe("trim");
  });

  it("is a no-op for an unknown entity id", async () => {
    seed([{ id: "b", type: "Line", p0: [0, 0], p1: [40, 0] }], []);
    const before = sketchStore.getState().session;
    await trimEntity(mockClient, "nope", [10, 0], { minSize: 4 });
    expect(sketchStore.getState().session).toBe(before); // untouched
    expect(sketchStore.getState().undoStack).toHaveLength(0);
  });

  it("is a no-op with no active session", async () => {
    await trimEntity(mockClient, "b", [10, 0], { minSize: 4 });
    expect(sketchStore.getState().session).toBeNull();
  });
});
