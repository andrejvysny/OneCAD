/*
 * BodyObject display-mode matrix (W3).
 *
 * The mode is expressed as CHILD visibility (face Mesh / edge LineSegments), not
 * as a material swap — the materials are shared engine-wide, so a
 * `material.wireframe = true` would leak onto preview bodies too.
 */
import { describe, it, expect, afterEach } from "vitest";
import * as THREE from "three";
import { buildBodyObject, createBodyMaterials } from "./BodyObject";
import { buildBodyObjects, __resetRegistryForTests, disposeAll } from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

afterEach(() => {
  disposeAll();
  __resetRegistryForTests();
});

function handleFor() {
  const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1);
  const materials = createBodyMaterials();
  const handle = buildBodyObject(entry, materials);
  const face = handle.group.children.find((c) => c.userData.kind === "face") as THREE.Mesh;
  const edges = handle.group.children.find(
    (c) => c.userData.kind === "edge",
  ) as THREE.LineSegments;
  return { handle, face, edges, entry, materials };
}

describe("BodyObjectHandle.setDisplayMode", () => {
  it("shaded = faces only · shadedEdges = both · wireframe = edges only", () => {
    const { handle, face, edges, entry, materials } = handleFor();
    expect(face).toBeDefined();
    expect(edges).toBeDefined();

    handle.setDisplayMode("shaded");
    expect([face.visible, edges.visible]).toEqual([true, false]);

    handle.setDisplayMode("shadedEdges");
    expect([face.visible, edges.visible]).toEqual([true, true]);

    handle.setDisplayMode("wireframe");
    expect([face.visible, edges.visible]).toEqual([false, true]);

    entry.dispose();
    materials.dispose();
  });

  it("never touches the SHARED materials (no material.wireframe flip)", () => {
    const { handle, entry, materials } = handleFor();
    handle.setDisplayMode("wireframe");
    expect(materials.face.wireframe).toBe(false);
    expect(materials.face.visible).toBe(true);
    entry.dispose();
    materials.dispose();
  });

  it("is independent of setVisible — the group flag survives a mode change", () => {
    const { handle, face, entry, materials } = handleFor();
    handle.setVisible(false);
    handle.setDisplayMode("shaded");
    expect(handle.group.visible).toBe(false);
    expect(face.visible).toBe(true); // the mode says faces; the group still hides them

    handle.setVisible(true);
    expect(handle.group.visible).toBe(true);
    entry.dispose();
    materials.dispose();
  });
});
