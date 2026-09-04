/*
 * tauriCommandMap — M4b raw EditCommand builders (repair rebind + history
 * affordances). The dual `edge_ids`/`edges` lockstep rule is the pinned invariant.
 */
import { describe, it, expect } from "vitest";
import {
  bareBodyId,
  edgeElementRef,
  rewriteFilletEdgeParams,
  updateFilletParamsCommand,
  updateScalarParamsCommand,
  filletEdgeRebindCommand,
  inputPathFor,
  rebindInputCommand,
  elementKindOfTopoKey,
  elementRefOfKind,
  suppressOperationCommand,
  rollbackToCursorCommand,
  removeOperationCommand,
  parseRefId,
  editCommandLabel,
  buildAddDatumPlane,
  deleteDatumCommand,
  operationToEditCommand,
  opLabelFor,
  wireOperation,
  type CurrentFilletParams,
  type WireElementRef,
  type WireEditCommand,
} from "./tauriCommandMap";
import type { OperationOp } from "./types";

/** Extract the AddOperation record's params (the wire `{opType, params}`). */
function addedParams(op: OperationOp): Record<string, unknown> {
  const cmd = operationToEditCommand(op);
  if (cmd.cmd !== "addOperation") throw new Error(`expected addOperation, got ${cmd.cmd}`);
  return cmd.record.params as unknown as Record<string, unknown>;
}

describe("parseRefId", () => {
  it("parses '<opId>.input<k>' into opId + index", () => {
    expect(parseRefId("op_5.input0")).toEqual({ opId: "op_5", index: 0 });
    expect(parseRefId("op_5.input2")).toEqual({ opId: "op_5", index: 2 });
    // opId may itself contain dots (uuid-ish) — only the trailing .input<k> is split.
    expect(parseRefId("a.b.c.input3")).toEqual({ opId: "a.b.c", index: 3 });
  });

  it("returns null for a shape that doesn't match", () => {
    expect(parseRefId("op_5")).toBeNull();
    expect(parseRefId("op_5.face0")).toBeNull();
  });
});

describe("edgeElementRef", () => {
  it("builds a typed edge ref with primary + anchor worldPoint", () => {
    expect(edgeElementRef("body1", "el_9", [1, 2, 3])).toEqual({
      primary: { bodyId: "body1", elementId: "el_9", kind: "edge" },
      anchor: { worldPoint: [1, 2, 3] },
    });
  });

  it("omits the anchor when no worldPos is given", () => {
    expect(edgeElementRef("body1", "el_9")).toEqual({
      primary: { bodyId: "body1", elementId: "el_9", kind: "edge" },
    });
  });

  it("strips a `body_<uuid>` wire-form bodyId to the bare core uuid", () => {
    // promoteSelection returns the worker wire form; the core EditCommand serde wants
    // a bare uuid (BodyId is #[serde(transparent)]).
    expect(bareBodyId("body_abc-123")).toBe("abc-123");
    expect(bareBodyId("abc-123")).toBe("abc-123"); // already bare — no-op
    expect(bareBodyId("body1")).toBe("body1"); // mock id (no underscore) — untouched
    expect(edgeElementRef("body_abc-123", "el_9").primary?.bodyId).toBe("abc-123");
  });
});

describe("filletParams — R-WP2.1 dual edge rule (AddOperation from UI selection)", () => {
  it("emits typed `edges` in lockstep with `edgeIds` (bodyId bare, elementId = edgeId, anchor)", () => {
    const bodyUuid = "11111111-1111-4111-8111-111111111111";
    const op: OperationOp = {
      opType: "Fillet",
      // The per-edge SemanticRefs the fillet tool authors: a `body_<uuid>` wire-form
      // body id (must be stripped) + the pick anchor world-point.
      inputs: [
        { primary: { bodyId: `body_${bodyUuid}`, kind: "edge" }, anchor: { worldPoint: [1, 2, 3] } },
        { primary: { bodyId: bodyUuid, kind: "edge" }, anchor: { worldPoint: [4, 5, 6] } },
      ],
      params: {
        mode: "Fillet",
        radius: 2,
        edgeIds: ["e:5", "e:9"],
        chainTangentEdges: true,
        tangentClosureVersion: 1,
      },
    };
    const p = addedParams(op);
    expect(p.radius).toEqual({ value: 2 });
    expect(p.edgeIds).toEqual(["e:5", "e:9"]);
    expect(p.chainTangentEdges).toBe(true);
    expect(p.tangentClosureVersion).toBe(1);
    const edges = p.edges as WireElementRef[];
    expect(edges).toHaveLength(2);
    // bodyId stripped to the bare uuid; primary.element == edgeIds[i] (F2 lockstep).
    expect(edges[0].primary).toEqual({ bodyId: bodyUuid, elementId: "e:5", kind: "edge" });
    expect(edges[1].primary).toEqual({ bodyId: bodyUuid, elementId: "e:9", kind: "edge" });
    // anchor world-point rides through so the worker's ladder resolves the edge.
    expect(edges[0].anchor).toEqual({ worldPoint: [1, 2, 3] });
    expect(edges[1].anchor).toEqual({ worldPoint: [4, 5, 6] });
  });

  it("marshals a bare-`edgeIds` fillet (no typed edges) when an edge carries no body", () => {
    const op: OperationOp = {
      opType: "Fillet",
      inputs: [{ primary: { bodyId: "", kind: "edge" }, anchor: {} }],
      params: { mode: "Fillet", radius: 1, edgeIds: ["e:3"] },
    };
    const p = addedParams(op);
    expect(p.edgeIds).toEqual(["e:3"]);
    expect("edges" in p).toBe(false);
    expect("tangentClosureVersion" in p).toBe(false);
  });

  it("marshals a bare-`edgeIds` fillet when the op carries no inputs (legacy path)", () => {
    const op: OperationOp = {
      opType: "Fillet",
      params: { mode: "Fillet", radius: 1, edgeIds: ["e:3"] },
    };
    const p = addedParams(op);
    expect("edges" in p).toBe(false);
  });

  // ── two-distance chamfer (SCHEMA §7.3, 2026-08-03 — WP-C T2a) ──────────────

  it("emits `distance2` for a Chamfer that has one, and NEVER for an equal-leg one", () => {
    const equal = addedParams({
      opType: "Chamfer",
      params: { mode: "Chamfer", radius: 1, edgeIds: ["e:3"] },
    });
    // Skip-none on the Rust side: an equal-leg chamfer's wire form must stay
    // byte-identical to every document authored before the field existed.
    expect("distance2" in equal).toBe(false);

    const asym = addedParams({
      opType: "Chamfer",
      params: { mode: "Chamfer", radius: 1, distance2: 2.5, edgeIds: ["e:3"] },
    });
    expect(asym.distance2).toEqual({ value: 2.5 });
    expect(asym.radius).toEqual({ value: 1 });
  });

  it("DROPS `distance2` from a Fillet — the field is Chamfer-only (core rejects it)", () => {
    const p = addedParams({
      opType: "Fillet",
      // A caller that re-typed a chamfer's params object to Fillet without
      // stripping the second leg: the seam marshals a plain fillet rather than a
      // record core would have to reject.
      params: { mode: "Fillet", radius: 1, distance2: 2.5, edgeIds: ["e:3"] },
    });
    expect("distance2" in p).toBe(false);
  });

  // ── distance-angle chamfer (SCHEMA §7.3) ──────────────────────────────────

  it("emits `angleDeg` (DEGREES) for a Chamfer that has one, never for one that has not", () => {
    const plain = addedParams({
      opType: "Chamfer",
      params: { mode: "Chamfer", radius: 1, edgeIds: ["e:3"] },
    });
    expect("angleDeg" in plain).toBe(false);

    const angled = addedParams({
      opType: "Chamfer",
      params: { mode: "Chamfer", radius: 1, angleDeg: 30, edgeIds: ["e:3"] },
    });
    // Degrees ride through UNCONVERTED — the op-param convention (`angleUnits` is
    // the sketch radians seam and has no business on this lane).
    expect(angled.angleDeg).toEqual({ value: 30 });
    expect(angled.radius).toEqual({ value: 1 });
  });

  it("DROPS `angleDeg` from a Fillet — Chamfer-only, exactly like `distance2`", () => {
    const p = addedParams({
      opType: "Fillet",
      params: { mode: "Fillet", radius: 1, angleDeg: 30, edgeIds: ["e:3"] },
    });
    expect("angleDeg" in p).toBe(false);
  });

  it("NEVER emits both chamfer modes — angle wins, the wire's own presence order", () => {
    // Core refuses a Chamfer carrying both by name, so no marshalling seam may
    // author one. The FSM already keeps at most one; this is the wire-side half.
    const both = addedParams({
      opType: "Chamfer",
      params: { mode: "Chamfer", radius: 1, distance2: 2.5, angleDeg: 30, edgeIds: ["e:3"] },
    });
    expect(both.angleDeg).toEqual({ value: 30 });
    expect("distance2" in both).toBe(false);
  });

  // ── reference faces (SCHEMA §7.3, kernel-hardening WP-F) ───────────────────

  const BODY = "22222222-2222-4222-8222-222222222222";

  /** An asymmetric two-edge chamfer with ONE reference-face pair, in slot order:
   *  the two EDGE inputs, then the pair's FACE input (`inputs[N + i]`). */
  const typedChamfer = (): OperationOp => ({
    opType: "Chamfer",
    inputs: [
      { primary: { bodyId: `body_${BODY}`, kind: "edge" }, anchor: { worldPoint: [1, 2, 3] } },
      { primary: { bodyId: BODY, kind: "edge" }, anchor: { worldPoint: [4, 5, 6] } },
      {
        primary: { bodyId: BODY, elementId: "el_f7", kind: "face" },
        anchor: { worldPoint: [7, 8, 9] },
      },
    ],
    params: {
      mode: "Chamfer",
      radius: 4,
      distance2: 1,
      edgeIds: ["el_e1", "el_e2"],
      referenceFaces: [{ edgeId: "el_e1", faceId: "el_f7" }],
      chainTangentEdges: true,
      tangentClosureVersion: 1,
    },
  });

  it("emits `referenceFaces` + typed `referenceFaceRefs` in lockstep, faces AFTER edges", () => {
    const p = addedParams(typedChamfer());
    expect(p.referenceFaces).toEqual([{ edgeId: "el_e1", faceId: "el_f7" }]);
    const refs = p.referenceFaceRefs as WireElementRef[];
    expect(refs).toHaveLength(1);
    // A FACE primary on the operated body (bare uuid), element id == the pair's
    // faceId, and the anchor core makes non-optional — without it two congruent
    // faces tie and the ref is NeedsRepair on every replay.
    expect(refs[0].primary).toEqual({ bodyId: BODY, elementId: "el_f7", kind: "face" });
    expect(refs[0].anchor).toEqual({ worldPoint: [7, 8, 9] });
    // The EDGE refs are untouched and still lockstep with `edgeIds`: the face input
    // must not be mistaken for an edge, or slot N would rewrite edge N.
    const edges = p.edges as WireElementRef[];
    expect(edges.map((e) => e.primary?.elementId)).toEqual(["el_e1", "el_e2"]);
  });

  it("DROPS the pairs from an equal-leg Chamfer — it has no reference face", () => {
    const op = typedChamfer();
    delete (op.params as unknown as Record<string, unknown>).distance2;
    const p = addedParams(op);
    expect("referenceFaces" in p).toBe(false);
    expect("referenceFaceRefs" in p).toBe(false);
  });

  it("DROPS the pairs from a Fillet — `referenceFaces` is Chamfer-only", () => {
    const op = typedChamfer();
    op.opType = "Fillet";
    (op.params as unknown as Record<string, unknown>).mode = "Fillet";
    const p = addedParams(op);
    expect("referenceFaces" in p).toBe(false);
    expect("referenceFaceRefs" in p).toBe(false);
  });

  it("emits NEITHER key when the face inputs are missing or do not match the pairs", () => {
    // Core refuses `referenceFaces` without `referenceFaceRefs` (and a length
    // mismatch) by name, so a half-authored chamfer must not marshal one of them.
    const noFace = typedChamfer();
    noFace.inputs = noFace.inputs!.slice(0, 2);
    expect("referenceFaces" in addedParams(noFace)).toBe(false);

    const mismatched = typedChamfer();
    mismatched.inputs![2] = {
      primary: { bodyId: BODY, elementId: "el_OTHER", kind: "face" },
      anchor: { worldPoint: [7, 8, 9] },
    };
    expect("referenceFaces" in addedParams(mismatched)).toBe(false);

    const anchorless = typedChamfer();
    anchorless.inputs![2] = { primary: { bodyId: BODY, elementId: "el_f7", kind: "face" } };
    expect("referenceFaces" in addedParams(anchorless)).toBe(false);
  });
});

describe("updateScalarParamsCommand — re-edit deep-merge (Findings 3+4)", () => {
  it("changes ONLY the scalar, preserving a revolve axis verbatim", () => {
    const stored = {
      angleDeg: { value: 360 },
      axis: { kind: "sketchLine", sketchId: "sk", lineId: "line-7" },
      booleanMode: "NewBody",
      profile: { sketchId: "sk", regionId: "r1" },
    };
    const cmd = updateScalarParamsCommand("rev-rec", "Revolve", stored, { angleDeg: { value: 90 } });
    if (cmd.cmd !== "updateOperationParams") throw new Error("unreachable");
    expect(cmd.record).toBe("rev-rec");
    expect(cmd.op.opType).toBe("Revolve");
    const p = cmd.op.params as unknown as Record<string, unknown>;
    expect(p.angleDeg).toEqual({ value: 90 }); // only the scalar changed
    expect(p.axis).toEqual({ kind: "sketchLine", sketchId: "sk", lineId: "line-7" }); // untouched
    expect(p.profile).toEqual({ sketchId: "sk", regionId: "r1" });
    expect(p.booleanMode).toBe("NewBody");
  });

  // SCHEMA §7.3 WP-B "Region anchor": persisted ONCE at pick, so a re-edit's
  // scalar-only patch (never a `profile` key) must leave the stored anchor
  // byte-identical through the shallow `{...storedParams, ...patch}` merge.
  it("preserves a stored profile.regionAnchor byte-identical across a distance re-edit", () => {
    const stored = {
      distance: { value: 10 },
      extrudeMode: "Blind",
      booleanMode: "NewBody",
      profile: { sketchId: "sk", regionId: "r1", regionAnchor: [1.5, -2.25] },
    };
    const cmd = updateScalarParamsCommand("ext-rec", "Extrude", stored, { distance: { value: 25 } });
    if (cmd.cmd !== "updateOperationParams") throw new Error("unreachable");
    const p = cmd.op.params as unknown as Record<string, unknown>;
    expect(p.distance).toEqual({ value: 25 }); // only the scalar changed
    expect(p.profile).toEqual({ sketchId: "sk", regionId: "r1", regionAnchor: [1.5, -2.25] }); // untouched
  });

  it("preserves shell openFaces + targetBodyId while changing thickness", () => {
    const stored = { thickness: { value: 2 }, openFaces: ["el_a", "el_b"], targetBodyId: "b-uuid" };
    const cmd = updateScalarParamsCommand("shell-rec", "Shell", stored, { thickness: { value: 4 } });
    if (cmd.cmd !== "updateOperationParams") throw new Error("unreachable");
    const p = cmd.op.params as unknown as Record<string, unknown>;
    expect(p.thickness).toEqual({ value: 4 });
    expect(p.openFaces).toEqual(["el_a", "el_b"]); // NOT wiped
    expect(p.targetBodyId).toBe("b-uuid");
  });

  it("preserves fillet edgeIds + typed edges while changing radius", () => {
    const edges = [{ primary: { bodyId: "b", elementId: "e:5", kind: "edge" } }];
    const stored = { radius: { value: 2 }, edgeIds: ["e:5"], edges, chainTangentEdges: true };
    const cmd = updateScalarParamsCommand("fil-rec", "Fillet", stored, { radius: { value: 5 } });
    if (cmd.cmd !== "updateOperationParams") throw new Error("unreachable");
    const p = cmd.op.params as unknown as Record<string, unknown>;
    expect(p.radius).toEqual({ value: 5 });
    expect(p.edgeIds).toEqual(["e:5"]); // NOT dropped
    expect(p.edges).toEqual(edges);
  });
});

describe("rewriteFilletEdgeParams — the dual edge_ids/edges lockstep rule", () => {
  const current: CurrentFilletParams = {
    radius: 2,
    edgeIds: ["el_a", "el_b", "el_c"],
    edges: [
      { primary: { bodyId: "b1", elementId: "el_a", kind: "edge" } },
      { primary: { bodyId: "b1", elementId: "el_b", kind: "edge" } },
      { primary: { bodyId: "b1", elementId: "el_c", kind: "edge" } },
    ],
    chainTangentEdges: true,
    tangentClosureVersion: 1,
  };

  it("replaces ONLY the target slot in BOTH arrays, keeping siblings", () => {
    const ref = edgeElementRef("b1", "el_NEW", [5, 6, 7]);
    const params = rewriteFilletEdgeParams(current, 1, ref);
    // Bare id array: only index 1 changed.
    expect(params.edgeIds).toEqual(["el_a", "el_NEW", "el_c"]);
    // Typed array: index 1 is the new ref (with anchor); siblings untouched.
    expect(params.edges?.[0].primary?.elementId).toBe("el_a");
    expect(params.edges?.[1]).toEqual(ref);
    expect(params.edges?.[2].primary?.elementId).toBe("el_c");
    // The two arrays stay the same length (lockstep).
    expect(params.edgeIds.length).toBe(params.edges?.length);
    // radius passes through as a Scalar; chain flag preserved.
    expect(params.radius).toEqual({ value: 2 });
    expect(params.chainTangentEdges).toBe(true);
    expect(params.tangentClosureVersion).toBe(1);
  });

  it("keeps edgeIds[index] and edges[index].primary.elementId identical", () => {
    const ref = edgeElementRef("b1", "el_X");
    const params = rewriteFilletEdgeParams(current, 0, ref);
    expect(params.edgeIds[0]).toBe("el_X");
    expect(params.edges?.[0].primary?.elementId).toBe("el_X");
  });

  it("grows both arrays (in lockstep) for a legacy fillet with no typed edges", () => {
    const legacy: CurrentFilletParams = { radius: 1, edgeIds: ["el_a"] };
    const params = rewriteFilletEdgeParams(legacy, 1, edgeElementRef("b1", "el_b"));
    expect(params.edgeIds).toEqual(["el_a", "el_b"]);
    expect(params.edges).toHaveLength(2);
    expect(params.edges?.[1].primary?.elementId).toBe("el_b");
    expect("tangentClosureVersion" in params).toBe(false);
  });

  it("PRESERVES the chamfer mode a rebind is not touching (both modes)", () => {
    // The rewrite returns the COMPLETE params object, so a mode field missing from
    // `CurrentFilletParams` was silently dropped: a two-distance chamfer came back
    // equal-leg and a distance-angle one came back plain, from a rebind that was
    // only ever asked to move ONE edge.
    const asym = rewriteFilletEdgeParams(
      { ...current, distance2: 2.5 },
      1,
      edgeElementRef("b1", "el_NEW"),
    );
    expect(asym.distance2).toEqual({ value: 2.5 });
    expect("angleDeg" in asym).toBe(false);

    const angled = rewriteFilletEdgeParams(
      { ...current, angleDeg: 30 },
      1,
      edgeElementRef("b1", "el_NEW"),
    );
    expect(angled.angleDeg).toEqual({ value: 30 });
    expect("distance2" in angled).toBe(false);

    // An equal-leg fillet/chamfer still marshals with NEITHER key (skip-none).
    const plain = rewriteFilletEdgeParams(current, 1, edgeElementRef("b1", "el_NEW"));
    expect("distance2" in plain).toBe(false);
    expect("angleDeg" in plain).toBe(false);
  });

  it("wraps into an UpdateOperationParams command (opType Fillet)", () => {
    const params = rewriteFilletEdgeParams(current, 2, edgeElementRef("b1", "el_Z"));
    const cmd = updateFilletParamsCommand("rec-1", params);
    expect(cmd.cmd).toBe("updateOperationParams");
    expect(cmd).toMatchObject({ record: "rec-1", op: { opType: "Fillet" } });
  });
});

describe("filletEdgeRebindCommand (EditOperationInput — the live rebind path)", () => {
  it("carries the FilletEdges{index} path + the element ref", () => {
    const ref = edgeElementRef("b1", "el_9", [1, 2, 3]);
    const cmd = filletEdgeRebindCommand("rec-1", 2, ref);
    expect(cmd).toEqual({
      cmd: "editOperationInput",
      record: "rec-1",
      path: { path: "filletEdges", index: 2 },
      reference: { element: ref },
    });
  });
});

describe("history-affordance command mapping", () => {
  it("suppressOperationCommand → SetOperationSuppression", () => {
    expect(suppressOperationCommand("rec-1", true)).toEqual({
      cmd: "setOperationSuppression",
      record: "rec-1",
      suppressed: true,
      cascade: false,
    });
    expect(suppressOperationCommand("rec-1", false, true)).toMatchObject({
      suppressed: false,
      cascade: true,
    });
  });

  it("rollbackToCursorCommand → SetRollback (cursor = applied op count)", () => {
    // "Roll to here" = index + 1; the caller passes that count.
    expect(rollbackToCursorCommand(3)).toEqual({ cmd: "setRollback", cursor: 3 });
    // Clamped to a non-negative integer.
    expect(rollbackToCursorCommand(-1)).toEqual({ cmd: "setRollback", cursor: 0 });
  });

  it("removeOperationCommand → RemoveOperation", () => {
    expect(removeOperationCommand("rec-1")).toEqual({ cmd: "removeOperation", record: "rec-1" });
  });

  it("editCommandLabel gives a human hint per command", () => {
    expect(editCommandLabel(removeOperationCommand("r"))).toBe("Delete feature");
    expect(editCommandLabel(suppressOperationCommand("r", true))).toBe("Suppress");
    expect(editCommandLabel(suppressOperationCommand("r", false))).toBe("Unsuppress");
    expect(editCommandLabel(rollbackToCursorCommand(2))).toBe("Rollback");
    expect(editCommandLabel(filletEdgeRebindCommand("r", 0, edgeElementRef("b", "e")))).toBe(
      "Repair reference",
    );
  });
});

// ── M6b op wire shapes — pinned against the Rust serde field names ────────────
//
// The names/units below are asserted 1:1 against record.rs (camelCase serde):
//   ShellParams          thickness:Scalar, openFaces:[ElementId], targetBodyId?
//   LinearPatternParams  sourceBodyId?, direction:Vec3, spacing:Scalar, count:u32, fuseResult
//   CircularPatternParams sourceBodyId?, axisOrigin:Vec3, axisDirection:Vec3, angleDeg:Scalar, count:u32, fuseResult
//   MirrorBodyParams     sourceBodyId?, planePoint:Vec3, planeNormal:Vec3, fuseWithOriginal
// Scalar wire form is `{value}`; Vec3 wire form is the `[x,y,z]` array; count is
// a BARE number (u32), not a Scalar.
describe("operationToEditCommand — M6b op wire mappings", () => {
  it("reuses one stable opId for PreviewOp and the committed operation record", () => {
    const opId = "11111111-1111-4111-8111-111111111111";
    const op: OperationOp = {
      opType: "Extrude",
      opId,
      sketchId: "sk",
      regionId: "r",
      params: { distance: 5 },
    };
    expect(wireOperation(op).opId).toBe(opId);
    const command = operationToEditCommand(op);
    if (command.cmd !== "addOperation") throw new Error("expected addOperation");
    expect(command.record.recordId).toBe(opId);
    expect("opId" in command.record).toBe(false);
  });

  it("preserves a v2 region identity version inside the stored profile", () => {
    const command = operationToEditCommand({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r_v2",
      regionIdentityVersion: 2,
      params: { distance: 5 },
    });
    if (command.cmd !== "addOperation") throw new Error("expected addOperation");
    expect((command.record.params as { profile?: unknown }).profile).toEqual({
      sketchId: "sk",
      regionId: "r_v2",
      regionIdentityVersion: 2,
    });
  });

  // SCHEMA §7.3 WP-B "Region anchor" — the pick-time `[u, v]` centroid rides
  // inside the SAME `profile` (Rust `SketchRegionRef.regionAnchor`).
  it("carries regionAnchor inside the stored profile for Extrude and Revolve", () => {
    const extrudeCmd = operationToEditCommand({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      regionAnchor: [1.5, -2.25],
      params: { distance: 5 },
    });
    if (extrudeCmd.cmd !== "addOperation") throw new Error("expected addOperation");
    expect((extrudeCmd.record.params as { profile?: unknown }).profile).toEqual({
      sketchId: "sk",
      regionId: "r",
      regionAnchor: [1.5, -2.25],
    });

    const revolveCmd = operationToEditCommand({
      opType: "Revolve",
      sketchId: "sk",
      regionId: "r",
      regionAnchor: [3, 4],
      params: { angleDeg: 90 },
    });
    if (revolveCmd.cmd !== "addOperation") throw new Error("expected addOperation");
    expect((revolveCmd.record.params as { profile?: unknown }).profile).toEqual({
      sketchId: "sk",
      regionId: "r",
      regionAnchor: [3, 4],
    });
  });

  it("omits a non-finite regionAnchor instead of sending it (the worker refuses one)", () => {
    const command = operationToEditCommand({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      regionAnchor: [Number.NaN, 1],
      params: { distance: 5 },
    });
    if (command.cmd !== "addOperation") throw new Error("expected addOperation");
    const profile = (command.record.params as { profile?: Record<string, unknown> }).profile;
    expect(profile).toEqual({ sketchId: "sk", regionId: "r" });
    expect("regionAnchor" in (profile ?? {})).toBe(false);
  });

  it("Shell maps openFaces + typed faces in lockstep", () => {
    const op: OperationOp = {
      opType: "Shell",
      inputs: [
        {
          primary: { bodyId: "body_body1", elementId: "el_a", kind: "face" },
          anchor: { worldPoint: [0, 0, 10] },
        },
        {
          primary: { bodyId: "body_body1", elementId: "el_b", kind: "face" },
          anchor: { worldPoint: [20, 0, 10] },
        },
      ],
      params: { thickness: 2.5, openFaces: ["el_a", "el_b"], targetBodyId: "body1" },
    };
    expect(addedParams(op)).toEqual({
      thickness: { value: 2.5 },
      openFaces: ["el_a", "el_b"],
      faces: [
        {
          primary: { bodyId: "body1", elementId: "el_a", kind: "face" },
          anchor: { worldPoint: [0, 0, 10] },
        },
        {
          primary: { bodyId: "body1", elementId: "el_b", kind: "face" },
          anchor: { worldPoint: [20, 0, 10] },
        },
      ],
      targetBodyId: "body1",
    });
  });

  it("Shell preserves legacy bare openFaces when typed inputs are absent", () => {
    const op: OperationOp = {
      opType: "Shell",
      params: { thickness: 2.5, openFaces: ["el_a"], targetBodyId: "body1" },
    };
    expect(addedParams(op)).toEqual({
      thickness: { value: 2.5 },
      openFaces: ["el_a"],
      targetBodyId: "body1",
    });
  });

  it("Shell rejects mismatched typed identity instead of rewriting it", () => {
    const op: OperationOp = {
      opType: "Shell",
      inputs: [
        {
          primary: { bodyId: "body_body1", elementId: "el_wrong", kind: "face" },
          anchor: { worldPoint: [0, 0, 10] },
        },
      ],
      params: { thickness: 2.5, openFaces: ["el_expected"], targetBodyId: "body1" },
    };
    expect(() => addedParams(op)).toThrow(/must match openFaces and targetBodyId in lockstep/);
  });

  /*
   * OffsetFace (SCHEMA §7.3 `op.offsetFace`). The expectations below are the
   * FROZEN Rust serde form, transcribed 1:1 from
   * `crates/onecad-core/tests/snapshots/schema_freeze__operation_offsetface_*.snap`
   * — `faceIds` (bare) + `faces` (typed) in lockstep, `distance` a Scalar,
   * `distanceType`/`chainTangentFaces` bare, the `oppositeFace*` pair skip-if-none,
   * and `targetBodyId` a BARE uuid (`BodyId` is `#[serde(transparent)]`).
   */
  it("OffsetFace maps faceIds + typed faces in LOCKSTEP (core validates the pairing)", () => {
    const op: OperationOp = {
      opType: "OffsetFace",
      params: {
        faces: [
          {
            primary: { bodyId: "body_b0d1", elementId: "el_f1", kind: "face" },
            anchor: { worldPoint: [0, 0, 10] },
          },
          {
            primary: { bodyId: "body_b0d1", elementId: "el_f2", kind: "face" },
            anchor: { worldPoint: [40, 0, 10] },
          },
        ],
        distance: 2.5,
        distanceType: "Offset",
        chainTangentFaces: true,
        targetBodyId: "body_b0d1",
      },
    };
    expect(addedParams(op)).toEqual({
      faceIds: ["el_f1", "el_f2"],
      faces: [
        {
          primary: { bodyId: "b0d1", elementId: "el_f1", kind: "face" },
          anchor: { worldPoint: [0, 0, 10] },
        },
        {
          primary: { bodyId: "b0d1", elementId: "el_f2", kind: "face" },
          anchor: { worldPoint: [40, 0, 10] },
        },
      ],
      distance: { value: 2.5 },
      distanceType: "Offset",
      chainTangentFaces: true,
      // `body_` STRIPPED: a promoted pick carries the worker wire form, and the
      // core EditCommand serde takes only the bare uuid.
      targetBodyId: "b0d1",
    });
  });

  it("OffsetFace Total carries BOTH oppositeFaceId and oppositeFace", () => {
    const op: OperationOp = {
      opType: "OffsetFace",
      params: {
        faces: [{ primary: { bodyId: "b0d1", elementId: "el_top", kind: "face" } }],
        distance: 12,
        distanceType: "Total",
        chainTangentFaces: false,
        oppositeFace: {
          primary: { bodyId: "b0d1", elementId: "el_bottom", kind: "face" },
          anchor: { worldPoint: [0, 0, 0] },
        },
        targetBodyId: "b0d1",
      },
    };
    expect(addedParams(op)).toMatchObject({
      faceIds: ["el_top"],
      distanceType: "Total",
      chainTangentFaces: false,
      oppositeFaceId: "el_bottom",
      oppositeFace: {
        primary: { bodyId: "b0d1", elementId: "el_bottom", kind: "face" },
        anchor: { worldPoint: [0, 0, 0] },
      },
    });
  });

  it("OffsetFace DROPS a stale opposite on a non-Total type (gated on the TYPE)", () => {
    // The holeParams rule restated: a caller that re-types away from Total must
    // marshal a clean record, not one the Rust session has to reject.
    const params = addedParams({
      opType: "OffsetFace",
      params: {
        faces: [{ primary: { bodyId: "b0d1", elementId: "el_cyl", kind: "face" } }],
        distance: 6,
        distanceType: "Radius",
        chainTangentFaces: true,
        oppositeFace: { primary: { bodyId: "b0d1", elementId: "el_stale", kind: "face" } },
        targetBodyId: "b0d1",
      },
    });
    expect("oppositeFace" in params).toBe(false);
    expect("oppositeFaceId" in params).toBe(false);
    expect(params.distanceType).toBe("Radius");
  });

  /*
   * The result-policy gate. 2 and 3 are two READINGS of one payload (SCHEMA §7.3,
   * WP3-C5), so both marshal identically and the version rides through verbatim —
   * clamping a 3 down to a 2 here would send the kernel the plain sweep for a
   * record the user authored as blend-aware.
   */
  it.each([2, 3] as const)(
    "OffsetFace forwards resultPolicyVersion %i with its primary subset",
    (version) => {
      const params = addedParams({
        opType: "OffsetFace",
        params: {
          faces: [
            { primary: { bodyId: "b0d1", elementId: "el_f1", kind: "face" } },
            { primary: { bodyId: "b0d1", elementId: "el_blend", kind: "face" } },
          ],
          primaryFaceIds: ["el_f1"],
          resultPolicyVersion: version,
          distance: 2.5,
          distanceType: "Offset",
          chainTangentFaces: true,
          targetBodyId: "b0d1",
        },
      });
      expect(params.resultPolicyVersion).toBe(version);
      expect(params.primaryFaceIds).toEqual(["el_f1"]);
      expect(params.faceIds).toEqual(["el_f1", "el_blend"]);
    },
  );

  it.each([2, 3] as const)(
    "OffsetFace v%i refuses a primary set that is not a non-empty subset",
    (version) => {
      const build = (primaryFaceIds: string[]): Record<string, unknown> =>
        addedParams({
          opType: "OffsetFace",
          params: {
            faces: [{ primary: { bodyId: "b0d1", elementId: "el_f1", kind: "face" } }],
            primaryFaceIds,
            resultPolicyVersion: version,
            distance: 2.5,
            distanceType: "Offset",
            chainTangentFaces: true,
            targetBodyId: "b0d1",
          },
        });
      expect(() => build([])).toThrow(/non-empty subset of faceIds/);
      expect(() => build(["el_not_in_closure"])).toThrow(/non-empty subset of faceIds/);
    },
  );

  it("OffsetFace refuses primaryFaceIds with NO version — the pair is inseparable", () => {
    expect(() =>
      addedParams({
        opType: "OffsetFace",
        params: {
          faces: [{ primary: { bodyId: "b0d1", elementId: "el_f1", kind: "face" } }],
          primaryFaceIds: ["el_f1"],
          distance: 2.5,
          distanceType: "Offset",
          chainTangentFaces: true,
          targetBodyId: "b0d1",
        },
      }),
    ).toThrow("OffsetFace primaryFaceIds requires resultPolicyVersion 2 or 3");
  });

  it("OffsetFace labels as `Offset face` (matching dto.rs default_label)", () => {
    expect(
      opLabelFor({
        opType: "OffsetFace",
        params: {
          faces: [{ primary: { bodyId: "b", elementId: "el", kind: "face" } }],
          distance: 1,
          distanceType: "Offset",
          chainTangentFaces: true,
          targetBodyId: "b",
        },
      }),
    ).toBe("Offset face");
  });

  it("LinearPattern maps versioned lineage with direction, Scalar spacing, and bare count", () => {
    const op: OperationOp = {
      opType: "LinearPattern",
      params: { sourceBodyId: "body1", direction: [1, 0, 0], spacing: 20, count: 4, resultPolicyVersion: 2 },
    };
    const params = addedParams(op);
    expect(params).toEqual({
      sourceBodyId: "body1",
      direction: [1, 0, 0],
      spacing: { value: 20 },
      count: 4,
      fuseResult: true, // Rust `default_true`
      resultPolicyVersion: 2,
    });
    // count is a bare number, NOT a Scalar object.
    expect(typeof params.count).toBe("number");
  });

  it("CircularPattern maps axisOrigin/axisDirection (Vec3) + angleDeg (Scalar) + count", () => {
    const op: OperationOp = {
      opType: "CircularPattern",
      params: {
        sourceBodyId: "body1",
        axisOrigin: [0, 0, 0],
        axisDirection: [0, 0, 1],
        angleDeg: 360,
        count: 6,
        fuseResult: false,
      },
    };
    expect(addedParams(op)).toEqual({
      sourceBodyId: "body1",
      axisOrigin: [0, 0, 0],
      axisDirection: [0, 0, 1],
      angleDeg: { value: 360 },
      count: 6,
      fuseResult: false,
    });
  });

  it("MirrorBody maps planePoint/planeNormal (Vec3) + fuseWithOriginal (defaults false)", () => {
    const op: OperationOp = {
      opType: "MirrorBody",
      params: { sourceBodyId: "body1", planePoint: [0, 0, 0], planeNormal: [1, 0, 0] },
    };
    expect(addedParams(op)).toEqual({
      sourceBodyId: "body1",
      planePoint: [0, 0, 0],
      planeNormal: [1, 0, 0],
      fuseWithOriginal: false, // Rust default (NOT default_true)
    });
  });

  // ── TransformBody (WP-B W1; SCHEMA §7.3) ────────────────────────────────────
  //
  //   TransformBodyParams   targets: Vec<BodyId>, translate: [Scalar; 3],
  //                         rotate: {center: Vec3, axis: Vec3, angleDeg: Scalar},
  //                         copy: bool
  //
  // The two easy mistakes both live in the SHAPES, not the values: each translate
  // component is expression-capable and therefore a Scalar (a bare number is
  // rejected), while `center`/`axis` are plain Vec3s (a Scalar there is equally
  // rejected). And `targets` uses the PLAIN `Vec<BodyId>` derive — no
  // `de_opt_body_id` leniency like `sourceBodyId` — so a `body_<uuid>` worker-wire
  // id would sink the whole EditCommand ("body_…" is not a uuid).

  it("TransformBody maps translate components as SCALARS and center/axis as Vec3", () => {
    const op: OperationOp = {
      opType: "TransformBody",
      params: {
        targets: ["body1"],
        translate: [30, 0, -5],
        rotate: { center: [1, 2, 3], axis: [0, 0, 1], angleDeg: 90 },
        copy: false,
      },
    };
    expect(addedParams(op)).toEqual({
      targets: ["body1"],
      translate: [{ value: 30 }, { value: 0 }, { value: -5 }],
      rotate: { center: [1, 2, 3], axis: [0, 0, 1], angleDeg: { value: 90 } },
      copy: false,
    });
  });

  it("TransformBody normalizes every target to the BARE uuid the core serde takes", () => {
    const uuid = "0f7e5d2c-1111-4222-8333-444455556666";
    const op: OperationOp = {
      opType: "TransformBody",
      params: {
        targets: [`body_${uuid}`, "body1"],
        translate: [0, 0, 0],
        rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 },
        copy: true,
      },
    };
    const params = addedParams(op) as { targets: string[]; copy: boolean };
    // Idempotent: an already-bare id (and a mock id with no underscore) passes through.
    expect(params.targets).toEqual([uuid, "body1"]);
    expect(params.copy).toBe(true);
  });

  it("a featureId re-targets a TransformBody via updateOperationParams (the FOLD path)", () => {
    const op: OperationOp = {
      opType: "TransformBody",
      featureId: "rec-move",
      params: {
        targets: ["body1"],
        translate: [50, 0, 0],
        rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 },
        copy: false,
      },
    };
    const cmd = operationToEditCommand(op);
    expect(cmd).toMatchObject({
      cmd: "updateOperationParams",
      record: "rec-move",
      op: { opType: "TransformBody" },
    });
  });

  it("a featureId re-targets a pattern op via updateOperationParams (parametric edit)", () => {
    const op: OperationOp = {
      opType: "LinearPattern",
      featureId: "rec-9",
      params: { sourceBodyId: "body1", direction: [1, 0, 0], spacing: 10, count: 3 },
    };
    const cmd = operationToEditCommand(op);
    expect(cmd).toMatchObject({ cmd: "updateOperationParams", record: "rec-9", op: { opType: "LinearPattern" } });
  });

  it("opLabelFor gives friendly labels for the M6b ops", () => {
    expect(opLabelFor({ opType: "Shell", params: { thickness: 1, openFaces: [] } })).toBe("Shell");
    expect(
      opLabelFor({ opType: "LinearPattern", params: { direction: [1, 0, 0], spacing: 1, count: 2 } }),
    ).toBe("Linear Pattern");
    expect(
      opLabelFor({
        opType: "CircularPattern",
        params: { axisOrigin: [0, 0, 0], axisDirection: [0, 0, 1], angleDeg: 360, count: 2 },
      }),
    ).toBe("Circular Pattern");
    expect(opLabelFor({ opType: "MirrorBody", params: { planePoint: [0, 0, 0], planeNormal: [1, 0, 0] } })).toBe(
      "Mirror",
    );
    // Matches `dto.rs default_label` — the history row reads "Move", so the
    // status hint must not say "TransformBody".
    expect(
      opLabelFor({
        opType: "TransformBody",
        params: {
          targets: ["body1"],
          translate: [1, 0, 0],
          rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 },
          copy: false,
        },
      }),
    ).toBe("Move");
  });
});

/**
 * Narrow an `addOperation` command to its record. A bare cast is unsound here —
 * the `WireEditCommand` union's `updateOperationParams` arm types `record` as a
 * string — so the assertion is a real check, not a `as unknown as` escape hatch.
 */
function addedRecord(cmd: WireEditCommand): { opType: string; params: Record<string, unknown> } {
  if (cmd.cmd !== "addOperation") throw new Error(`expected addOperation, got ${cmd.cmd}`);
  // The wire params are a union of closed interfaces (no index signature), so the
  // read-only view goes through the record's own JSON shape.
  return {
    opType: cmd.record.opType,
    params: cmd.record.params as unknown as Record<string, unknown>,
  };
}

// ── MODEL-OPS W1: end conditions + Chamfer on the wire ──────────────────────
describe("extrude end conditions marshalling", () => {
  const base = { opType: "Extrude" as const, sketchId: "sk", regionId: "r" };
  const faceRef = {
    primary: { bodyId: "body_11111111-1111-1111-1111-111111111111", elementId: "el_9", kind: "face" as const },
    anchor: { worldPoint: [1, 2, 3] as [number, number, number] },
  };

  it("sends the authored extrudeMode through unchanged", () => {
    const cmd = operationToEditCommand({ ...base, params: { distance: 5, extrudeMode: "ThroughAll" } });
    const op = addedRecord(cmd).params;
    expect(op.extrudeMode).toBe("ThroughAll");
  });

  it("marshals a ToFace target as a TYPED ref with a bare-uuid body", () => {
    const cmd = operationToEditCommand({
      ...base,
      params: { distance: 5, extrudeMode: "ToFace", targetFace: faceRef },
    });
    const p = addedRecord(cmd).params;
    expect(p.targetFace).toEqual({
      // `body_<uuid>` is the worker/promote form; the core EditCommand serde takes
      // the BARE uuid, so it must be stripped exactly as the fillet edges are.
      primary: { bodyId: "11111111-1111-1111-1111-111111111111", elementId: "el_9", kind: "face" },
      anchor: { worldPoint: [1, 2, 3] },
    });
  });

  // SCHEMA §7.3: "Absent for non-ToFace extrudes" — a mode switch away from ToFace
  // must not leave a stale target behind for the worker to resolve.
  it("omits targetFace when the mode is not ToFace", () => {
    const cmd = operationToEditCommand({
      ...base,
      params: { distance: 5, extrudeMode: "Blind", targetFace: faceRef },
    });
    const p = addedRecord(cmd).params;
    expect(p.targetFace).toBeUndefined();
  });

  it("omits targetFace2 unless BOTH twoDirections and ToFace are set", () => {
    const one = operationToEditCommand({
      ...base,
      params: { distance: 5, extrudeMode2: "ToFace", targetFace2: faceRef },
    });
    expect(addedRecord(one).params.targetFace2).toBeUndefined();
    const both = operationToEditCommand({
      ...base,
      params: { distance: 5, twoDirections: true, extrudeMode2: "ToFace", targetFace2: faceRef },
    });
    expect(addedRecord(both).params.targetFace2).toBeDefined();
  });

  it("carries the draft angle and the second direction's distance", () => {
    const cmd = operationToEditCommand({
      ...base,
      params: { distance: 5, draftAngleDeg: 8, twoDirections: true, distance2: 3 },
    });
    const p = addedRecord(cmd).params;
    expect(p.draftAngleDeg).toEqual({ value: 8 });
    expect(p.twoDirections).toBe(true);
    expect(p.distance2).toEqual({ value: 3 });
  });
});

describe("Chamfer marshalling", () => {
  it("authors opType Chamfer over the shared FilletChamferParams", () => {
    const cmd = operationToEditCommand({
      opType: "Chamfer",
      params: { mode: "Chamfer", radius: 1.5, edgeIds: ["e:1"] },
    });
    const rec = addedRecord(cmd);
    expect(rec.opType).toBe("Chamfer");
    expect(rec.params.radius).toEqual({ value: 1.5 });
    expect(rec.params.edgeIds).toEqual(["e:1"]);
  });
});

// ── Hole thread (WP-T1) ──────────────────────────────────────────────────────

describe("Hole marshalling — thread (WP-T1)", () => {
  const holeFace = {
    primary: { bodyId: "body_11111111-1111-1111-1111-111111111111", elementId: "el_top", kind: "face" as const },
  };
  const baseParams = {
    targetBodyId: "body_11111111-1111-1111-1111-111111111111",
    face: holeFace,
    point: [1, 2, 3] as [number, number, number],
    holeType: "simple" as const,
    diameter: 5.0,
    depth: 8.0,
  };

  it("emits no `thread` key at all when absent — byte-identical to before the field existed", () => {
    const cmd = operationToEditCommand({ opType: "Hole", params: baseParams });
    const rec = addedRecord(cmd);
    expect(rec.params).not.toHaveProperty("thread");
  });

  it("round-trips a full threaded HoleParams to the wire shape", () => {
    const cmd = operationToEditCommand({
      opType: "Hole",
      params: {
        ...baseParams,
        thread: {
          standard: "ISO261",
          designation: "M6x1",
          majorDiameterMm: 6.0,
          pitchMm: 1.0,
          depthMm: null,
          detail: "cosmetic",
        },
      },
    });
    const rec = addedRecord(cmd);
    expect(rec.params.thread).toEqual({
      standard: "ISO261",
      designation: "M6x1",
      // `majorDiameterMm` is a PLAIN wire number, not a Scalar `{value}` object.
      majorDiameterMm: 6.0,
      // `pitchMm` IS a Scalar (expression-drivable).
      pitchMm: { value: 1.0 },
      // `depthMm: null` is explicit — through the full hole depth, not omitted.
      depthMm: null,
      detail: "cosmetic",
    });
  });

  it("marshals a finite depthMm as a Scalar too", () => {
    const cmd = operationToEditCommand({
      opType: "Hole",
      params: {
        ...baseParams,
        thread: {
          standard: "ISO261",
          designation: "M6x1",
          majorDiameterMm: 6.0,
          pitchMm: 1.0,
          depthMm: 4.0,
          detail: "cosmetic",
        },
      },
    });
    const rec = addedRecord(cmd);
    expect(rec.params.thread).toMatchObject({ depthMm: { value: 4.0 } });
  });
});

// ── Datum planes (DATUM W1) ──────────────────────────────────────────────────

describe("buildAddDatumPlane", () => {
  it("emits the raw core DatumPlane struct with every serde-REQUIRED field", () => {
    const cmd = buildAddDatumPlane("d1", "Datum 1", "XY", 10);
    if (cmd.cmd !== "addDatumPlane") throw new Error(`expected addDatumPlane, got ${cmd.cmd}`);
    // The Rust `DatumPlane` has serde defaults ONLY for basePlaneId / the typed
    // refs / extra — everything below must be present or the whole command is
    // rejected at deserialize.
    expect(Object.keys(cmd.datum).sort()).toEqual(
      ["angleDeg", "basePlaneId", "id", "kind", "name", "offset", "resolvedPlane", "resolvedValid"].sort(),
    );
    expect(cmd.datum).toMatchObject({
      id: "d1",
      name: "Datum 1",
      kind: "OffsetFromPlane",
      basePlaneId: "XY",
      offset: 10,
      angleDeg: 0,
    });
  });

  it("sends an UNRESOLVED placeholder frame — the backend is the basis authority", () => {
    const cmd = buildAddDatumPlane("d1", "Datum 1", "XZ", 10);
    if (cmd.cmd !== "addDatumPlane") throw new Error("expected addDatumPlane");
    // Marked invalid on purpose: the core overwrites both fields in add_datum, so
    // a client that somehow had its frame honoured would still not claim it is
    // resolved. The basis matches the non-standard XY identity (Sketch.h).
    expect(cmd.datum.resolvedValid).toBe(false);
    expect(cmd.datum.resolvedPlane).toEqual({
      origin: [0, 0, 0],
      xAxis: [0, 1, 0],
      yAxis: [-1, 0, 0],
      normal: [0, 0, 1],
    });
  });

  it("accepts another datum's id as the base (chained offsets)", () => {
    const cmd = buildAddDatumPlane("d2", "Datum 2", "d1", -4);
    if (cmd.cmd !== "addDatumPlane") throw new Error("expected addDatumPlane");
    expect(cmd.datum.basePlaneId).toBe("d1");
    expect(cmd.datum.offset).toBe(-4);
  });
});

describe("deleteDatumCommand", () => {
  it("targets the datum by bare id", () => {
    expect(deleteDatumCommand("d1")).toEqual({ cmd: "deleteDatum", datum: "d1" });
  });
});

describe("editCommandLabel for datums", () => {
  it("labels the datum commands distinctly", () => {
    expect(editCommandLabel(buildAddDatumPlane("d1", "Datum 1", "XY", 10))).toBe("Create datum plane");
    expect(editCommandLabel(deleteDatumCommand("d1"))).toBe("Delete datum plane");
  });
});

// ── HISTORY-HARDEN H9: the repair SLOT TABLE, both sides ─────────────────────

describe("inputPathFor mirrors the Rust wire_op_inputs slot table", () => {
  /*
   * THE TWIN of `src-tauri/src/worker/wire.rs`
   * `wire_op_inputs_slot_order_is_the_repair_slot_table`. That test pins, per
   * opType, WHAT the k-th entry of the wire `inputs[]` array is; this one pins
   * which `InputPath` writes it. The two lists below are the same cases in the
   * same order — read them side by side when either changes.
   *
   * A DIVERGENCE IS A SILENT MIS-REPAIR: the panel would send a well-formed
   * `EditOperationInput` that overwrites a DIFFERENT input than the one the user
   * clicked. Before H9 every rebind went out as `filletEdges{k}` regardless of
   * op — the core rejected that loudly for Hole/Shell, but it would have quietly
   * rewritten edge k of any fillet.
   */
  const faceRef = { primary: { bodyId: "b", elementId: "el_2", kind: "face" as const } };

  const cases: Array<{
    name: string;
    opType: string;
    /** Rust slot kinds, VERBATIM from the Rust test's `expected` column. */
    rustSlots: string[];
    params?: Record<string, unknown>;
    /** Expected `inputPathFor(opType, k, params)` for k = 0..rustSlots.length. */
    paths: Array<ReturnType<typeof inputPathFor>>;
    /**
     * Fillet/Chamfer/Shell inputs are LIST-shaped, so the path is
     * index-parametric BY DESIGN: the projection does not expose the op's
     * current edge/face count, which is the whole reason the rebind is
     * slot-index-only (see `filletEdgeRebindCommand`). Bounds are enforced by
     * the CORE (`set_fillet_edge` / `set_shell_open_face` → "out of range"),
     * pinned in `edit_session.rs`. Every other op has FIXED arity, so one past
     * the last slot must resolve to null here.
     */
    listShaped?: true;
  }> = [
    {
      name: "fillet: one slot per edge, in `edges` order",
      opType: "Fillet",
      rustSlots: ["edge", "edge"],
      paths: [{ path: "filletEdges", index: 0 }, { path: "filletEdges", index: 1 }],
      listShaped: true,
    },
    {
      // Equal-leg / legacy chamfer: the edge table, exactly as Fillet's.
      name: "chamfer without referenceFaces: same table as fillet",
      opType: "Chamfer",
      rustSlots: ["edge"],
      params: { edgeIds: ["el_e1"] },
      paths: [{ path: "filletEdges", index: 0 }],
      listShaped: true,
    },
    {
      // SCHEMA §7.3 (WP-F): the `N` edge refs, THEN one face ref per pair. Slot 2
      // is `<opId>.input2` — the refId a §9 repair names for pair 0.
      name: "chamfer with referenceFaces: N edge slots, then one face slot per pair",
      opType: "Chamfer",
      rustSlots: ["edge", "edge", "face"],
      params: {
        edgeIds: ["el_e1", "el_e2"],
        referenceFaces: [{ edgeId: "el_e1", faceId: "el_f7" }],
      },
      paths: [
        { path: "filletEdges", index: 0 },
        { path: "filletEdges", index: 1 },
        { path: "chamferReferenceFace", index: 0 },
      ],
      listShaped: true,
    },
    {
      name: "shell: one slot per open face, in `openFaces` order",
      opType: "Shell",
      rustSlots: ["face", "face"],
      paths: [{ path: "shellOpenFaces", index: 0 }, { path: "shellOpenFaces", index: 1 }],
      listShaped: true,
    },
    {
      name: "hole: [host body, host face] — slot 0 is NOT element-addressable",
      opType: "Hole",
      rustSlots: ["body", "face"],
      paths: [null, { path: "holeFace" }],
    },
    {
      name: "boolean: [target, tool] — both whole bodies",
      opType: "Boolean",
      rustSlots: ["body", "body"],
      paths: [null, null],
    },
    {
      name: "extrude Blind: no ToFace target ⇒ NO slots at all",
      opType: "Extrude",
      rustSlots: [],
      params: { extrudeMode: "Blind", twoDirections: false, extrudeMode2: "Blind" },
      paths: [],
    },
    {
      name: "extrude ToFace, one direction: slot 0 = targetFace",
      opType: "Extrude",
      rustSlots: ["face"],
      params: {
        extrudeMode: "ToFace",
        targetFace: faceRef,
        twoDirections: false,
        extrudeMode2: "Blind",
      },
      paths: [{ path: "extrudeTargetFace", second: false }],
    },
    {
      name: "extrude ToFace both directions: slot 0 = targetFace, slot 1 = targetFace2",
      opType: "Extrude",
      rustSlots: ["face", "face"],
      params: {
        extrudeMode: "ToFace",
        targetFace: faceRef,
        twoDirections: true,
        extrudeMode2: "ToFace",
        targetFace2: faceRef,
      },
      paths: [
        { path: "extrudeTargetFace", second: false },
        { path: "extrudeTargetFace", second: true },
      ],
    },
    {
      // THE trap: direction 1 is Blind, so `targetFace2` COLLAPSES to slot 0.
      name: "extrude Blind + second direction ToFace: slot 0 = targetFace2",
      opType: "Extrude",
      rustSlots: ["face"],
      params: {
        extrudeMode: "Blind",
        targetFace: faceRef,
        twoDirections: true,
        extrudeMode2: "ToFace",
        targetFace2: faceRef,
      },
      paths: [{ path: "extrudeTargetFace", second: true }],
    },
    {
      name: "extrude ToFace + twoDirections but mode2 Blind: only slot 0",
      opType: "Extrude",
      rustSlots: ["face"],
      params: {
        extrudeMode: "ToFace",
        targetFace: faceRef,
        twoDirections: true,
        extrudeMode2: "Blind",
        targetFace2: faceRef,
      },
      paths: [{ path: "extrudeTargetFace", second: false }],
    },
    {
      name: "revolve typed edge axis: edgeRef is inputs[0]",
      opType: "Revolve",
      rustSlots: ["edge"],
      params: { axis: { kind: "edge", edgeRef: { primary: { kind: "edge" } } } },
      paths: [{ path: "revolveAxis" }],
    },
    { name: "linearPattern: the source body", opType: "LinearPattern", rustSlots: ["body"], paths: [null] },
    { name: "circularPattern: the source body", opType: "CircularPattern", rustSlots: ["body"], paths: [null] },
    { name: "mirrorBody: the source body", opType: "MirrorBody", rustSlots: ["body"], paths: [null] },
    {
      name: "transformBody: one slot per target, in `targets` order",
      opType: "TransformBody",
      rustSlots: ["body", "body"],
      paths: [null, null],
    },
    {
      // NORMATIVE slot order (SCHEMA §7.3): operative faces in stored order, then
      // the Total opposite LAST. Mirrors Rust `InputPath::OffsetFaceFace{index}` /
      // `OffsetFaceOpposite`.
      name: "offsetFace Offset: one slot per operative face, no opposite",
      opType: "OffsetFace",
      rustSlots: ["face", "face"],
      params: { faceIds: ["el_a", "el_b"], distanceType: "Offset" },
      paths: [
        { path: "offsetFaceFace", index: 0 },
        { path: "offsetFaceFace", index: 1 },
      ],
    },
    {
      name: "offsetFace Total: the opposite face is the LAST slot",
      opType: "OffsetFace",
      rustSlots: ["face", "face"],
      params: { faceIds: ["el_top"], oppositeFace: faceRef, distanceType: "Total" },
      paths: [{ path: "offsetFaceFace", index: 0 }, { path: "offsetFaceOpposite" }],
    },
    { name: "sketch: hostFace is core-only ⇒ no slots", opType: "Sketch", rustSlots: [], paths: [] },
    { name: "loft: profile sketches only", opType: "Loft", rustSlots: [], paths: [] },
    { name: "sweep: profile + path sketches only", opType: "Sweep", rustSlots: [], paths: [] },
    { name: "importStep: no inputs at all", opType: "ImportStep", rustSlots: [], paths: [] },
  ];

  it.each(cases)("$name", ({ opType, rustSlots, params, paths, listShaped }) => {
    expect(paths).toHaveLength(rustSlots.length);
    paths.forEach((expected, k) => {
      expect(inputPathFor(opType, k, params)).toEqual(expected);
      // A BODY slot must map to null — a repair candidate is an element TopoKey,
      // so there is nothing that could be written into a whole-body input.
      if (rustSlots[k] === "body") expect(inputPathFor(opType, k, params)).toBeNull();
    });
    // One past the last real slot never resolves to a path — except for the
    // list-shaped ops, whose bounds only the core knows (see `listShaped`).
    if (!listShaped) expect(inputPathFor(opType, rustSlots.length, params)).toBeNull();
  });

  it("refuses a missing/garbage opType and a non-integer slot", () => {
    expect(inputPathFor(undefined, 0)).toBeNull();
    expect(inputPathFor("NoSuchOp", 0)).toBeNull();
    expect(inputPathFor("Fillet", -1)).toBeNull();
    expect(inputPathFor("Fillet", 0.5)).toBeNull();
  });

  it("an OffsetFace with NO stored params refuses rather than guessing the opposite slot", () => {
    // The opposite's INDEX is `faceIds.length`, which nothing else knows — a guess
    // would rebind a different face than the one the user clicked.
    expect(inputPathFor("OffsetFace", 0)).toBeNull();
    expect(inputPathFor("OffsetFace", 1)).toBeNull();
  });

  it("a Chamfer with NO stored params refuses rather than guessing where the edges end", () => {
    // Without `edgeIds` there is no boundary between the edge slots and the
    // trailing reference-FACE slots (SCHEMA §7.3, WP-F), so slot k could be either.
    // Guessing `filletEdges{k}` would write a face ref into an EDGE slot — the H9
    // silent mis-repair this table exists to prevent.
    expect(inputPathFor("Chamfer", 0)).toBeNull();
    expect(inputPathFor("Chamfer", 0, { edgeIds: "nope" })).toBeNull();
    // A Fillet has no trailing slots at all, so it still answers without params.
    expect(inputPathFor("Fillet", 0)).toEqual({ path: "filletEdges", index: 0 });
  });

  it("a legacyReferenceFace repair CREATES the pair, keyed by the item's seedEdgeId", () => {
    // SCHEMA §9: the refId names an EMPTY slot (`index === referenceFaces.length`),
    // and core refuses a create whose `edgeId` is not one of the chamfer's edgeIds —
    // so the seed edge is echoed back verbatim rather than re-derived.
    const params = { edgeIds: ["el_e1", "el_e2"], referenceFaces: [] };
    expect(inputPathFor("Chamfer", 2, params, "el_e1")).toEqual({
      path: "chamferReferenceFace",
      index: 0,
      edgeId: "el_e1",
    });
    expect(inputPathFor("Chamfer", 3, params, "el_e2")).toEqual({
      path: "chamferReferenceFace",
      index: 1,
      edgeId: "el_e2",
    });
    // No seed edge (a REBIND of an existing pair): the key is omitted, and core
    // then leaves the stored pair's own edge in place.
    expect(
      inputPathFor("Chamfer", 2, { edgeIds: ["el_e1", "el_e2"], referenceFaces: [] }),
    ).toEqual({ path: "chamferReferenceFace", index: 0 });
  });

  it("an Extrude with NO stored params refuses rather than guessing a direction", () => {
    // Both ToFace slots exist only conditionally, so without the params slot 0
    // could be either `targetFace` or `targetFace2` — a guess rebinds the wrong
    // direction's target face.
    expect(inputPathFor("Extrude", 0)).toBeNull();
  });
});

describe("rebindInputCommand / element ref kinds", () => {
  it("wraps any InputPath as an EditOperationInput with an element ref", () => {
    const ref = elementRefOfKind("body_abc", "el_9", "face", [1, 2, 3]);
    expect(rebindInputCommand("op_1", { path: "holeFace" }, ref)).toEqual({
      cmd: "editOperationInput",
      record: "op_1",
      path: { path: "holeFace" },
      reference: { element: ref },
    });
  });

  it("derives primary.kind from the candidate TopoKey", () => {
    // A hole/shell rebind binds a FACE. Labelling it "edge" (the M4b default)
    // would put a lie into the ref the descriptor rung scores on the next edit.
    expect(elementKindOfTopoKey("f:22")).toBe("face");
    expect(elementKindOfTopoKey("e:5")).toBe("edge");
    expect(elementKindOfTopoKey("v:3")).toBe("vertex");
    expect(elementKindOfTopoKey("weird")).toBe("edge");
  });

  it("normalizes the body id to the bare uuid core serde accepts", () => {
    const ref = elementRefOfKind("body_00000000-0000-0000-0000-0000000000b0", "el_1", "face");
    expect(ref.primary?.bodyId).toBe("00000000-0000-0000-0000-0000000000b0");
    expect(ref.anchor).toBeUndefined();
  });
});
