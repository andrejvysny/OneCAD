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

  it("drops a stale (lower-revision) projection", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 5, title: "keep" });
    const applied = applyProjectionToStore(proj(3, { title: "STALE" }));
    expect(applied).toBe(false);
    expect(documentStore.getState().revision).toBe(5);
    expect(documentStore.getState().title).toBe("keep");
  });

  it("always applies the empty projection (close), even at revision 0", () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 5 });
    const applied = applyProjectionToStore({ status: "empty", revision: 0, title: "", dirty: false, bodies: {}, sketches: {}, features: [] });
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
