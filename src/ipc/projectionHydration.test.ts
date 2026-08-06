/*
 * projectionHydration — the projection-updated → documentStore bridge (F-WP8
 * flag 2), with revision reconciliation.
 */
import { afterEach, describe, expect, it } from "vitest";
import { applyProjectionToStore, projectionToStore } from "./projectionHydration";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import type { DocumentProjectionWire } from "./types";

afterEach(() => {
  documentStore.getState().applySnapshot(seedMockDocument());
});

const proj = (revision: number, over: Partial<DocumentProjectionWire> = {}): DocumentProjectionWire => ({
  status: "ready",
  revision,
  title: "P",
  dirty: true,
  bodies: {},
  sketches: {},
  datums: {},
  features: [],
  ...over,
});

describe("applyProjectionToStore", () => {
  it("applies a newer-or-equal projection and maps sketches/features 1:1", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 2 });
    const applied = applyProjectionToStore(
      proj(5, {
        title: "Opened",
        bodies: { b1: { id: "b1", name: "B1", visible: true } },
        sketches: {
          s1: {
            id: "s1",
            name: "S1",
            visible: true,
            dof: 3,
            status: "under",
            geometryToken: "geometry-s1",
          },
        },
        features: [{ id: "f1", kind: "extrude", label: "Extrude", valueText: "10.0 mm", status: "ok" }],
      }),
    );
    expect(applied).toBe(true);
    const s = documentStore.getState();
    expect(s.revision).toBe(5);
    expect(s.title).toBe("Opened");
    expect(s.bodies.b1.name).toBe("B1");
    expect(s.sketches.s1.status).toBe("under");
    expect(s.sketches.s1.geometryToken).toBe("geometry-s1");
    expect(s.features[0].valueText).toBe("10.0 mm");
  });

  it("carries a sketch's hostFace through, and leaves world/datum sketches without one", () => {
    // SKETCH-ON-FACE W3: the double-click-a-face re-entry matches a promoted pick
    // against this id, so dropping it here would silently turn every re-entry
    // into "create a second sketch on the same face".
    applyProjectionToStore(
      proj(9, {
        sketches: {
          onFace: {
            id: "onFace",
            name: "On Face",
            visible: true,
            dof: 0,
            status: "ok",
            geometryToken: "g1",
            hostFace: { bodyId: "b1", elementId: "el_7" },
          },
          world: { id: "world", name: "World", visible: true, dof: 3, status: "under", geometryToken: "g2" },
        },
      }),
    );
    const s = documentStore.getState();
    expect(s.sketches.onFace.hostFace).toEqual({ bodyId: "b1", elementId: "el_7" });
    expect(s.sketches.world.hostFace).toBeUndefined();
  });

  it("carries the authored opType through so a Chamfer re-edit does not route as Fillet", () => {
    applyProjectionToStore(
      proj(7, {
        features: [
          { id: "f-ch", kind: "fillet", opType: "Chamfer", label: "Chamfer", valueText: "1.0 mm", status: "ok" },
          { id: "f-fi", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
        ],
      }),
    );
    const features = documentStore.getState().features;
    expect(features.map((f) => f.opType)).toEqual(["Chamfer", "Fillet"]);
    expect(features.map((f) => f.kind)).toEqual(["fillet", "fillet"]); // kind still folded
  });

  it("a freshly opened DOCUMENT always applies — revisions never compare across documentIds", () => {
    // The reopen bug class: a new runtime restarts revisions at 1, so with the
    // store still holding the old document's high revision, a cross-document
    // revision compare dropped every projection of the opened document forever.
    documentStore.getState().applySnapshot({
      ...seedMockDocument(),
      documentId: "doc-old",
      revision: 42,
    });
    const applied = applyProjectionToStore(
      proj(1, {
        documentId: "doc-new",
        title: "Reopened",
        bodies: { b1: { id: "b1", name: "B1", visible: true } },
      }),
    );
    expect(applied).toBe(true);
    const s = documentStore.getState();
    expect(s.documentId).toBe("doc-new");
    expect(s.revision).toBe(1);
    expect(s.title).toBe("Reopened");
    expect(s.bodies.b1).toBeDefined();
  });

  it("still drops a stale projection WITHIN the same documentId", () => {
    documentStore.getState().applySnapshot({
      ...seedMockDocument(),
      documentId: "doc-1",
      revision: 7,
    });
    expect(applyProjectionToStore(proj(3, { documentId: "doc-1" }))).toBe(false);
    expect(documentStore.getState().revision).toBe(7);
  });

  it("treats a missing documentId (mock lane) as same-document semantics", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 7 });
    expect(applyProjectionToStore(proj(3))).toBe(false); // stale within the doc
    expect(applyProjectionToStore(proj(8))).toBe(true); // newer applies
  });

  it("drops a stale (lower-revision) projection", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 5, title: "keep" });
    const applied = applyProjectionToStore(proj(3, { title: "STALE" }));
    expect(applied).toBe(false);
    expect(documentStore.getState().revision).toBe(5);
    expect(documentStore.getState().title).toBe("keep");
  });

  it("always applies the empty projection (close), even at revision 0", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 5 });
    const applied = applyProjectionToStore({ status: "empty", revision: 0, title: "", dirty: false, bodies: {}, sketches: {}, datums: {}, features: [] });
    expect(applied).toBe(true);
    expect(documentStore.getState().status).toBe("empty");
    expect(documentStore.getState().bodies).toEqual({});
  });

  it("coerces an unknown sketch status token to 'under'", () => {
    const store = projectionToStore(
      proj(1, {
        sketches: {
          x: {
            id: "x",
            name: "X",
            visible: true,
            dof: 1,
            status: "weird",
            geometryToken: "geometry-x",
          },
        },
      }),
    );
    expect(store.sketches.x.status).toBe("under");
  });

  it("carries a feature's statusMessage through to the store (error surfacing)", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 2 });
    applyProjectionToStore(
      proj(5, {
        features: [
          {
            id: "f1",
            kind: "revolve",
            label: "Revolve",
            valueText: "360.0°",
            status: "error",
            statusMessage: "revolve axis lineId not found",
          },
          { id: "f2", kind: "extrude", label: "Extrude", valueText: "10.0 mm", status: "ok" },
        ],
      }),
    );
    const feats = documentStore.getState().features;
    expect(feats[0].status).toBe("error");
    expect(feats[0].statusMessage).toBe("revolve axis lineId not found");
    // A non-error feature carries no message.
    expect(feats[1].statusMessage).toBeUndefined();
  });
});

// ── Datum planes (DATUM W1) ──────────────────────────────────────────────────

describe("datum hydration", () => {
  const wireDatum = {
    id: "d1",
    name: "Datum 1",
    kind: "OffsetFromPlane",
    basePlaneId: "XZ",
    offset: 10,
    // XZ's normal is +X (the non-standard Sketch.h basis), so the backend
    // resolved this to world +X — NOT to −Y as the plane's NAME suggests.
    plane: {
      origin: [10, 0, 0] as [number, number, number],
      xAxis: [0, 1, 0] as [number, number, number],
      yAxis: [0, 0, 1] as [number, number, number],
      normal: [1, 0, 0] as [number, number, number],
    },
    resolvedValid: true,
  };

  it("carries the backend-resolved frame VERBATIM and tags it custom", () => {
    applyProjectionToStore(proj(11, { datums: { d1: wireDatum } }));
    const d = documentStore.getState().datums.d1;
    expect(d).toEqual({
      id: "d1",
      name: "Datum 1",
      basePlaneId: "XZ",
      offset: 10,
      plane: { kind: "custom", origin: [10, 0, 0], xAxis: [0, 1, 0], yAxis: [0, 0, 1], normal: [1, 0, 0] },
      resolvedValid: true,
    });
  });

  it("keeps an unresolved datum in the tree, flagged", () => {
    applyProjectionToStore(
      proj(12, { datums: { d2: { ...wireDatum, id: "d2", basePlaneId: "NOPE", resolvedValid: false } } }),
    );
    expect(documentStore.getState().datums.d2.resolvedValid).toBe(false);
  });

  it("a projection with no datums clears them (snapshot semantics, not a merge)", () => {
    applyProjectionToStore(proj(13, { datums: { d1: wireDatum } }));
    expect(Object.keys(documentStore.getState().datums)).toHaveLength(1);
    applyProjectionToStore(proj(14, { datums: {} }));
    expect(documentStore.getState().datums).toEqual({});
  });

  it("survives a backend that omits datums entirely, instead of taking down all hydration", () => {
    const legacy = proj(15, { title: "Legacy" });
    delete (legacy as { datums?: unknown }).datums;
    expect(applyProjectionToStore(legacy)).toBe(true);
    expect(documentStore.getState().title).toBe("Legacy");
    expect(documentStore.getState().datums).toEqual({});
  });
});

/*
 * H7b — the ROLLBACK CURSOR is hydrated authoritatively. It drives the grayed
 * rows / marker / banner now, not just the one-shot open hint, so a projection
 * that moves it must move the store with it.
 */
describe("projectionToStore — the rollback cursor", () => {
  const three = [
    { id: "a", kind: "sketch" as const, label: "Sketch 1", valueText: "", status: "ok" as const },
    { id: "b", kind: "extrude" as const, label: "Extrude", valueText: "", status: "ok" as const },
    { id: "c", kind: "fillet" as const, label: "Fillet", valueText: "", status: "ok" as const },
  ];

  it("carries appliedOps verbatim from the projection", () => {
    expect(projectionToStore(proj(3, { features: three, appliedOps: 1, totalOps: 3 })).appliedOps).toBe(1);
    applyProjectionToStore(proj(20, { features: three, appliedOps: 2, totalOps: 3 }));
    expect(documentStore.getState().appliedOps).toBe(2);
  });

  it("a payload WITHOUT the field reads as fully applied (older backend / mock lane)", () => {
    expect(projectionToStore(proj(5, { features: three })).appliedOps).toBe(3);
  });

  it("clamps a cursor past the last row — the marker must land on a real index", () => {
    expect(projectionToStore(proj(6, { features: three, appliedOps: 9, totalOps: 9 })).appliedOps).toBe(3);
  });

  it("totalOps is `features.length` by construction, so the store keeps no copy", () => {
    // `document_runtime.rs` builds `features` from ALL timeline records and sets
    // `total_ops = timeline.len()`; a second stored copy could only disagree.
    const wire = proj(7, { features: three, appliedOps: 1, totalOps: 3 });
    expect(projectionToStore(wire).features).toHaveLength(wire.totalOps!);
  });
});

// ── geometrySource (persisted-cache lane) ─────────────────────────────────────

describe("projectionToStore — geometrySource", () => {
  it("carries the backend's value through verbatim", () => {
    expect(projectionToStore(proj(1, { geometrySource: "cached" })).geometrySource).toBe("cached");
    expect(projectionToStore(proj(1, { geometrySource: "live" })).geometrySource).toBe("live");
    expect(projectionToStore(proj(1, { geometrySource: "none" })).geometrySource).toBe("none");
  });

  it("an ABSENT field reads as 'none' — no chip, never an assumed 'live'", () => {
    // The mock lane and any backend older than the field omit it. Defaulting to
    // "live" would label cached geometry as authoritative on exactly the builds
    // that cannot tell us otherwise.
    expect(projectionToStore(proj(1)).geometrySource).toBe("none");
  });

  it("an UNRECOGNISED token reads as 'none' rather than being trusted", () => {
    const wire = proj(1);
    // Deliberately off-contract: what a NEWER backend could send. The cast is the
    // test — TypeScript cannot police a value that crossed the IPC boundary.
    (wire as { geometrySource?: string }).geometrySource = "stale-from-the-future";
    expect(projectionToStore(wire).geometrySource).toBe("none");
  });

  it("reaches the store through the full apply path", () => {
    applyProjectionToStore(proj(42, { geometrySource: "cached" }));
    expect(documentStore.getState().geometrySource).toBe("cached");
  });
});
