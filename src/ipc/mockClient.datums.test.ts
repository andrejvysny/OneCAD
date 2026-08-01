/*
 * mockClient — datum planes (DATUM W1, data half).
 *
 * The mock lane has no backend, so it MIRRORS the core's datum rules itself:
 * resolve at creation from the parametric definition (discarding the client's
 * frame), refuse to delete a datum a sketch is hosted on, and carry datums
 * through undo/redo. These tests pin that mirror against the same numbers the
 * Rust side asserts (`src-tauri/tests/datum_planes.rs`,
 * `crates/onecad-core/tests/edit_session.rs`), so the two lanes cannot drift.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { mockAttachSketchToDatum, mockClient, resetMockDocument, setMockLatency } from "./mockClient";
import { buildAddDatumPlane, deleteDatumCommand } from "./tauriCommandMap";
import { documentStore } from "@/stores/documentStore";

const datums = () => documentStore.getState().datums;

describe("mockClient datum planes", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
  });

  it("resolves an XY offset at creation and DISCARDS the client-sent frame", async () => {
    const res = await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    expect(res.opLabel).toBe("Create datum plane");
    const d = datums().d1;
    expect(d.resolvedValid).toBe(true);
    // The command carried the identity XY placeholder at the ORIGIN; the resolved
    // frame is 10mm along XY's +Z normal, with XY's axes carried verbatim.
    expect(d.plane.origin).toEqual([0, 0, 10]);
    expect(d.plane.xAxis).toEqual([0, 1, 0]);
    expect(d.plane.yAxis).toEqual([-1, 0, 0]);
    expect(d.plane.kind).toBe("custom");
    expect(d.name).toBe("Datum 1");
    expect(d.offset).toBe(10);
  });

  // LEGACY-SWAPPED BASES PIN — the same one the core test
  // `offset_from_xz_slides_along_world_x` holds. "XZ" has world normal +X.
  it("offsets along the BASE NORMAL, so XZ moves along world +X and YZ along +Y", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("dxz", "Datum 1", "XZ", 10));
    expect(datums().dxz.plane.origin).toEqual([10, 0, 0]);
    await mockClient.applyEditCommand(buildAddDatumPlane("dyz", "Datum 2", "YZ", -4));
    expect(datums().dyz.plane.origin).toEqual([0, -4, 0]);
  });

  it("chains off another resolved datum", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    await mockClient.applyEditCommand(buildAddDatumPlane("d2", "Datum 2", "d1", 5));
    expect(datums().d2.plane.origin).toEqual([0, 0, 15]);
    expect(datums().d2.resolvedValid).toBe(true);
  });

  it("stores an unresolvable base leniently, flagged invalid — and does not chain off it", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("bad", "Datum 1", "NOPE", 3));
    expect(datums().bad.resolvedValid).toBe(false);
    await mockClient.applyEditCommand(buildAddDatumPlane("child", "Datum 2", "bad", 1));
    expect(datums().child.resolvedValid).toBe(false);
  });

  it("deletes an unreferenced datum and rejects an unknown one", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    await mockClient.applyEditCommand(deleteDatumCommand("d1"));
    expect(datums()).toEqual({});
    await expect(mockClient.applyEditCommand(deleteDatumCommand("d1"))).rejects.toThrow(/unknown datum/);
  });

  it("refuses to delete a datum a sketch is hosted on, and names the blocker", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    mockAttachSketchToDatum("sketchA", "d1");
    await expect(mockClient.applyEditCommand(deleteDatumCommand("d1"))).rejects.toThrow(/sketchA/);
    expect(datums().d1).toBeDefined();

    // Detach ⇒ the same delete succeeds.
    mockAttachSketchToDatum("sketchA", null);
    await mockClient.applyEditCommand(deleteDatumCommand("d1"));
    expect(datums()).toEqual({});
  });

  it("carries datums through undo/redo (a snap that forgot them would resurrect a deleted datum)", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    await mockClient.applyEditCommand(deleteDatumCommand("d1"));
    expect(datums()).toEqual({});

    await mockClient.undo();
    expect(datums().d1.plane.origin).toEqual([0, 0, 10]);

    await mockClient.redo();
    expect(datums()).toEqual({});

    // One more undo pair takes the ADD itself back out.
    await mockClient.undo();
    expect(datums().d1).toBeDefined();
    await mockClient.undo();
    expect(datums()).toEqual({});
  });

  // ── the tool half's attach wiring (DATUM W1, surface half) ─────────────────
  //
  // `mockAttachSketchToDatum` had no PRODUCER when the data half landed, which
  // made the referenced-guard technically real but practically unreachable in
  // the mock lane. `enterSketch({newOnDatum})` is that producer — these two pin
  // it, so the guard the e2e spec relies on is demonstrably non-vacuous.

  it("enterSketch({newOnDatum}) opens on the datum's frame AND registers the attachment", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    const plane = datums().d1.plane;

    const session = await mockClient.enterSketch({
      newOnDatum: { datumId: "d1" },
      plane,
      sketchId: "sk-on-datum",
    });
    // The session sits on the datum's resolved frame, not on world XY.
    expect(session.sketchId).toBe("sk-on-datum");
    expect(session.plane.origin).toEqual([0, 0, 10]);
    expect(session.plane.kind).toBe("custom");

    // …and the datum is now REFERENCED: the delete guard fires by itself, with
    // no test-only `mockAttachSketchToDatum` call to set it up.
    await expect(mockClient.applyEditCommand(deleteDatumCommand("d1"))).rejects.toThrow(
      /referenced by 1 sketch\(es\): sk-on-datum/,
    );
    expect(datums().d1).toBeDefined();
  });

  it("deleting the hosted sketch releases the datum (no phantom blocker)", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    await mockClient.enterSketch({
      newOnDatum: { datumId: "d1" },
      plane: datums().d1.plane,
      sketchId: "sk-on-datum",
    });

    // The SketchController's orphan-cleanup path (mode left mid-enter) — the
    // attachment must go with the sketch, or the datum is undeletable forever.
    await mockClient.deleteSketch("sk-on-datum");
    await mockClient.applyEditCommand(deleteDatumCommand("d1"));
    expect(datums()).toEqual({});
  });

  it("getProjection returns datums in the WIRE shape (bare basis, no plane kind)", async () => {
    await mockClient.applyEditCommand(buildAddDatumPlane("d1", "Datum 1", "XY", 10));
    const p = await mockClient.getProjection();
    expect(p.datums.d1).toEqual({
      id: "d1",
      name: "Datum 1",
      kind: "OffsetFromPlane",
      basePlaneId: "XY",
      offset: 10,
      plane: { origin: [0, 0, 10], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
      resolvedValid: true,
    });
    expect(p.datums.d1.plane).not.toHaveProperty("kind");
  });
});
