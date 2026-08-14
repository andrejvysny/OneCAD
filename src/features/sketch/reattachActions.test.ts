/*
 * Sketch REATTACH (H9) — the wire builders, the action's guards, and the mock
 * lane's honest behaviour.
 *
 * The command shape is pinned against the CORE test
 * `edit_session.rs::reattaching_a_sketch_moves_the_timeline_record_plane` /
 * `…::reattaching_to_a_datum_stamps_the_resolved_frame_on_the_record`, which
 * accept the same JSON on the Rust side.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import {
  buildUpdateSketchAttachmentDatum,
  buildUpdateSketchAttachmentWorld,
} from "@/ipc/sketchWireMap";
import {
  REATTACH_HOST_FACE_REFUSAL,
  attachTargetLabel,
  reattachSketch,
  sketchIdOfFeature,
} from "./reattachActions";
import { planeFor } from "@/ipc/mockSketch";
import { mockClient } from "@/ipc/mockClient";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

beforeEach(() => resetStores());

describe("UpdateSketchAttachment wire builders", () => {
  it("world: internally tagged on `cmd`, bare sketch uuid, canonical basis for the kind", () => {
    const cmd = buildUpdateSketchAttachmentWorld("sk-uuid", "XZ");
    expect(cmd).toEqual({
      cmd: "updateSketchAttachment",
      sketch: "sk-uuid",
      plane: {
        origin: planeFor("XZ").origin,
        xAxis: planeFor("XZ").xAxis,
        yAxis: planeFor("XZ").yAxis,
        normal: planeFor("XZ").normal,
      },
      attachment: { kind: "world", plane: "XZ" },
    });
  });

  it("world: the basis is the SAME table AddSketch uses (one sketch frame source)", () => {
    for (const k of ["XY", "XZ", "YZ"] as const) {
      const { origin, xAxis, yAxis, normal } = planeFor(k);
      expect(buildUpdateSketchAttachmentWorld("s", k).plane).toEqual({ origin, xAxis, yAxis, normal });
    }
  });

  it("datum: carries the RESOLVED frame verbatim + the datum id", () => {
    const plane = {
      origin: [0, 0, 10] as [number, number, number],
      xAxis: [1, 0, 0] as [number, number, number],
      yAxis: [0, 1, 0] as [number, number, number],
      normal: [0, 0, 1] as [number, number, number],
    };
    expect(buildUpdateSketchAttachmentDatum("sk-uuid", plane, "datum-uuid")).toEqual({
      cmd: "updateSketchAttachment",
      sketch: "sk-uuid",
      plane,
      attachment: { kind: "datum", datum: "datum-uuid" },
    });
  });
});

describe("reattachSketch action", () => {
  it("dispatches through the client and hints the target", async () => {
    const spy = vi.spyOn(mockClient, "reattachSketch");
    const ok = await reattachSketch("sketch2", { kind: "world", plane: "XZ" });
    expect(ok).toBe(true);
    expect(spy).toHaveBeenCalledWith("sketch2", { kind: "world", plane: "XZ" });
    expect(viewportStore.getState().statusHint?.message).toBe("Sketch reattached to XZ plane");
    spy.mockRestore();
  });

  it("REFUSES a face-hosted sketch — its geometry is projected in the face's frame", async () => {
    documentStore.setState({
      sketches: {
        sketch2: {
          id: "sketch2",
          name: "Sketch 2",
          visible: true,
          dof: 0,
          status: "ok",
          geometryToken: "t",
          hostFace: { bodyId: "body1", elementId: "el_1" },
        },
      },
    });
    const spy = vi.spyOn(mockClient, "reattachSketch");
    const ok = await reattachSketch("sketch2", { kind: "world", plane: "XZ" });
    expect(ok).toBe(false);
    expect(spy).not.toHaveBeenCalled();
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toBe(REATTACH_HOST_FACE_REFUSAL);
    spy.mockRestore();
  });

  it("surfaces a rejection as a sticky error rather than a silent no-op", async () => {
    const spy = vi
      .spyOn(mockClient, "reattachSketch")
      .mockRejectedValue(new Error("datum 'Datum 1' has an unresolved frame"));
    const ok = await reattachSketch("sketch2", { kind: "datum", datumId: "d1" });
    expect(ok).toBe(false);
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("unresolved frame");
    spy.mockRestore();
  });

  /*
   * U0 red evidence. This family decides success with its own heuristic —
   * `res.revision > before || changedBodies.length > 0` (reattachActions.ts:54).
   * A regen that bumps the revision AND resolves carrying `terminal:"failed"` +
   * `errorMessage` therefore reads as a successful reattach: the classifier
   * (`src/ipc/regenOutcome.ts`) is never consulted. RED until U1 adopts it here.
   */
  it("reports a RESOLVED failure as a failure, not a revision bump", async () => {
    const spy = vi.spyOn(mockClient, "reattachSketch").mockResolvedValue({
      revision: documentStore.getState().revision + 1,
      features: [],
      changedBodies: [],
      removedBodies: [],
      terminal: "failed",
      errorMessage: "host plane no longer resolves",
    } as unknown as Awaited<ReturnType<typeof mockClient.reattachSketch>>);

    const ok = await reattachSketch("sketch2", { kind: "world", plane: "XZ" });

    expect(ok).toBe(false);
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("host plane no longer resolves");
    spy.mockRestore();
  });

  it("reports a NO-OP result instead of claiming success (the core's silent host-face drop)", async () => {
    // `UpdateSketchAttachment` skips the host-face stamp when the sketch record
    // precedes its host body, and says nothing on the wire. The observable is
    // that nothing regenerated — never report that as a successful reattach.
    const spy = vi.spyOn(mockClient, "reattachSketch").mockResolvedValue({
      revision: documentStore.getState().revision,
      changedBodies: [],
      removedBodies: [],
      features: documentStore.getState().features,
    });
    const ok = await reattachSketch("sketch2", { kind: "world", plane: "XZ" });
    expect(ok).toBe(false);
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("precede its host");
    spy.mockRestore();
  });

  it("labels a datum target by its tree name", () => {
    documentStore.getState().addDatum({
      id: "d1",
      name: "Datum 1",
      basePlaneId: "XY",
      offset: 10,
      plane: { ...planeFor("XY"), kind: "custom" },
      resolvedValid: true,
    });
    expect(attachTargetLabel({ kind: "datum", datumId: "d1" })).toBe("Datum 1");
    expect(attachTargetLabel({ kind: "world", plane: "YZ" })).toBe("YZ plane");
  });
});

describe("sketchIdOfFeature", () => {
  it("reads params.sketchId — the projection's feature id is a RECORD id", async () => {
    const spy = vi
      .spyOn(mockClient, "getOperationParams")
      .mockResolvedValue({ sketchId: "sk-uuid", plane: {} });
    expect(await sketchIdOfFeature("f1")).toBe("sk-uuid");
    spy.mockRestore();
  });

  it("fails soft (null) on an unknown record or a params object with no sketchId", async () => {
    const spy = vi.spyOn(mockClient, "getOperationParams").mockResolvedValue({});
    expect(await sketchIdOfFeature("f1")).toBeNull();
    spy.mockRejectedValue(new Error("unknown record"));
    expect(await sketchIdOfFeature("nope")).toBeNull();
    spy.mockRestore();
  });
});

describe("mock lane reattach (honest, not faked)", () => {
  it("rejects an unknown sketch and an unresolved datum", async () => {
    await expect(mockClient.reattachSketch("nope", { kind: "world", plane: "XY" })).rejects.toThrow(
      /unknown sketch/,
    );
    documentStore.getState().addDatum({
      id: "bad",
      name: "Broken",
      basePlaneId: "nothing",
      offset: 1,
      plane: { ...planeFor("XY"), kind: "custom" },
      resolvedValid: false,
    });
    await expect(
      mockClient.reattachSketch("sketch2", { kind: "datum", datumId: "bad" }),
    ).rejects.toThrow(/unresolved frame/);
  });

  it("bumps the revision and republishes the timeline", async () => {
    const before = (await mockClient.reattachSketch("sketch2", { kind: "world", plane: "XY" }))
      .revision;
    const res = await mockClient.reattachSketch("sketch2", { kind: "world", plane: "XZ" });
    expect(res.revision).toBeGreaterThan(before);
    expect(res.opLabel).toBe("Reattach Sketch");
    expect(res.features.length).toBeGreaterThan(0);
  });

  it("MOVES the geometry of an extrude standing on the reattached sketch", async () => {
    // Author a real extrude on the mock lane so there is a body to move, then
    // reattach the profile sketch to a different world plane. The whole point of
    // reattach is that downstream geometry follows.
    const { regions } = await mockClient.getSketchRegions("sketch2");
    const built = await mockClient.applyOperation({
      opType: "Extrude",
      sketchId: "sketch2",
      regionId: regions[0].regionId,
      params: { distance: 12, booleanMode: "NewBody" },
    });
    const bodyId = built.changedBodies[0]?.bodyId;
    expect(bodyId).toBeTruthy();
    const before = await mockClient.getBodyMesh(bodyId!, "coarse");

    const res = await mockClient.reattachSketch("sketch2", { kind: "world", plane: "XZ" });
    expect(res.changedBodies.map((b) => b.bodyId)).toContain(bodyId);
    const after = await mockClient.getBodyMesh(bodyId!, "coarse");
    expect(new Uint8Array(after)).not.toEqual(new Uint8Array(before));
  });
});
