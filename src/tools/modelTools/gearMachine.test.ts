/**
 * `gearMachine` — the Gear tool's pure reducer (Gear Generator G1).
 *
 * The reducer is pure, so these are plain `(state, event) → state` assertions.
 * What is worth testing here is not the arithmetic but the CONTRACTS the rest of
 * the stack depends on:
 *
 *   - the two placement modes stay mutually exclusive, because the wire, the
 *     Rust session and the worker all reject a payload carrying both;
 *   - `gearParamsOf` emits only the active recipe block and spells the rest
 *     `null`, and gates bore dimensions on their own flag;
 *   - params are only editable while armed, so a stray chip event after a commit
 *     cannot mutate a record that is already on its way to the kernel;
 *   - a re-edit seed round-trips a stored record.
 */

import { describe, expect, it } from "vitest";

import {
  DEFAULT_GEAR_AXLE_HOLE_DIAMETER,
  DEFAULT_GEAR_MODULE,
  DEFAULT_GEAR_TEETH,
  GEAR_MAX_HEIGHT,
  GEAR_MAX_SAMPLE_COUNT,
  GEAR_MAX_TEETH,
  GEAR_MIN_TEETH,
  gearFsmFromParams,
  gearInit,
  gearParamsOf,
  gearStep,
  type GearEvent,
  type GearFsm,
} from "./gearMachine";

const FACE = { primary: { bodyId: "body_1", elementId: "el_top", kind: "face" } };
const POINT: [number, number, number] = [10, 10, 25];

function run(events: GearEvent[], from: GearFsm = gearInit()): GearFsm {
  return events.reduce((s, e) => gearStep(s, e).state, from);
}

/** An armed gear seated on a picked face. */
function armedOnFace(): GearFsm {
  return run([{ kind: "start" }, { kind: "pickFace", face: FACE, point: POINT }]);
}

/** An armed gear on an explicit frozen frame. */
function armedOnFrame(): GearFsm {
  return run([
    { kind: "start" },
    { kind: "pickFrame", origin: [0, 0, 0], axis: [0, 0, 1], xDir: [1, 0, 0] },
  ]);
}

describe("gearMachine phases", () => {
  it("starts idle and enters facePick on start", () => {
    expect(gearInit().phase).toBe("idle");
    expect(gearStep(gearInit(), { kind: "start" }).state.phase).toBe("facePick");
  });

  it("arms on a face pick and asks for a preview session", () => {
    const step = gearStep(gearStep(gearInit(), { kind: "start" }).state, {
      kind: "pickFace",
      face: FACE,
      point: POINT,
    });
    expect(step.state.phase).toBe("armed");
    expect(step.state.point).toEqual(POINT);
    expect(step.effect).toBe("begin");
  });

  it("arms on a frame pick, pinning the point to the frame origin", () => {
    // SCHEMA §7.3 requires point == frame.origin; the reducer maintains it
    // rather than leaving two positions to drift apart.
    const s = armedOnFrame();
    expect(s.phase).toBe("armed");
    expect(s.point).toEqual([0, 0, 0]);
    expect(s.frame?.origin).toEqual([0, 0, 0]);
  });

  it("commits only from armed with a placement", () => {
    expect(gearStep(gearInit(), { kind: "confirm" }).effect).toBe("none");
    // facePick has no placement yet.
    const picking = gearStep(gearInit(), { kind: "start" }).state;
    expect(gearStep(picking, { kind: "confirm" }).effect).toBe("none");

    const armed = armedOnFace();
    const step = gearStep(armed, { kind: "confirm" });
    expect(step.state.phase).toBe("committing");
    expect(step.effect).toBe("commit");
  });

  it("returns to armed with everything intact when the kernel refuses", () => {
    const committing = run([{ kind: "setTeeth", teeth: 33 }, { kind: "confirm" }], armedOnFace());
    expect(committing.phase).toBe("committing");
    const back = gearStep(committing, { kind: "commitFailed" }).state;
    expect(back.phase).toBe("armed");
    expect(back.teeth).toBe(33);
    expect(back.point).toEqual(POINT);
  });

  it("cancel resets to a pristine idle state", () => {
    const step = gearStep(armedOnFace(), { kind: "cancel" });
    expect(step.state).toEqual(gearInit());
    expect(step.effect).toBe("cancel");
  });
});

describe("gearMachine placement modes are mutually exclusive", () => {
  it("picking a frame clears a previously picked face", () => {
    const s = run(
      [{ kind: "pickFrame", origin: [1, 2, 3], axis: [0, 0, 1], xDir: [1, 0, 0] }],
      armedOnFace(),
    );
    expect(s.face).toBeNull();
    expect(s.frame).not.toBeNull();
  });

  it("picking a face clears a previously picked frame", () => {
    const s = run([{ kind: "pickFace", face: FACE, point: POINT }], armedOnFrame());
    expect(s.frame).toBeNull();
    expect(s.face).not.toBeNull();
  });

  it("moving a framed gear keeps its origin equal to its point", () => {
    const s = run([{ kind: "movePoint", point: [4, 5, 6] }], armedOnFrame());
    expect(s.point).toEqual([4, 5, 6]);
    expect(s.frame?.origin).toEqual([4, 5, 6]);
  });
});

describe("gearMachine editing guards", () => {
  it("ignores parameter events outside the armed phase", () => {
    // A chip event arriving after confirm must not mutate a record already on
    // its way to the kernel.
    const committing = gearStep(armedOnFace(), { kind: "confirm" }).state;
    const after = gearStep(committing, { kind: "setTeeth", teeth: 99 });
    expect(after.state.teeth).toBe(DEFAULT_GEAR_TEETH);
    expect(after.effect).toBe("none");
  });

  it("clamps teeth to the minimum a real involute gear allows", () => {
    expect(run([{ kind: "setTeeth", teeth: 1 }], armedOnFace()).teeth).toBe(GEAR_MIN_TEETH);
    expect(run([{ kind: "setTeeth", teeth: 25.6 }], armedOnFace()).teeth).toBe(26);
  });

  it("keeps a non-finite dimension out of the record", () => {
    const s = run([{ kind: "setModule", module: Number.NaN }], armedOnFace());
    expect(s.module).toBe(DEFAULT_GEAR_MODULE);
  });

  it("allows a NEGATIVE profile shift but not a negative clearance", () => {
    // Shift is a coefficient and negative shift is a real gear; clearance is a
    // depth coefficient and cannot be negative.
    expect(run([{ kind: "setShift", shift: -0.4 }], armedOnFace()).shift).toBeCloseTo(-0.4);
    expect(run([{ kind: "setClearance", clearance: -1 }], armedOnFace()).clearance).toBe(0);
  });

  it("keeps the pressure angle inside the worker's open domain", () => {
    expect(run([{ kind: "setPressureAngle", angleDeg: 0 }], armedOnFace()).pressureAngleDeg)
      .toBeGreaterThan(0);
    expect(run([{ kind: "setPressureAngle", angleDeg: 120 }], armedOnFace()).pressureAngleDeg)
      .toBeLessThan(90);
  });

  it("keeps sampleCount at or above the interpolator's minimum", () => {
    expect(run([{ kind: "setSampleCount", sampleCount: 0 }], armedOnFace()).sampleCount).toBe(2);
  });

  // WP-I: refuse-never-clamp. Above the bound the value is left UNCHANGED and
  // the step names the bound in `hint` — never silently rewritten.
  it("refuses teeth above the worker's ceiling instead of clamping it", () => {
    const armed = armedOnFace();
    const step = gearStep(armed, { kind: "setTeeth", teeth: GEAR_MAX_TEETH + 1 });
    expect(step.state.teeth).toBe(armed.teeth);
    expect(step.effect).toBe("none");
    expect(step.hint).toMatch(new RegExp(String(GEAR_MAX_TEETH)));
    expect(run([{ kind: "setTeeth", teeth: GEAR_MAX_TEETH }], armed).teeth).toBe(GEAR_MAX_TEETH);
  });

  it("refuses a height above the ceiling instead of clamping it", () => {
    const armed = armedOnFace();
    const step = gearStep(armed, { kind: "setHeight", height: GEAR_MAX_HEIGHT + 1 });
    expect(step.state.height).toBe(armed.height);
    expect(step.hint).toMatch(new RegExp(String(GEAR_MAX_HEIGHT)));
  });

  it("refuses a sampleCount above the ceiling instead of clamping it", () => {
    const armed = armedOnFace();
    const step = gearStep(armed, { kind: "setSampleCount", sampleCount: GEAR_MAX_SAMPLE_COUNT + 1 });
    expect(step.state.sampleCount).toBe(armed.sampleCount);
    expect(step.hint).toMatch(new RegExp(String(GEAR_MAX_SAMPLE_COUNT)));
  });
});

describe("gearParamsOf", () => {
  it("emits the active recipe block and a face placement", () => {
    const p = gearParamsOf(armedOnFace());
    expect(p.recipe).toBe("involuteExternal");
    expect(p.involuteExternal).not.toBeNull();
    expect(p.placement.face).toBe(FACE);
    expect(p.placement.frame).toBeNull();
    expect(p.placement.point).toEqual(POINT);
  });

  it("emits a frame placement with a null face", () => {
    const p = gearParamsOf(armedOnFrame());
    expect(p.placement.face).toBeNull();
    expect(p.placement.frame).toEqual({ origin: [0, 0, 0], axis: [0, 0, 1], xDir: [1, 0, 0] });
  });

  it("always emits helical parameters as the only values the kernel accepts", () => {
    // The tool offers no control for these; emitting the accepted values keeps
    // the payload shape correct for when helical support lands.
    const p = gearParamsOf(armedOnFace());
    expect(p.involuteExternal?.helixAngleDeg).toBe(0);
    expect(p.involuteExternal?.doubleHelix).toBe(false);
  });

  it("gates bore dimensions on their own flag", () => {
    const off = gearParamsOf(armedOnFace());
    expect(off.involuteExternal?.axleHole).toBe(false);
    expect(off.involuteExternal?.axleHoleDiameter).toBeNull();
    expect(off.involuteExternal?.offsetHoleDiameter).toBeNull();
    expect(off.involuteExternal?.offsetHoleOffset).toBeNull();

    const on = gearParamsOf(run([{ kind: "setAxleHole", on: true }], armedOnFace()));
    expect(on.involuteExternal?.axleHoleDiameter).toBe(DEFAULT_GEAR_AXLE_HOLE_DIAMETER);
  });

  it("does not resurrect a stale bore dimension after the bore is switched off", () => {
    // The dimension stays in FSM state (so toggling back on returns the user's
    // number) but must NOT reach the record — the Rust session rejects it.
    const s = run(
      [
        { kind: "setAxleHole", on: true },
        { kind: "setAxleHoleDiameter", value: 12 },
        { kind: "setAxleHole", on: false },
      ],
      armedOnFace(),
    );
    expect(s.axleHoleDiameter).toBe(12);
    expect(gearParamsOf(s).involuteExternal?.axleHoleDiameter).toBeNull();
  });

  it("throws rather than emitting a partial op when the placement is missing", () => {
    expect(() => gearParamsOf(gearInit())).toThrow();
  });
});

describe("gearFsmFromParams (re-edit seed)", () => {
  it("round-trips a face-placed gear", () => {
    const authored = gearParamsOf(
      run(
        [
          { kind: "setTeeth", teeth: 31 },
          { kind: "setModule", module: 1.5 },
          { kind: "setAxleHole", on: true },
          { kind: "setAxleHoleDiameter", value: 8 },
        ],
        armedOnFace(),
      ),
    );
    const seeded = gearFsmFromParams(authored as unknown as Record<string, unknown>);
    expect(seeded).not.toBeNull();
    expect(seeded?.phase).toBe("armed");
    expect(seeded?.teeth).toBe(31);
    expect(seeded?.module).toBe(1.5);
    expect(seeded?.axleHole).toBe(true);
    expect(seeded?.axleHoleDiameter).toBe(8);
    expect(seeded?.point).toEqual(POINT);
  });

  it("round-trips a frame-placed gear", () => {
    const authored = gearParamsOf(armedOnFrame());
    const seeded = gearFsmFromParams(authored as unknown as Record<string, unknown>);
    expect(seeded?.face).toBeNull();
    expect(seeded?.frame).toEqual({ origin: [0, 0, 0], axis: [0, 0, 1], xDir: [1, 0, 0] });
  });

  it("refuses a record with no usable placement rather than arming somewhere arbitrary", () => {
    expect(gearFsmFromParams(undefined)).toBeNull();
    expect(gearFsmFromParams({})).toBeNull();
    // Neither face nor frame.
    expect(
      gearFsmFromParams({ placement: { face: null, frame: null, point: [0, 0, 0] } }),
    ).toBeNull();
    // Both — the wire forbids it, so a seed must not silently prefer one.
    expect(
      gearFsmFromParams({
        placement: {
          face: FACE,
          frame: { origin: [0, 0, 0], axis: [0, 0, 1], xDir: [1, 0, 0] },
          point: [0, 0, 0],
        },
      }),
    ).toBeNull();
    // A malformed point.
    expect(gearFsmFromParams({ placement: { face: FACE, frame: null, point: [0, 0] } })).toBeNull();
  });

  it("reads wire Scalars in both their bare and {value} forms", () => {
    const seeded = gearFsmFromParams({
      placement: { face: FACE, frame: null, point: POINT },
      involuteExternal: { teeth: 12, module: { value: 3 }, height: 7 },
    });
    expect(seeded?.module).toBe(3);
    expect(seeded?.height).toBe(7);
    expect(seeded?.teeth).toBe(12);
  });
});
