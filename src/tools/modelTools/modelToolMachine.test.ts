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
  DRAFT_ANGLE_LIMIT_DEG,
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

/**
 * What a FACE-HOSTED sketch's seed carries: the host is flush against the profile
 * on the side its face looks away from, so pushing that way starts INSIDE it.
 * `materialSeed` synthesizes exactly this rather than raycasting a coplanar face.
 */
const HOST_SIDES = {
  pos: null,
  neg: { bodyId: "host1", gap: 0, inside: true },
};


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

  it("keeps Intersect and its target through the shared target-pick path", () => {
    const armed: ExtrudeFsm = extrudeStep(extrudeInit(), { kind: "arm" }).state;
    const pick = extrudeStep(armed, { kind: "setBooleanMode", mode: "Intersect", needsPick: true });
    expect(pick.state.phase).toBe("targetPick");
    expect(pick.state.booleanMode).toBe("Intersect");

    const chosen = extrudeStep(pick.state, { kind: "pickTarget", bodyId: "b-intersect" });
    expect(chosen.state.phase).toBe("armed");
    expect(chosen.state.booleanMode).toBe("Intersect");
    expect(chosen.state.targetBodyId).toBe("b-intersect");
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

  // ── HOST-BOOLEAN: a face-hosted arm defaults to modifying its host ───────────
  //
  // The seed comes from the controller (it owns the `hostFace` lookup); the reducer
  // only carries it and applies the direction rule while the seed is untouched.

  it("arms with a boolean seed: Add on the host, target bound, auto lane open", () => {
    const step = extrudeStep(extrudeInit(), {
      kind: "arm",
      depth: 5,
      boolean: { mode: "Add", targetBodyId: "host1", auto: true, sides: HOST_SIDES },
    });
    expect(step.effect).toBe("begin");
    expect(step.state.phase).toBe("armed");
    expect(step.state.depth).toBe(5);
    expect(step.state.booleanMode).toBe("Add");
    expect(step.state.targetBodyId).toBe("host1");
    expect(step.state.booleanAuto).toBe(true);
    // An UNSEEDED arm is byte-identical to before: NewBody, no target, no auto lane.
    const plain = extrudeStep(extrudeInit(), { kind: "arm" }).state;
    expect(plain.booleanMode).toBe("NewBody");
    expect(plain.targetBodyId).toBeNull();
    expect(plain.booleanAuto).toBe(false);
  });

  it("an auto drag flips with the direction: away from the host = Add, into it = Cut", () => {
    const armed = extrudeStep(extrudeInit(), {
      kind: "arm",
      boolean: { mode: "Add", targetBodyId: "host1", auto: true, sides: HOST_SIDES },
    }).state;
    let s = extrudeStep(armed, { kind: "grab" }).state;

    s = extrudeStep(s, { kind: "drag", depth: 12 }).state;
    expect(s.booleanMode).toBe("Add"); // pulling away adds material

    s = extrudeStep(s, { kind: "drag", depth: -3 }).state;
    expect(s.booleanMode).toBe("Cut"); // pushing in removes it
    expect(s.targetBodyId).toBe("host1"); // the host stays bound across the flip

    // Zero holds the current mode — a gesture crossing zero must not blink through
    // a third state on the way past.
    s = extrudeStep(s, { kind: "drag", depth: 0 }).state;
    expect(s.booleanMode).toBe("Cut");

    s = extrudeStep(s, { kind: "drag", depth: 4 }).state;
    expect(s.booleanMode).toBe("Add"); // and back again, live
  });

  it("a SYMMETRIC drag never flips (it grows both ways — there is no direction)", () => {
    const armed = extrudeStep(extrudeInit(), {
      kind: "arm",
      boolean: { mode: "Add", targetBodyId: "host1", auto: true, sides: HOST_SIDES },
    }).state;
    const dragging = extrudeStep(armed, { kind: "grab" }).state;
    const sym = extrudeStep(dragging, { kind: "drag", depth: -9, symmetric: true });
    expect(sym.state.symmetric).toBe(true);
    expect(sym.state.booleanMode).toBe("Add");
  });

  it("an UNSEEDED (world-plane) arm never flips on a negative drag", () => {
    const dragging = extrudeStep(extrudeStep(extrudeInit(), { kind: "arm" }).state, { kind: "grab" }).state;
    expect(extrudeStep(dragging, { kind: "drag", depth: -20 }).state.booleanMode).toBe("NewBody");
  });

  it("a manual setBooleanMode kills the auto lane — the override sticks past a sign change", () => {
    const armed = extrudeStep(extrudeInit(), {
      kind: "arm",
      boolean: { mode: "Add", targetBodyId: "host1", auto: true, sides: HOST_SIDES },
    }).state;
    const manual = extrudeStep(armed, { kind: "setBooleanMode", mode: "Add", targetBodyId: "host1" });
    expect(manual.state.booleanAuto).toBe(false);

    const dragging = extrudeStep(manual.state, { kind: "grab" }).state;
    const negative = extrudeStep(dragging, { kind: "drag", depth: -30 });
    expect(negative.state.booleanMode).toBe("Add"); // NOT flipped — the user chose Add
    expect(negative.state.depth).toBe(-30);
  });

  it("a NewBody override on a host-seeded arm clears the target and the auto lane", () => {
    const armed = extrudeStep(extrudeInit(), {
      kind: "arm",
      boolean: { mode: "Add", targetBodyId: "host1", auto: true, sides: HOST_SIDES },
    }).state;
    const plain = extrudeStep(armed, { kind: "setBooleanMode", mode: "NewBody" });
    expect(plain.state.booleanMode).toBe("NewBody");
    expect(plain.state.targetBodyId).toBeNull();
    expect(plain.state.booleanAuto).toBe(false);
    // …and it stays NewBody for the rest of the session, whichever way the drag goes.
    const dragging = extrudeStep(plain.state, { kind: "grab" }).state;
    expect(extrudeStep(dragging, { kind: "drag", depth: -7 }).state.booleanMode).toBe("NewBody");
  });

  it("a targetPick override clears the auto lane too (mode chosen, target still open)", () => {
    const armed = extrudeStep(extrudeInit(), {
      kind: "arm",
      boolean: { mode: "Add", targetBodyId: "host1", auto: true, sides: HOST_SIDES },
    }).state;
    const pick = extrudeStep(armed, { kind: "setBooleanMode", mode: "Cut", needsPick: true });
    expect(pick.state.phase).toBe("targetPick");
    expect(pick.state.booleanAuto).toBe(false);
    expect(pick.state.targetBodyId).toBeNull();
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
  // EDGE-OP PREVIEW wave: release no longer commits. The three assertions below
  // that changed are the whole behaviour change — a released drag stays ARMED (so
  // the kernel preview stays live and the chip stays editable), `confirm` is the
  // only path to `committing`, and `commitFailed` returns to armed with the user's
  // radius + edge selection intact.
  it("arms only with ≥1 edge, then drags radius; release stays ARMED", () => {
    expect(filletStep(filletInit(), { kind: "arm", edgeCount: 0 }).state.phase).toBe("idle");

    let s: FilletFsm = filletStep(filletInit(), { kind: "arm", edgeCount: 2, radius: 3 }).state;
    expect(s.phase).toBe("armed");
    expect(s.edgeCount).toBe(2);
    expect(s.radius).toBe(3);
    expect(s.edgeOp).toBe("Fillet"); // the default arm
    expect(s.auto).toBe(false);
    expect(s.touched).toBe(false);

    s = filletStep(s, { kind: "grabEdge" }).state;
    expect(s.phase).toBe("dragging");

    const dragged = filletStep(s, { kind: "drag", signed: 4.5 });
    expect(dragged.effect).toBe("update");
    expect(dragged.state.radius).toBe(4.5);

    const released = filletStep(dragged.state, { kind: "release" });
    expect(released.effect).toBe("update"); // flush the final radius, do NOT commit
    expect(released.state.phase).toBe("armed");
    expect(released.state.radius).toBe(4.5); // the dragged size survives
  });

  it("confirm is the only commit path, and a failed commit returns to armed", () => {
    const armed = filletStep(filletInit(), { kind: "arm", edgeCount: 1, radius: 2 }).state;

    const committing = filletStep(armed, { kind: "confirm" });
    expect(committing.effect).toBe("commit");
    expect(committing.state.phase).toBe("committing");

    // A second confirm while committing is inert (no double-write).
    expect(filletStep(committing.state, { kind: "confirm" }).effect).toBe("none");
    // Confirm from a DRAG is inert too — the release must land first.
    const dragging = filletStep(armed, { kind: "grabEdge" }).state;
    expect(filletStep(dragging, { kind: "confirm" }).effect).toBe("none");

    const failed = filletStep(committing.state, { kind: "commitFailed" });
    expect(failed.effect).toBe("none");
    expect(failed.state.phase).toBe("armed");
    expect(failed.state.radius).toBe(2); // work preserved for the retry
    expect(failed.state.edgeCount).toBe(1);
    // commitFailed outside `committing` is inert.
    expect(filletStep(armed, { kind: "commitFailed" }).state.phase).toBe("armed");

    expect(filletStep(committing.state, { kind: "settle" }).state.phase).toBe("idle");
  });

  // ── FILLET-CHAMFER-UNIFY: the drag DIRECTION types the op ───────────────────
  //
  // Away from the body (positive) rounds it, into it (negative) bevels. Every
  // boundary below is load-bearing: the flip band is deliberately WIDER than the
  // magnitude clamp, so a gesture parked at the clamped floor cannot ping-pong the
  // op type — and each flip tears down and reopens the kernel preview session.

  const autoArm = (radius = 2): FilletFsm =>
    filletStep(filletInit(), { kind: "arm", edgeCount: 1, radius, edgeOp: "Fillet", auto: true }).state;
  const grabbed = (s: FilletFsm): FilletFsm => filletStep(s, { kind: "grabEdge" }).state;

  it("an auto arm types the op from the drag SIGN", () => {
    const s = grabbed(autoArm());

    const away = filletStep(s, { kind: "drag", signed: 4.5 });
    expect(away.state.radius).toBe(4.5);
    expect(away.state.edgeOp).toBe("Fillet");
    expect(away.state.touched).toBe(true);

    const into = filletStep(s, { kind: "drag", signed: -3 });
    expect(into.state.radius).toBe(3); // MAGNITUDE — the sign never reaches the wire
    expect(into.state.edgeOp).toBe("Chamfer");
    expect(into.state.auto).toBe(true); // a direction flip does not lock the type
  });

  it("holds the type inside the flip band, and flips just past it", () => {
    const s = grabbed(autoArm());
    expect(filletStep(s, { kind: "drag", signed: -0.2 }).state.edgeOp).toBe("Fillet");
    expect(filletStep(s, { kind: "drag", signed: -0.3 }).state.edgeOp).toBe("Chamfer");

    // …and symmetrically on the way back: the band is hysteresis, not a dead zone.
    const chamfered = filletStep(s, { kind: "drag", signed: -0.3 }).state;
    expect(filletStep(chamfered, { kind: "drag", signed: 0.2 }).state.edgeOp).toBe("Chamfer");
    expect(filletStep(chamfered, { kind: "drag", signed: 0.3 }).state.edgeOp).toBe("Fillet");
  });

  it("clamps the magnitude to the minimum without re-typing inside the clamp", () => {
    const s = grabbed(autoArm());
    const tiny = filletStep(s, { kind: "drag", signed: 0.01 });
    expect(tiny.state.radius).toBe(0.1);
    expect(tiny.state.edgeOp).toBe("Fillet");
  });

  it("a LOCKED type projects the drag onto its own half-line (no V-bounce)", () => {
    // auto:false — the drag sizes, it never types.
    const locked = grabbed(
      filletStep(filletInit(), { kind: "arm", edgeCount: 1, radius: 2, edgeOp: "Fillet" }).state,
    );
    const backwards = filletStep(locked, { kind: "drag", signed: -5 });
    expect(backwards.state.edgeOp).toBe("Fillet");
    expect(backwards.state.radius).toBe(0.1); // bottomed out, NOT |−5| = 5
    // …and it recovers monotonically from there.
    expect(filletStep(backwards.state, { kind: "drag", signed: 3 }).state.radius).toBe(3);

    // A locked Chamfer is the mirror image: its half-line is the negative one.
    const lockedChamfer = grabbed(
      filletStep(filletInit(), { kind: "arm", edgeCount: 1, radius: 1, edgeOp: "Chamfer" }).state,
    );
    const away = filletStep(lockedChamfer, { kind: "drag", signed: 5 });
    expect(away.state.edgeOp).toBe("Chamfer");
    expect(away.state.radius).toBe(0.1);
    expect(filletStep(away.state, { kind: "drag", signed: -2 }).state.radius).toBe(2);
  });

  it("setEdgeOp locks the type for the session (a later drag cannot re-type)", () => {
    const picked = filletStep(autoArm(), { kind: "setEdgeOp", edgeOp: "Chamfer" });
    expect(picked.effect).toBe("update");
    expect(picked.state.edgeOp).toBe("Chamfer");
    expect(picked.state.auto).toBe(false);

    const dragged = filletStep(grabbed(picked.state), { kind: "drag", signed: -5 });
    expect(dragged.state.edgeOp).toBe("Chamfer"); // still the user's choice
    expect(dragged.state.radius).toBe(5);
  });

  it("setEdgeOp reseeds a PRISTINE arm's size, and preserves a touched one", () => {
    // Untouched: the picked op's own default (a chamfer distance is not a radius).
    const chamfer = filletStep(autoArm(), { kind: "setEdgeOp", edgeOp: "Chamfer" }).state;
    expect(chamfer.radius).toBe(1);
    // A segment pick authors no SIZE, so the arm is still pristine and picking back
    // reseeds again rather than stranding the fillet at the chamfer's default.
    expect(chamfer.touched).toBe(false);
    expect(filletStep(chamfer, { kind: "setEdgeOp", edgeOp: "Fillet" }).state.radius).toBe(2);

    // Touched (dragged or typed): the size is the user's and survives the swap.
    const typed = filletStep(autoArm(), { kind: "setRadius", radius: 7 }).state;
    expect(typed.touched).toBe(true);
    expect(filletStep(typed, { kind: "setEdgeOp", edgeOp: "Chamfer" }).state.radius).toBe(7);
  });

  it("a TYPED value is a magnitude and never re-types the op", () => {
    const s = filletStep(autoArm(), { kind: "setRadius", radius: -7 });
    expect(s.state.radius).toBe(7);
    expect(s.state.edgeOp).toBe("Fillet");
    expect(s.state.auto).toBe(true); // typing does not end the direction lane
    expect(filletStep(autoArm(), { kind: "setRadius", radius: 0 }).state.radius).toBe(0.1);
  });

  it("setEdgeOp is inert outside armed/dragging", () => {
    expect(filletStep(filletInit(), { kind: "setEdgeOp", edgeOp: "Chamfer" }).effect).toBe("none");
  });

  it("arm can seed `touched` — a RE-EDIT's size is already the user's", () => {
    // A fresh pick arms pristine (the reseed above is correct there)…
    expect(filletStep(filletInit(), { kind: "arm", edgeCount: 1, radius: 5 }).state.touched).toBe(
      false,
    );
    // …but a re-edit seeds the COMMITTED size, so a later segment pick must keep it
    // rather than rewriting a committed 5 mm fillet to the 1 mm chamfer default.
    const reedit = filletStep(filletInit(), {
      kind: "arm",
      edgeCount: 1,
      radius: 5,
      edgeOp: "Fillet",
      auto: false,
      touched: true,
    }).state;
    expect(reedit.touched).toBe(true);
    const flipped = filletStep(reedit, { kind: "setEdgeOp", edgeOp: "Chamfer" }).state;
    expect(flipped.edgeOp).toBe("Chamfer");
    expect(flipped.radius).toBe(5);
  });

  // ── two-distance chamfer (SCHEMA §7.3, 2026-08-03 — WP-C T2a) ──────────────

  const chamferArm = (distance2: number | null = null) =>
    filletStep(filletInit(), {
      kind: "arm",
      edgeCount: 1,
      radius: 1,
      edgeOp: "Chamfer",
      touched: true,
      distance2,
    }).state;

  it("a fresh arm is equal-leg; setDistance2 makes it asymmetric and null clears it", () => {
    expect(filletInit().distance2).toBeNull();
    const armed = chamferArm();
    expect(armed.distance2).toBeNull();

    const asym = filletStep(armed, { kind: "setDistance2", distance2: 2.5 });
    expect(asym.effect).toBe("update");
    expect(asym.state.distance2).toBe(2.5);

    // Back to `=` — the chip's equal-leg state.
    expect(filletStep(asym.state, { kind: "setDistance2", distance2: null }).state.distance2)
      .toBeNull();
  });

  it("setDistance2 refuses a non-positive / non-finite second leg (equal-leg, never 0)", () => {
    // SCHEMA §7.3 requires `distance2 > 0` when present — a zero-width bevel is
    // not a value the wire may carry, so it collapses to equal-leg.
    for (const bad of [0, -3, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(filletStep(chamferArm(2.5), { kind: "setDistance2", distance2: bad }).state.distance2)
        .toBeNull();
    }
    // …and a tiny positive one is clamped to the shared magnitude floor.
    expect(filletStep(chamferArm(), { kind: "setDistance2", distance2: 0.0001 }).state.distance2)
      .toBe(0.1);
  });

  it("distance2 is CHAMFER-only: a Fillet arm ignores it (arm seed and event)", () => {
    const filletSeeded = filletStep(filletInit(), {
      kind: "arm",
      edgeCount: 1,
      radius: 2,
      edgeOp: "Fillet",
      distance2: 2.5,
    }).state;
    // The arm seed is normalized, not gated — a Fillet is never SHOWN the field
    // and never EMITS the value (`edgeOpParams`), so the reducer keeps it simple.
    const evented = filletStep(filletSeeded, { kind: "setDistance2", distance2: 4 });
    expect(evented.effect).toBe("none");
    expect(evented.state.distance2).toBe(filletSeeded.distance2);
  });

  it("a type flip KEEPS the second leg so flipping back hands it straight back", () => {
    const asym = chamferArm(2.5);
    const toFillet = filletStep(asym, { kind: "setEdgeOp", edgeOp: "Fillet" }).state;
    expect(toFillet.edgeOp).toBe("Fillet");
    expect(toFillet.distance2).toBe(2.5); // parked, never emitted for a Fillet
    const back = filletStep(toFillet, { kind: "setEdgeOp", edgeOp: "Chamfer" }).state;
    expect(back.distance2).toBe(2.5);
  });

  it("the DRAG lane sizes d1 only — it never touches the second leg", () => {
    const dragged = filletStep(grabbed(chamferArm(2.5)), { kind: "drag", signed: -4 });
    expect(dragged.state.radius).toBe(4);
    expect(dragged.state.distance2).toBe(2.5);
  });

  it("setDistance2 is inert outside armed/dragging, and a re-arm resets to equal-leg", () => {
    expect(filletStep(filletInit(), { kind: "setDistance2", distance2: 2 }).effect).toBe("none");
    // `settle`/`cancel` go through filletInit, so the next arm starts equal-leg.
    expect(filletStep(chamferArm(2.5), { kind: "cancel" }).state.distance2).toBeNull();
  });

  // ── distance-angle chamfer (SCHEMA §7.3) ──────────────────────────────────

  const angleArm = (angleDeg: number | null = null) =>
    filletStep(filletInit(), {
      kind: "arm",
      edgeCount: 1,
      radius: 1,
      edgeOp: "Chamfer",
      touched: true,
      angleDeg,
    }).state;

  it("a fresh arm carries no angle; setAngleDeg authors one and null clears it", () => {
    expect(filletInit().angleDeg).toBeNull();
    expect(angleArm().angleDeg).toBeNull();

    const angled = filletStep(angleArm(), { kind: "setAngleDeg", angleDeg: 30 });
    expect(angled.effect).toBe("update");
    expect(angled.state.angleDeg).toBe(30);
    expect(filletStep(angled.state, { kind: "setAngleDeg", angleDeg: null }).state.angleDeg)
      .toBeNull();
  });

  it("setAngleDeg refuses anything outside (0, 180) — core rejects both endpoints", () => {
    for (const bad of [0, 180, 181, -30, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(filletStep(angleArm(30), { kind: "setAngleDeg", angleDeg: bad }).state.angleDeg)
        .toBeNull();
    }
    // No magnitude clamp: EDGE_OP_MIN_VALUE is a LENGTH floor, and a 0.05° chamfer
    // is a legal (if silly) angle the backend accepts.
    expect(filletStep(angleArm(), { kind: "setAngleDeg", angleDeg: 0.05 }).state.angleDeg)
      .toBe(0.05);
  });

  it("the two chamfer modes are MUTUALLY EXCLUSIVE — last authored wins", () => {
    // Core refuses a Chamfer carrying both by name, so the FSM can never hold both.
    const withD2 = filletStep(angleArm(), { kind: "setDistance2", distance2: 2.5 }).state;
    const nowAngled = filletStep(withD2, { kind: "setAngleDeg", angleDeg: 30 }).state;
    expect(nowAngled.angleDeg).toBe(30);
    expect(nowAngled.distance2).toBeNull();

    const backToD2 = filletStep(nowAngled, { kind: "setDistance2", distance2: 4 }).state;
    expect(backToD2.distance2).toBe(4);
    expect(backToD2.angleDeg).toBeNull();

    // CLEARING one leaves the other alone — nothing was authored.
    const clearedD2 = filletStep(backToD2, { kind: "setDistance2", distance2: null }).state;
    expect(clearedD2.distance2).toBeNull();
    expect(clearedD2.angleDeg).toBeNull();
    // …and the arm seed makes the exclusion true too (angle wins, the wire's order).
    const both = filletStep(filletInit(), {
      kind: "arm",
      edgeCount: 1,
      radius: 1,
      edgeOp: "Chamfer",
      distance2: 2.5,
      angleDeg: 30,
    }).state;
    expect(both.angleDeg).toBe(30);
    expect(both.distance2).toBeNull();
  });

  it("angleDeg is CHAMFER-only, parked across a flip, and untouched by the drag", () => {
    const filletSeeded = filletStep(filletInit(), {
      kind: "arm",
      edgeCount: 1,
      radius: 2,
      edgeOp: "Fillet",
      angleDeg: 30,
    }).state;
    const evented = filletStep(filletSeeded, { kind: "setAngleDeg", angleDeg: 45 });
    expect(evented.effect).toBe("none");
    expect(evented.state.angleDeg).toBe(filletSeeded.angleDeg);

    // A flip PARKS the value (a Fillet simply never emits it), so flipping back
    // hands it straight back — same rule the second leg follows.
    const angled = angleArm(30);
    const toFillet = filletStep(angled, { kind: "setEdgeOp", edgeOp: "Fillet" }).state;
    expect(toFillet.angleDeg).toBe(30);
    expect(filletStep(toFillet, { kind: "setEdgeOp", edgeOp: "Chamfer" }).state.angleDeg).toBe(30);

    // The drag sizes d1 alone.
    const dragged = filletStep(grabbed(angleArm(30)), { kind: "drag", signed: -4 });
    expect(dragged.state.radius).toBe(4);
    expect(dragged.state.angleDeg).toBe(30);
  });

  it("setAngleDeg is inert outside armed/dragging, and a re-arm drops the angle", () => {
    expect(filletStep(filletInit(), { kind: "setAngleDeg", angleDeg: 30 }).effect).toBe("none");
    expect(filletStep(angleArm(30), { kind: "cancel" }).state.angleDeg).toBeNull();
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

  // HOST-BOOLEAN: the revolve half seeds the mode + host target and stops there —
  // there is no direction to read from an angle sweep, so nothing ever flips.
  it("arms with a boolean seed that survives the axis pick, and never flips on a drag", () => {
    const axisPick = revolveStep(revolveInit(), {
      kind: "arm",
      boolean: { mode: "Add", targetBodyId: "host1" },
    }).state;
    expect(axisPick.phase).toBe("axisPick");
    expect(axisPick.booleanMode).toBe("Add");
    expect(axisPick.targetBodyId).toBe("host1");

    const armed = revolveStep(axisPick, { kind: "pickAxis", lineId: "L1", valid: true }).state;
    expect(armed.booleanMode).toBe("Add"); // the seed rides through the axis pick
    expect(armed.targetBodyId).toBe("host1");

    const dragged = revolveStep(revolveStep(armed, { kind: "grab" }).state, { kind: "drag", angle: -90 });
    expect(dragged.state.booleanMode).toBe("Add"); // no direction rule here
    // An UNSEEDED revolve arm is unchanged: NewBody, no target.
    const plain = revolveStep(revolveInit(), { kind: "arm" }).state;
    expect(plain.booleanMode).toBe("NewBody");
    expect(plain.targetBodyId).toBeNull();
  });

  it("keeps an Intersect target through the revolve axis and target-pick states", () => {
    const axisPick = revolveStep(revolveInit(), { kind: "arm" }).state;
    const armed = revolveStep(axisPick, { kind: "pickAxis", lineId: "L1", valid: true }).state;
    const pick = revolveStep(armed, { kind: "setBooleanMode", mode: "Intersect", needsPick: true });
    expect(pick.state.phase).toBe("targetPick");
    expect(pick.state.booleanMode).toBe("Intersect");

    const chosen = revolveStep(pick.state, { kind: "pickTarget", bodyId: "b-intersect" });
    expect(chosen.state.phase).toBe("armed");
    expect(chosen.state.booleanMode).toBe("Intersect");
    expect(chosen.state.targetBodyId).toBe("b-intersect");
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
  // Same behaviour change as the fillet FSM above (release → armed, explicit
  // confirm, commitFailed → armed) — one gesture across both value-drag tools.
  it("arms only with ≥1 face, then drags thickness; release stays ARMED", () => {
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

    const released = shellStep(dragged.state, { kind: "release" });
    expect(released.effect).toBe("update"); // flush the final thickness, no commit
    expect(released.state.phase).toBe("armed");
    expect(released.state.thickness).toBe(4.5);
  });

  it("confirm is the only commit path, and a failed commit returns to armed", () => {
    const armed = shellStep(shellInit(), { kind: "arm", faceCount: 2, thickness: 3 }).state;

    const committing = shellStep(armed, { kind: "confirm" });
    expect(committing.effect).toBe("commit");
    expect(committing.state.phase).toBe("committing");
    expect(shellStep(committing.state, { kind: "confirm" }).effect).toBe("none");
    expect(shellStep(shellStep(armed, { kind: "grab" }).state, { kind: "confirm" }).effect).toBe("none");

    const failed = shellStep(committing.state, { kind: "commitFailed" });
    expect(failed.effect).toBe("none");
    expect(failed.state.phase).toBe("armed");
    expect(failed.state.thickness).toBe(3); // work preserved for the retry
    expect(failed.state.faceCount).toBe(2);
    expect(shellStep(armed, { kind: "commitFailed" }).state.phase).toBe("armed");

    expect(shellStep(committing.state, { kind: "settle" }).state.phase).toBe("idle");
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

  /*
   * U6: ONE range across TS / Rust / worker. The authoring range is 2–128 (the
   * worker's `kMaxPatternCount`); the +/− buttons stop at 12 for ergonomics, but
   * that is a CHIP bound, not a value policy. Out of range is REFUSED, never
   * clamped — a clamp would commit a count the user never saw previewed.
   */
  it("accepts the full authoring range and refuses (never clamps) outside it", () => {
    const s = linearPatternStep(linearPatternInit(), { kind: "arm", bodyId: "b" }).state;
    expect(linearPatternStep(s, { kind: "setCount", count: 20 }).state.count).toBe(20);
    expect(linearPatternStep(s, { kind: "setCount", count: 128 }).state.count).toBe(128);
    expect(linearPatternStep(s, { kind: "setCount", count: 4.7 }).state.count).toBe(5);

    // Refused: the state is untouched and nothing previews.
    for (const count of [1, 0, 129, Number.NaN]) {
      const step = linearPatternStep(s, { kind: "setCount", count });
      expect(step.state.count, `count ${count}`).toBe(s.count);
      expect(step.effect).toBe("none");
    }
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

// ── WP-C3: the draft angle is authored, clamped, and re-armable ──────────────
describe("extrude draft angle", () => {
  const armed = () => extrudeStep(extrudeInit(), { kind: "arm" }).state;

  it("a setDraftAngle asks for a preview update (the drafted prism must re-render)", () => {
    const r = extrudeStep(armed(), { kind: "setDraftAngle", deg: 10 });
    expect(r.state.draftAngleDeg).toBe(10);
    expect(r.effect).toBe("update");
  });

  // Oracle: EditParameterDialog.cpp kMinDraft/kMaxDraft = ∓89 — ±90° would make
  // the side face parallel to the extrude direction (a degenerate prism).
  it("clamps into the legacy ±89° range instead of authoring nonsense", () => {
    expect(extrudeStep(armed(), { kind: "setDraftAngle", deg: 120 }).state.draftAngleDeg).toBe(
      DRAFT_ANGLE_LIMIT_DEG,
    );
    expect(extrudeStep(armed(), { kind: "setDraftAngle", deg: -120 }).state.draftAngleDeg).toBe(
      -DRAFT_ANGLE_LIMIT_DEG,
    );
    expect(extrudeStep(armed(), { kind: "setDraftAngle", deg: -12 }).state.draftAngleDeg).toBe(-12);
  });

  it("a non-finite entry falls back to 0 rather than poisoning the params", () => {
    const s = extrudeStep(armed(), { kind: "setDraftAngle", deg: 8 }).state;
    expect(extrudeStep(s, { kind: "setDraftAngle", deg: Number.NaN }).state.draftAngleDeg).toBe(0);
  });

  it("is ignored outside armed/dragging", () => {
    const idle = extrudeInit();
    expect(extrudeStep(idle, { kind: "setDraftAngle", deg: 10 }).state).toEqual(idle);
    const committing = extrudeStep(armed(), { kind: "confirm" }).state;
    expect(extrudeStep(committing, { kind: "setDraftAngle", deg: 10 }).state.draftAngleDeg).toBe(0);
  });

  // The re-edit commit writes the ARMED draft back, so an arm that ignored the
  // stored value would silently zero a drafted feature on its first ✓.
  it("arm seeds the stored draft (re-edit) and clamps it too", () => {
    expect(extrudeStep(extrudeInit(), { kind: "arm", draft: 7 }).state.draftAngleDeg).toBe(7);
    expect(extrudeStep(extrudeInit(), { kind: "arm", draft: 400 }).state.draftAngleDeg).toBe(
      DRAFT_ANGLE_LIMIT_DEG,
    );
    expect(extrudeStep(extrudeInit(), { kind: "arm" }).state.draftAngleDeg).toBe(0);
  });
});
