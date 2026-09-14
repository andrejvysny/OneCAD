/*
 * HighlightLayer: the pure slice math, the overlay material contract, and the
 * OWNERSHIP contract of VP-HARDENING WP03 (spec §8.2/§8.3, finding R02) — face
 * and edge overlays own compact buffers and are disposed; a whole-body overlay
 * borrows the registry's exact geometry object under a lease and disposes
 * nothing; no overlay ever shares a BufferAttribute with the body.
 *
 * Face ranges are triangle units → index units (×3). Edge ranges stay in
 * SEGMENT units: body edges are instanced fat lines, where one instance IS one
 * segment, so there is no ×2 vertex expansion to undo any more.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import * as THREE from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { faceDrawRange, edgeSegmentSlice, HighlightLayer } from "./HighlightLayer";
import { palette } from "./palette";
import {
  buildBodyObjects,
  swap,
  disposeAll,
  getEntry,
  openLeases,
  __resetRegistryForTests,
} from "../mesh/meshRegistry";
import { readViewportCounters } from "../vph/instrumentation";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { encodeMesh1, makeBoxMesh } from "@/ipc/mockMeshes";

afterEach(() => {
  disposeAll();
  __resetRegistryForTests();
});

const deps = () => ({ root: new THREE.Group(), invalidate: vi.fn() });

/** A registered box — addHighlight resolves bodies through the REGISTRY. */
function registerBox(bodyId = "body1"): void {
  swap(bodyId, buildBodyObjects(parseMeshPayload(makeBoxMesh()), bodyId, 1));
}

describe("faceDrawRange (indexed geometry, 3 indices/triangle)", () => {
  // 3 faces: face0 tris[0,3), face1[3,2), face2[5,3).
  const faceRanges = new Uint32Array([0, 3, 3, 2, 5, 3]);
  it("maps face ordinal → {start,count} in index units", () => {
    expect(faceDrawRange(faceRanges, 0)).toEqual({ start: 0, count: 9 });
    expect(faceDrawRange(faceRanges, 1)).toEqual({ start: 9, count: 6 });
    expect(faceDrawRange(faceRanges, 2)).toEqual({ start: 15, count: 9 });
  });
});

describe("edgeSegmentSlice (instanced fat lines, 1 instance/segment)", () => {
  // 2 edges: edge0 segs[0,2), edge1 segs[2,1).
  const segRanges = new Uint32Array([0, 2, 2, 1]);

  it("maps edge ordinal → {first,count} in SEGMENT units (no ×2)", () => {
    expect(edgeSegmentSlice(segRanges, 0)).toEqual({ first: 0, count: 2 });
    expect(edgeSegmentSlice(segRanges, 1)).toEqual({ first: 2, count: 1 });
  });

  it("addresses the segment buffer at 6 floats per segment", () => {
    // The contract buildEdge relies on: [first*6, (first+count)*6) is exactly
    // this edge's endpoints in the entry's expanded positions array.
    const positions = new Float32Array(3 * 6).map((_, i) => i);
    const { first, count } = edgeSegmentSlice(segRanges, 1);
    expect([...positions.subarray(first * 6, (first + count) * 6)]).toEqual([
      12, 13, 14, 15, 16, 17,
    ]);
  });
});

/*
 * Selection/hover read as three distinct states in the viewport, so the three
 * face materials must not collapse into one another: cyan hover, selected tint
 * per element, and a LIGHTER selected tint for a whole body (a body's worth of
 * surface at the per-face opacity buries the shading that reads as shape).
 */
describe("overlay materials", () => {
  it("hover is the cyan viewport token, selection is the selected tint", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState({ kind: "face", id: "body1#f:1", bodyId: "body1", topoKey: "f:1" }, [
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" },
    ]);

    const mats = d.root.children.map(
      (o) => (o as THREE.Mesh).material as THREE.MeshBasicMaterial,
    );
    expect(mats).toHaveLength(2);
    const hover = mats.find((m) => m.opacity === 0.45)!;
    const selected = mats.find((m) => m.opacity === 0.55)!;
    expect(hover.color.getHex()).toBe(palette.hover3d().getHex());
    expect(selected.color.getHex()).toBe(palette.selected3d().getHex());
    layer.dispose();
  });

  it("a whole selected BODY uses its own, lighter tint material", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState(null, [{ kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" }]);
    const faceMat = (d.root.children[0] as THREE.Mesh).material as THREE.MeshBasicMaterial;

    layer.setState(null, [{ kind: "body", id: "body1" }]);
    const bodyMat = (d.root.children[0] as THREE.Mesh).material as THREE.MeshBasicMaterial;

    expect(bodyMat).not.toBe(faceMat);
    expect(bodyMat.color.getHex()).toBe(faceMat.color.getHex()); // same token…
    expect(bodyMat.opacity).toBe(0.45); // …lighter than the 0.55 face tint
    layer.dispose();
  });

  it("edge overlays are fat lines, heavier than a body edge, drawn over it", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState({ kind: "edge", id: "body1#e:1", bodyId: "body1", topoKey: "e:1" }, []);
    const line = d.root.children[0] as LineSegments2;
    expect(line).toBeInstanceOf(LineSegments2);

    const mat = line.material;
    expect(mat.color.getHex()).toBe(palette.hover3d().getHex());
    expect(mat.linewidth).toBeGreaterThan(1.5); // > BODY_EDGE_WIDTH at dpr 1
    expect(mat.depthTest).toBe(false);
    expect(mat.transparent).toBe(true);
    layer.dispose();
  });

  it("refreshColors re-reads every shared material", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    layer.setState({ kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" }, [
      { kind: "body", id: "body1" },
    ]);
    const before = d.root.children.map((o) =>
      ((o as THREE.Mesh).material as THREE.Material & { color: THREE.Color }).color.getHex(),
    );

    layer.refreshColors();

    // No theme flip here — the point is that refreshColors touches the live
    // materials rather than throwing or swapping them out.
    expect(
      d.root.children.map((o) =>
        ((o as THREE.Mesh).material as THREE.Material & { color: THREE.Color }).color.getHex(),
      ),
    ).toEqual(before);
    layer.dispose();
  });
});

/*
 * Section clipping reaches materials, not geometry: the six shared overlay
 * materials sit outside both BodyMaterialLibrary instances, so nothing else can
 * clip them.
 */
describe("setClippingPlanes (section view)", () => {
  const planes = () => [new THREE.Plane(new THREE.Vector3(0, 0, -1), 0)];

  it("reaches all SIX overlay materials, including the ones not drawn yet", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const p = planes();

    layer.setClippingPlanes(p);

    // Hover + selection, face + edge, at once: that is four of the six; the
    // whole-body tint is the fifth and the degraded outline the sixth.
    layer.setState({ kind: "edge", id: "body1#e:1", bodyId: "body1", topoKey: "e:1" }, [
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" },
      { kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" },
      { kind: "body", id: "body1" },
    ]);
    const mats = new Set<THREE.Material>();
    d.root.traverse((o) => {
      const m = (o as THREE.Mesh).material as THREE.Material | undefined;
      if (m) mats.add(m);
    });
    expect(mats.size).toBeGreaterThan(0);
    for (const m of mats) expect(m.clippingPlanes).toBe(p);

    layer.setClippingPlanes(null);
    for (const m of mats) expect(m.clippingPlanes).toBeNull();
    layer.dispose();
  });

  it("recompiles only on a plane COUNT change, and repaints every time", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    layer.setState(null, [{ kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" }]);
    const mat = (d.root.children[0] as THREE.Mesh).material as THREE.Material;
    const p = planes();

    layer.setClippingPlanes(p);
    const version = mat.version;
    const repaints = d.invalidate.mock.calls.length; // setState repainted too
    layer.setClippingPlanes(p);
    layer.setClippingPlanes(p);

    expect(mat.version).toBe(version);
    // Still one repaint per call: the plane MOVED even when nothing recompiled.
    expect(d.invalidate.mock.calls.length).toBe(repaints + 2);
    layer.dispose();
  });
});

/*
 * WP03 ownership (spec §8.2/§8.3). The rejected shortcuts of guide §22 are all
 * measured AGAINST here: no overlay may share a BufferAttribute with the body,
 * no undisposed wrapper may accumulate, and a scene-child count is not evidence
 * of anything — every assertion below reads the registry, the cache, or a
 * dispose spy.
 */
describe("geometry ownership on rebuild", () => {
  const faceRef = (ord: number) => ({
    kind: "face" as const,
    id: `body1#f:${ord}`,
    bodyId: "body1",
    topoKey: `f:${ord}`,
  });

  it("TEST-RES-01: 1,000 alternating face hovers plateau and never touch the source", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const entry = getEntry("body1")!;
    const sourcePosition = entry.geometry.getAttribute("position");
    const sourceIndex = entry.geometry.getIndex();

    // Warm up, THEN start counting: the first two hovers are the allocations
    // the plateau is measured from (ACCEPTANCE §3.3).
    layer.setState(faceRef(0), []);
    layer.setState(faceRef(1), []);
    const warm = layer.resourceStats();
    const disposeSpy = vi.spyOn(THREE.BufferGeometry.prototype, "dispose");

    for (let i = 0; i < 1000; i++) layer.setState(faceRef(i % 2), []);

    // No owned geometry was created or destroyed after warm-up: the per-(body,
    // role) buffer is rewritten in place, so nothing accumulates and nothing
    // churns.
    expect(disposeSpy).not.toHaveBeenCalled();
    expect(layer.resourceStats().entries).toBe(warm.entries);
    expect(layer.resourceStats().entries).toBeLessThanOrEqual(2);
    expect(layer.resourceStats().bytes).toBe(warm.bytes);
    expect(d.root.children).toHaveLength(1);

    // The body is untouched: same geometry, same attribute OBJECTS, still the
    // zero-copy arrays the parser produced.
    expect(entry.geometry.getAttribute("position")).toBe(sourcePosition);
    expect(entry.geometry.getIndex()).toBe(sourceIndex);
    expect(sourcePosition.array).toBe(entry.view.positions);

    // And the overlay never shares a buffer with it.
    const overlay = (d.root.children[0] as THREE.Mesh).geometry;
    expect(overlay).not.toBe(entry.geometry);
    expect(overlay.getAttribute("position")).not.toBe(sourcePosition);
    expect(overlay.getAttribute("position").array).not.toBe(sourcePosition.array);

    // The instrumentation counters see the same plateau (ACCEPTANCE §3.3).
    expect(readViewportCounters().highlightEntries).toBe(warm.entries);
    expect(readViewportCounters().highlightBytesOwned).toBe(warm.bytes);

    disposeSpy.mockRestore();
    layer.dispose();
    // Teardown balances the warm-up allocations rather than leaking them.
    expect(layer.resourceStats()).toEqual({ entries: 0, bytes: 0, displayed: 0 });
    expect(readViewportCounters().highlightBytesOwned).toBe(0);
  });

  it("TEST-RES-02: a whole-body overlay borrows the exact geometry under ONE lease", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const entry = getEntry("body1")!;

    layer.setState({ kind: "body", id: "body1" }, []);

    expect(d.root.children).toHaveLength(1);
    expect((d.root.children[0] as THREE.Mesh).geometry).toBe(entry.geometry);
    expect(openLeases(entry)).toEqual(["highlight:body"]);
    // Nothing owned: a leased overlay costs the cache nothing.
    expect(layer.resourceStats().bytes).toBe(0);

    layer.setState(null, []);
    expect(openLeases(entry)).toEqual([]);
    layer.dispose();
  });

  it("a 5-face selection is ONE owned buffer, and a 6th face reuses that buffer", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const entry = getEntry("body1")!;

    layer.setState(null, [0, 1, 2, 3, 4].map(faceRef));

    // One object, one cache slot — not five overlays and not five buffers.
    expect(d.root.children).toHaveLength(1);
    expect(layer.resourceStats().entries).toBe(1);
    const five = (d.root.children[0] as THREE.Mesh).geometry;
    // 5 box faces × 2 triangles × 3 indices, and the ordinals stay EXTERNAL.
    expect(five.drawRange.count).toBe(30);
    expect(d.root.children[0].userData.faceOrdinals).toEqual([0, 1, 2, 3, 4]);
    expect(five.getAttribute("position").array).not.toBe(entry.view.positions);

    // A different 5-face set: cache MISS (different ordinals) that reuses the
    // slot's buffers in place — same geometry object, no second cache entry.
    layer.setState(null, [0, 1, 2, 3, 5].map(faceRef));
    expect((d.root.children[0] as THREE.Mesh).geometry).toBe(five);
    expect(layer.resourceStats().entries).toBe(1);

    // Six faces no longer fit: the buffer grows and the slot moves with it —
    // still exactly one owned buffer for this (body, role).
    layer.setState(null, [0, 1, 2, 3, 4, 5].map(faceRef));
    expect(d.root.children).toHaveLength(1);
    expect(layer.resourceStats().entries).toBe(1);
    expect((d.root.children[0] as THREE.Mesh).geometry.drawRange.count).toBe(36);
    layer.dispose();
  });

  it("dispose releases every lease and frees every owned overlay", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const entry = getEntry("body1")!;

    layer.setState({ kind: "body", id: "body1" }, [
      faceRef(0),
      { kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" },
    ]);
    expect(layer.resourceStats().entries).toBe(2); // face set + edge
    expect(openLeases(entry)).toEqual(["highlight:body"]);

    layer.dispose();

    expect(openLeases(entry)).toEqual([]);
    expect(layer.resourceStats()).toEqual({ entries: 0, bytes: 0, displayed: 0 });
    expect(d.root.children).toHaveLength(0);
    // The source survived its overlays being freed.
    expect(entry.geometry.getAttribute("position").array).toBe(entry.view.positions);
  });

  it("an edge overlay owns its segment buffer and is disposed with the cache", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const entry = getEntry("body1")!;

    layer.setState(null, [{ kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" }]);
    const edge = d.root.children[0] as LineSegments2;
    const edgeSpy = vi.spyOn(edge.geometry, "dispose");
    const sourceSpy = vi.spyOn(entry.geometry, "dispose");

    // Deselecting only unpins — the overlay stays cached for the next hover.
    layer.setState(null, []);
    expect(edgeSpy).not.toHaveBeenCalled();
    expect(layer.resourceStats().entries).toBe(1);

    layer.dispose();
    expect(edgeSpy).toHaveBeenCalledTimes(1);
    expect(sourceSpy).not.toHaveBeenCalled();
  });

  it("a mesh swap retires the overlays cut from the old resource", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const first = getEntry("body1")!;

    layer.setState(null, [faceRef(0)]);
    const before = (d.root.children[0] as THREE.Mesh).geometry;
    expect(layer.resourceStats().entries).toBe(1);

    swap("body1", buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 2));

    // The overlay was rebuilt against the NEW resource, and the old owned
    // buffer went with the resource it was cut from.
    expect(layer.resourceStats().entries).toBe(1);
    expect((d.root.children[0] as THREE.Mesh).geometry).not.toBe(before);
    expect(openLeases(first)).toEqual([]);
    layer.dispose();
  });

  it("the edge overlay covers only its own edge's segments", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState(null, [{ kind: "edge", id: "body1#e:3", bodyId: "body1", topoKey: "e:3" }]);
    const geo = (d.root.children[0] as LineSegments2).geometry;
    // A box edge is a single 2-point polyline ⇒ exactly one instanced segment.
    expect(geo.instanceCount).toBe(1);
    layer.dispose();
  });
});

/*
 * Element lookup goes through `ordinalForRef`, not the raw TopoKey: a TopoKey is
 * snapshot-scoped and a regen may rename it, while a PROMOTED ref also carries
 * the persistent ElementId — which is what a MESH1 blob with IDS_HAVE_ELEMENTIDS
 * puts in its id tables. Resolving one key only would drop the overlay of a
 * selection that is still perfectly well identified.
 */
describe("resolving a ref against the mesh", () => {
  it("drops the overlay when NEITHER id names anything (never a guess)", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState(null, [
      { kind: "face", id: "body1#f:99", bodyId: "body1", topoKey: "f:99", elementId: "el_x" },
      { kind: "edge", id: "body1#e:99", bodyId: "body1", topoKey: "e:99" },
    ]);

    expect(d.root.children).toHaveLength(0);
    layer.dispose();
  });

  it("falls back to the promoted ElementId when the mesh's ids ARE ElementIds", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    // A blob whose face id table holds minted ids (IDS_HAVE_ELEMENTIDS set).
    swap(
      "body1",
      buildBodyObjects(
        parseMeshPayload(
          encodeMesh1({
            positions: [0, 0, 0, 10, 0, 0, 10, 10, 0, 0, 10, 0],
            faces: [{ id: "el_top", triangles: [[0, 1, 2], [0, 2, 3]] }],
            idsHaveElementIds: true,
          }),
        ),
        "body1",
        1,
      ),
    );

    // The TopoKey is stale; only the ElementId still names the face.
    layer.setState(null, [
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0", elementId: "el_top" },
    ]);

    expect(d.root.children).toHaveLength(1);
    layer.dispose();
  });

  it("does NOT reach for the ElementId when the mesh's ids are TopoKeys", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox(); // IDS_HAVE_ELEMENTIDS clear — "f:0" is a TopoKey, not an id

    layer.setState(null, [
      { kind: "face", id: "body1#f:99", bodyId: "body1", topoKey: "f:99", elementId: "f:0" },
    ]);

    expect(d.root.children).toHaveLength(0);
    layer.dispose();
  });
});

/*
 * DEV-WP03-1 — the DEGRADED display (spec §8.3 last paragraph). When the exact
 * overlays cannot fit the budget the semantic selection is kept, one diagnostic
 * is emitted per change, and the body is drawn as a leased OUTLINE plus a
 * selection COUNT — never as a whole-body tint, which reads as "the body is
 * selected" and loses which faces the user picked.
 */
describe("degraded selection display", () => {
  const faceRef = (ord: number, bodyId = "body1") => ({
    kind: "face" as const,
    id: `${bodyId}#f:${ord}`,
    bodyId,
    topoKey: `f:${ord}`,
  });

  /** A budget that fits one 4-face overlay (384 B) but not that plus a hover. */
  const tightDeps = () => ({
    root: new THREE.Group(),
    invalidate: vi.fn(),
    budget: { maxBytes: 400 },
    onDegraded: vi.fn(),
  });

  it("draws the leased body OUTLINE plus a count, not a whole-body tint", () => {
    const d = tightDeps();
    const layer = new HighlightLayer(d);
    registerBox("body1");
    registerBox("body2");
    const two = getEntry("body2")!;

    // body1's 4-face selection fills the budget and is PINNED, so body2's
    // hover has nothing left to evict.
    layer.setState(faceRef(0, "body2"), [0, 1, 2, 3].map((o) => faceRef(o, "body1")));

    expect(layer.degraded).toBe(true);
    const degraded = d.root.children.find((o) => o.userData.bodyId === "body2")!;
    expect(degraded).toBeInstanceOf(LineSegments2);
    // The EXACT leased edge geometry — no copy, nothing to dispose.
    expect((degraded as LineSegments2).geometry).toBe(two.edgeGeometry);
    expect(openLeases(two)).toEqual(["highlight:body"]);
    // Not the whole-body tint: a translucent MeshBasicMaterial over the FACE
    // geometry is the display DEV-WP03-1 replaced.
    expect((degraded as LineSegments2).material).toBeInstanceOf(LineMaterial);
    expect((degraded as LineSegments2).geometry).not.toBe(two.geometry);

    // …and the count reaches the chip layer.
    expect(d.onDegraded).toHaveBeenCalled();
    const calls = d.onDegraded.mock.calls;
    const last = calls[calls.length - 1][0];
    expect(last).toEqual([expect.objectContaining({ bodyId: "body2", count: 1 })]);

    layer.dispose();
  });

  it("re-attempts the exact overlay on every rebuild, and recovers", () => {
    const d = tightDeps();
    const layer = new HighlightLayer(d);
    registerBox("body1");
    registerBox("body2");
    const two = getEntry("body2")!;

    layer.setState(faceRef(0, "body2"), [0, 1, 2, 3].map((o) => faceRef(o, "body1")));
    expect(layer.degraded).toBe(true);

    // The pinned pressure goes away; the SAME hover is still desired, and the
    // displayed-key short-circuit must not keep the degraded overlay on screen.
    layer.setState(faceRef(0, "body2"), []);

    expect(layer.degraded).toBe(false);
    const exact = d.root.children[0] as THREE.Mesh;
    expect(exact.geometry).not.toBe(two.edgeGeometry);
    expect(exact.geometry.getAttribute("position").array).not.toBe(two.view.positions);
    expect(openLeases(two)).toEqual([]);
    const calls = d.onDegraded.mock.calls;
    expect(calls[calls.length - 1][0]).toEqual([]);

    layer.dispose();
  });

  /** A body with NO edges — `entry.edgeGeometry` is null, so there is no outline. */
  function registerEdgelessBody(bodyId: string): void {
    const positions: number[] = [];
    for (let v = 0; v < 12; v++) positions.push(v, v * 2, v * 3);
    swap(
      bodyId,
      buildBodyObjects(
        parseMeshPayload(
          encodeMesh1({
            positions,
            faces: [0, 1, 2, 3].map((f) => ({
              id: `f:${f}`,
              triangles: [[f * 3, f * 3 + 1, f * 3 + 2] as [number, number, number]],
            })),
          }),
        ),
        bodyId,
        1,
      ),
    );
  }

  /*
   * A body with no edge geometry has no outline to lease, so the degraded
   * display falls back to the whole-body tint. That fallback is still a
   * DEGRADED overlay: it must publish its count, read `degraded`, and be
   * re-attempted on the next rebuild. Returning the ordinary body overlay here
   * silently reinstated exactly the display DEV-WP03-1 removed.
   */
  it("an edgeless body still counts as degraded, and still recovers", () => {
    const d = tightDeps();
    const layer = new HighlightLayer(d);
    registerBox("body1");
    registerEdgelessBody("body2");
    const two = getEntry("body2")!;
    expect(two.edgeGeometry).toBeNull();

    layer.setState(faceRef(0, "body2"), [0, 1, 2, 3].map((o) => faceRef(o, "body1")));

    expect(layer.degraded).toBe(true);
    const calls = d.onDegraded.mock.calls;
    expect(calls[calls.length - 1][0]).toEqual([
      expect.objectContaining({ bodyId: "body2", count: 1 }),
    ]);
    // The leased whole-body object, not an owned overlay.
    const shown = d.root.children.find((o) => o.userData.bodyId === "body2") as THREE.Mesh;
    expect(shown.geometry).toBe(two.geometry);
    expect(openLeases(two)).toEqual(["highlight:body"]);

    // …and the same hover recovers once the budget frees.
    layer.setState(faceRef(0, "body2"), []);

    expect(layer.degraded).toBe(false);
    expect((d.root.children[0] as THREE.Mesh).geometry).not.toBe(two.geometry);
    expect(openLeases(two)).toEqual([]);
    const after = d.onDegraded.mock.calls;
    expect(after[after.length - 1][0]).toEqual([]);
    layer.dispose();
  });

  /*
   * The chip is anchored in WORLD space, so a body that moved needs a fresh
   * notice even though its id and count are unchanged.
   */
  it("republishes the notice when the anchor moves", () => {
    const d = tightDeps();
    const layer = new HighlightLayer(d);
    registerBox("body1");
    registerBox("body2");
    const two = getEntry("body2")!;

    layer.setState(faceRef(0, "body2"), [0, 1, 2, 3].map((o) => faceRef(o, "body1")));
    const before = d.onDegraded.mock.calls.length;
    const first = d.onDegraded.mock.calls[before - 1][0][0];

    two.geometry.boundingSphere!.center.set(100, 200, 300);
    layer.refresh();

    expect(d.onDegraded.mock.calls.length).toBe(before + 1);
    const latest = d.onDegraded.mock.calls[before][0][0];
    expect(latest.world).toEqual([100, 200, 300]);
    expect(latest.world).not.toEqual(first.world);
    layer.dispose();
  });

  it("keeps the whole semantic selection while degraded", () => {
    const d = tightDeps();
    const layer = new HighlightLayer(d);
    registerBox("body1");
    registerBox("body2");

    layer.setState(faceRef(0, "body2"), [0, 1, 2, 3].map((o) => faceRef(o, "body1")));

    // Both bodies still have an overlay — the refused one is degraded, not dropped.
    const ids = d.root.children.map((o) => o.userData.bodyId).sort();
    expect(ids).toEqual(["body1", "body2"]);
    layer.dispose();
  });
});
