/*
 * previewOps — the SINGLE preview↔commit op mapper (SCHEMA §7.6: "Preview callers
 * MUST NOT maintain a second ad-hoc mapper").
 *
 * The fixtures below are transcribed from the COMMIT call sites in
 * `ModelToolController` — the `OperationOp` each tool hands to `applyOperation` /
 * `endPreview(commit)`. A builder that drifts from its call site makes the preview
 * a different op than the one that materializes, which is the exact class of defect
 * this module exists to prevent.
 *
 * Call sites (ModelToolController.ts):
 *   confirmRevolve  — the Revolve op inside the per-region commit loop
 *   commitFillet    — `{opType: kind, featureId, inputs, params:{mode,radius,edgeIds,chainTangentEdges}}`
 *   commitShell     — `{opType:"Shell", featureId, inputs, params:{thickness,openFaces,targetBodyId}}`
 *   commitBoolean   — `{opType:"Boolean", inputs:[target,tool], params:{operation,targetBodyId,toolBodyId}}`
 * Extrude's fixture is the lane's own committed op (localSolver.endPreview), whose
 * shape is additionally pinned by the Rust `preview_and_commit_share_*` test.
 */
import { describe, it, expect, vi } from "vitest";
import { OP_BUILDERS, buildPreviewOp, supportsPreview, type PreviewSessionState } from "./previewOps";
import { createLocalSolverLane } from "./localSolver";
import type {
  ApplyOperationResult,
  OffsetFaceParams,
  OperationOp,
  OpType,
  PreviewParams,
  PreviewResult,
  SemanticRef,
} from "./types";

const OP_ID = "op-fixed";

function session(over: Partial<PreviewSessionState> & { opType: OpType; latestParams: PreviewParams }): PreviewSessionState {
  return { opId: OP_ID, ...over };
}

// ── the six builders vs their commit call-site construction ──────────────────

describe("previewOps OP_BUILDERS mirror their commit call sites", () => {
  it("Extrude matches the lane's committed op", () => {
    const op = buildPreviewOp(
      session({
        opType: "Extrude",
        sketchId: "sk",
        regionId: "r",
        latestParams: { distance: 12, booleanMode: "Cut", targetBodyId: "body7" },
      }),
    );
    expect(op).toEqual({
      opType: "Extrude",
      opId: OP_ID,
      sketchId: "sk",
      regionId: "r",
      featureId: undefined,
      inputs: [{ primary: { bodyId: "", kind: "face" }, anchor: {} }],
      params: {
        distance: 12,
        extrudeMode: "Blind",
        booleanMode: "Cut",
        draftAngleDeg: 0,
        twoDirections: false,
        extrudeMode2: "Blind",
        distance2: 0,
        targetBodyId: "body7",
      },
    } satisfies OperationOp);
  });

  it("Revolve matches confirmRevolve's op", () => {
    // confirmRevolve:
    //   const axis = { kind: "sketchLine", sketchId, lineId: this.revolveAxisLineId }
    //   { opType: "Revolve", sketchId, regionId: regionIds[k],
    //     inputs: [{ primary: { bodyId: "", kind: "face" }, anchor: {} }],
    //     params: { angleDeg: angle, axis, booleanMode, ...(targetBodyId ? { targetBodyId } : {}) } }
    const angle = 90;
    const sketchId = "sk";
    const axis = { kind: "sketchLine" as const, sketchId, lineId: "l1" };
    const booleanMode = "Add" as const;
    const targetBodyId = "body3";
    const callSite: OperationOp = {
      opType: "Revolve",
      sketchId,
      regionId: "r2",
      inputs: [{ primary: { bodyId: "", kind: "face" }, anchor: {} }],
      params: { angleDeg: angle, axis, booleanMode, ...(targetBodyId ? { targetBodyId } : {}) },
    };

    const op = buildPreviewOp(
      session({
        opType: "Revolve",
        sketchId,
        regionId: "r2",
        latestParams: { angleDeg: angle, axis, booleanMode, targetBodyId },
      }),
    );
    expect(op).toEqual({ ...callSite, opId: OP_ID, featureId: undefined });
  });

  it("Revolve with no axis and no boolean target matches the call site's NewBody shape", () => {
    const callSite: OperationOp = {
      opType: "Revolve",
      sketchId: "sk",
      regionId: "r",
      inputs: [{ primary: { bodyId: "", kind: "face" }, anchor: {} }],
      params: { angleDeg: 360, axis: undefined, booleanMode: "NewBody" },
    };
    const op = buildPreviewOp(
      session({ opType: "Revolve", sketchId: "sk", regionId: "r", latestParams: { angleDeg: 360 } }),
    );
    expect(op).toEqual({ ...callSite, opId: OP_ID, featureId: undefined });
    // `targetBodyId` is absent (not present-as-undefined) exactly as at the call site.
    if (op.opType !== "Revolve") throw new Error("expected Revolve");
    expect("targetBodyId" in op.params).toBe(false);
  });

  it.each(["Fillet", "Chamfer"] as const)("%s matches commitFillet's op", (kind) => {
    // commitFillet:
    //   { opType: kind, featureId: editFeatureId,
    //     inputs: edges.map((e) => this.semanticRefFor(e)),
    //     params: { mode: kind, radius, edgeIds: edges.map((e) => e.topoKey ?? e.id),
    //               chainTangentEdges: true } }
    const edges = [
      { id: "e1", topoKey: "e:1", elementId: "el_1", bodyId: "body1", anchor: { worldPoint: [1, 2, 3] as [number, number, number] } },
      { id: "e2", topoKey: "e:2", elementId: "el_2", bodyId: "body1", anchor: undefined },
    ];
    const semanticRefFor = (e: (typeof edges)[number]): SemanticRef => ({
      primary: { bodyId: e.bodyId, elementId: e.elementId, kind: "edge" },
      anchor: e.anchor ? { worldPoint: e.anchor.worldPoint, surfaceUv: undefined } : undefined,
    });
    const radius = 2.5;
    const callSite: OperationOp = {
      opType: kind,
      featureId: undefined,
      inputs: edges.map(semanticRefFor),
      params: { mode: kind, radius, edgeIds: edges.map((e) => e.topoKey ?? e.id), chainTangentEdges: true },
    };

    const op = buildPreviewOp(
      session({
        opType: kind,
        inputs: edges.map(semanticRefFor),
        latestParams: { mode: kind, radius, edgeIds: edges.map((e) => e.topoKey ?? e.id) },
      }),
    );
    expect(op).toEqual({ ...callSite, opId: OP_ID });
  });

  // ── two-distance chamfer (SCHEMA §7.3, 2026-08-03 — WP-C T2a) ──────────────

  const edgeSession = (opType: "Fillet" | "Chamfer", latestParams: Record<string, unknown>) =>
    session({
      opType,
      inputs: [{ primary: { bodyId: "body1", elementId: "el_1", kind: "edge" } }],
      latestParams: { mode: opType, radius: 1, edgeIds: ["e:1"], ...latestParams },
    });

  it("a Chamfer draft carries `distance2` through; an equal-leg one emits no key", () => {
    const asym = buildPreviewOp(edgeSession("Chamfer", { distance2: 2.5 }));
    if (asym.opType !== "Chamfer") throw new Error("expected Chamfer");
    expect(asym.params.distance2).toBe(2.5);

    const equal = buildPreviewOp(edgeSession("Chamfer", {}));
    if (equal.opType !== "Chamfer") throw new Error("expected Chamfer");
    expect("distance2" in equal.params).toBe(false);
  });

  it("REFUSES a distance2 on a Fillet, or a non-positive one on a Chamfer", () => {
    // Structural guards mirroring core: an invalid draft must fail the preview
    // loudly (which blocks commit) rather than degrade to an equal-leg cut.
    expect(() => buildPreviewOp(edgeSession("Fillet", { distance2: 2.5 }))).toThrow(
      /Chamfer-only/,
    );
    for (const bad of [0, -1, Number.NaN]) {
      expect(() => buildPreviewOp(edgeSession("Chamfer", { distance2: bad }))).toThrow(
        /greater than zero/,
      );
    }
  });

  it("Shell matches commitShell's op", () => {
    // commitShell:
    //   const bodyId = faces[0]?.bodyId;
    //   { opType: "Shell", featureId, inputs: faces.map(semanticRefFor),
    //     params: { thickness, openFaces: faces.map((f) => f.elementId ?? f.topoKey ?? f.id),
    //               targetBodyId: bodyId } }
    const faces = [
      { id: "f1", topoKey: "f:1", elementId: "el_f1", bodyId: "body2" },
      { id: "f2", topoKey: "f:2", elementId: undefined, bodyId: "body2" },
    ];
    const semanticRefFor = (f: (typeof faces)[number]): SemanticRef => ({
      primary: { bodyId: f.bodyId, elementId: f.elementId, kind: "face" },
      anchor: undefined,
    });
    const thickness = 1.5;
    const callSite: OperationOp = {
      opType: "Shell",
      featureId: undefined,
      inputs: faces.map(semanticRefFor),
      params: {
        thickness,
        openFaces: faces.map((f) => f.elementId ?? f.topoKey ?? f.id),
        targetBodyId: faces[0].bodyId,
      },
    };

    const op = buildPreviewOp(
      session({
        opType: "Shell",
        inputs: faces.map(semanticRefFor),
        latestParams: {
          thickness,
          openFaces: faces.map((f) => f.elementId ?? f.topoKey ?? f.id),
          targetBodyId: faces[0].bodyId,
        },
      }),
    );
    expect(op).toEqual({ ...callSite, opId: OP_ID });
  });

  it("Hole matches commitHole's op", () => {
    // commitHole:
    //   { opType: "Hole", featureId, inputs: holeInputs(),
    //     params: holeParamsOf(this.hole) }
    // — and `holeParamsOf` emits the INACTIVE conditional blocks as explicit
    // `null`, which is exactly what the builder must reproduce.
    const face: SemanticRef = {
      primary: { bodyId: "body2", elementId: "el_top", kind: "face" },
      anchor: { worldPoint: [10, 10, 25] },
    };
    const inputs: SemanticRef[] = [{ primary: { bodyId: "body2", kind: "body" } }, face];
    const callSite: OperationOp = {
      opType: "Hole",
      featureId: undefined,
      inputs,
      params: {
        targetBodyId: "body2",
        face,
        point: [10, 10, 25],
        holeType: "counterbore",
        diameter: 6.6,
        depth: 20,
        cbDiameter: 11,
        cbDepth: 6.8,
        csDiameter: null,
        csAngleDeg: null,
      },
    };
    const op = buildPreviewOp(
      session({
        opType: "Hole",
        inputs,
        latestParams: {
          targetBodyId: "body2",
          face,
          point: [10, 10, 25],
          holeType: "counterbore",
          diameter: 6.6,
          depth: 20,
          cbDiameter: 11,
          cbDepth: 6.8,
          csDiameter: null,
          csAngleDeg: null,
        },
      }),
    );
    expect(op).toEqual({ ...callSite, opId: OP_ID });
  });

  it("Hole through-all keeps a null depth (a real end condition, not a gap)", () => {
    const face: SemanticRef = {
      primary: { bodyId: "b", elementId: "el", kind: "face" },
      anchor: undefined,
    };
    const op = buildPreviewOp(
      session({
        opType: "Hole",
        latestParams: {
          targetBodyId: "b",
          face,
          point: [0, 0, 0],
          holeType: "simple",
          diameter: 5.5,
          depth: null,
        },
      }),
    );
    expect(op.params).toMatchObject({ depth: null, cbDiameter: null, csAngleDeg: null });
  });

  it("Hole REFUSES the invariants the backend would refuse", () => {
    const face: SemanticRef = { primary: { bodyId: "b", elementId: "el", kind: "face" } };
    const base = { targetBodyId: "b", face, point: [0, 0, 0] as [number, number, number], diameter: 6, depth: null };
    const build = (p: Record<string, unknown>) => () =>
      buildPreviewOp(session({ opType: "Hole", latestParams: p as PreviewParams }));
    // An unknown profile is a caller bug, not a value to marshal — the union is
    // widened here on purpose so the refusal can be exercised at all.
    expect(build({ ...base, holeType: "spotface" })).toThrow(/holeType/);
    expect(build({ ...base, holeType: "simple", diameter: 0 })).toThrow(/diameter/);
    expect(build({ ...base, holeType: "counterbore" })).toThrow(/cbDiameter/);
    expect(build({ ...base, holeType: "counterbore", cbDiameter: 6, cbDepth: 2 })).toThrow(
      /cbDiameter must exceed/,
    );
    expect(build({ ...base, holeType: "countersink", csDiameter: 12, csAngleDeg: 95 })).toThrow(
      /csAngleDeg must be one of/,
    );
    expect(
      build({ ...base, holeType: "countersink", csDiameter: 5, csAngleDeg: 90 }),
    ).toThrow(/csDiameter must exceed/);
    // A missing seat is a caller bug, never a partial op on the wire.
    expect(build({ ...base, face: undefined, holeType: "simple" })).toThrow(/host face ref/);
  });

  it("Boolean matches commitBoolean's op", () => {
    // commitBoolean:
    //   { opType: "Boolean",
    //     inputs: [{ primary: { bodyId: targetBodyId, kind: "body" } },
    //              { primary: { bodyId: toolBodyId, kind: "body" } }],
    //     params: { operation, targetBodyId, toolBodyId } }
    const targetBodyId = "bodyA";
    const toolBodyId = "bodyB";
    const operation = "Cut" as const;
    const callSite: OperationOp = {
      opType: "Boolean",
      inputs: [
        { primary: { bodyId: targetBodyId, kind: "body" } },
        { primary: { bodyId: toolBodyId, kind: "body" } },
      ],
      params: { operation, targetBodyId, toolBodyId },
    };

    const op = buildPreviewOp(
      session({ opType: "Boolean", latestParams: { operation, targetBodyId, toolBodyId } }),
    );
    expect(op).toEqual({ ...callSite, opId: OP_ID, featureId: undefined });
  });

  it("carries editFeatureId through as the op's featureId for every builder", () => {
    const drafts: PreviewSessionState[] = [
      session({ opType: "Extrude", sketchId: "sk", regionId: "r", editFeatureId: "F", latestParams: { distance: 1 } }),
      session({ opType: "Revolve", sketchId: "sk", regionId: "r", editFeatureId: "F", latestParams: { angleDeg: 90 } }),
      session({
        opType: "Fillet",
        editFeatureId: "F",
        inputs: [{ primary: { bodyId: "b", elementId: "el", kind: "edge" } }],
        latestParams: { radius: 1, edgeIds: ["e:1"] },
      }),
      session({
        opType: "Shell",
        editFeatureId: "F",
        latestParams: { thickness: 1, openFaces: ["el_f"], targetBodyId: "b" },
      }),
      session({
        opType: "Boolean",
        editFeatureId: "F",
        latestParams: { operation: "Union", targetBodyId: "a", toolBodyId: "b" },
      }),
      session({
        opType: "Hole",
        editFeatureId: "F",
        latestParams: {
          targetBodyId: "b",
          face: { primary: { bodyId: "b", elementId: "el", kind: "face" } },
          point: [0, 0, 0],
          holeType: "simple",
          diameter: 6,
          depth: null,
        },
      }),
    ];
    for (const d of drafts) expect(buildPreviewOp(d).featureId).toBe("F");
  });
});

// ── the DUAL-FIELD trap (fixture case #1) ────────────────────────────────────

describe("previewOps edge-op dual-field lockstep", () => {
  it("refuses bare edgeIds with no typed inputs (filletParams would drop `edges`)", () => {
    // `filletParams` emits the typed `params.edges` ONLY when
    // edgeInputs.length === edgeIds.length && every one carries a bodyId. Without
    // it the worker fails "Fillet requires body input" — so refuse here, loudly.
    expect(() =>
      buildPreviewOp(session({ opType: "Fillet", latestParams: { radius: 1, edgeIds: ["e:1"] } })),
    ).toThrow(/one edge input per edgeId/);
  });

  it("refuses a count mismatch between inputs and edgeIds", () => {
    expect(() =>
      buildPreviewOp(
        session({
          opType: "Chamfer",
          inputs: [{ primary: { bodyId: "b", elementId: "el", kind: "edge" } }],
          latestParams: { radius: 1, edgeIds: ["e:1", "e:2"] },
        }),
      ),
    ).toThrow(/one edge input per edgeId/);
  });

  it("refuses an edge input with no bodyId", () => {
    expect(() =>
      buildPreviewOp(
        session({
          opType: "Fillet",
          inputs: [{ primary: { bodyId: "", elementId: "el", kind: "edge" } }],
          latestParams: { radius: 1, edgeIds: ["e:1"] },
        }),
      ),
    ).toThrow(/edge bodyId/);
  });

  it("keeps edgeIds and inputs in the SAME order the caller supplied", () => {
    const inputs: SemanticRef[] = [
      { primary: { bodyId: "b", elementId: "el_a", kind: "edge" } },
      { primary: { bodyId: "b", elementId: "el_b", kind: "edge" } },
      { primary: { bodyId: "b", elementId: "el_c", kind: "edge" } },
    ];
    const edgeIds = ["e:a", "e:b", "e:c"];
    const op = buildPreviewOp(session({ opType: "Fillet", inputs, latestParams: { radius: 1, edgeIds } }));
    if (op.opType !== "Fillet") throw new Error("expected Fillet");
    expect(op.params.edgeIds).toEqual(edgeIds);
    expect(op.inputs?.map((r) => r.primary.elementId)).toEqual(["el_a", "el_b", "el_c"]);
  });
});

// ── builder validation ───────────────────────────────────────────────────────

describe("previewOps builder validation", () => {
  it.each([
    ["non-finite revolve angle", { opType: "Revolve" as const, sketchId: "sk", regionId: "r", latestParams: { angleDeg: Number.NaN } }, /must be finite/],
    ["degenerate revolve angle", { opType: "Revolve" as const, sketchId: "sk", regionId: "r", latestParams: { angleDeg: 0.1 } }, /at least/],
    ["boolean revolve with no target", { opType: "Revolve" as const, sketchId: "sk", regionId: "r", latestParams: { angleDeg: 90, booleanMode: "Cut" as const } }, /requires targetBodyId/],
    ["zero fillet radius", { opType: "Fillet" as const, inputs: [{ primary: { bodyId: "b", kind: "edge" as const } }], latestParams: { radius: 0, edgeIds: ["e:1"] } }, /greater than zero/],
    ["empty fillet edges", { opType: "Fillet" as const, inputs: [], latestParams: { radius: 1, edgeIds: [] } }, /at least one edge/],
    ["zero shell thickness", { opType: "Shell" as const, latestParams: { thickness: 0, openFaces: ["f"], targetBodyId: "b" } }, /greater than zero/],
    ["shell with no target", { opType: "Shell" as const, latestParams: { thickness: 1, openFaces: ["f"] } }, /requires targetBodyId/],
    ["shell with no open faces", { opType: "Shell" as const, latestParams: { thickness: 1, openFaces: [], targetBodyId: "b" } }, /at least one open face/],
    ["unknown boolean operation", { opType: "Boolean" as const, latestParams: { operation: "Nope" as never, targetBodyId: "a", toolBodyId: "b" } }, /Unsupported Boolean operation/],
    ["boolean target === tool", { opType: "Boolean" as const, latestParams: { operation: "Union" as const, targetBodyId: "a", toolBodyId: "a" } }, /different bodies/],
  ])("throws on %s", (_name, draft, pattern) => {
    expect(() => buildPreviewOp(session(draft))).toThrow(pattern);
  });

  it("refuses an unknown opType instead of silently building nothing", () => {
    expect(() => buildPreviewOp(session({ opType: "LinearPattern", latestParams: {} }))).toThrow(
      /does not support LinearPattern/,
    );
  });
});

// ── OffsetFace (SCHEMA §7.3 `op.offsetFace`) ─────────────────────────────────
//
// Every guard here restates one of core `OffsetFaceParams::validate`'s, so a draft
// the Rust session would reject dies as a STRUCTURAL preview failure (which blocks
// the ✓ and names the reason) instead of reaching the kernel.

const OFFSET_FACE = (bodyId = "body1", elementId = "el-f2"): SemanticRef => ({
  primary: { bodyId, elementId, kind: "face" },
  anchor: { worldPoint: [0, 0, 10] },
});

describe("previewOps offsetFaceOp mirrors commitOffsetFace", () => {
  it("builds the canonical multi-face Offset", () => {
    const faces = [OFFSET_FACE("body1", "el-a"), OFFSET_FACE("body1", "el-b")];
    const op = buildPreviewOp(
      session({
        opType: "OffsetFace",
        inputs: faces,
        latestParams: {
          faces,
          distance: 2.5,
          distanceType: "Offset",
          chainTangentFaces: true,
          targetBodyId: "body1",
        },
      }),
    );
    expect(op).toEqual({
      opType: "OffsetFace",
      opId: OP_ID,
      featureId: undefined,
      inputs: faces,
      params: {
        faces,
        distance: 2.5,
        distanceType: "Offset",
        chainTangentFaces: true,
        targetBodyId: "body1",
      },
    });
  });

  it("carries the Total opposite face and nothing else does", () => {
    const face = OFFSET_FACE("body1", "el-top");
    const opposite = OFFSET_FACE("body1", "el-bottom");
    const op = buildPreviewOp(
      session({
        opType: "OffsetFace",
        latestParams: {
          faces: [face],
          distance: 12,
          distanceType: "Total",
          chainTangentFaces: false,
          oppositeFace: opposite,
          targetBodyId: "body1",
        },
      }),
    );
    expect(op.params).toMatchObject({ distanceType: "Total", oppositeFace: opposite });
  });

  it("defaults chainTangentFaces to true (the wire default)", () => {
    const op = buildPreviewOp(
      session({
        opType: "OffsetFace",
        latestParams: { faces: [OFFSET_FACE()], distance: 1, targetBodyId: "body1" },
      }),
    );
    expect(op.params).toMatchObject({ distanceType: "Offset", chainTangentFaces: true });
  });

  it.each([
    [
      "a non-finite distance",
      { faces: [OFFSET_FACE()], distance: Number.NaN, targetBodyId: "body1" },
      /must be finite/,
    ],
    [
      "an unknown distanceType",
      { faces: [OFFSET_FACE()], distance: 1, distanceType: "Sideways" as never, targetBodyId: "body1" },
      /Unsupported OffsetFace distanceType/,
    ],
    ["an empty operative set", { faces: [], distance: 1, targetBodyId: "body1" }, /at least one operative face/],
    [
      "a non-face ref",
      {
        faces: [{ primary: { bodyId: "body1", elementId: "el", kind: "edge" as const } }],
        distance: 1,
        targetBodyId: "body1",
      },
      /must each carry a face bodyId/,
    ],
    [
      "a face with no body",
      {
        faces: [{ primary: { bodyId: "", elementId: "el", kind: "face" as const } }],
        distance: 1,
        targetBodyId: "body1",
      },
      /must each carry a face bodyId/,
    ],
    ["no targetBodyId", { faces: [OFFSET_FACE()], distance: 1 }, /requires targetBodyId/],
    [
      "a CROSS-BODY closure",
      { faces: [OFFSET_FACE("body1"), OFFSET_FACE("body2")], distance: 1, targetBodyId: "body1" },
      /every face must belong to targetBodyId/,
    ],
    [
      "an absolute type over several faces",
      {
        faces: [OFFSET_FACE("body1", "a"), OFFSET_FACE("body1", "b")],
        distance: 6,
        distanceType: "Radius" as const,
        targetBodyId: "body1",
      },
      /operates on exactly one face/,
    ],
    [
      "Total with no opposite face",
      {
        faces: [OFFSET_FACE()],
        distance: 12,
        distanceType: "Total" as const,
        chainTangentFaces: false,
        targetBodyId: "body1",
      },
      /requires an opposite face/,
    ],
    [
      "Total with the tangent chain still on",
      {
        faces: [OFFSET_FACE()],
        distance: 12,
        distanceType: "Total" as const,
        chainTangentFaces: true,
        oppositeFace: OFFSET_FACE("body1", "el-bottom"),
        targetBodyId: "body1",
      },
      /requires chainTangentFaces = false/,
    ],
    [
      "an opposite face on a non-Total offset (the conditional, checked BOTH ways)",
      {
        faces: [OFFSET_FACE()],
        distance: 2,
        distanceType: "Offset" as const,
        oppositeFace: OFFSET_FACE("body1", "el-bottom"),
        targetBodyId: "body1",
      },
      /oppositeFace is Total-only/,
    ],
    [
      "a non-positive Radius",
      {
        faces: [OFFSET_FACE()],
        distance: 0,
        distanceType: "Radius" as const,
        targetBodyId: "body1",
      },
      /needs a positive distance/,
    ],
    [
      "a non-positive Diameter",
      {
        faces: [OFFSET_FACE()],
        distance: -8,
        distanceType: "Diameter" as const,
        targetBodyId: "body1",
      },
      /needs a positive distance/,
    ],
  ])("throws on %s", (_name, latestParams, pattern) => {
    expect(() => buildPreviewOp(session({ opType: "OffsetFace", latestParams }))).toThrow(pattern);
  });

  it("ACCEPTS a negative Offset — shrinking is a legal direction", () => {
    const op = buildPreviewOp(
      session({
        opType: "OffsetFace",
        latestParams: { faces: [OFFSET_FACE()], distance: -2.5, targetBodyId: "body1" },
      }),
    );
    expect(op.opType).toBe("OffsetFace");
    expect((op.params as OffsetFaceParams).distance).toBe(-2.5);
  });
});

describe("previewOps supportsPreview", () => {
  it("accepts the kernel-previewable opTypes", () => {
    for (const t of [
      "Extrude",
      "Revolve",
      "Fillet",
      "Chamfer",
      "Shell",
      "Boolean",
      "Hole",
      "OffsetFace",
    ] as const) {
      expect(supportsPreview(t)).toBe(true);
      expect(OP_BUILDERS[t]).toBeTypeOf("function");
    }
  });

  it("rejects the chip-driven ghost ops (no kernel candidate)", () => {
    for (const t of ["LinearPattern", "CircularPattern", "MirrorBody"] as const) {
      expect(supportsPreview(t)).toBe(false);
    }
  });
});

// ── the lane wired onto the generalized builders ─────────────────────────────

type BackendPreview = {
  bodies: { bodyId: string; mesh: ArrayBuffer }[];
  changedBodies?: string[];
  deletedBodies?: string[];
  needsRepair?: unknown[];
};

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

function makeLane(previewOp?: (op: OperationOp, sketchId: string | undefined) => Promise<BackendPreview>) {
  const commit = vi.fn(
    (_op: OperationOp): Promise<ApplyOperationResult> =>
      Promise.resolve({ revision: 1, changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }], removedBodies: [], features: [] }),
  );
  const lane = createLocalSolverLane({ commit, latencyMs: () => 0, previewOp });
  lane.cacheSketchPlane("sk", {
    kind: "XY",
    origin: [0, 0, 0],
    xAxis: [1, 0, 0],
    yAxis: [0, 1, 0],
    normal: [0, 0, 1],
  });
  lane.cacheFinishedRegions("sk", [
    {
      regionId: "r",
      outerLoop: [],
      holes: [],
      previewTriangles: { positions: [0, 0, 10, 0, 10, 10, 0, 10], indices: [0, 1, 2, 0, 2, 3], holesSubtracted: 0 },
    },
  ]);
  return { lane, commit };
}

describe("localSolver beginPreview accepts every builder-backed opType", () => {
  it.each([
    ["Extrude", { opType: "Extrude" as const, sketchId: "sk", regionId: "r", params: { distance: 5 } }],
    ["Revolve", { opType: "Revolve" as const, sketchId: "sk", regionId: "r", params: { angleDeg: 90 } }],
    ["Fillet", { opType: "Fillet" as const, params: { radius: 1, edgeIds: ["e:1"] } }],
    ["Chamfer", { opType: "Chamfer" as const, params: { radius: 1, edgeIds: ["e:1"] } }],
    ["Shell", { opType: "Shell" as const, params: { thickness: 1, openFaces: ["f"], targetBodyId: "b" } }],
    ["Boolean", { opType: "Boolean" as const, params: { operation: "Union" as const, targetBodyId: "a", toolBodyId: "b" } }],
    [
      "Hole",
      {
        opType: "Hole" as const,
        params: {
          targetBodyId: "b",
          face: { primary: { bodyId: "b", elementId: "el", kind: "face" as const } },
          point: [0, 0, 0] as [number, number, number],
          holeType: "simple" as const,
          diameter: 6,
          depth: null,
        },
      },
    ],
    [
      "OffsetFace",
      {
        opType: "OffsetFace" as const,
        params: {
          faces: [
            {
              primary: { bodyId: "b", elementId: "el", kind: "face" as const },
              anchor: { worldPoint: [0, 0, 1] as [number, number, number] },
            },
          ],
          distance: 2,
          distanceType: "Offset" as const,
          chainTangentFaces: true,
          targetBodyId: "b",
        },
      },
    ],
  ])("opens a %s session", async (_name, draft) => {
    const { lane } = makeLane();
    const s = await lane.beginPreview(draft);
    expect(s.sessionId).toMatch(/^pv-/);
    expect(s.previewBodyId).toBe(`preview:${s.sessionId}`);
    await lane.endPreview(s.sessionId, false);
  });

  it("still rejects an opType with no builder", async () => {
    const { lane } = makeLane();
    await expect(lane.beginPreview({ opType: "MirrorBody", params: {} })).rejects.toThrow(
      /does not support MirrorBody/,
    );
  });

  it("resolves a profile ONLY for the profile-based ops", async () => {
    const { lane } = makeLane();
    // Extrude/Revolve bind a region up-front — a stale id is refused with the
    // available identity (never silently bound to the first region).
    await expect(
      lane.beginPreview({ opType: "Extrude", sketchId: "sk", regionId: "nope", params: { distance: 1 } }),
    ).rejects.toThrow(/Unknown region nope.*available: r/);
    await expect(
      lane.beginPreview({ opType: "Revolve", sketchId: "sk", regionId: "nope", params: { angleDeg: 90 } }),
    ).rejects.toThrow(/Unknown region nope.*available: r/);
    // The solid-operating ops carry no profile at all, so no region is required.
    await expect(
      lane.beginPreview({ opType: "Shell", params: { thickness: 1, openFaces: ["f"], targetBodyId: "b" } }),
    ).resolves.toMatchObject({ sessionId: expect.stringMatching(/^pv-/) });
  });

  it("short-circuits exact preview + whole-param commit on a re-edit, for every opType", async () => {
    const drafts = [
      { opType: "Extrude" as const, sketchId: "sk", regionId: "r", params: { distance: 5, featureId: "F" } },
      { opType: "Revolve" as const, sketchId: "sk", regionId: "r", params: { angleDeg: 90, featureId: "F" } },
      { opType: "Fillet" as const, params: { radius: 1, edgeIds: ["e:1"], featureId: "F" } },
      { opType: "Chamfer" as const, params: { radius: 1, edgeIds: ["e:1"], featureId: "F" } },
      { opType: "Shell" as const, params: { thickness: 1, openFaces: ["f"], targetBodyId: "b", featureId: "F" } },
      { opType: "Boolean" as const, params: { operation: "Union" as const, targetBodyId: "a", toolBodyId: "b", featureId: "F" } },
      {
        opType: "Hole" as const,
        params: {
          targetBodyId: "b",
          face: { primary: { bodyId: "b", elementId: "el", kind: "face" as const } },
          point: [0, 0, 0] as [number, number, number],
          holeType: "simple" as const,
          diameter: 6,
          depth: null,
          featureId: "F",
        },
      },
    ];
    for (const draft of drafts) {
      const previewOp = vi.fn(async (): Promise<BackendPreview> => ({ bodies: [] }));
      const { lane } = makeLane(previewOp);
      const s = await lane.beginPreview(draft);
      lane.updatePreview(s.sessionId, { ...draft.params }, 1);
      await flush();
      expect(previewOp, `${draft.opType} must not run exact preview on a re-edit`).not.toHaveBeenCalled();
      await expect(lane.endPreview(s.sessionId, true)).rejects.toThrow(/scalar update/);
    }
  });

  it("keeps latest-wins under a burst for a non-Extrude op", async () => {
    let resolveFirst!: (v: BackendPreview) => void;
    const first = new Promise<BackendPreview>((r) => (resolveFirst = r));
    const previewOp = vi
      .fn<(op: OperationOp, sketchId: string | undefined) => Promise<BackendPreview>>()
      .mockReturnValueOnce(first)
      .mockResolvedValue({ bodies: [{ bodyId: "b", mesh: new ArrayBuffer(1) }] });
    const { lane } = makeLane(previewOp);
    const s = await lane.beginPreview({ opType: "Shell", params: { thickness: 1, openFaces: ["f"], targetBodyId: "b" } });

    lane.updatePreview(s.sessionId, { thickness: 1, openFaces: ["f"], targetBodyId: "b" }, 1);
    lane.updatePreview(s.sessionId, { thickness: 2, openFaces: ["f"], targetBodyId: "b" }, 2);
    lane.updatePreview(s.sessionId, { thickness: 3, openFaces: ["f"], targetBodyId: "b" }, 3);
    expect(previewOp).toHaveBeenCalledTimes(1); // ≤1 in flight

    resolveFirst({ bodies: [{ bodyId: "b", mesh: new ArrayBuffer(1) }] });
    await flush();
    expect(previewOp).toHaveBeenCalledTimes(2);
    const newest = previewOp.mock.calls[1][0];
    if (newest.opType !== "Shell") throw new Error("expected Shell");
    expect(newest.params.thickness).toBe(3); // only the NEWEST snapshot went out
    await lane.endPreview(s.sessionId, false);
  });

  it("publishes a STRUCTURAL failure when a builder throws (commit stays blocked)", async () => {
    const previewOp = vi.fn(async (): Promise<BackendPreview> => ({ bodies: [] }));
    const { lane } = makeLane(previewOp);
    const seen: PreviewResult[] = [];
    lane.onPreviewResult((r) => seen.push(r));
    const s = await lane.beginPreview({
      opType: "Boolean",
      params: { operation: "Union", targetBodyId: "a", toolBodyId: "b" },
    });
    // Target === tool: the builder refuses, so nothing reaches the kernel.
    lane.updatePreview(s.sessionId, { operation: "Union", targetBodyId: "a", toolBodyId: "a" }, 1);
    await flush();
    expect(previewOp).not.toHaveBeenCalled();
    expect(seen[0]?.error).toMatchObject({ structural: true });
    expect(seen[0]?.error?.message).toMatch(/different bodies/);
  });
});

describe("localSolver mock-lane local synthesis table", () => {
  it("synthesizes an L2 mesh for Extrude only", async () => {
    const { lane } = makeLane();
    const seen: PreviewResult[] = [];
    lane.onPreviewResult((r) => seen.push(r));
    const s = await lane.beginPreview({ opType: "Extrude", sketchId: "sk", regionId: "r", params: { distance: 5 } });
    lane.updatePreview(s.sessionId, { distance: 5 }, 1);
    await flush();
    expect(seen[0]?.mesh?.byteLength).toBeGreaterThan(0);
  });

  it.each(["Revolve", "Fillet", "Chamfer", "Shell", "Hole", "OffsetFace"] as const)(
    "settles the epoch for %s with NO faked geometry",
    async (opType) => {
      const { lane } = makeLane();
      const seen: PreviewResult[] = [];
      lane.onPreviewResult((r) => seen.push(r));
      const params: PreviewParams =
        opType === "Revolve"
          ? { angleDeg: 90 }
          : opType === "Shell"
            ? { thickness: 1, openFaces: ["f"], targetBodyId: "b" }
            : opType === "OffsetFace"
              ? {
                  faces: [
                    {
                      primary: { bodyId: "b", elementId: "el", kind: "face" as const },
                      anchor: { worldPoint: [0, 0, 1] as [number, number, number] },
                    },
                  ],
                  distance: 2,
                  distanceType: "Offset" as const,
                  chainTangentFaces: true,
                  targetBodyId: "b",
                }
              : opType === "Hole"
              ? {
                  targetBodyId: "b",
                  face: { primary: { bodyId: "b", elementId: "el", kind: "face" as const } },
                  point: [0, 0, 0] as [number, number, number],
                  holeType: "simple" as const,
                  diameter: 6,
                  depth: null,
                }
              : { radius: 1, edgeIds: ["e:1"] };
      const draft =
        opType === "Revolve"
          ? { opType, sketchId: "sk", regionId: "r", params }
          : { opType, params };
      const s = await lane.beginPreview(draft);
      lane.updatePreview(s.sessionId, params, 7);
      await flush();
      expect(seen).toHaveLength(1);
      expect(seen[0]).toMatchObject({ sessionId: s.sessionId, epoch: 7 });
      expect(seen[0].mesh).toBeUndefined();
      expect(seen[0].bodies).toBeUndefined();
      expect(seen[0].error).toBeUndefined();
      expect(seen[0].committed).toBeUndefined();
    },
  );

  it("hides the consumed tool body for a mock Boolean (truthful, no fake mesh)", async () => {
    const { lane } = makeLane();
    const seen: PreviewResult[] = [];
    lane.onPreviewResult((r) => seen.push(r));
    const s = await lane.beginPreview({
      opType: "Boolean",
      params: { operation: "Cut", targetBodyId: "a", toolBodyId: "b" },
    });
    lane.updatePreview(s.sessionId, { operation: "Cut", targetBodyId: "a", toolBodyId: "b" }, 3);
    await flush();
    expect(seen[0]).toMatchObject({ epoch: 3, bodies: [], replacedBodyIds: ["b"] });
    expect(seen[0].mesh).toBeUndefined();
  });
});

// ── PlaceComponent's source (Component Library WP-3.2) ───────────────────────

describe("PlaceComponent ghost carries the backend's resolved source", () => {
  function placeSession(source: unknown): PreviewSessionState {
    return session({
      opType: "PlaceComponent",
      latestParams: {
        componentId: "acme.bracket",
        componentVersion: "1.0.0",
        componentRevision: `sha256:${"0".repeat(64)}`,
        source,
        translate: [1, 2, 3],
      } as unknown as PreviewParams,
    });
  }

  it("passes a blob-backed source through verbatim", () => {
    // The ghost previews the SAME source the commit records. Before WP-3.2 this
    // builder synthesized a generator source, so a document-source component
    // previewed the generator stub's screw and committed something else.
    const op = OP_BUILDERS.PlaceComponent!(
      placeSession({
        kind: "document",
        documentSha256: "a".repeat(64),
        sha256: "b".repeat(64),
        codec: "xbf",
        brepFormat: 12,
      }),
    );
    expect(op).toMatchObject({
      opType: "PlaceComponent",
      params: {
        source: {
          kind: "document",
          documentSha256: "a".repeat(64),
          sha256: "b".repeat(64),
          codec: "xbf",
          brepFormat: 12,
        },
      },
    });
  });

  it("throws on an under-specified source instead of defaulting one", () => {
    // A missing pointer must fail at the arm, where the component is named —
    // never silently preview (and then commit) a different body.
    expect(() => OP_BUILDERS.PlaceComponent!(placeSession({ kind: "embedded", codec: "brep" }))).toThrow(
      /sha256/,
    );
    expect(() =>
      OP_BUILDERS.PlaceComponent!(placeSession({ kind: "document", sha256: "b".repeat(64), codec: "brep" })),
    ).toThrow(/documentSha256/);
    expect(() => OP_BUILDERS.PlaceComponent!(placeSession({ kind: "registry" }))).toThrow(/unknown/);
    expect(() => OP_BUILDERS.PlaceComponent!(placeSession(undefined))).toThrow(/source/);
  });

  it("still accepts a generator source", () => {
    const op = OP_BUILDERS.PlaceComponent!(
      placeSession({ kind: "generator", generatorId: "iso4762", generatorVersion: 1 }),
    );
    expect(op.params).toMatchObject({
      source: { kind: "generator", generatorId: "iso4762", generatorVersion: 1 },
    });
    // No params ⇒ the key is absent, not an empty object: the generator's own
    // defaults must stay byte-identical for every caller that never auto-sizes.
    expect((op.params as { source: Record<string, unknown> }).source.params).toBeUndefined();
  });

  // WP-A3: the auto-sized thread rides in `source.params`, which is what the
  // worker's table lookup reads. A ghost that dropped it would preview the
  // generator's DEFAULT size and commit the chosen one.
  it("carries a generator source's free params through", () => {
    const op = OP_BUILDERS.PlaceComponent!(
      placeSession({
        kind: "generator",
        generatorId: "iso4762",
        generatorVersion: 1,
        params: { thread: "M8", length: 20, thread_detail: "cosmetic" },
      }),
    );
    expect(op.params).toMatchObject({
      source: { params: { thread: "M8", length: 20, thread_detail: "cosmetic" } },
    });
  });

  it("refuses a malformed params map rather than dropping it", () => {
    const bad = (params: unknown) =>
      placeSession({ kind: "generator", generatorId: "iso4762", generatorVersion: 1, params });
    expect(() => OP_BUILDERS.PlaceComponent!(bad([1, 2]))).toThrow(/params/);
    expect(() => OP_BUILDERS.PlaceComponent!(bad({ thread: { nested: true } }))).toThrow(/thread/);
    expect(() => OP_BUILDERS.PlaceComponent!(bad({ length: Number.NaN }))).toThrow(/finite/);
  });
});
