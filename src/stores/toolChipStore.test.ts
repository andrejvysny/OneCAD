/*
 * toolChipStore — the edge-op chip surface (FILLET-CHAMFER-UNIFY).
 *
 * Every `show*` action resets from CLEARED, which is what stops one tool's chip
 * state bleeding into the next tool's chip. These specs pin that for the fields
 * the unified edge op added: a shell/extrude chip after a fillet chip must not
 * still be carrying `Chamfer` + a live `onEdgeOp` handler bound to a dead arm.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { toolChipStore } from "./toolChipStore";

const WORLD: [number, number, number] = [1, 2, 3];

describe("toolChipStore edge-op fields", () => {
  beforeEach(() => toolChipStore.getState().clear());

  it("defaults to a Fillet chip with no segments and no handler", () => {
    const s = toolChipStore.getState();
    expect(s.edgeOp).toBe("Fillet");
    expect(s.showEdgeOpSegments).toBe(false);
    expect(s.onEdgeOp).toBeNull();
  });

  it("showFillet takes the segment opts; setEdgeOp updates only the active op", () => {
    const onEdgeOp = vi.fn();
    toolChipStore
      .getState()
      .showFillet(3, WORLD, vi.fn(), undefined, {
        edgeOp: "Chamfer",
        showEdgeOpSegments: true,
        onEdgeOp,
      });
    expect(toolChipStore.getState().edgeOp).toBe("Chamfer");
    expect(toolChipStore.getState().showEdgeOpSegments).toBe(true);
    expect(toolChipStore.getState().onEdgeOp).toBe(onEdgeOp);

    toolChipStore.getState().setEdgeOp("Fillet");
    expect(toolChipStore.getState().edgeOp).toBe("Fillet");
    expect(toolChipStore.getState().value).toBe(3); // nothing else moved
    expect(toolChipStore.getState().showEdgeOpSegments).toBe(true);
  });

  it("omitting the opts keeps the bare re-edit chip (no segments, no handler)", () => {
    toolChipStore.getState().showFillet(4, WORLD, vi.fn());
    expect(toolChipStore.getState().showEdgeOpSegments).toBe(false);
    expect(toolChipStore.getState().onEdgeOp).toBeNull();
    expect(toolChipStore.getState().edgeOp).toBe("Fillet");
  });

  it("the next chip — and clear() — resets every edge-op field", () => {
    toolChipStore
      .getState()
      .showFillet(3, WORLD, vi.fn(), undefined, {
        edgeOp: "Chamfer",
        showEdgeOpSegments: true,
        onEdgeOp: vi.fn(),
      });

    toolChipStore.getState().showShell(2, WORLD, vi.fn());
    expect(toolChipStore.getState().edgeOp).toBe("Fillet");
    expect(toolChipStore.getState().showEdgeOpSegments).toBe(false);
    expect(toolChipStore.getState().onEdgeOp).toBeNull();

    toolChipStore.getState().setEdgeOp("Chamfer");
    toolChipStore.getState().clear();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(toolChipStore.getState().edgeOp).toBe("Fillet");
    expect(toolChipStore.getState().showEdgeOpSegments).toBe(false);
    expect(toolChipStore.getState().onEdgeOp).toBeNull();
  });
});
