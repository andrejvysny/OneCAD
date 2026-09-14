/*
 * rebindPick — resolving a ref against a mesh, and what happens to one a regen
 * renamed or consumed.
 *
 * The whole point of this module is what it REFUSES. It used to search the new
 * mesh for the nearest element matching the old one's position and direction;
 * D-5 (PLAN 2026-09-11) removed that, because a plausible answer the backend
 * never agreed to is exactly the silent-wrong-bind class the identity ladder
 * exists to prevent. So the specs below are about EVIDENCE: the new mesh names
 * it, or the backend resolves its promoted ElementId, or it goes away.
 *
 * A SNAPSHOT ORDINAL IS NOT EVIDENCE ACROSS A REGEN (Astra break F1). Element
 * counts only grow over the measured regens — a 3 mm chamfer takes the box from
 * 12 to 15 edges, each hole adds two faces and three edges — so an old `f:N` /
 * `e:N` almost always still names SOMETHING afterwards, just not the element
 * that was picked. The transient key is therefore only evidence WITHIN the one
 * publication it was read from; across a regen the authority is the persistent
 * ElementId (named by the new mesh, or confirmed by the backend) or nothing.
 *
 * Geometry is real MESH1: `encodeMesh1` writes the same bytes the worker would,
 * so the face/edge ordinals, id tables and buffers are the ones the app resolves
 * through — no hand-built stand-in for `BodyMeshView`.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { encodeMesh1, type EdgeSource, type FaceSource } from "@/ipc/mockMeshes";
import type { CadClient } from "@/ipc/client";
import type { ElementInfo } from "@/ipc/types";
import { buildBodyObjects, swap, __resetRegistryForTests, type MeshEntry } from "./meshRegistry";
import { parseMeshPayload } from "./parseMeshPayload";
import { dropSelectionForBody, ordinalForRef, reconcileSelectionForBody } from "./rebindPick";
import { attachPickProof, pickProofFor } from "./pickProof";
import { selectionStore, topoRefId, type EntityRef } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";

type V3 = [number, number, number];

const live: MeshEntry[] = [];
afterEach(() => {
  __resetRegistryForTests();
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
function meshOf(quads: Quad[], edges: EdgeSource[] = [], idsHaveElementIds = false): MeshEntry {
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
  const blob = encodeMesh1({ positions, faces, edges, idsHaveElementIds });
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

const edgeRef = (topoKey: string, elementId?: string): EntityRef => ({
  kind: "edge",
  id: topoRefId("b1", topoKey),
  bodyId: "b1",
  topoKey,
  elementId,
});

/** One edge the new mesh keeps — enough to give it a populated EDGE table. */
const SURVIVING_EDGE: EdgeSource[] = [{ id: "e:9", points: [[-10, 10, 5], [10, 10, 5]] }];

/** A slab whose three faces carry the given ids (the shape never changes). */
const slab = (ids: [string, string, string]): MeshEntry =>
  meshOf([top(ids[0]), under(ids[1], -5), side(ids[2])]);

/**
 * A face table with the given ids in ORDINAL order (one +Z quad per id, stacked
 * in z so every face is distinct). `idsHaveElementIds` makes it the MESH1
 * two-namespace table `Tessellate.cpp` emits: minted ids for bound elements,
 * snapshot TopoKeys for everything else.
 */
const stack = (ids: string[], idsHaveElementIds = true): MeshEntry =>
  meshOf(ids.map((id, i) => top(id, i)), [], idsHaveElementIds);

/** Register `entry` as the body's live mesh (what the supersede guard reads). */
function publish(entry: MeshEntry): MeshEntry {
  swap("b1", entry);
  return entry;
}

/**
 * One REGEN swap: `prev` was on screen, `next` has just been published. This is
 * the ONLY reload that reconciles — `meshSync` does not call it for a colour /
 * visibility / self-heal reload, and there is no `prev` on a first load.
 */
function regen(prev: MeshEntry, next: MeshEntry, client: CadClient | null): void {
  publish(next);
  reconcileSelectionForBody("b1", prev, next, client);
}

/** A promise plus its resolver — an `elementInfo` answer held mid-flight. */
function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/** A `CadClient` stub whose ONLY live method is `elementInfo`. */
function clientWith(elementInfo: CadClient["elementInfo"]): CadClient {
  return { elementInfo } as unknown as CadClient;
}

/** The fields `reconcileSelectionForBody` fences an `elementInfo` answer on. */
function infoNaming(topoKey: string, over?: Partial<ElementInfo>): ElementInfo {
  return {
    elementId: "el_x",
    topoKey,
    bodyId: "b1",
    kind: "face",
    surfaceType: 0,
    curveType: -1,
    center: [0, 0, 0],
    normal: [0, 0, 1],
    hasNormal: true,
    size: 1,
    magnitude: 1,
    ...over,
  };
}

const selected = (): EntityRef[] => selectionStore.getState().selected;
const flush = () => new Promise((r) => setTimeout(r, 0));

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

  it("PREFERS the promoted ElementId over a TopoKey the same table also names", () => {
    // Astra break F1, the proved counterexample. Nine faces: the ref's old
    // ordinal `f:7` is STILL in the table (ordinal 6) but now names a different
    // face, while the element the user actually picked is bound and therefore
    // labelled `el_x` (ordinal 8). Reading the transient key first draws 6.
    const e = stack(["f:1", "f:2", "f:3", "f:4", "f:5", "f:6", "f:7", "f:8", "el_x"]);
    expect(e.faceIndex.ordinalForId("f:7")).toBe(6); // the trap is really set
    expect(ordinalForRef(e.faceIndex, e.view, faceRef("f:7", undefined, "el_x"))).toBe(8);
  });

  it("falls back to the promoted ElementId only when the mesh carries ElementIds", () => {
    // IDS_HAVE_ELEMENTIDS clear: the id table holds TopoKeys, so an ElementId in
    // it would be a coincidence — never a match to lean on.
    const plain = meshOf([top("el_top")]);
    expect(ordinalForRef(plain.faceIndex, plain.view, faceRef("f:gone", undefined, "el_top"))).toBe(-1);

    const minted = meshOf([top("el_top")], [], true);
    expect(ordinalForRef(minted.faceIndex, minted.view, faceRef("f:gone", undefined, "el_top"))).toBe(0);
  });
});

// ── the swap policy (D-5: selection never guesses) ──────────────────────────

describe("reconcileSelectionForBody", () => {
  // ── (a) the new mesh names the persistent id ──────────────────────────────

  it("keeps a ref the NEW mesh names by its promoted id, and asks the backend nothing", async () => {
    // `Tessellate.cpp` substitutes the minted ElementId for a bound element, so
    // the id table itself is authority enough. The ref adopts that label, which
    // is what `HighlightLayer` then draws through.
    const prev = stack(["f:1", "f:2"]);
    const next = stack(["f:7", "f:8", "el_top"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);
    const elementInfo = vi.fn();

    regen(prev, next, clientWith(elementInfo));
    await flush();

    expect(selected()).toHaveLength(1);
    expect(selected()[0].topoKey).toBe("el_top"); // drawable in THIS publication
    expect(selected()[0].id).toBe("b1#f:1"); // identity never moves
    expect(selected()[0].elementId).toBe("el_top");
    expect(elementInfo).not.toHaveBeenCalled();
  });

  it("resolves the nine-face trap to the PROMOTED element, with zero queries", async () => {
    // Astra break F1 end to end: `f:7` survives at ordinal 6 naming another
    // face; the picked element is bound and sits at ordinal 8 as `el_x`.
    const prev = stack(["f:7"]);
    const next = stack(["f:1", "f:2", "f:3", "f:4", "f:5", "f:6", "f:7", "f:8", "el_x"]);
    selectionStore.getState().set([faceRef("f:7", [2, 2, 5], "el_x")]);
    const elementInfo = vi.fn();

    regen(prev, next, clientWith(elementInfo));
    await flush();

    expect(elementInfo).not.toHaveBeenCalled();
    expect(ordinalForRef(next.faceIndex, next.view, selected()[0])).toBe(8);
  });

  // ── (c) an unpromoted ref has no authority across a regen ─────────────────

  it("DROPS an unpromoted ref even when its old ordinal still resolves", async () => {
    // The consumed-edge counterexample: after the chamfer the box has MORE
    // edges than before, so `e:12` still names something — a different edge.
    // Nothing has confirmed it, so it goes.
    const prev = meshOf([top("f:1")], [{ id: "e:12", points: [[-10, 10, 5], [10, 10, 5]] }]);
    const next = meshOf(
      [top("f:1")],
      [
        { id: "e:12", points: [[-10, -10, 5], [10, -10, 5]] },
        { id: "e:13", points: [[-10, 10, 5], [10, 10, 5]] },
      ],
    );
    const ref = edgeRef("e:12");
    selectionStore.getState().set([ref]);
    const elementInfo = vi.fn();

    regen(prev, next, clientWith(elementInfo));
    await flush();

    expect(selected()).toEqual([]);
    expect(elementInfo).not.toHaveBeenCalled(); // nothing to ask about
  });

  it("DROPS a stale unpromoted ref synchronously, and silently", () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5]), faceRef("f:7")]);

    regen(prev, next, clientWith(vi.fn()));

    expect(selected()).toEqual([]);
  });

  // ── (b) the backend confirms the persistent id ────────────────────────────

  it("holds a promoted ref in place, then adopts the TopoKey the BACKEND returns", async () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);
    const elementInfo = vi.fn(async () => infoNaming("f:7"));

    regen(prev, next, clientWith(elementInfo));
    expect(selected()).toHaveLength(1); // no flicker while the query is in flight
    await flush();

    expect(elementInfo).toHaveBeenCalledWith("b1", "el_top", "");
    expect(selected()[0].topoKey).toBe("f:7");
    expect(selected()[0].id).toBe("b1#f:1"); // identity is the pick-time composite
    expect(selected()[0].elementId).toBe("el_top");
  });

  it("DROPS a promoted ref the backend cannot resolve (the consumed edge)", async () => {
    const prev = meshOf([top("f:7")], SURVIVING_EDGE);
    const next = meshOf([top("f:7")], SURVIVING_EDGE);
    selectionStore.getState().set([edgeRef("e:1", "el_edge")]);

    regen(prev, next, clientWith(vi.fn(async () => null)));
    await flush();

    expect(selected()).toEqual([]);
  });

  it("DROPS a promoted ref when the query itself fails — a refusal is not evidence", async () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(
      prev,
      next,
      clientWith(
        vi.fn(async () => {
          throw new Error("worker gone");
        }),
      ),
    );
    await flush();

    expect(selected()).toEqual([]);
  });

  it("DROPS a promoted ref when there is no client to confirm it with", () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(prev, next, null);

    expect(selected()).toEqual([]);
  });

  // ── F2: the answer has to be about THIS publication ───────────────────────

  it("DROPS an answer whose TopoKey the DISPLAYED mesh does not name", async () => {
    // The head raced ahead: `elementInfo` answers from S=3 while S=2 is still on
    // screen. An undrawable label would leave an invisible, authorable selection.
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(prev, next, clientWith(vi.fn(async () => infoNaming("f:99"))));
    await flush();

    expect(selected()).toEqual([]);
  });

  it("DROPS an answer that belongs to another body", async () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(prev, next, clientWith(vi.fn(async () => infoNaming("f:7", { bodyId: "b2" }))));
    await flush();

    expect(selected()).toEqual([]);
  });

  it("DROPS an answer whose kind is not the ref's", async () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(prev, next, clientWith(vi.fn(async () => infoNaming("f:7", { kind: "edge" }))));
    await flush();

    expect(selected()).toEqual([]);
  });

  it("ignores an answer a LATER mesh swap superseded", async () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(prev, next, clientWith(vi.fn(async () => infoNaming("f:7"))));
    publish(slab(["f:11", "f:12", "f:13"])); // a second regen lands first
    await flush();

    // Neither adopted nor dropped by the stale answer: the newer swap owns the
    // ref now (and ran its own reconcile pass).
    expect(selected()[0].topoKey).toBe("f:1");
  });

  // ── F3: a reply applies to the ref OBJECT, never to its reusable id ────────

  it("never re-adds a ref the user deselected while the query was in flight", async () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().set([faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(prev, next, clientWith(vi.fn(async () => infoNaming("f:7"))));
    selectionStore.getState().clear(); // the user moved on
    await flush();

    expect(selected()).toEqual([]);
  });

  it("leaves a FRESH pick that reuses the deselected ref's id untouched", async () => {
    // `EntityRef.id` is the PICK-TIME `${bodyId}#${topoKey}` and never moves, so
    // a ref whose label an earlier confirmation rewrote (`f:7` → `f:12`) keeps an
    // id a later pick of the current `f:7` mints again. Matching a reply on that
    // string deleted the new selection outright.
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    const stale: EntityRef = {
      kind: "face",
      id: topoRefId("b1", "f:7"),
      bodyId: "b1",
      topoKey: "f:12",
      elementId: "el_top",
    };
    selectionStore.getState().set([stale]);
    const gate = deferred<ElementInfo | null>();

    regen(prev, next, clientWith(vi.fn(() => gate.promise)));
    const fresh = faceRef("f:7", [3, 3, 5]); // same id, a DIFFERENT object
    selectionStore.getState().set([fresh]);
    gate.resolve(null); // the old query finally answers "gone"
    await flush();

    expect(selected()).toEqual([fresh]);
  });

  // ── F4: no evidence is not a licence to keep an invisible selection ───────

  it("CONFIRMS an edge ref when the new blob carries no edge table at all", async () => {
    // A blob with no EDGE sections cannot draw an edge highlight, so "absent
    // evidence" is not a reason to keep the ref — it is a reason to ask.
    const prev = meshOf([top("f:1")], SURVIVING_EDGE);
    const next = meshOf([top("f:1")]);
    expect(next.edgeIndex).toBeNull();
    selectionStore.getState().set([edgeRef("e:1", "el_edge")]);
    const elementInfo = vi.fn(async () => null);

    regen(prev, next, clientWith(elementInfo));
    await flush();

    expect(elementInfo).toHaveBeenCalledWith("b1", "el_edge", "");
    expect(selected()).toEqual([]);
  });

  it("DROPS an unpromoted edge ref when the new blob carries no edge table", () => {
    const prev = meshOf([top("f:1")], SURVIVING_EDGE);
    const next = meshOf([top("f:1")]);
    selectionStore.getState().set([edgeRef("e:1")]);

    regen(prev, next, clientWith(vi.fn()));

    expect(selected()).toEqual([]);
  });

  // ── hover, other bodies, and the reloads that are NOT a regen ─────────────

  it("drops a stale HOVER and never queries the backend for one", async () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    selectionStore.getState().setHover(faceRef("f:1", [2, 2, 5], "el_top"));
    const elementInfo = vi.fn(async () => infoNaming("f:7"));

    regen(prev, next, clientWith(elementInfo));
    await flush();

    expect(selectionStore.getState().hover).toBeNull();
    expect(elementInfo).not.toHaveBeenCalled();
  });

  it("clears hover even when the new mesh names its promoted id", () => {
    const prev = stack(["f:1"]);
    const next = stack(["f:7", "el_top"]);
    selectionStore.getState().setHover(faceRef("f:1", undefined, "el_top"));

    regen(prev, next, clientWith(vi.fn()));

    expect(selectionStore.getState().hover).toBeNull();
  });

  it("leaves other bodies and other kinds alone", () => {
    const prev = slab(["f:1", "f:2", "f:3"]);
    const next = slab(["f:7", "f:8", "f:9"]);
    const foreign: EntityRef = { kind: "face", id: "b2#f:1", bodyId: "b2", topoKey: "f:1" };
    const body: EntityRef = { kind: "body", id: "b1" };
    const sketch: EntityRef = { kind: "sketch", id: "sk1" };
    const stale = faceRef("f:2");
    selectionStore.getState().set([foreign, body, sketch, stale]);

    regen(prev, next, clientWith(vi.fn()));

    expect(selected()).toEqual([foreign, body, sketch]);
  });

  it("does NOTHING on a first load — there is no previous publication to survive", async () => {
    // The body's very first mesh. The pick that armed the tool was taken against
    // this same publication, so there is nothing to reconcile and nothing to ask.
    const next = publish(slab(["f:7", "f:8", "f:9"]));
    const before = [faceRef("f:1", [2, 2, 5]), faceRef("f:2", undefined, "el_top")];
    selectionStore.getState().set(before);
    const elementInfo = vi.fn();

    reconcileSelectionForBody("b1", undefined, next, clientWith(elementInfo));
    await flush();

    expect(selected()).toBe(before); // identity: no store write, no repaint
    expect(elementInfo).not.toHaveBeenCalled();
  });

  it("does NOTHING when the swap re-publishes the very same entry", async () => {
    const next = publish(slab(["f:7", "f:8", "f:9"]));
    const before = [faceRef("f:1", [2, 2, 5])];
    selectionStore.getState().set(before);

    reconcileSelectionForBody("b1", next, next, clientWith(vi.fn()));
    await flush();

    expect(selected()).toBe(before);
  });

  // ── the pick-time proof does not survive a regen (PR-01) ──────────────────

  /*
   * A ref kept across a regen is NOT a viewport hit any more. It was kept
   * because something authoritative still names it — its promoted ElementId —
   * and the publication its pick-time proof describes has been replaced. So the
   * proof is cleared: a later promotion of that ref has to find nothing rather
   * than re-prove currentness against a mesh the user never clicked.
   *
   * Nothing is lost by that. A ref only survives here by CARRYING an
   * `elementId`, and an `elementId`-bearing ref never needs promotion — every
   * acquisition path reads it first (see
   * `ModelToolController.proofGate.test.ts` for that half, end to end).
   */
  it("clears the pick-time proof of every ref it KEEPS", async () => {
    // Both publications label the bound element with its minted id, so the
    // verdict keeps the ref OBJECT itself — nothing is rewritten, and the proof
    // would otherwise ride straight through the regen.
    const prev = stack(["el_top", "f:2"]);
    const next = stack(["f:7", "f:8", "el_top"]);
    const kept = faceRef("el_top", [2, 2, 5], "el_top");
    attachPickProof(kept, { entry: prev, kind: "face", topoKey: "el_top" });
    selectionStore.getState().set([kept]);

    regen(prev, next, clientWith(vi.fn()));
    await flush();

    expect(selected()).toEqual([kept]); // same object: the label already matched
    expect(pickProofFor(kept)).toBeUndefined();
    // …and the ref still has the handle that actually speaks for it.
    expect(selected()[0].elementId).toBe("el_top");
  });

  it("clears the proof of a ref it keeps under a REWRITTEN label", async () => {
    const prev = stack(["f:1"]);
    const next = stack(["f:9", "el_top"]);
    const picked = faceRef("f:1", [2, 2, 5], "el_top");
    attachPickProof(picked, { entry: prev, kind: "face", topoKey: "f:1" });
    selectionStore.getState().set([picked]);

    regen(prev, next, clientWith(vi.fn()));
    await flush();

    expect(pickProofFor(picked)).toBeUndefined();
    // The stored ref is a NEW object (the topoKey moved), so it never had one.
    expect(selected()[0].topoKey).toBe("el_top");
    expect(pickProofFor(selected()[0])).toBeUndefined();
  });

  it("clears the proof of a ref held pending backend CONFIRMATION", async () => {
    const prev = stack(["f:1"]);
    const next = stack(["f:9"]);
    const pending = faceRef("f:1", [2, 2, 5], "el_x");
    attachPickProof(pending, { entry: prev, kind: "face", topoKey: "f:1" });
    selectionStore.getState().set([pending]);

    regen(prev, next, clientWith(vi.fn(async () => infoNaming("f:9"))));
    expect(pickProofFor(pending)).toBeUndefined(); // cleared SYNCHRONOUSLY
    await flush();

    expect(selected()[0].topoKey).toBe("f:9");
    expect(pickProofFor(selected()[0])).toBeUndefined();
  });

  it("leaves ANOTHER body's proof alone — only the regenerated body reconciles", async () => {
    const prev = stack(["f:1"]);
    const next = stack(["f:9", "el_top"]);
    const foreign: EntityRef = { kind: "face", id: "b2#f:4", bodyId: "b2", topoKey: "f:4" };
    const foreignProof = { entry: prev, kind: "face" as const, topoKey: "f:4" };
    attachPickProof(foreign, foreignProof);
    selectionStore.getState().set([foreign, faceRef("f:1", [2, 2, 5], "el_top")]);

    regen(prev, next, clientWith(vi.fn()));
    await flush();

    expect(pickProofFor(foreign)).toBe(foreignProof);
  });
});

// ── a body that left the document ───────────────────────────────────────────

describe("dropSelectionForBody", () => {
  it("removes the removed body's body/element refs and hover, and nothing else", () => {
    const foreign: EntityRef = { kind: "face", id: "b2#f:1", bodyId: "b2", topoKey: "f:1" };
    const bodyRow: EntityRef = { kind: "body", id: "b1" };
    const sketch: EntityRef = { kind: "sketch", id: "sk1" };
    selectionStore.getState().set([faceRef("f:1"), edgeRef("e:2", "el_e"), foreign, bodyRow, sketch]);
    selectionStore.getState().setHover(faceRef("f:1"));

    dropSelectionForBody("b1");

    expect(selected()).toEqual([foreign, sketch]);
    expect(selectionStore.getState().hover).toBeNull();
  });

  it("writes nothing when the body owns no body or element ref", () => {
    const before = [{ kind: "body", id: "b2" } as EntityRef];
    selectionStore.getState().set(before);

    dropSelectionForBody("b1");

    expect(selected()).toBe(before);
  });
});
