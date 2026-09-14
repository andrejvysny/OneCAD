/*
 * GhostLayer OWNERSHIP (VP-HARDENING spec §8.2, decision D6, finding PR-06).
 *
 * The pattern/mirror/transform ghosts are the last borrowers that were neither
 * owners nor lessees: a whole-body ghost pointed a Mesh at the registry's
 * geometry with no lease, and a RANGED ghost (OffsetFace) built a shallow
 * drawRange wrapper over the body's BufferAttributes that could never be
 * disposed — R02 in a second editing path.
 *
 * The two shapes now split cleanly: unranged ghosts LEASE the exact geometry,
 * ranged ghosts OWN a compact copy. `hide()` gives back exactly what it took.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import * as THREE from "three";
import { GhostLayer } from "./GhostLayer";
import { faceDrawRange } from "./HighlightLayer";
import {
  buildBodyObjects,
  swap,
  remove,
  disposeAll,
  flushDisposals,
  getEntry,
  openLeases,
  __resetRegistryForTests,
  type MeshEntry,
} from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

afterEach(() => {
  disposeAll();
  __resetRegistryForTests();
});

const deps = () => ({ root: new THREE.Group(), invalidate: vi.fn() });

function registerBox(bodyId = "body1"): MeshEntry {
  const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), bodyId, 1);
  swap(bodyId, entry);
  return entry;
}

const MOVE = [{ kind: "translate" as const, offset: [1, 0, 0] as [number, number, number] }];

/** The ghost clones currently attached (the layer keeps them under one group). */
function ghostMeshes(d: { root: THREE.Object3D }): THREE.Mesh[] {
  const out: THREE.Mesh[] = [];
  d.root.traverse((o) => {
    if ((o as THREE.Mesh).isMesh) out.push(o as THREE.Mesh);
  });
  return out;
}

describe("unranged ghosts lease the exact registry geometry", () => {
  it("draws the entry's own geometry under a `ghost` lease, released by hide", () => {
    const d = deps();
    const layer = new GhostLayer(d);
    const entry = registerBox();

    layer.show(entry, MOVE);

    const meshes = ghostMeshes(d);
    expect(meshes).toHaveLength(1);
    expect(meshes[0].geometry).toBe(entry.geometry);
    expect(openLeases(entry)).toEqual(["ghost"]);

    layer.hide();
    expect(openLeases(entry)).toEqual([]);
    layer.dispose();
  });

  it("takes ONE lease per source body however many clones it draws", () => {
    const d = deps();
    const layer = new GhostLayer(d);
    const entry = registerBox();

    layer.show(entry, [MOVE[0], { kind: "translate", offset: [2, 0, 0] }, MOVE[0]]);

    expect(ghostMeshes(d)).toHaveLength(3);
    expect(openLeases(entry)).toEqual(["ghost"]);
    layer.dispose();
    expect(openLeases(entry)).toEqual([]);
  });

  it("a replacing show releases the previous borrow", () => {
    const d = deps();
    const layer = new GhostLayer(d);
    const first = registerBox("body1");
    const second = registerBox("body2");

    layer.show(first, MOVE);
    layer.show(second, MOVE);

    expect(openLeases(first)).toEqual([]);
    expect(openLeases(second)).toEqual(["ghost"]);
    layer.dispose();
  });

  it("keeps a retired source alive until the ghost clears", () => {
    const d = deps();
    const layer = new GhostLayer(d);
    const entry = registerBox();

    layer.show(entry, MOVE);
    remove("body1"); // the source retires mid-preview
    flushDisposals();
    flushDisposals();

    // The borrow outranks the retirement clock — the ghost is still drawing it.
    expect(entry.resourceState).toBe("retired");

    layer.hide();
    flushDisposals();
    expect(entry.resourceState).toBe("disposed");
    layer.dispose();
  });
});

describe("ranged ghosts own a compact copy", () => {
  it("copies the triangle range and disposes it on hide (OffsetFace)", () => {
    const d = deps();
    const layer = new GhostLayer(d);
    const entry = registerBox();
    const range = faceDrawRange(entry.view.faceRanges, 2);

    layer.showMulti([{ entry, transforms: MOVE, range }]);

    const geometry = ghostMeshes(d)[0].geometry;
    expect(geometry).not.toBe(entry.geometry);
    expect(geometry.getAttribute("position").array).not.toBe(entry.view.positions);
    expect(geometry.getIndex()!.count).toBe(range.count);
    // Owned, so nothing is leased and nothing of the body's is shared.
    expect(openLeases(entry)).toEqual([]);
    expect(geometry.getAttribute("position")).not.toBe(entry.geometry.getAttribute("position"));

    const ghostSpy = vi.spyOn(geometry, "dispose");
    const sourceSpy = vi.spyOn(entry.geometry, "dispose");

    layer.hide();

    expect(ghostSpy).toHaveBeenCalledTimes(1);
    expect(sourceSpy).not.toHaveBeenCalled();
    layer.dispose();
  });

  it("frees every owned ghost geometry when the layer is disposed", () => {
    const d = deps();
    const layer = new GhostLayer(d);
    const entry = registerBox();
    const ranges = [0, 1, 2].map((o) => faceDrawRange(entry.view.faceRanges, o));

    layer.showMulti(ranges.map((range) => ({ entry, transforms: MOVE, range })));
    const spies = ghostMeshes(d).map((m) => vi.spyOn(m.geometry, "dispose"));
    expect(spies).toHaveLength(3);

    layer.dispose();

    for (const spy of spies) expect(spy).toHaveBeenCalledTimes(1);
    expect(openLeases(entry)).toEqual([]);
  });

  it("mixes leased and owned sources in one showMulti", () => {
    const d = deps();
    const layer = new GhostLayer(d);
    const whole = registerBox("body1");
    const sliced = registerBox("body2");
    const range = faceDrawRange(sliced.view.faceRanges, 0);

    layer.showMulti([
      { entry: whole, transforms: MOVE },
      { entry: sliced, transforms: MOVE, range },
    ]);

    expect(openLeases(whole)).toEqual(["ghost"]);
    expect(openLeases(sliced)).toEqual([]);
    const owned = ghostMeshes(d).find((m) => m.geometry !== whole.geometry)!.geometry;
    const ownedSpy = vi.spyOn(owned, "dispose");

    layer.hide();

    expect(openLeases(whole)).toEqual([]);
    expect(ownedSpy).toHaveBeenCalledTimes(1);
    expect(getEntry("body1")!.geometry.getAttribute("position").array).toBe(whole.view.positions);
    layer.dispose();
  });
});
