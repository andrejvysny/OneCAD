/*
 * Registry: zero-copy geometry build, edge-segment expansion, double-buffer
 * swap (old disposed only on flush), and the document-close leak tripwire.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import type * as THREE from "three";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import * as reg from "./meshRegistry";
import { parseMeshPayload } from "./parseMeshPayload";
import { MeshAdmission } from "./meshAdmission";
import { makeBoxMesh, makeCylinderMesh, type FaceColor } from "@/ipc/mockMeshes";
import { validateMeshView } from "./validateMesh";
import { planMeshPreparation } from "./meshPreparationPlan";
import type { Rgba } from "@/ipc/types";
import { __resetLogForTests, logSnapshot } from "@/debug/log";
import { readViewportCounters, resetViewportCounters } from "../vph/instrumentation";

/** Stand-in authored STEP colors (sRGB 0–255) for the colored-body cases. */
const RED: FaceColor = [214, 74, 62, 255];
const BLUE: FaceColor = [58, 122, 196, 255];

function box(rev: number): reg.MeshEntry {
  return reg.buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", rev);
}

beforeEach(() => {
  reg.disposeAll();
  reg.__resetRegistryForTests();
});

describe("buildBodyObjects", () => {
  it("builds indexed face geometry with zero-copy position/normal attributes", () => {
    const view = parseMeshPayload(makeBoxMesh());
    const entry = reg.buildBodyObjects(view, "body1", 1);
    const pos = entry.geometry.getAttribute("position");
    expect(pos.count).toBe(24);
    // Attribute array is the SAME Float32Array the parser viewed (no copy).
    expect(pos.array).toBe(view.positions);
    expect(entry.geometry.getIndex()!.count).toBe(36); // 12 tris × 3
    expect(entry.edgeGeometry).not.toBeNull();
    expect(entry.faceIndex.count).toBe(6);
    expect(entry.edgeIndex!.count).toBe(12);
    entry.dispose();
  });

  /*
   * Edges are a fat-line geometry: INSTANCED, one instance per segment. That is
   * why `drawRange` is meaningless on it (HighlightLayer slices the position
   * array instead) and why a Picker hit reports `faceIndex` = segment ordinal.
   */
  it("builds edges as an instanced LineSegmentsGeometry over the expanded segments", () => {
    const view = parseMeshPayload(makeBoxMesh());
    const entry = reg.buildBodyObjects(view, "body1", 1);

    expect(entry.edgeGeometry).toBeInstanceOf(LineSegmentsGeometry);
    // A box: 12 edges, each a 2-point polyline ⇒ 12 segments ⇒ 12 instances.
    expect(entry.edgeGeometry!.instanceCount).toBe(12);
    // The array is exposed for the highlight slice, and is the SAME one the
    // geometry was built from — setPositions retains it by reference.
    expect(entry.edgeSegmentPositions).toBeInstanceOf(Float32Array);
    expect(entry.edgeSegmentPositions!.length).toBe(12 * 6); // 2 xyz endpoints
    expect(entry.edgeGeometry!.getAttribute("instanceStart").array).toBe(
      entry.edgeSegmentPositions,
    );
    // Bounds are computed eagerly by setPositions — the LineSegments2 raycast
    // needs them before the first render.
    expect(entry.edgeGeometry!.boundingSphere).not.toBeNull();
    entry.dispose();
  });

  it("maps a picked triangle to its face id via faceIndex", () => {
    const view = parseMeshPayload(makeBoxMesh());
    const entry = reg.buildBodyObjects(view, "body1", 1);
    // Triangle 0 belongs to face 0; triangle 11 to face 5.
    expect(entry.faceIndex.idAt(0)).toBe("f:0");
    expect(entry.faceIndex.idAt(11)).toBe("f:5");
    entry.dispose();
  });
});

describe("expandEdgeSegments", () => {
  it("expands polylines into GL_LINES endpoints with correct segment ranges", () => {
    // 2 edges: edge0 = 3-point polyline (2 segs), edge1 = 2-point (1 seg).
    const edgePositions = new Float32Array([
      0, 0, 0, 1, 0, 0, 2, 0, 0, // edge0 points p0,p1,p2
      0, 1, 0, 0, 2, 0, // edge1 points p3,p4
    ]);
    const edgeRanges = new Uint32Array([0, 3, 3, 2]);
    const { positions, segRanges, segTotal } = reg.expandEdgeSegments(edgePositions, edgeRanges, 2);
    expect(segTotal).toBe(3); // (3-1) + (2-1)
    expect([...segRanges]).toEqual([0, 2, 2, 1]); // edge0 segs[0,2), edge1 segs[2,3)
    // First segment endpoints = p0..p1.
    expect([...positions.slice(0, 6)]).toEqual([0, 0, 0, 1, 0, 0]);
    // Third (edge1's only) segment endpoints = p3..p4.
    expect([...positions.slice(12, 18)]).toEqual([0, 1, 0, 0, 2, 0]);
  });
});

describe("double-buffer swap", () => {
  it("publishes the new entry but disposes the old only on flush", () => {
    const first = box(1);
    reg.swap("body1", first);
    expect(reg.getEntry("body1")).toBe(first);

    const second = box(2);
    const disposeSpy = vi.spyOn(first, "dispose");
    reg.swap("body1", second);
    expect(reg.getEntry("body1")).toBe(second); // new is live immediately
    expect(disposeSpy).not.toHaveBeenCalled(); // old survives this frame

    reg.flushDisposals();
    expect(disposeSpy).toHaveBeenCalledTimes(1); // disposed next frame
  });

  it("swapping the same entry is a no-op for disposal", () => {
    const only = box(1);
    reg.swap("body1", only);
    const spy = vi.spyOn(only, "dispose");
    reg.swap("body1", only);
    reg.flushDisposals();
    expect(spy).not.toHaveBeenCalled();
    only.dispose();
  });
});

/*
 * A body with authored FACE_COLORS trades the zero-copy indexed layout for a
 * de-indexed one carrying a baked `color` attribute. The thing that must NOT
 * change is triangle ordinals: `faceIndex` (picking) and HighlightLayer's
 * drawRanges both address triangles by ordinal, and `drawRange` silently
 * switches from index units to vertex units when the index is dropped.
 */
describe("buildBodyObjects with FACE_COLORS", () => {
  const coloredBox = () =>
    parseMeshPayload(makeBoxMesh(40, 40, 40, 0, [0, 0, 0], [RED, null, null, null, BLUE, null]));

  it("de-indexes and attaches a color attribute", () => {
    const view = coloredBox();
    const entry = reg.buildBodyObjects(view, "body1", 1);

    expect(entry.hasVertexColors).toBe(true);
    expect(entry.geometry.getIndex()).toBeNull();
    const pos = entry.geometry.getAttribute("position");
    expect(pos.count).toBe(view.indices.length); // 3 verts per triangle, no sharing
    expect(entry.geometry.getAttribute("color").count).toBe(view.indices.length);
    expect(entry.geometry.getAttribute("normal").count).toBe(view.indices.length);
    // drawRange counts VERTICES now, and 3·T is the same number either way.
    expect(entry.geometry.drawRange).toEqual({ start: 0, count: view.indices.length });
    entry.dispose();
  });

  it("keeps triangle ordinals — so faceIndex still binds the right face", () => {
    const view = coloredBox();
    const entry = reg.buildBodyObjects(view, "body1", 1);
    // Same expectations as the plain box: 12 triangles, 2 per face, in order.
    expect(entry.faceIndex.idAt(0)).toBe("f:0");
    expect(entry.faceIndex.idAt(1)).toBe("f:0");
    expect(entry.faceIndex.idAt(8)).toBe("f:4");
    expect(entry.faceIndex.idAt(11)).toBe("f:5");
    entry.dispose();
  });

  it("leaves a color-less body on the zero-copy indexed path", () => {
    const view = parseMeshPayload(makeBoxMesh());
    const entry = reg.buildBodyObjects(view, "body1", 1);
    expect(entry.hasVertexColors).toBe(false);
    expect(entry.geometry.getAttribute("color")).toBeUndefined();
    expect(entry.geometry.getAttribute("position").array).toBe(view.positions);
    entry.rebakeFaceColors(); // must be a no-op, not a throw
    entry.dispose();
  });

  it("rebakeFaceColors rewrites the SAME array and flags it for upload", () => {
    const entry = reg.buildBodyObjects(coloredBox(), "body1", 1);
    const attr = entry.geometry.getAttribute("color") as THREE.BufferAttribute;
    const array = attr.array;
    const version = attr.version;

    entry.rebakeFaceColors();

    expect(entry.geometry.getAttribute("color")).toBe(attr); // no attribute swap
    expect(attr.array).toBe(array); // no reallocation
    // `needsUpdate` is write-only on BufferAttribute — the re-upload it schedules
    // is observable only as a version bump.
    expect(attr.version).toBeGreaterThan(version);
    entry.dispose();
  });

  /*
   * VP-HARDENING PR-03A — priced == built.
   *
   * The plan decides the layout and the admission charges its byte totals. If
   * the geometry that comes out does not add up to those totals, the budget is
   * describing something other than what is on the GPU, which is exactly the
   * defect this package closes. The DEV assertion inside `buildBodyObjects` is
   * the tripwire; these cases prove it stays quiet on every real fixture AND
   * that it actually fires when the two disagree.
   */
  it("never reports off-plan across the fixtures, coloured or not", () => {
    __resetLogForTests();
    try {
      const authored = new Map([["f:1", BLUE]]);
      const cases: ReadonlyArray<[ReturnType<typeof coloredBox>, Rgba | undefined, ReadonlyMap<string, Rgba> | undefined]> = [
        [parseMeshPayload(makeBoxMesh()), undefined, undefined],
        [parseMeshPayload(makeBoxMesh()), RED, undefined],
        [parseMeshPayload(makeBoxMesh()), undefined, authored],
        [coloredBox(), undefined, undefined],
        [coloredBox(), RED, authored],
        [parseMeshPayload(makeCylinderMesh()), undefined, undefined],
        [parseMeshPayload(makeCylinderMesh()), RED, undefined],
      ];
      for (const [view, bodyColor, faceColors] of cases) {
        const entry = reg.buildBodyObjects(view, "body1", 1, bodyColor, faceColors);
        // The plan is the layout that came out, in both directions.
        expect(entry.plan.layout).toBe(entry.hasVertexColors ? "deindexed" : "indexed");
        expect(entry.geometry.getIndex() === null).toBe(entry.plan.layout === "deindexed");
        entry.dispose();
      }
      expect(logSnapshot().filter((e) => e.level === "error")).toEqual([]);
    } finally {
      __resetLogForTests({ enabled: false });
    }
  });

  it("REPORTS off-plan when a caller supplies a plan for a different appearance", () => {
    __resetLogForTests();
    try {
      const view = parseMeshPayload(makeBoxMesh());
      const validated = validateMeshView(view, "body1");
      if (!validated.ok) throw new Error("the mock box must validate");
      // Priced uncoloured, built with a body colour: the classic PR-03A pairing.
      const stale = planMeshPreparation(validated.mesh, {}, "body1");
      if (!stale.ok) throw new Error("the mock box must be plannable");

      const entry = reg.buildBodyObjects(view, "body1", 1, RED, undefined, undefined, stale.plan);

      const errors = logSnapshot().filter((e) => e.level === "error").map((e) => e.msg);
      expect(errors).toContain("mesh plan does not match the appearance it is being built with");
      // The build FOLLOWED the plan it was given, so the geometry is indexed and
      // the byte totals agree — the appearance check is what catches this pair.
      expect(entry.plan.layout).toBe("indexed");
      expect(entry.hasVertexColors).toBe(false);
      entry.dispose();
    } finally {
      __resetLogForTests({ enabled: false });
    }
  });

  it("REPORTS off-plan when the built bytes do not add up to the priced bytes", () => {
    __resetLogForTests();
    try {
      const view = parseMeshPayload(makeBoxMesh());
      const validated = validateMeshView(view, "body1");
      if (!validated.ok) throw new Error("the mock box must validate");
      const real = planMeshPreparation(validated.mesh, {}, "body1");
      if (!real.ok) throw new Error("the mock box must be plannable");
      const understated = { ...real.plan, positionBytes: real.plan.positionBytes - 4 };

      const entry = reg.buildBodyObjects(view, "body1", 1, undefined, undefined, undefined, understated);

      const off = logSnapshot().filter((e) => e.msg === "mesh built off-plan");
      expect(off).toHaveLength(1);
      expect(off[0].ctx).toMatchObject({ bodyId: "body1", layout: "indexed" });
      entry.dispose();
    } finally {
      __resetLogForTests({ enabled: false });
    }
  });

  it("refreshFaceColors reaches every REGISTERED entry", () => {
    const colored = reg.buildBodyObjects(coloredBox(), "body1", 1);
    const plain = reg.buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body2", 1);
    reg.swap("body1", colored);
    reg.swap("body2", plain);
    const spy = vi.spyOn(colored, "rebakeFaceColors");
    const plainSpy = vi.spyOn(plain, "rebakeFaceColors");

    reg.refreshFaceColors();

    expect(spy).toHaveBeenCalledTimes(1);
    expect(plainSpy).toHaveBeenCalledTimes(1); // uniform call, no-op inside
  });
});

describe("disposeAll leak tripwire", () => {
  it("empties the registry with no console.error when clean", () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    reg.swap("body1", box(1));
    reg.swap("body2", box(1));
    expect(reg.registrySize()).toBe(2);
    const before = reg.leakTripwireCount;
    reg.disposeAll();
    expect(reg.registrySize()).toBe(0);
    expect(reg.leakTripwireCount).toBe(before); // no leak detected
    expect(err).not.toHaveBeenCalled();
    err.mockRestore();
  });
});

/*
 * WP03 ownership (spec §8.2). The registry is the UNIQUE disposer, and
 * retirement is ordered against the frame loop: nothing is freed inside the
 * frame it was retired in, and nothing is freed while a borrower holds it.
 */
describe("TEST-RES-02 — leases and ordered retirement", () => {
  it("carries a full resource identity on every entry", () => {
    const entry = box(1);
    expect(entry.identity.bodyId).toBe("body1");
    expect(entry.identity.geometryRevision).toBe(1);
    expect(entry.identity.topologySignature).toMatch(/^[0-9a-f]{16}$/);
    expect(entry.resourceState).toBe("installed");
    entry.dispose();
  });

  it("does not dispose a swapped-out entry on the frame it was swapped", () => {
    const first = box(1);
    reg.swap("body1", first);
    const spy = vi.spyOn(first, "dispose");

    reg.swap("body1", box(2));

    // Retired immediately, freed later: a draw call already recorded for this
    // frame may still name it.
    expect(first.resourceState).toBe("retired");
    expect(spy).not.toHaveBeenCalled();

    reg.flushDisposals(); // the frame ends
    expect(spy).toHaveBeenCalledTimes(1);
    expect(first.resourceState).toBe("disposed");
  });

  it("keeps a retired entry alive while a highlight lease is open", () => {
    const first = box(1);
    reg.swap("body1", first);
    const lease = reg.acquireLease(first, "highlight:body");
    const spy = vi.spyOn(first, "dispose");

    reg.swap("body1", box(2));
    expect(first.resourceState).toBe("retired");
    expect(reg.openLeases(first)).toEqual(["highlight:body"]);

    // Any number of frames may pass; the borrow is what holds it.
    reg.flushDisposals();
    reg.flushDisposals();
    expect(spy).not.toHaveBeenCalled();
    expect(first.geometry.getAttribute("position").array).toBe(first.view.positions);

    lease.release();
    expect(reg.openLeases(first)).toEqual([]);
    // Released mid-frame ⇒ freed at the NEXT boundary, not this one.
    expect(spy).not.toHaveBeenCalled();

    reg.flushDisposals();
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("release is idempotent and counts per owner", () => {
    const entry = box(1);
    const a = reg.acquireLease(entry, "body");
    const b = reg.acquireLease(entry, "section");
    const c = reg.acquireLease(entry, "section");
    expect([...reg.openLeases(entry)].sort()).toEqual(["body", "section", "section"]);

    b.release();
    b.release(); // no double decrement
    expect([...reg.openLeases(entry)].sort()).toEqual(["body", "section"]);

    a.release();
    c.release();
    expect(reg.openLeases(entry)).toEqual([]);
    entry.dispose();
  });

  it("reports the OWNER TAGS when disposeAll finds an undetached borrower", () => {
    __resetLogForTests({ enabled: true, console: false });
    const entry = box(1);
    reg.swap("body1", entry);
    reg.acquireLease(entry, "section"); // never released — the leak
    const before = reg.leakTripwireCount;

    reg.disposeAll();

    expect(reg.leakTripwireCount).toBe(before + 1);
    const errors = logSnapshot().filter((e) => e.level === "error");
    expect(errors.map((e) => e.msg)).toContain("mesh resource disposed with OPEN leases");
    expect(errors[0].ctx).toMatchObject({ bodyId: "body1", owners: ["section"] });
    expect(entry.resourceState).toBe("disposed");
    __resetLogForTests({ enabled: false });
  });

  it("a clean close leaves no open leases and no tripwire", () => {
    const entry = box(1);
    reg.swap("body1", entry);
    const lease = reg.acquireLease(entry, "body");
    const before = reg.leakTripwireCount;

    lease.release();
    reg.disposeAll();

    expect(reg.leakTripwireCount).toBe(before);
    expect(reg.registryDebugSnapshot()).toMatchObject({
      openLeases: 0,
      installed: [],
      retired: [],
    });
  });

  it("a lease on an already-disposed entry is refused, once, with a live bail-out", () => {
    __resetLogForTests({ enabled: true, console: false });
    const entry = box(1);
    entry.dispose();

    const first = reg.acquireLease(entry, "body");
    const second = reg.acquireLease(entry, "highlight:body");

    expect(first.entry.resourceState).toBe("disposed");
    expect(second.entry.resourceState).toBe("disposed");
    expect(reg.openLeases(entry)).toEqual([]);
    expect(
      logSnapshot().filter((e) => e.msg === "lease acquired on a DISPOSED mesh resource"),
    ).toHaveLength(1);
    __resetLogForTests({ enabled: false });
  });

  it("reports open leases and cache occupancy to the viewport counters", () => {
    resetViewportCounters();
    const entry = box(1);
    const a = reg.acquireLease(entry, "body");
    const b = reg.acquireLease(entry, "section");
    expect(readViewportCounters().leasesOpen).toBe(2);

    a.release();
    b.release();
    expect(readViewportCounters().leasesOpen).toBe(0);
    entry.dispose();
  });

  it("remove retires rather than freeing, and notifies retirement listeners", () => {
    const entry = box(1);
    reg.swap("body1", entry);
    const seen: string[] = [];
    const off = reg.onEntryRetired((e) => seen.push(e.bodyId));

    reg.remove("body1");

    expect(seen).toEqual(["body1"]);
    expect(entry.resourceState).toBe("retired");
    reg.flushDisposals();
    expect(entry.resourceState).toBe("disposed");
    off();
  });
});

/*
 * VP-HARDENING PR-02 / PR-03B — the commit phase is observer-free, and a
 * reservation lives exactly as long as the buffers it paid for.
 *
 * `commitSwap` / `commitRemove` are the whole state transition; `notifyRetired`
 * is the separate, isolated announcement. Publishing a mesh calls the first
 * pair inside its transaction and the second one only after the scene, the
 * handle map and the budget have all moved.
 */
describe("TEST-PUB-03 — two-phase commit and reservation lifetime", () => {
  it("commitSwap moves the state and calls NO listener", () => {
    const first = box(1);
    reg.swap("body1", first);
    const seen: string[] = [];
    const off = reg.onEntryRetired((e) => seen.push(e.bodyId));
    const second = box(2);

    const prev = reg.commitSwap("body1", second);

    expect(prev).toBe(first);
    expect(reg.getEntry("body1")).toBe(second);
    expect(first.resourceState).toBe("retired");
    expect(seen).toEqual([]); // nothing observable ran inside the transaction

    reg.notifyRetired(prev);
    expect(seen).toEqual(["body1"]);
    off();
  });

  it("commitRemove retires silently, and notifyRetired(undefined) is a no-op", () => {
    const only = box(1);
    reg.swap("body1", only);
    const seen: string[] = [];
    const off = reg.onEntryRetired((e) => seen.push(e.bodyId));

    const prev = reg.commitRemove("body1");

    expect(prev).toBe(only);
    expect(reg.getEntry("body1")).toBeUndefined();
    expect(seen).toEqual([]);
    // The first publication of a body has no predecessor — the announcement
    // must tolerate that rather than being guarded at every call site.
    expect(() => reg.notifyRetired(reg.commitRemove("never-registered"))).not.toThrow();
    expect(seen).toEqual([]);

    reg.notifyRetired(prev);
    expect(seen).toEqual(["body1"]);
    off();
  });

  it("isolates a throwing retirement listener and still calls the others", () => {
    __resetLogForTests({ enabled: true, console: false });
    const entry = box(1);
    reg.swap("body1", entry);
    const seen: string[] = [];
    const offA = reg.onEntryRetired(() => {
      throw new Error("listener exploded");
    });
    const offB = reg.onEntryRetired((e) => seen.push(e.bodyId));
    try {
      expect(() => reg.remove("body1")).not.toThrow();

      expect(seen).toEqual(["body1"]);
      expect(entry.resourceState).toBe("retired");
      const errors = logSnapshot().filter((e) => e.level === "error");
      expect(errors.map((e) => e.msg)).toEqual(["retire listener threw"]);
      expect(errors[0].ctx).toMatchObject({ bodyId: "body1" });
    } finally {
      // Subscriptions are module state: a failed assertion must not leave a
      // throwing listener wired into every later case in this file.
      offA();
      offB();
      __resetLogForTests({ enabled: false });
    }
  });

  it("releases the reservation when the buffers are freed, never at the swap", () => {
    const admission = new MeshAdmission();
    const first = box(1);
    const reservation = admission.reserve("body1", first.plan);
    if (!reservation.ok) throw new Error("the mock box must be admissible");
    first.reservation = reservation;
    reg.swap("body1", first);
    const charged = admission.snapshot();
    expect(charged.holdings).toBe(1);

    reg.swap("body1", box(2));
    // Retired is not freed: the buffers are still there, so they still cost.
    expect(first.resourceState).toBe("retired");
    expect(admission.snapshot()).toEqual(charged);

    const lease = reg.acquireLease(first, "highlight:body");
    reg.flushDisposals();
    reg.flushDisposals();
    expect(first.resourceState).toBe("retired"); // a borrower holds it open
    expect(admission.snapshot()).toEqual(charged);

    lease.release();
    expect(admission.snapshot()).toEqual(charged); // freed at the NEXT boundary

    reg.flushDisposals();
    expect(first.resourceState).toBe("disposed");
    expect(first.reservation).toBeNull();
    expect(admission.snapshot()).toEqual({
      preparedCpuBytes: 0,
      estimatedGpuBytes: 0,
      holdings: 0,
    });
  });

  it("gives the reservation back on the dispose of an entry that was never installed", () => {
    const admission = new MeshAdmission();
    const entry = box(1);
    const reservation = admission.reserve("body1", entry.plan);
    if (!reservation.ok) throw new Error("the mock box must be admissible");
    entry.reservation = reservation;
    expect(admission.snapshot().holdings).toBe(1);

    entry.dispose(); // preparation failure: no registry, no scene, no frame

    expect(entry.reservation).toBeNull();
    expect(admission.snapshot().holdings).toBe(0);
    entry.dispose(); // idempotent
    expect(admission.snapshot().holdings).toBe(0);
  });
});
