/*
 * BodyObject — a body's scene presence: a face Mesh + an edge LineSegments,
 * both wrapping the registry's zero-copy geometry. The face material carries a
 * polygonOffset so edge lines sit cleanly on top without z-fighting.
 *
 * Materials come from a BodyMaterialLibrary and are SHARED across every body
 * drawing in the same render mode — a BodyObject owns neither geometry
 * (registry-owned) nor materials (library-owned), so tearing it down is just
 * removing the group from its root. `userData.bodyId`/`kind` let the Picker
 * resolve an intersection back to a body + element.
 */
import * as THREE from "three";
import type { MeshEntry } from "../mesh/meshRegistry";
import type { BodyMaterialLibrary } from "./bodyMaterials";
import { DEFAULT_RENDER_MODE, RENDER_MODES, type RenderModeDef } from "./renderModes";

export interface BodyObjectHandle {
  bodyId: string;
  group: THREE.Group;
  setVisible(visible: boolean): void;
  /**
   * Apply a render-mode descriptor: toggle the two CHILDREN to the mode's
   * face/edge visibility, then point both at the mode's shared material set.
   *
   * Deliberately NOT `material.wireframe = true` for the wireframe mode, on two
   * counts: the edge LineSegments already carry the kernel's real topological
   * edges — the wireframe a CAD user expects, rather than a triangulation — and
   * the face material is shared by every body of that kind, so flipping a flag
   * on it would leak across all of them.
   *
   * ACCEPTED CONSEQUENCE: in `wireframe` the face Mesh is invisible, and the
   * Picker raycasts with `traverseVisible`, so faces are not pickable there —
   * you pick what you see.
   *
   * Independent of {@link setVisible}, which owns the GROUP's flag.
   */
  applyMode(def: RenderModeDef): void;
}

/**
 * Build the face + edge objects for `entry` under one group, materialled from
 * `library` at the DEFAULT mode. The eager assignment is load-bearing: preview
 * bodies are built and shown without anyone ever calling {@link applyMode}.
 */
export function buildBodyObject(entry: MeshEntry, library: BodyMaterialLibrary): BodyObjectHandle {
  const materials = library.get(RENDER_MODES[DEFAULT_RENDER_MODE].materialKind);
  const group = new THREE.Group();
  group.name = `body:${entry.bodyId}`;
  group.userData.bodyId = entry.bodyId;

  const faceMesh = new THREE.Mesh(entry.geometry, materials.face);
  faceMesh.userData.bodyId = entry.bodyId;
  faceMesh.userData.kind = "face";
  group.add(faceMesh);

  let edges: THREE.LineSegments | null = null;
  if (entry.edgeGeometry) {
    edges = new THREE.LineSegments(entry.edgeGeometry, materials.edge);
    edges.userData.bodyId = entry.bodyId;
    edges.userData.kind = "edge";
    group.add(edges);
  }

  return {
    bodyId: entry.bodyId,
    group,
    setVisible(visible: boolean) {
      group.visible = visible;
    },
    applyMode(def: RenderModeDef) {
      faceMesh.visible = def.faceVisible;
      const set = library.get(def.materialKind);
      faceMesh.material = set.face;
      if (edges) {
        edges.visible = def.edgeVisible;
        edges.material = set.edge;
      }
    },
  };
}
