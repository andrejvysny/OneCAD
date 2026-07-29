import { describe, it, expect } from "vitest";
import {
  extrudeInit,
  extrudeStep,
  filletInit,
  filletStep,
  booleanInit,
  booleanStep,
  revolveInit,
  revolveStep,
  shellInit,
  shellStep,
  linearPatternInit,
  linearPatternStep,
  circularPatternInit,
  circularPatternStep,
  mirrorInit,
  mirrorStep,
  regionSelectInit,
  regionSelectStep,
  DEFAULT_EXTRUDE_DEPTH,
  DEFAULT_REVOLVE_ANGLE,
  DEFAULT_SHELL_THICKNESS,
  type ExtrudeFsm,
  type FilletFsm,
  type BooleanFsm,
  type RevolveFsm,
  type ShellFsm,
  type LinearPatternFsm,
  type CircularPatternFsm,
  type MirrorFsm,
} from "./modelToolMachine";

describe("extrude FSM", () => {
  it("runs the Wave 1 gesture arm → grab → drag → release(armed) → confirm → settle", () => {
    let s = extrudeInit();
    expect(s.phase).toBe("idle");
    expect(s.booleanMode).toBe("NewBody");
    expect(s.targetBodyId).toBeNull();

    let step = extrudeStep(s, { kind: "arm" });
    expect(step.effect).toBe("begin");
    expect(step.state.phase).toBe("armed");
    expect(step.state.depth).toBe(DEFAULT_EXTRUDE_DEPTH);
    s = step.state;

    s = extrudeStep(s, { kind: "grab" }).state;
    expect(s.phase).toBe("dragging");

    step = extrudeStep(s, { kind: "drag", depth: 25, symmetric: true });
    expect(step.effect).toBe("update");
    expect(step.state.depth).toBe(25);
    expect(step.state.symmetric).toBe(true);
    s = step.state;

    // MODEL-HARDEN Wave 1: release KEEPS the tool armed (effect update, NOT commit).
    step = extrudeStep(s, { kind: "release" });
    expect(step.effect).toBe("update");
    expect(step.state.phase).toBe("armed");
    expect(step.state.depth).toBe(25); // final depth preserved
    s = step.state;

    // Explicit confirm → committing (commit effect).
    step = extrudeStep(s, { kind: "confirm" });
    expect(step.effect).toBe("commit");
    expect(step.state.phase).toBe("committing");
    s = step.state;

    s = extrudeStep(s, { kind: "settle" }).state;
    expect(s.phase).toBe("idle");
  });

  it("release is a no-op unless dragging; confirm only from armed", () => {
    const armed: ExtrudeFsm = extrudeStep(extrudeInit(), { kind: "arm" }).state;
    expect(extrudeStep(armed, { kind: "release" }).effect).toBe("none");
    const committing = extrudeStep(armed, { kind: "confirm" }).state;
    // confirm from committing is a no-op (guards a double-confirm).
    expect(extrudeStep(committing, { kind: "confirm" }).effect).toBe("none");
  });

  it("commitFailed returns committing → armed with no effect (controller re-arms)", () => {
    const committing = extrudeStep(
      extrudeStep(extrudeInit(), { kind: "arm", depth: 8 }).state,
      { kind: "confirm" },
    ).state;
    const step = extrudeStep(committing, { kind: "commitFailed" });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("armed");
    expect(step.state.depth).toBe(8); // depth preserved so re-arm keeps the work
    // commitFailed is only legal from committing.
    const armed = extrudeStep(extrudeInit(), { kind: "arm" }).state;
    expect(extrudeStep(armed, { kind: "commitFailed" }).effect).toBe("none");
  });

  it("drag is ignored unless dragging; setDepth + setSymmetric work while armed", () => {
    const armed: ExtrudeFsm = extrudeStep(extrudeInit(), { kind: "arm" }).state;
    expect(extrudeStep(armed, { kind: "drag", depth: 9 }).effect).toBe("none");
    const set = extrudeStep(armed, { kind: "setDepth", depth: 12 });
    expect(set.effect).toBe("update");
    expect(set.state.depth).toBe(12);
    const sym = extrudeStep(armed, { kind: "setSymmetric", symmetric: true });
    expect(sym.effect).toBe("update");
    expect(sym.state.symmetric).toBe(true);
  });

  it("setBooleanMode: NewBody clears the target; Add with an auto target arms; Add needing a pick → targetPick", () => {
    const armed: ExtrudeFsm = extrudeStep(extrudeInit(), { kind: "arm" }).state;
    // Auto-target: stays armed, target adopted.
    const auto = extrudeStep(armed, { kind: "setBooleanMode", mode: "Add", targetBodyId: "b1" });
    expect(auto.state.phase).toBe("armed");
    expect(auto.state.booleanMode).toBe("Add");
    expect(auto.state.targetBodyId).toBe("b1");
    // NewBody clears the target back out.
    const back = extrudeStep(auto.state, { kind: "setBooleanMode", mode: "NewBody" });
    expect(back.state.booleanMode).toBe("NewBody");
    expect(back.state.targetBodyId).toBeNull();
    // needsPick → targetPick (no target yet).
    const pick = extrudeStep(armed, { kind: "setBooleanMode", mode: "Cut", needsPick: true });
    expect(pick.state.phase).toBe("targetPick");
    expect(pick.state.booleanMode).toBe("Cut");
    expect(pick.state.targetBodyId).toBeNull();
  });

  it("pickTarget arms with the body; cancelTargetPick reverts to NewBody", () => {
    const targetPick = extrudeStep(
      extrudeStep(extrudeInit(), { kind: "arm" }).state,
      { kind: "setBooleanMode", mode: "Cut", needsPick: true },
    ).state;
    const picked = extrudeStep(targetPick, { kind: "pickTarget", bodyId: "b7" });
    expect(picked.state.phase).toBe("armed");
    expect(picked.state.booleanMode).toBe("Cut");
    expect(picked.state.targetBodyId).toBe("b7");

    const canceled = extrudeStep(targetPick, { kind: "cancelTargetPick" });
    expect(canceled.state.phase).toBe("armed");
    expect(canceled.state.booleanMode).toBe("NewBody");
    expect(canceled.state.targetBodyId).toBeNull();
    // pickTarget is ignored outside targetPick.
    const armed = extrudeStep(extrudeInit(), { kind: "arm" }).state;
    expect(extrudeStep(armed, { kind: "pickTarget", bodyId: "x" }).effect).toBe("none");
  });

  it("cancel from any active phase resets + emits cancel", () => {
    const dragging = extrudeStep(extrudeStep(extrudeInit(), { kind: "arm" }).state, { kind: "grab" }).state;
    const step = extrudeStep(dragging, { kind: "cancel" });
    expect(step.effect).toBe("cancel");
    expect(step.state.phase).toBe("idle");
    // Cancel while idle is a no-op.
    expect(extrudeStep(extrudeInit(), { kind: "cancel" }).effect).toBe("none");
  });
});

describe("fillet FSM", () => {
  it("arms only with ≥1 edge, then drags radius to commit", () => {
    expect(filletStep(filletInit(), { kind: "arm", edgeCount: 0 }).state.phase).toBe("idle");

    let s: FilletFsm = filletStep(filletInit(), { kind: "arm", edgeCount: 2, radius: 3 }).state;
    expect(s.phase).toBe("armed");
    expect(s.edgeCount).toBe(2);
    expect(s.radius).toBe(3);

    s = filletStep(s, { kind: "grabEdge" }).state;
    expect(s.phase).toBe("dragging");

    const dragged = filletStep(s, { kind: "drag", radius: 4.5 });
    expect(dragged.effect).toBe("update");
    expect(dragged.state.radius).toBe(4.5);

    const committed = filletStep(dragged.state, { kind: "release" });
    expect(committed.effect).toBe("commit");
    expect(committed.state.phase).toBe("committing");
  });
});

describe("revolve FSM", () => {
  it("runs arm → axisPick → pickAxis → grab → drag → release(armed) → confirm → settle", () => {
    let s: RevolveFsm = revolveInit();
    expect(s.phase).toBe("idle");
    expect(s.angle).toBe(DEFAULT_REVOLVE_ANGLE);
    expect(s.booleanMode).toBe("NewBody");

    let step = revolveStep(s, { kind: "arm" });
    expect(step.effect).toBe("none"); // no axis yet → axis-pick, no preview
    expect(step.state.phase).toBe("axisPick");
    s = step.state;

    step = revolveStep(s, { kind: "pickAxis", lineId: "L1", valid: true });
    expect(step.effect).toBe("begin"); // axis chosen → L1 lathe begins
    expect(step.state.phase).toBe("armed");
    expect(step.state.axisLineId).toBe("L1");
    s = step.state;

    s = revolveStep(s, { kind: "grab" }).state;
    expect(s.phase).toBe("dragging");

    step = revolveStep(s, { kind: "drag", angle: 90 });
    expect(step.effect).toBe("update");
    expect(step.state.angle).toBe(90);
    s = step.state;

    // Wave 1: release keeps the tool armed (no implicit commit).
    step = revolveStep(s, { kind: "release" });
    expect(step.effect).toBe("update");
    expect(step.state.phase).toBe("armed");
    expect(step.state.angle).toBe(90);
    s = step.state;

    step = revolveStep(s, { kind: "confirm" });
    expect(step.effect).toBe("commit");
    expect(step.state.phase).toBe("committing");
    s = step.state;

    expect(revolveStep(s, { kind: "settle" }).state.phase).toBe("idle");
  });

  it("rejects an invalid axis and stays in axis-pick", () => {
    const axisPick = revolveStep(revolveInit(), { kind: "arm" }).state;
    const step = revolveStep(axisPick, { kind: "pickAxis", lineId: "L9", valid: false });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("axisPick");
    expect(step.state.axisLineId).toBeNull();
  });

  it("has NO quickCommit event; confirm from armed commits at the 360° default", () => {
    const armed = revolveStep(
      revolveStep(revolveInit(), { kind: "arm" }).state,
      { kind: "pickAxis", lineId: "L1", valid: true },
    ).state;
    expect(armed.angle).toBe(360);
    // The deleted quickCommit event is unknown to the reducer — a plain click
    // now confirms via the controller's click-away path; the machine confirms
    // through `confirm`.
    const step = revolveStep(armed, { kind: "confirm" });
    expect(step.effect).toBe("commit");
    expect(step.state.phase).toBe("committing");
    expect(step.state.angle).toBe(360);
    // confirm is only legal from armed (not from axis-pick).
    const axisPick = revolveStep(revolveInit(), { kind: "arm" }).state;
    expect(revolveStep(axisPick, { kind: "confirm" }).effect).toBe("none");
  });

  it("ignores a confirm at a ~0° angle (degenerate revolve guard — finding 13)", () => {
    const armed = revolveStep(
      revolveStep(revolveInit(), { kind: "arm", angle: 360 }).state,
      { kind: "pickAxis", lineId: "L1", valid: true },
    ).state;
    // Set a sub-epsilon angle, then confirm → no transition (stays armed, no commit).
    const tiny = revolveStep(armed, { kind: "setAngle", angle: 0.3 }).state;
    const step = revolveStep(tiny, { kind: "confirm" });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("armed");
    // Below the 0.5° threshold is refused; at/above it commits (strict `< MIN`).
    expect(revolveStep(revolveStep(armed, { kind: "setAngle", angle: 0.49 }).state, { kind: "confirm" }).effect).toBe("none");
    expect(revolveStep(revolveStep(armed, { kind: "setAngle", angle: 0.5 }).state, { kind: "confirm" }).effect).toBe("commit");
  });

  it("commitFailed returns committing → armed (controller keeps the lathe preview)", () => {
    const committing = revolveStep(
      revolveStep(
        revolveStep(revolveInit(), { kind: "arm" }).state,
        { kind: "pickAxis", lineId: "L1", valid: true },
      ).state,
      { kind: "confirm" },
    ).state;
    const step = revolveStep(committing, { kind: "commitFailed" });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("armed");
  });

  it("re-edit arms straight into armed with the seeded angle (skips axis-pick)", () => {
    const step = revolveStep(revolveInit(), { kind: "arm", angle: 120, hasAxis: true, axisLineId: "L2" });
    expect(step.effect).toBe("begin");
    expect(step.state.phase).toBe("armed");
    expect(step.state.angle).toBe(120);
    expect(step.state.axisLineId).toBe("L2");
  });

  it("resetAxis returns to axis-pick, clearing the axis; setAngle works while armed", () => {
    const armed = revolveStep(
      revolveStep(revolveInit(), { kind: "arm" }).state,
      { kind: "pickAxis", lineId: "L1", valid: true },
    ).state;
    const set = revolveStep(armed, { kind: "setAngle", angle: 45 });
    expect(set.effect).toBe("update");
    expect(set.state.angle).toBe(45);

    const reset = revolveStep(set.state, { kind: "resetAxis" });
    expect(reset.state.phase).toBe("axisPick");
    expect(reset.state.axisLineId).toBeNull();
    // drag/confirm are ignored back in axis-pick.
    expect(revolveStep(reset.state, { kind: "drag", angle: 10 }).effect).toBe("none");
    expect(revolveStep(reset.state, { kind: "confirm" }).effect).toBe("none");
  });

  it("cancel from any active phase resets; idle cancel is a no-op", () => {
    const armed = revolveStep(
      revolveStep(revolveInit(), { kind: "arm" }).state,
      { kind: "pickAxis", lineId: "L1", valid: true },
    ).state;
    const step = revolveStep(armed, { kind: "cancel" });
    expect(step.effect).toBe("cancel");
    expect(step.state.phase).toBe("idle");
    expect(revolveStep(revolveInit(), { kind: "cancel" }).effect).toBe("none");
  });
});

describe("boolean FSM", () => {
  it("runs start → pickTool → setOp → apply → settle", () => {
    let s: BooleanFsm = booleanStep(booleanInit(), { kind: "start", targetBodyId: "body1" }).state;
    expect(s.phase).toBe("pickTool");
    expect(s.targetBodyId).toBe("body1");

    const picked = booleanStep(s, { kind: "pickTool", toolBodyId: "body2" });
    expect(picked.effect).toBe("ghost");
    expect(picked.state.phase).toBe("armed");
    s = picked.state;

    s = booleanStep(s, { kind: "setOp", op: "Cut" }).state;
    expect(s.op).toBe("Cut");

    const applied = booleanStep(s, { kind: "apply" });
    expect(applied.effect).toBe("commit");
    expect(applied.state.phase).toBe("committing");

    expect(booleanStep(applied.state, { kind: "settle" }).state.phase).toBe("idle");
  });

  it("ignores picking the target body as the tool body", () => {
    const s = booleanStep(booleanInit(), { kind: "start", targetBodyId: "body1" }).state;
    const step = booleanStep(s, { kind: "pickTool", toolBodyId: "body1" });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("pickTool");
  });
});

describe("shell FSM", () => {
  it("arms only with ≥1 face, then drags thickness to commit", () => {
    expect(shellStep(shellInit(), { kind: "arm", faceCount: 0 }).state.phase).toBe("idle");

    let s: ShellFsm = shellStep(shellInit(), { kind: "arm", faceCount: 2, thickness: 3 }).state;
    expect(s.phase).toBe("armed");
    expect(s.faceCount).toBe(2);
    expect(s.thickness).toBe(3);

    s = shellStep(s, { kind: "grab" }).state;
    expect(s.phase).toBe("dragging");

    const dragged = shellStep(s, { kind: "drag", thickness: 4.5 });
    expect(dragged.effect).toBe("update");
    expect(dragged.state.thickness).toBe(4.5);

    const committed = shellStep(dragged.state, { kind: "release" });
    expect(committed.effect).toBe("commit");
    expect(committed.state.phase).toBe("committing");

    expect(shellStep(committed.state, { kind: "settle" }).state.phase).toBe("idle");
  });

  it("defaults the thickness + honors setThickness while armed", () => {
    const armed = shellStep(shellInit(), { kind: "arm", faceCount: 1 }).state;
    expect(armed.thickness).toBe(DEFAULT_SHELL_THICKNESS);
    // drag is ignored unless dragging.
    expect(shellStep(armed, { kind: "drag", thickness: 9 }).effect).toBe("none");
    const set = shellStep(armed, { kind: "setThickness", thickness: 5 });
    expect(set.effect).toBe("update");
    expect(set.state.thickness).toBe(5);
  });

  it("cancel from an active phase resets; idle cancel is a no-op", () => {
    const dragging = shellStep(shellStep(shellInit(), { kind: "arm", faceCount: 1 }).state, { kind: "grab" }).state;
    expect(shellStep(dragging, { kind: "cancel" }).effect).toBe("cancel");
    expect(shellStep(dragging, { kind: "cancel" }).state.phase).toBe("idle");
    expect(shellStep(shellInit(), { kind: "cancel" }).effect).toBe("none");
  });
});

describe("linear pattern FSM", () => {
  it("arms only with a body, defaults axis X, then configures + applies", () => {
    expect(linearPatternStep(linearPatternInit(), { kind: "arm" }).state.phase).toBe("idle");

    let s: LinearPatternFsm = linearPatternStep(linearPatternInit(), { kind: "arm", bodyId: "body1" }).state;
    const armed = linearPatternStep(linearPatternInit(), { kind: "arm", bodyId: "body1" });
    expect(armed.effect).toBe("ghost");
    expect(s.phase).toBe("armed");
    expect(s.axis).toBe("X");
    expect(s.bodyId).toBe("body1");

    s = linearPatternStep(s, { kind: "setAxis", axis: "Y" }).state;
    expect(s.axis).toBe("Y");
    s = linearPatternStep(s, { kind: "setSpacing", spacing: 30 }).state;
    expect(s.spacing).toBe(30);

    const apply = linearPatternStep(s, { kind: "apply" });
    expect(apply.effect).toBe("commit");
    expect(apply.state.phase).toBe("committing");
  });

  it("clamps count to [2, 12] (integer)", () => {
    const s = linearPatternStep(linearPatternInit(), { kind: "arm", bodyId: "b" }).state;
    expect(linearPatternStep(s, { kind: "setCount", count: 1 }).state.count).toBe(2);
    expect(linearPatternStep(s, { kind: "setCount", count: 99 }).state.count).toBe(12);
    expect(linearPatternStep(s, { kind: "setCount", count: 4.7 }).state.count).toBe(5);
  });

  it("cancel resets; idle cancel is a no-op", () => {
    const s = linearPatternStep(linearPatternInit(), { kind: "arm", bodyId: "b" }).state;
    expect(linearPatternStep(s, { kind: "cancel" }).effect).toBe("cancel");
    expect(linearPatternStep(s, { kind: "cancel" }).state.phase).toBe("idle");
    expect(linearPatternStep(linearPatternInit(), { kind: "cancel" }).effect).toBe("none");
  });
});

describe("circular pattern FSM", () => {
  it("arms with a body, defaults axis Z + 360°, configures + applies", () => {
    let s: CircularPatternFsm = circularPatternStep(circularPatternInit(), { kind: "arm", bodyId: "body1" }).state;
    expect(s.phase).toBe("armed");
    expect(s.axis).toBe("Z");
    expect(s.angle).toBe(DEFAULT_REVOLVE_ANGLE); // 360

    s = circularPatternStep(s, { kind: "setAngle", angle: 180 }).state;
    expect(s.angle).toBe(180);
    // angle clamps to [1, 360].
    expect(circularPatternStep(s, { kind: "setAngle", angle: 999 }).state.angle).toBe(360);
    expect(circularPatternStep(s, { kind: "setAngle", angle: 0 }).state.angle).toBe(1);

    const apply = circularPatternStep(s, { kind: "apply" });
    expect(apply.effect).toBe("commit");
    expect(apply.state.phase).toBe("committing");
  });

  it("arm without a body stays idle", () => {
    expect(circularPatternStep(circularPatternInit(), { kind: "arm" }).state.phase).toBe("idle");
  });
});

describe("mirror FSM", () => {
  it("arms with a body, defaults plane XY, sets plane + applies", () => {
    let s: MirrorFsm = mirrorInit();
    expect(mirrorStep(s, { kind: "arm" }).state.phase).toBe("idle");

    s = mirrorStep(mirrorInit(), { kind: "arm", bodyId: "body1" }).state;
    expect(s.phase).toBe("armed");
    expect(s.plane).toBe("XY");

    const setPlane = mirrorStep(s, { kind: "setPlane", plane: "YZ" });
    expect(setPlane.effect).toBe("ghost");
    expect(setPlane.state.plane).toBe("YZ");

    const apply = mirrorStep(setPlane.state, { kind: "apply" });
    expect(apply.effect).toBe("commit");
    expect(apply.state.phase).toBe("committing");
    expect(mirrorStep(apply.state, { kind: "settle" }).state.phase).toBe("idle");
  });

  it("cancel resets; idle cancel is a no-op", () => {
    const s = mirrorStep(mirrorInit(), { kind: "arm", bodyId: "b" }).state;
    expect(mirrorStep(s, { kind: "cancel" }).effect).toBe("cancel");
    expect(mirrorStep(mirrorInit(), { kind: "cancel" }).effect).toBe("none");
  });
});

describe("regionSelect reducer (Wave 2 wiring; pure now)", () => {
  it("enter activates; toggle adds ordered + deduped, re-toggle removes", () => {
    let s = regionSelectInit();
    expect(s.active).toBe(false);
    s = regionSelectStep(s, { kind: "enter" }).state;
    expect(s.active).toBe(true);
    expect(s.selected).toEqual([]);

    s = regionSelectStep(s, { kind: "toggle", id: "r1" }).state;
    s = regionSelectStep(s, { kind: "toggle", id: "r2" }).state;
    expect(s.selected).toEqual(["r1", "r2"]); // click order preserved

    // re-toggling r1 removes it (idempotent membership, no dup).
    s = regionSelectStep(s, { kind: "toggle", id: "r1" }).state;
    expect(s.selected).toEqual(["r2"]);
    // toggling before enter is inert.
    expect(regionSelectStep(regionSelectInit(), { kind: "toggle", id: "x" }).effect).toBe("none");
  });

  it("confirm fires only with ≥1 region; empty selection is rejected", () => {
    const empty = regionSelectStep(regionSelectInit(), { kind: "enter" }).state;
    expect(regionSelectStep(empty, { kind: "confirm" }).effect).toBe("none");

    const one = regionSelectStep(empty, { kind: "toggle", id: "r1" }).state;
    const confirmed = regionSelectStep(one, { kind: "confirm" });
    expect(confirmed.effect).toBe("confirm");
    expect(confirmed.state.selected).toEqual(["r1"]); // retained for the controller
  });

  it("cancel always resets to inactive/empty", () => {
    const one = regionSelectStep(
      regionSelectStep(regionSelectInit(), { kind: "enter" }).state,
      { kind: "toggle", id: "r1" },
    ).state;
    const canceled = regionSelectStep(one, { kind: "cancel" });
    expect(canceled.effect).toBe("cancel");
    expect(canceled.state).toEqual({ active: false, selected: [] });
  });
});

// ── MODEL-OPS W1: extrude end conditions ────────────────────────────────────
describe("extrude end conditions", () => {
  const armed = () => extrudeStep(extrudeInit(), { kind: "arm" }).state;

  it("defaults to Blind with no draft and one direction", () => {
    const s = armed();
    expect(s.endCondition).toBe("Blind");
    expect(s.draftAngleDeg).toBe(0);
    expect(s.twoDirections).toBe(false);
    expect(s.targetFace).toBeNull();
  });

  it("applies a non-ToFace end condition immediately", () => {
    const r = extrudeStep(armed(), { kind: "setEndCondition", end: "ThroughAll" });
    expect(r.state.phase).toBe("armed");
    expect(r.state.endCondition).toBe("ThroughAll");
    expect(r.effect).toBe("update");
  });

  it("ToFace with no target enters facePick and emits no effect", () => {
    const r = extrudeStep(armed(), { kind: "setEndCondition", end: "ToFace" });
    expect(r.state.phase).toBe("facePick");
    expect(r.state.facePickFor).toBe(1);
    expect(r.state.endCondition).toBe("ToFace");
    expect(r.effect).toBe("none"); // nothing to preview until a face is picked
  });

  it("ToFace WITH a supplied target skips the pick (the re-edit path)", () => {
    const ref = { primary: { bodyId: "b1", kind: "face" } };
    const r = extrudeStep(armed(), { kind: "setEndCondition", end: "ToFace", targetFace: ref });
    expect(r.state.phase).toBe("armed");
    expect(r.state.targetFace).toBe(ref);
    expect(r.effect).toBe("update");
  });

  it("picking a face leaves facePick and stores the ref", () => {
    const picking = extrudeStep(armed(), { kind: "setEndCondition", end: "ToFace" }).state;
    const ref = { primary: { bodyId: "b1", elementId: "el_1", kind: "face" } };
    const r = extrudeStep(picking, { kind: "pickFace", ref });
    expect(r.state.phase).toBe("armed");
    expect(r.state.targetFace).toBe(ref);
    expect(r.state.facePickFor).toBeNull();
  });

  it("abandoning the face pick falls back to Blind, never an unreachable ToFace", () => {
    const picking = extrudeStep(armed(), { kind: "setEndCondition", end: "ToFace" }).state;
    const r = extrudeStep(picking, { kind: "cancelFacePick" });
    expect(r.state.phase).toBe("armed");
    expect(r.state.endCondition).toBe("Blind");
    expect(r.state.targetFace).toBeNull();
  });

  it("a face pick outside facePick is ignored", () => {
    const s = armed();
    expect(extrudeStep(s, { kind: "pickFace", ref: {} }).state).toEqual(s);
  });

  it("direction 2 has its own end condition and target", () => {
    let s = extrudeStep(armed(), { kind: "setTwoDirections", on: true }).state;
    s = extrudeStep(s, { kind: "setEndCondition", end: "ThroughAll", direction: 2 }).state;
    expect(s.endCondition).toBe("Blind"); // direction 1 untouched
    expect(s.endCondition2).toBe("ThroughAll");
    s = extrudeStep(s, { kind: "setEndCondition", end: "ToFace", direction: 2 }).state;
    expect(s.phase).toBe("facePick");
    expect(s.facePickFor).toBe(2);
    const ref = { primary: { bodyId: "b2", kind: "face" } };
    s = extrudeStep(s, { kind: "pickFace", ref }).state;
    expect(s.targetFace2).toBe(ref);
    expect(s.targetFace).toBeNull(); // direction 1's target is NOT clobbered
  });

  // The worker rejects the combination outright ("Symmetric is not valid with two
  // directions"), so the reducer makes it unrepresentable rather than letting a
  // doomed op reach commit.
  it("two directions and symmetric are mutually exclusive", () => {
    let s = extrudeStep(armed(), { kind: "setSymmetric", symmetric: true }).state;
    expect(s.symmetric).toBe(true);
    s = extrudeStep(s, { kind: "setTwoDirections", on: true }).state;
    expect(s.twoDirections).toBe(true);
    expect(s.symmetric).toBe(false);
    s = extrudeStep(s, { kind: "setSymmetric", symmetric: true }).state;
    expect(s.symmetric).toBe(true);
    expect(s.twoDirections).toBe(false);
  });

  it("carries a draft angle and a second distance", () => {
    let s = extrudeStep(armed(), { kind: "setDraftAngle", deg: 7 }).state;
    expect(s.draftAngleDeg).toBe(7);
    s = extrudeStep(s, { kind: "setDepth2", depth: 4 }).state;
    expect(s.depth2).toBe(4);
  });

  it("re-arming clears every end-condition field", () => {
    let s = extrudeStep(armed(), { kind: "setEndCondition", end: "ThroughAll" }).state;
    s = extrudeStep(s, { kind: "setDraftAngle", deg: 9 }).state;
    s = extrudeStep(s, { kind: "arm" }).state;
    expect(s.endCondition).toBe("Blind");
    expect(s.draftAngleDeg).toBe(0);
  });
});
