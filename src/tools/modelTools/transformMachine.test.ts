/*
 * `transformStep` — the placement FSM (WP-B W1).
 *
 * The subtle contract is that the state is the record's CUMULATIVE canonical
 * form, not a per-gesture delta, and the chip's number is a VIEW of the one
 * component the current mode+axis addresses. These specs pin the consequences:
 * an axis switch reveals a different stored component instead of re-writing the
 * old one, a move never disturbs the stored rotation (and vice versa), and the
 * frozen pivot survives every event.
 */
import { describe, it, expect } from "vitest";
import {
  transformInit,
  transformStep,
  transformValue,
  transformParamsOf,
  isIdentityPlacement,
  axisIndex,
  dominantAxis,
  type TransformFsm,
} from "./modelToolMachine";
import type { Vec3 } from "@/tools/preview/depthProjection";

const armed = (): TransformFsm =>
  transformStep(transformInit(), { kind: "arm", targets: ["b1"], center: [5, 6, 7] }).state;

const step = (s: TransformFsm, ...events: Parameters<typeof transformStep>[1][]): TransformFsm =>
  events.reduce((acc, e) => transformStep(acc, e).state, s);

describe("transformStep — arm", () => {
  it("arms on a non-empty target list and asks for a ghost", () => {
    const r = transformStep(transformInit(), { kind: "arm", targets: ["b1", "b2"] });
    expect(r.state.phase).toBe("armed");
    expect(r.state.targets).toEqual(["b1", "b2"]);
    expect(r.effect).toBe("ghost");
  });

  it("refuses to arm with no targets", () => {
    const r = transformStep(transformInit(), { kind: "arm", targets: [] });
    expect(r.state.phase).toBe("idle");
    expect(r.effect).toBe("none");
  });

  it("defaults to Move · X · zero placement", () => {
    const s = armed();
    expect(s.mode).toBe("move");
    expect(s.axis).toBe("X");
    expect(transformValue(s)).toBe(0);
    expect(isIdentityPlacement(s)).toBe(true);
  });

  it("seeds the FULL stored shape at arm (fold / re-edit)", () => {
    const s = transformStep(transformInit(), {
      kind: "arm",
      targets: ["b1"],
      mode: "rotate",
      axis: "Y",
      translate: [30, -4, 0],
      angleDeg: 45,
      rotAxis: "Y",
      center: [1, 2, 3],
      copy: true,
    }).state;
    expect(s.translate).toEqual([30, -4, 0]);
    expect(s.angleDeg).toBe(45);
    expect(s.center).toEqual([1, 2, 3]);
    expect(s.copy).toBe(true);
    expect(transformValue(s)).toBe(45);
  });

  it("copies the seeded vectors rather than aliasing the caller's arrays", () => {
    const center: [number, number, number] = [1, 2, 3];
    const s = transformStep(transformInit(), { kind: "arm", targets: ["b1"], center }).state;
    center[0] = 999;
    expect(s.center[0]).toBe(1);
  });
});

describe("transformStep — chip edits", () => {
  it("setValue writes the component the current mode+axis addresses", () => {
    const s = step(armed(), { kind: "setValue", value: 30 });
    expect(s.translate).toEqual([30, 0, 0]);
    expect(s.angleDeg).toBe(0);
  });

  it("an axis switch REVEALS the other stored component, it does not move the value", () => {
    const s = step(
      armed(),
      { kind: "setValue", value: 30 }, // X = 30
      { kind: "setAxis", axis: "Y" },
    );
    expect(transformValue(s)).toBe(0); // Y is still untouched
    expect(s.translate).toEqual([30, 0, 0]); // …and X survived the switch
    const both = step(s, { kind: "setValue", value: 12 });
    expect(both.translate).toEqual([30, 12, 0]);
  });

  it("a rotation leaves the stored translation alone, and vice versa", () => {
    const s = step(
      armed(),
      { kind: "setValue", value: 30 },
      { kind: "setMode", mode: "rotate" },
      { kind: "setAxis", axis: "Z" },
      { kind: "setValue", value: 90 },
    );
    expect(s.translate).toEqual([30, 0, 0]);
    expect(s.angleDeg).toBe(90);
    expect(s.rotAxis).toBe("Z");
    const back = step(s, { kind: "setMode", mode: "move" }, { kind: "setAxis", axis: "X" });
    expect(transformValue(back)).toBe(30);
    expect(back.angleDeg).toBe(90);
  });

  it("typing an angle CLAIMS the axis — the record carries one rotation", () => {
    const s = step(
      armed(),
      { kind: "setMode", mode: "rotate" },
      { kind: "setValue", value: 15 }, // about the default X
      { kind: "setAxis", axis: "Y" },
    );
    expect(transformValue(s)).toBe(0); // Y is not the stored rotation axis yet
    const claimed = step(s, { kind: "setValue", value: 90 });
    expect(claimed.rotAxis).toBe("Y");
    expect(claimed.angleDeg).toBe(90);
  });

  it("a non-finite typed value falls back to 0 rather than poisoning the record", () => {
    const s = step(armed(), { kind: "setValue", value: Number.NaN });
    expect(s.translate).toEqual([0, 0, 0]);
  });

  it("the frozen pivot survives every chip edit", () => {
    const s = step(
      armed(),
      { kind: "setValue", value: 30 },
      { kind: "setMode", mode: "rotate" },
      { kind: "setValue", value: 90 },
    );
    expect(s.center).toEqual([5, 6, 7]);
  });

  it("ignores chip edits while idle", () => {
    const r = transformStep(transformInit(), { kind: "setValue", value: 30 });
    expect(r.effect).toBe("none");
    expect(r.state.translate).toEqual([0, 0, 0]);
  });

  it("`seed` re-seeds an ALREADY-armed placement (async re-edit params)", () => {
    const s = transformStep(armed(), {
      kind: "seed",
      translate: [0, 50, 0],
      axis: "Y",
      center: [9, 9, 9],
    }).state;
    expect(s.translate).toEqual([0, 50, 0]);
    expect(s.center).toEqual([9, 9, 9]);
    expect(transformValue(s)).toBe(50);
    expect(s.targets).toEqual(["b1"]); // the arm's targets are untouched
  });
});

describe("transformStep — lifecycle", () => {
  it("apply moves to committing and emits commit", () => {
    const r = transformStep(armed(), { kind: "apply" });
    expect(r.state.phase).toBe("committing");
    expect(r.effect).toBe("commit");
  });

  it("cancel from armed resets and emits cancel; from idle it is inert", () => {
    expect(transformStep(armed(), { kind: "cancel" }).effect).toBe("cancel");
    expect(transformStep(transformInit(), { kind: "cancel" }).effect).toBe("none");
  });
});

describe("transformParamsOf — the canonical record shape", () => {
  it("a pure move carries a zero rotation about the FROZEN centre", () => {
    const p = transformParamsOf(step(armed(), { kind: "setValue", value: 30 }));
    expect(p).toEqual({
      targets: ["b1"],
      translate: [30, 0, 0],
      // The pivot is stored even though a translation does not use it — the next
      // re-edit must recompose against the SAME point.
      rotate: { center: [5, 6, 7], axis: [0, 0, 1], angleDeg: 0 },
      copy: false,
    });
  });

  it("a pure rotate carries a zero translation and the picked world axis", () => {
    const p = transformParamsOf(
      step(armed(), { kind: "setMode", mode: "rotate" }, { kind: "setAxis", axis: "Y" }, { kind: "setValue", value: 90 }),
    );
    expect(p.translate).toEqual([0, 0, 0]);
    expect(p.rotate).toEqual({ center: [5, 6, 7], axis: [0, 1, 0], angleDeg: 90 });
  });

  it("a combined placement carries BOTH halves in one record", () => {
    const p = transformParamsOf(
      step(
        armed(),
        { kind: "setValue", value: 30 },
        { kind: "setAxis", axis: "Z" },
        { kind: "setValue", value: -5 },
        { kind: "setMode", mode: "rotate" },
        { kind: "setValue", value: 45 },
      ),
    );
    expect(p.translate).toEqual([30, 0, -5]);
    expect(p.rotate.angleDeg).toBe(45);
    expect(p.rotate.axis).toEqual([0, 0, 1]);
  });

  it("mirrors targets in order and does not alias the FSM's arrays", () => {
    const s = transformStep(transformInit(), { kind: "arm", targets: ["b2", "b1"] }).state;
    const p = transformParamsOf(s);
    expect(p.targets).toEqual(["b2", "b1"]);
    p.targets.push("b3");
    expect(s.targets).toEqual(["b2", "b1"]);
  });
});

describe("axisIndex / isIdentityPlacement", () => {
  it("maps the three world axes to their vector slots", () => {
    expect([axisIndex("X"), axisIndex("Y"), axisIndex("Z")]).toEqual([0, 1, 2]);
  });

  it("identity is zero translation AND zero angle", () => {
    expect(isIdentityPlacement(armed())).toBe(true);
    expect(isIdentityPlacement(step(armed(), { kind: "setValue", value: 1 }))).toBe(false);
    expect(
      isIdentityPlacement(step(armed(), { kind: "setMode", mode: "rotate" }, { kind: "setValue", value: 1 })),
    ).toBe(false);
  });
});

/*
 * Gizmo events (WP-B W2). The gizmo is the chip's PEER, not a second authority:
 * every grab and drag frame lands as an FSM event, so the chip segments re-render
 * from the same state and the two surfaces cannot drift apart.
 */
describe("transformStep — gizmo grab", () => {
  it("a ring grab re-types the placement to Rotate about that axis", () => {
    const s = step(armed(), { kind: "grab", grab: { kind: "ring", axis: "Y" } });
    expect(s.mode).toBe("rotate");
    expect(s.axis).toBe("Y");
  });

  it("an arrow grab re-types it to Move along that axis", () => {
    const s = step(
      armed(),
      { kind: "setMode", mode: "rotate" },
      { kind: "grab", grab: { kind: "axis", axis: "Z" } },
    );
    expect(s.mode).toBe("move");
    expect(s.axis).toBe("Z");
  });

  it("a plane grab KEEPS an already-selected in-plane axis (the standing choice)", () => {
    const s = step(
      armed(),
      { kind: "setAxis", axis: "Y" },
      { kind: "grab", grab: { kind: "plane", axis: "Z" } }, // the Z quad moves X+Y
    );
    expect(s.axis).toBe("Y");
  });

  it("a plane grab NEVER leaves the chip on the quad's own normal", () => {
    // The X quad cannot move X at all, so showing dx would peg the chip.
    const s = step(
      armed(),
      { kind: "setAxis", axis: "X" },
      { kind: "grab", grab: { kind: "plane", axis: "X" } },
    );
    expect(s.axis).toBe("Y");
  });

  it("Alt at grab sets copy; a plain grab leaves it exactly as it was", () => {
    expect(step(armed(), { kind: "grab", grab: { kind: "axis", axis: "X" }, copy: true }).copy).toBe(true);
    const sticky = step(
      armed(),
      { kind: "setCopy", copy: true },
      { kind: "grab", grab: { kind: "axis", axis: "X" } },
    );
    expect(sticky.copy).toBe(true);
  });

  it("a grab never disturbs the placement values or the frozen pivot", () => {
    const s = step(
      armed(),
      { kind: "setValue", value: 30 },
      { kind: "grab", grab: { kind: "ring", axis: "X" } },
    );
    expect(s.translate).toEqual([30, 0, 0]);
    expect(s.center).toEqual([5, 6, 7]);
  });
});

describe("transformStep — gizmo drag frames", () => {
  it("dragTranslate writes the WHOLE vector (a plane drag moves two axes at once)", () => {
    const s = step(armed(), { kind: "dragTranslate", translate: [12, -7, 0] });
    expect(s.translate).toEqual([12, -7, 0]);
    expect(transformValue(s)).toBe(12); // the chip still shows the addressed one
  });

  it("dragAngle CLAIMS the current axis as the rotation axis", () => {
    const s = step(
      armed(),
      { kind: "grab", grab: { kind: "ring", axis: "Y" } },
      { kind: "dragAngle", angleDeg: 45 },
    );
    expect(s.angleDeg).toBe(45);
    expect(s.rotAxis).toBe("Y");
    expect(transformParamsOf(s).rotate.axis).toEqual([0, 1, 0]);
  });

  it("a translate drag leaves the stored rotation alone, and vice versa", () => {
    const s = step(
      armed(),
      { kind: "grab", grab: { kind: "ring", axis: "Z" } },
      { kind: "dragAngle", angleDeg: 90 },
      { kind: "grab", grab: { kind: "axis", axis: "X" } },
      { kind: "dragTranslate", translate: [40, 0, 0] },
    );
    expect(s.angleDeg).toBe(90);
    expect(s.translate).toEqual([40, 0, 0]);
  });

  it("REFUSES a non-finite drag frame rather than poisoning the placement", () => {
    const base = step(armed(), { kind: "setValue", value: 30 });
    expect(step(base, { kind: "dragTranslate", translate: [NaN, 0, 0] }).translate).toEqual([30, 0, 0]);
    expect(step(base, { kind: "dragAngle", angleDeg: Infinity }).angleDeg).toBe(0);
  });

  it("ignores every gizmo event while idle", () => {
    const idle = transformInit();
    const events: Parameters<typeof transformStep>[1][] = [
      { kind: "grab", grab: { kind: "ring", axis: "X" } },
      { kind: "dragTranslate", translate: [1, 2, 3] },
      { kind: "dragAngle", angleDeg: 10 },
      { kind: "setCopy", copy: true },
    ];
    for (const e of events) {
      const r = transformStep(idle, e);
      expect(r.state).toEqual(idle);
      expect(r.effect).toBe("none");
    }
  });
});

// ── ALIGN sub-flow (WP-B W2.5) ───────────────────────────────────────────────
//
// The align is a PHASE OF THE ARMED PLACEMENT, not a tool. What has to hold:
// the two picks advance in order and nothing else, Esc walks BACK one rung at a
// time without disarming, and `alignApply` REPLACES the placement — including
// with a rotation axis the three-letter chip cannot express, which must then
// survive out to `transformParamsOf` verbatim.

describe("transformStep — align", () => {
  const alignApply = (axis: Vec3, angleDeg = 90, translate: Vec3 = [110, -60, 0]) =>
    ({ kind: "alignApply", translate, angleDeg, rotAxisVec: axis }) as const;

  const inAlign = (): TransformFsm => step(armed(), { kind: "beginAlign" });
  const atDest = (): TransformFsm => step(inAlign(), { kind: "alignPickedMoving" });

  it("beginAlign waits for the FIRST pick, and only while armed", () => {
    const s = transformStep(armed(), { kind: "beginAlign" });
    expect(s.state.alignPhase).toBe("pickMoving");
    expect(s.effect).toBe("ghost");
    expect(transformStep(transformInit(), { kind: "beginAlign" }).state.alignPhase).toBeNull();
  });

  it("advances pickMoving → pickDest, and refuses an out-of-order advance", () => {
    expect(atDest().alignPhase).toBe("pickDest");
    // A second "moving" pick while waiting for the destination is not a thing.
    expect(step(atDest(), { kind: "alignPickedMoving" }).alignPhase).toBe("pickDest");
    expect(step(armed(), { kind: "alignPickedMoving" }).alignPhase).toBeNull();
  });

  it("leaves the arm, the targets and the FROZEN pivot untouched across the flow", () => {
    const s = atDest();
    expect(s.phase).toBe("armed");
    expect(s.targets).toEqual(["b1"]);
    expect(s.center).toEqual([5, 6, 7]);
  });

  it("Esc walks BACK one rung: pickDest → pickMoving → off, arm intact", () => {
    const back1 = step(atDest(), { kind: "alignCancel" });
    expect(back1.alignPhase).toBe("pickMoving");
    const back2 = step(back1, { kind: "alignCancel" });
    expect(back2.alignPhase).toBeNull();
    expect(back2.phase).toBe("armed");
    // A third Esc is a no-op HERE — the tool-level cancel owns that rung.
    const r = transformStep(back2, { kind: "alignCancel" });
    expect(r.state).toEqual(back2);
    expect(r.effect).toBe("none");
  });

  it("alignApply REPLACES the placement and reads out as Move · dominant axis", () => {
    const s = step(atDest(), alignApply([0, 0, 1]));
    expect(s.alignPhase).toBeNull();
    expect(s.translate).toEqual([110, -60, 0]);
    expect(s.angleDeg).toBe(90);
    expect(s.mode).toBe("move");
    expect(s.axis).toBe("X"); // |110| > |−60| > 0
    expect(transformValue(s)).toBe(110); // the chip shows the component it named
  });

  it("REPLACES rather than composes: an earlier nudge does not survive", () => {
    const nudged = step(armed(), { kind: "setValue", value: 30 }, { kind: "beginAlign" }, { kind: "alignPickedMoving" });
    expect(step(nudged, alignApply([0, 0, 1])).translate).toEqual([110, -60, 0]);
  });

  it("carries the FREE axis out to the record verbatim", () => {
    const axis: Vec3 = [0.6, 0, 0.8];
    const s = step(atDest(), alignApply(axis, 37.5));
    expect(s.rotAxisVec).toEqual(axis);
    const p = transformParamsOf(s);
    expect(p.rotate.axis).toEqual(axis); // NOT snapped to the nearest world axis
    expect(p.rotate.angleDeg).toBe(37.5);
    expect(p.rotate.center).toEqual([5, 6, 7]);
  });

  it("reads 0° on every world axis while a free axis is stored (nothing to rewrite)", () => {
    const s = step(atDest(), alignApply([0.6, 0, 0.8], 37.5), { kind: "setMode", mode: "rotate" });
    for (const axis of ["X", "Y", "Z"] as const) {
      expect(transformValue(step(s, { kind: "setAxis", axis }))).toBe(0);
    }
  });

  it("authoring a rotation CLAIMS a world axis and drops the solved one", () => {
    const aligned = step(atDest(), alignApply([0.6, 0, 0.8], 37.5), { kind: "setMode", mode: "rotate" });
    const typed = step(aligned, { kind: "setAxis", axis: "Y" }, { kind: "setValue", value: 15 });
    expect(typed.rotAxisVec).toBeNull();
    expect(typed.rotAxis).toBe("Y");
    expect(transformParamsOf(typed).rotate.axis).toEqual([0, 1, 0]);
    // …and so does a ring drag, which is the same authorship by another gesture.
    const dragged = step(aligned, { kind: "grab", grab: { kind: "ring", axis: "X" } }, { kind: "dragAngle", angleDeg: 20 });
    expect(dragged.rotAxisVec).toBeNull();
    expect(transformParamsOf(dragged).rotate.axis).toEqual([1, 0, 0]);
  });

  it("a MOVE never disturbs the solved rotation", () => {
    const aligned = step(atDest(), alignApply([0.6, 0, 0.8], 37.5));
    const moved = step(aligned, { kind: "setAxis", axis: "Z" }, { kind: "setValue", value: 9 });
    expect(moved.rotAxisVec).toEqual([0.6, 0, 0.8]);
    expect(moved.angleDeg).toBe(37.5);
    expect(moved.translate).toEqual([110, -60, 9]);
  });

  it("REFUSES a non-finite solve rather than poisoning the placement", () => {
    const base = atDest();
    for (const bad of [
      alignApply([0, 0, 1], NaN),
      alignApply([NaN, 0, 1]),
      alignApply([0, 0, 1], 0, [Infinity, 0, 0]),
    ]) {
      const r = transformStep(base, bad);
      expect(r.state).toEqual(base);
      expect(r.effect).toBe("none");
      expect(r.state.alignPhase).toBe("pickDest"); // still waiting — nothing lost
    }
  });

  it("ignores alignApply when no destination pick is outstanding", () => {
    expect(transformStep(armed(), alignApply([0, 0, 1])).state.rotAxisVec).toBeNull();
    expect(transformStep(inAlign(), alignApply([0, 0, 1])).state.alignPhase).toBe("pickMoving");
  });

  it("a seed can CLEAR a free axis back to the world one (an explicit null)", () => {
    const aligned = step(atDest(), alignApply([0.6, 0, 0.8], 37.5));
    expect(step(aligned, { kind: "seed", rotAxisVec: null }).rotAxisVec).toBeNull();
    expect(step(aligned, { kind: "seed", angleDeg: 1 }).rotAxisVec).toEqual([0.6, 0, 0.8]); // absent ⇒ kept
  });

  it("cancel clears the whole flow along with the placement", () => {
    expect(transformStep(atDest(), { kind: "cancel" }).state).toEqual(transformInit());
  });
});

describe("dominantAxis", () => {
  it("names the largest |component|, ties to the lowest index", () => {
    expect(dominantAxis([110, -60, 0])).toBe("X");
    expect(dominantAxis([0, -60, 5])).toBe("Y");
    expect(dominantAxis([-1, 2, -3])).toBe("Z");
    expect(dominantAxis([0, 0, 0])).toBe("X");
    expect(dominantAxis([5, 5, 5])).toBe("X");
    expect(dominantAxis([0, 5, 5])).toBe("Y");
  });
});
