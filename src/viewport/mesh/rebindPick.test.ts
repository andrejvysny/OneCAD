/*
 * rebindPick — resolving a ref against a mesh, and re-resolving one a regen
 * renamed.
 *
 * The whole point of this module is what it REFUSES, so most of these specs are
 * negative: a rebind that guesses is worse than a highlight that vanishes, and
 * the two pieces of evidence it guesses from (a world point, and the direction
 * the OLD mesh gave the element) each have to be able to veto on their own.
 *
 * Geometry is real MESH1: `encodeMesh1` writes the same bytes the worker would,
 * so the face/edge ordinals, id tables and buffers are the ones the app resolves
 * through — no hand-built stand-in for `BodyMeshView`.
 */
import { describe, it, expect, afterEach } from "vitest";
import { encodeMesh1, type EdgeSource, type FaceSource } from "@/ipc/mockMeshes";
import { buildBodyObjects, type MeshEntry } from "./meshRegistry";
import { parseMeshPayload } from "./parseMeshPayload";
import { ordinalForRef, rebindRef, rebindRefs, rebindSelectionForBody } from "./rebindPick";
import { selectionStore, topoRefId, type EntityRef } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";

type V3 = [number, number, number];

const live: MeshEntry[] = [];
afterEach(() => {
  for (const e of live.splice(0)) e.dispose();
  resetStores();
});

/** One planar quad, split into the two triangles `encodeMesh1` concatenates. */
interface Quad {
  id: string;
  corners: [V3, V3, V3, V3];
}

/**
 * Build a registry entry from quads (+ optional polyline edges). Face normals
 * come out of the corner WINDING, exactly as they would from a real tessellation.
 */
function meshOf(quads: Quad[], edges: EdgeSource[] = [], faceBboxes = false): MeshEntry {
  const positions: number[] = [];
  const faces: FaceSource[] = [];
  for (const q of quads) {
    const base = positions.length / 3;
    for (const c of q.corners) positions.push(c[0], c[1], c[2]);
    faces.push({
      id: q.id,
      triangles: [
        [base, base + 1, base + 2],
        [base, base + 2, base + 3],
      ],
    });
  }
  const blob = encodeMesh1({ positions, faces, edges, faceBboxes });
  const entry = buildBodyObjects(parseMeshPayload(blob), "b1", 1);
  live.push(entry);
  return entry;
}

/** +Z-facing quad at height `z`, spanning ±10 in x/y. */
const top = (id: string, z = 5): Quad => ({
  id,
  corners: [
    [-10, -10, z],
    [10, -10, z],
    [10, 10, z],
    [-10, 10, z],
  ],
});

/** −Z-facing quad at height `z` (reversed winding) — same plane, opposite side. */
const under = (id: string, z = 5): Quad => ({
  id,
  corners: [
    [-10, -10, z],
    [-10, 10, z],
    [10, 10, z],
    [10, -10, z],
  ],
});

/** +X-facing quad at x=10, spanning z ∈ [−5,5]. */
const side = (id: string): Quad => ({
  id,
  corners: [
    [10, -10, -5],
    [10, 10, -5],
    [10, 10, 5],
    [10, -10, 5],
  ],
});

const faceRef = (topoKey: string, worldPoint?: V3, elementId?: string): EntityRef => ({
  kind: "face",
  id: topoRefId("b1", topoKey),
  bodyId: "b1",
  topoKey,
  elementId,
  ...(worldPoint ? { anchor: { worldPoint } } : {}),
});

const edgeRef = (topoKey: string, worldPoint?: V3): EntityRef => ({
  kind: "edge",
  id: topoRefId("b1", topoKey),
  bodyId: "b1",
  topoKey,
  ...(worldPoint ? { anchor: { worldPoint } } : {}),
});

/** A slab whose three faces carry the given ids (the shape never changes). */
const slab = (ids: [string, string, string], faceBboxes = false): MeshEntry =>
  meshOf([top(ids[0]), under(ids[1], -5), side(ids[2])], [], faceBboxes);

// ── ordinalForRef ───────────────────────────────────────────────────────────

describe("ordinalForRef", () => {
  it("resolves through the TopoKey when the mesh names it", () => {
    const e = slab(["f:1", "f:2", "f:3"]);
    expect(ordinalForRef(e.faceIndex, e.view, faceRef("f:3"))).toBe(2);
  });

  it("returns -1 for a TopoKey the mesh no longer names", () => {
    const e = slab(["f:1", "f:2", "f:3"]);
    expect(ordinalForRef(e.faceIndex, e.view, faceRef("f:99"))).toBe(-1);
  });

  it("falls back to the promoted ElementId only when the mesh carries ElementIds", () => {
    // IDS_HAVE_ELEMENTIDS clear: the id table holds TopoKeys, so an ElementId in
    // it would be a coincidence — never a match to lean on.
    const plain = meshOf([top("el_top")]);
    expect(ordinalForRef(plain.faceIndex, plain.view, faceRef("f:gone", undefined, "el_top"))).toBe(-1);

    const minted = buildBodyObjects(
      parseMeshPayload(
        encodeMesh1({
          positions: [-10, -10, 5, 10, -10, 5, 10, 10, 5, -10, 10, 5],
          faces: [{ id: "el_top", triangles: [[0, 1, 2], [0, 2, 3]] }],
          idsHaveElementIds: true,
        }),
      ),
      "b1",
      1,
    );
    live.push(minted);
    expect(ordinalForRef(minted.faceIndex, minted.view, faceRef("f:gone", undefined, "el_top"))).toBe(0);
  });
});

// ── faces ───────────────────────────────────────────────────────────────────

describe("rebindRef (face)", () => {
  it("re-binds a pure renumber: same geometry, new ids", () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    expect(rebindRef(faceRef("f:1", [2, 2, 5]), prev, next)).toBe("f:7");
    expect(rebindRef(faceRef("f:3", [10, 3, 0]), prev, next)).toBe("f:9");
  });

  it("uses the old face's centroid when the ref carries no anchor", () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    expect(rebindRef(faceRef("f:1"), prev, next)).toBe("f:7");
  });

  it("REFUSES when the face moved out from under its anchor", () => {
    // The classic parametric edit: grow the extrude and the selected top face is
    // 20 mm away. Nothing is at the anchor any more, so nothing is bound.
    const prev = meshOf([top("f:1", 5)]);
    const next = meshOf([top("f:7", 25)]);
    expect(rebindRef(faceRef("f:1", [2, 2, 5]), prev, next)).toBeNull();
  });

  it("REFUSES a face sitting exactly at the anchor that points the OTHER way", () => {
    // Distance alone would take this: it is coincident with the old face. The
    // direction test is the only thing between the user and a silent mis-bind.
    const prev = meshOf([top("f:1", 5)]);
    const next = meshOf([under("f:7", 5)]);
    expect(rebindRef(faceRef("f:1", [2, 2, 5]), prev, next)).toBeNull();
  });

  it("REFUSES when the previous mesh cannot name the ref either (no evidence)", () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    expect(rebindRef(faceRef("f:404", [2, 2, 5]), prev, next)).toBeNull();
  });

  it("picks the nearest same-facing candidate, not merely the first", () => {
    // Two parallel +Z faces; the anchor is on the upper one.
    const prev = meshOf([top("f:1", 5)]);
    const next = meshOf([top("f:low", -5), top("f:high", 5)]);
    expect(rebindRef(faceRef("f:1", [2, 2, 5]), prev, next)).toBe("f:high");
  });

  it("agrees with itself whether or not FACE_BBOXES is present", () => {
    // The bbox section is a PREFILTER: it may only make the search cheaper.
    const prev = slab(["f:1", "f:2", "f:3"], true);
    const next = slab(["f:7", "f:8", "f:9"], true);
    expect(next.view.hasFaceBboxes).toBe(true);
    expect(rebindRef(faceRef("f:1", [2, 2, 5]), prev, next)).toBe("f:7");
    expect(rebindRef(faceRef("f:1", [2, 2, 25]), prev, next)).toBeNull();
  });
});

// ── edges ───────────────────────────────────────────────────────────────────

const EDGE_ALONG_X: EdgeSource[] = [{ id: "e:1", points: [[-10, -10, 5], [10, -10, 5]] }];

describe("rebindRef (edge)", () => {
  it("re-binds an edge that only changed its id", () => {
    const prev = meshOf([top("f:1")], EDGE_ALONG_X);
    const next = meshOf([top("f:7")], [{ id: "e:9", points: [[-10, -10, 5], [10, -10, 5]] }]);
    expect(rebindRef(edgeRef("e:1", [0, -10, 5]), prev, next)).toBe("e:9");
  });

  it("re-binds a REVERSED edge — direction is absolute for a 1-D element", () => {
    const prev = meshOf([top("f:1")], EDGE_ALONG_X);
    const next = meshOf([top("f:7")], [{ id: "e:9", points: [[10, -10, 5], [-10, -10, 5]] }]);
    expect(rebindRef(edgeRef("e:1", [0, -10, 5]), prev, next)).toBe("e:9");
  });

  it("REFUSES an edge crossing the anchor at a different angle", () => {
    const prev = meshOf([top("f:1")], EDGE_ALONG_X);
    const next = meshOf([top("f:7")], [{ id: "e:9", points: [[0, -30, 5], [0, 10, 5]] }]);
    expect(rebindRef(edgeRef("e:1", [0, -10, 5]), prev, next)).toBeNull();
  });

  it("REFUSES an edge that moved away (a fillet consumed it)", () => {
    const prev = meshOf([top("f:1")], EDGE_ALONG_X);
    const next = meshOf([top("f:7")], [{ id: "e:9", points: [[-10, -5, 5], [10, -5, 5]] }]);
    expect(rebindRef(edgeRef("e:1", [0, -10, 5]), prev, next)).toBeNull();
  });

  it("REFUSES when the new mesh has no edges at all", () => {
    const prev = meshOf([top("f:1")], EDGE_ALONG_X);
    const next = meshOf([top("f:7")]);
    expect(rebindRef(edgeRef("e:1", [0, -10, 5]), prev, next)).toBeNull();
  });
});

// ── the ref rewrite ─────────────────────────────────────────────────────────

describe("rebindRefs", () => {
  const prev = (): MeshEntry => slab(["f:1", "f:2", "f:3"]);
  const next = (): MeshEntry => slab(["f:7", "f:8", "f:9"]);

  it("rewrites topoKey and NEVER the id (identity is the pick-time composite)", () => {
    const ref = faceRef("f:1", [2, 2, 5]);
    const out = rebindRefs("b1", prev(), next(), [ref])!;
    expect(out[0].topoKey).toBe("f:7");
    expect(out[0].id).toBe("b1#f:1");
    expect(out[0].anchor).toEqual(ref.anchor); // untouched
  });

  it("returns null when nothing was stale (no store write, no repaint)", () => {
    const p = prev();
    const n = next();
    expect(rebindRefs("b1", p, n, [faceRef("f:7", [2, 2, 5])])).toBeNull();
    expect(rebindRefs("b1", p, n, [])).toBeNull();
  });

  it("leaves other bodies, other kinds and un-rebindable refs alone", () => {
    const p = prev();
    const n = next();
    const foreign: EntityRef = { kind: "face", id: "b2#f:1", bodyId: "b2", topoKey: "f:1" };
    const body: EntityRef = { kind: "body", id: "b1" };
    const stale = faceRef("f:2", [2, 2, -5]);
    const out = rebindRefs("b1", p, n, [foreign, body, stale])!;
    expect(out[0]).toBe(foreign);
    expect(out[1]).toBe(body);
    expect(out[2].topoKey).toBe("f:8"); // the −Z face, matched on its own normal
  });
});

describe("rebindSelectionForBody", () => {
  it("re-points both the selection and the hover at the new ids", () => {
    const p = slab(["f:1", "f:2", "f:3"]);
    const n = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5])]);
    selectionStore.getState().setHover(faceRef("f:3", [10, 3, 0]));

    rebindSelectionForBody("b1", p, n);

    expect(selectionStore.getState().selected[0].topoKey).toBe("f:7");
    expect(selectionStore.getState().hover?.topoKey).toBe("f:9");
  });

  it("is a no-op with no previous entry — a first load has nothing to rebind FROM", () => {
    const n = slab(["f:7", "f:8", "f:9"]);
    const before = [faceRef("f:1", [2, 2, 5])];
    selectionStore.getState().set(before);

    rebindSelectionForBody("b1", undefined, n);

    expect(selectionStore.getState().selected).toBe(before);
  });
});
