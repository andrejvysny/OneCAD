/*
 * The Hole reducer (WP-C T3).
 *
 * The rule worth most of this file is the CONDITIONAL BLOCK contract: the FSM
 * keeps both the `cb*` and the `cs*` numbers alive so a profile flip is
 * reversible, while `holeParamsOf` emits ONLY the active pair. Getting that
 * backwards in either direction is a real defect — emitting both makes the record
 * un-committable (the Rust session rejects it), and clearing on flip silently
 * loses the user's counterbore.
 */
import { describe, it, expect } from "vitest";
import {
  DEFAULT_HOLE_CB_DEPTH,
  DEFAULT_HOLE_CB_DIAMETER,
  DEFAULT_HOLE_CS_ANGLE,
  DEFAULT_HOLE_CS_DIAMETER,
  DEFAULT_HOLE_DIAMETER,
  HOLE_CS_ANGLES,
  holeFsmFromParams,
  holeInit,
  holeParamsOf,
  holeStep,
  holeValueText,
  type HoleFsm,
} from "./holeMachine";
import { holeStandardPatch } from "./holeStandards";

const FACE = { primary: { bodyId: "body1", elementId: "el_top", kind: "face" as const } };
const POINT: [number, number, number] = [10, 10, 25];

/** An armed hole on `body1`'s top face. */
function armed(): HoleFsm {
  const started = holeStep(holeInit(), { kind: "start" }).state;
  return holeStep(started, { kind: "pickFace", face: FACE, targetBodyId: "body1", point: POINT })
    .state;
}

describe("holeStep — phases", () => {
  it("starts in facePick and arms only on a face pick", () => {
    const s0 = holeInit();
    expect(s0.phase).toBe("idle");
    const s1 = holeStep(s0, { kind: "start" }).state;
    expect(s1.phase).toBe("facePick");
    // Nothing is editable before a seat exists.
    expect(holeStep(s1, { kind: "setDiameter", diameter: 9 }).state.diameter).toBe(
      DEFAULT_HOLE_DIAMETER,
    );
    expect(holeStep(s1, { kind: "confirm" }).effect).toBe("none");

    const s2 = holeStep(s1, {
      kind: "pickFace",
      face: FACE,
      targetBodyId: "body1",
      point: POINT,
    });
    expect(s2.effect).toBe("begin");
    expect(s2.state.phase).toBe("armed");
    expect(s2.state.point).toEqual(POINT);
    expect(s2.state.targetBodyId).toBe("body1");
  });

  it("a fresh start forgets the previous seat entirely", () => {
    const s = holeStep(armed(), { kind: "setDiameter", diameter: 12 }).state;
    const restarted = holeStep(s, { kind: "start" }).state;
    expect(restarted.face).toBeNull();
    expect(restarted.point).toBeNull();
    expect(restarted.diameter).toBe(DEFAULT_HOLE_DIAMETER);
  });

  it("moves the hole on its own face without re-arming", () => {
    const step = holeStep(armed(), { kind: "movePoint", point: [1, 2, 25] });
    expect(step.effect).toBe("update");
    expect(step.state.phase).toBe("armed");
    expect(step.state.point).toEqual([1, 2, 25]);
    // A move while merely picking is meaningless and is refused.
    const picking = holeStep(holeInit(), { kind: "start" }).state;
    expect(holeStep(picking, { kind: "movePoint", point: [0, 0, 0] }).effect).toBe("none");
  });

  it("commits only from armed, and returns to armed on a kernel refusal", () => {
    const step = holeStep(armed(), { kind: "confirm" });
    expect(step.effect).toBe("commit");
    expect(step.state.phase).toBe("committing");
    const back = holeStep(step.state, { kind: "commitFailed" }).state;
    expect(back.phase).toBe("armed");
    // The seat and the numbers survive a refusal — the work is not thrown away.
    expect(back.point).toEqual(POINT);
    expect(back.face).toBe(FACE);
  });

  it("cancel resets to idle and emits the cancel effect", () => {
    const step = holeStep(armed(), { kind: "cancel" });
    expect(step.effect).toBe("cancel");
    expect(step.state).toEqual(holeInit());
    // Cancelling an already-idle tool is a no-op, not a second teardown.
    expect(holeStep(holeInit(), { kind: "cancel" }).effect).toBe("none");
  });
});

describe("holeStep — dimensions", () => {
  it("clamps a dimension to the authorable domain and refuses non-finite input", () => {
    const s = armed();
    expect(holeStep(s, { kind: "setDiameter", diameter: -5 }).state.diameter).toBeGreaterThan(0);
    expect(holeStep(s, { kind: "setDiameter", diameter: Number.NaN }).state.diameter).toBe(
      s.diameter,
    );
    expect(holeStep(s, { kind: "setDiameter", diameter: 8 }).state.diameter).toBe(8);
  });

  it("treats a null depth as THROUGH-ALL, not as a cleared field", () => {
    const s = armed();
    expect(s.depth).toBeNull(); // a fresh hole is through-all
    const blind = holeStep(s, { kind: "setDepth", depth: 12 }).state;
    expect(blind.depth).toBe(12);
    expect(holeStep(blind, { kind: "setDepth", depth: null }).state.depth).toBeNull();
  });

  it("admits ONLY the four SCHEMA countersink angles", () => {
    const s = holeStep(armed(), { kind: "setHoleType", holeType: "countersink" }).state;
    for (const a of HOLE_CS_ANGLES) {
      expect(holeStep(s, { kind: "setCsAngle", angleDeg: a }).state.csAngleDeg).toBe(a);
    }
    for (const bad of [0, 60, 95, 180]) {
      const step = holeStep(s, { kind: "setCsAngle", angleDeg: bad });
      expect(step.effect).toBe("none");
      expect(step.state.csAngleDeg).toBe(DEFAULT_HOLE_CS_ANGLE);
    }
  });
});

describe("holeStep — the conditional-block contract", () => {
  it("KEEPS the other profile's numbers across a flip", () => {
    let s = holeStep(armed(), { kind: "setHoleType", holeType: "counterbore" }).state;
    s = holeStep(s, { kind: "setCbDiameter", value: 14 }).state;
    s = holeStep(s, { kind: "setCbDepth", value: 7 }).state;
    // Flip away …
    s = holeStep(s, { kind: "setHoleType", holeType: "countersink" }).state;
    s = holeStep(s, { kind: "setCsDiameter", value: 15 }).state;
    expect(s.cbDiameter).toBe(14);
    expect(s.cbDepth).toBe(7);
    // … and back: the counterbore is handed straight back.
    s = holeStep(s, { kind: "setHoleType", holeType: "counterbore" }).state;
    expect(s.cbDiameter).toBe(14);
    expect(s.csDiameter).toBe(15);
  });

  it("emits ONLY the active block, spelling the other one null", () => {
    const simple = holeParamsOf(armed());
    expect(simple).toMatchObject({
      cbDiameter: null,
      cbDepth: null,
      csDiameter: null,
      csAngleDeg: null,
    });

    const cb = holeParamsOf(holeStep(armed(), { kind: "setHoleType", holeType: "counterbore" }).state);
    expect(cb.cbDiameter).toBe(DEFAULT_HOLE_CB_DIAMETER);
    expect(cb.cbDepth).toBe(DEFAULT_HOLE_CB_DEPTH);
    expect(cb.csDiameter).toBeNull();
    expect(cb.csAngleDeg).toBeNull();

    const cs = holeParamsOf(holeStep(armed(), { kind: "setHoleType", holeType: "countersink" }).state);
    expect(cs.csDiameter).toBe(DEFAULT_HOLE_CS_DIAMETER);
    expect(cs.csAngleDeg).toBe(DEFAULT_HOLE_CS_ANGLE);
    expect(cs.cbDiameter).toBeNull();
    expect(cs.cbDepth).toBeNull();
  });

  it("refuses to build params without a seat", () => {
    expect(() => holeParamsOf(holeInit())).toThrow(/picked face/);
  });

  it("carries the frozen point and the host body verbatim", () => {
    const p = holeParamsOf(armed());
    expect(p.point).toEqual(POINT);
    expect(p.targetBodyId).toBe("body1");
    expect(p.face).toBe(FACE);
    expect(p.depth).toBeNull();
  });
});

describe("holeStep — standards presets", () => {
  it("fills raw mm from an M6 normal-clearance counterbore pick", () => {
    const s = holeStep(armed(), { kind: "setHoleType", holeType: "counterbore" }).state;
    const patch = holeStandardPatch("M6", "normal", "counterbore")!;
    const next = holeStep(s, { kind: "applyStandard", patch }).state;
    expect(next.diameter).toBe(6.6);
    expect(next.cbDiameter).toBe(11);
    expect(next.cbDepth).toBe(6.8);
    // The record still carries raw mm only — no thread designation anywhere.
    expect(JSON.stringify(holeParamsOf(next))).not.toContain("M6");
  });

  it("leaves the INACTIVE block untouched (the patch names only its own fields)", () => {
    const s = holeStep(armed(), { kind: "setHoleType", holeType: "countersink" }).state;
    const before = s.cbDiameter;
    const next = holeStep(s, {
      kind: "applyStandard",
      patch: holeStandardPatch("M8", "close", "countersink")!,
    }).state;
    expect(next.diameter).toBe(8.4);
    expect(next.csDiameter).toBe(16.4);
    expect(next.cbDiameter).toBe(before);
  });

  it("marks the hole as touched so a later flip cannot undo the preset", () => {
    expect(armed().touched).toBe(false);
    const next = holeStep(armed(), {
      kind: "applyStandard",
      patch: holeStandardPatch("M10", "normal", "simple")!,
    }).state;
    expect(next.touched).toBe(true);
    expect(next.diameter).toBe(11);
  });
});

describe("holeFsmFromParams — re-edit seed", () => {
  it("restores every field, including the frozen seat", () => {
    const seeded = holeFsmFromParams({
      targetBodyId: "body7",
      face: { primary: { bodyId: "body7", elementId: "el_x", kind: "face" } },
      point: [1, 2, 3],
      holeType: "counterbore",
      diameter: { value: 8.4 },
      depth: { value: 14 },
      cbDiameter: { value: 15 },
      cbDepth: { value: 9 },
      csDiameter: null,
      csAngleDeg: null,
    });
    expect(seeded).not.toBeNull();
    expect(seeded!.phase).toBe("armed");
    expect(seeded!.targetBodyId).toBe("body7");
    expect(seeded!.point).toEqual([1, 2, 3]);
    expect(seeded!.holeType).toBe("counterbore");
    expect(seeded!.diameter).toBe(8.4);
    expect(seeded!.depth).toBe(14);
    expect(seeded!.cbDiameter).toBe(15);
    expect(seeded!.cbDepth).toBe(9);
    // The inactive block has nothing authored to restore, so it takes the default
    // rather than 0 — a later flip must start from a usable size.
    expect(seeded!.csDiameter).toBe(DEFAULT_HOLE_CS_DIAMETER);
    expect(seeded!.touched).toBe(true);
  });

  it("reads a null depth back as through-all", () => {
    const seeded = holeFsmFromParams({
      targetBodyId: "b",
      face: { primary: { bodyId: "b", elementId: "e", kind: "face" } },
      point: [0, 0, 0],
      holeType: "simple",
      diameter: 5.5,
      depth: null,
    });
    expect(seeded!.depth).toBeNull();
    expect(seeded!.diameter).toBe(5.5); // a bare number is a legal wire Scalar
  });

  it("refuses a seed with no usable seat rather than arming somewhere arbitrary", () => {
    expect(holeFsmFromParams(undefined)).toBeNull();
    expect(holeFsmFromParams({ diameter: 6 })).toBeNull();
    expect(
      holeFsmFromParams({
        face: { primary: { bodyId: "b", elementId: "e", kind: "face" } },
        diameter: 6,
      }),
      "no point ⇒ refuse",
    ).toBeNull();
    expect(
      holeFsmFromParams({
        face: { primary: { bodyId: "b", elementId: "e", kind: "face" } },
        point: [0, 0, 0],
      }),
      "no diameter ⇒ refuse",
    ).toBeNull();
  });
});

describe("holeValueText", () => {
  it("mirrors dto.rs feature_value_text exactly", () => {
    expect(holeValueText(5.5)).toBe("Ø5.5");
    expect(holeValueText(6)).toBe("Ø6.0");
    expect(holeValueText(13.5)).toBe("Ø13.5");
  });
});
