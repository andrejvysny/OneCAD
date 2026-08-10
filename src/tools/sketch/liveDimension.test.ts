import { describe, it, expect } from "vitest";
import {
  ANGLE_QUANTUM_DEG,
  aliasOf,
  decadeFloor,
  describeDims,
  dimQuantum,
  liveDimInit,
  liveDimStep,
  norm360,
  roundTo,
  type DimFieldId,
  type DimMeasureFrame,
  type LiveDimState,
} from "./liveDimension";
import type { Point2 } from "@/viewport/engine/sketchBasis";

describe("decadeFloor — 1/2/5 ladder, rounds DOWN", () => {
  it("keeps a value already on a rung", () => {
    for (const v of [1, 2, 5, 10, 20, 50, 0.1, 0.2, 0.5, 0.05, 0.005]) {
      expect(decadeFloor(v)).toBeCloseTo(v, 12);
    }
  });

  it("rounds DOWN to the rung below (never up, unlike snapToDecade)", () => {
    expect(decadeFloor(1.9)).toBe(1);
    expect(decadeFloor(3.7)).toBe(2);
    expect(decadeFloor(4.99)).toBe(2);
    expect(decadeFloor(9.99)).toBe(5);
    expect(decadeFloor(0.0393700787401574)).toBeCloseTo(0.02, 12);
  });

  it("survives the 0.5/0.1 = 4.999999999999999 float trap", () => {
    // Without the ladder epsilon this returns 0.2 — a rung sitting exactly on 5.
    expect(decadeFloor(0.5)).toBe(0.5);
    expect(decadeFloor(0.05)).toBe(0.05);
    expect(decadeFloor(5)).toBe(5);
    expect(decadeFloor(500)).toBe(500);
  });

  it("handles tiny and huge magnitudes", () => {
    expect(decadeFloor(1e-12)).toBeCloseTo(1e-12, 24);
    expect(decadeFloor(1e12)).toBe(1e12);
    expect(decadeFloor(7e9)).toBe(5e9);
  });

  it("returns 0 for non-positive / non-finite input (⇒ no rounding)", () => {
    expect(decadeFloor(0)).toBe(0);
    expect(decadeFloor(-3)).toBe(0);
    expect(decadeFloor(Number.NaN)).toBe(0);
    expect(decadeFloor(Number.POSITIVE_INFINITY)).toBe(0);
  });
});

describe("dimQuantum — a tenth of the minor grid cell, in the DISPLAY unit", () => {
  it("walks the mm ladder", () => {
    const cases: Array<[number, number]> = [
      [10, 1],
      [5, 0.5],
      [1, 0.1],
      [0.5, 0.05],
      [0.05, 0.005],
    ];
    for (const [gridMinor, expected] of cases) {
      expect(dimQuantum(gridMinor, "mm").length).toBeCloseTo(expected, 12);
    }
  });

  it("floors at the unit's own display precision", () => {
    // mm renders 3 decimals — a finer quantum would round to unprintable numbers.
    expect(dimQuantum(0.005, "mm").length).toBeCloseTo(0.001, 12);
    expect(dimQuantum(1e-9, "mm").length).toBeCloseTo(0.001, 12);
  });

  it("rounds to whole DISPLAYED numbers in an inch session", () => {
    // 10 mm minor ⇒ 0.3937 in ⇒ /10 ⇒ 0.0394 in ⇒ floored to 0.02 in = 0.508 mm.
    expect(dimQuantum(10, "in").length).toBeCloseTo(0.508, 9);
    expect(dimQuantum(25.4, "in").length).toBeCloseTo(0.1 * 25.4, 9);
  });

  it("keeps angles at whole degrees regardless of zoom", () => {
    expect(dimQuantum(1000, "mm").angle).toBe(ANGLE_QUANTUM_DEG);
    expect(dimQuantum(0.01, "in").angle).toBe(ANGLE_QUANTUM_DEG);
  });
});

describe("roundTo / norm360", () => {
  it("rounds to a multiple of q", () => {
    expect(roundTo(22.3607, 0.1)).toBeCloseTo(22.4, 9);
    expect(roundTo(22.3607, 1)).toBe(22);
    expect(roundTo(-3.4, 1)).toBe(-3);
  });

  it("q <= 0 is the identity (nothing to round to)", () => {
    expect(roundTo(22.3607, 0)).toBe(22.3607);
    expect(roundTo(22.3607, -1)).toBe(22.3607);
  });

  it("folds degrees into [0, 360)", () => {
    expect(norm360(0)).toBe(0);
    expect(norm360(360)).toBe(0);
    expect(norm360(-90)).toBe(270);
    expect(norm360(450)).toBe(90);
  });
});

describe("aliasOf — diameter ↔ radius are one number", () => {
  it("pairs the two views and nothing else", () => {
    expect(aliasOf("diameter")).toBe("radius");
    expect(aliasOf("radius")).toBe("diameter");
    expect(aliasOf("length")).toBeNull();
    expect(aliasOf("sides")).toBeNull();
  });
});

// ── input FSM ─────────────────────────────────────────────────────────────────

const FIELDS: DimFieldId[] = ["length", "angle"];
const focused = (field: DimFieldId, locks = {}): LiveDimState => ({ focus: field, locks, text: "" });

describe("liveDimStep — full transition table", () => {
  it("typeStart opens fields[0] and seeds it with the keystroke", () => {
    const step = liveDimStep(liveDimInit(), { kind: "typeStart", ch: "5", fields: FIELDS });
    expect(step.state).toEqual({ focus: "length", locks: {}, text: "5" });
    expect(step.locked).toBeUndefined();
  });

  it("typeStart is a no-op while a field already owns the input", () => {
    const s = focused("angle");
    expect(liveDimStep(s, { kind: "typeStart", ch: "5", fields: FIELDS }).state).toBe(s);
    expect(liveDimStep(liveDimInit(), { kind: "typeStart", ch: "5", fields: [] }).state).toEqual(
      liveDimInit(),
    );
  });

  it("focus moves the caret and clears the pending text", () => {
    const s: LiveDimState = { focus: "length", locks: {}, text: "12" };
    expect(liveDimStep(s, { kind: "focus", field: "angle" }).state).toEqual({
      focus: "angle",
      locks: {},
      text: "",
    });
  });

  it("text replaces the buffer only while focused", () => {
    expect(liveDimStep(focused("length"), { kind: "text", text: "42" }).state.text).toBe("42");
    const idle = liveDimInit();
    expect(liveDimStep(idle, { kind: "text", text: "42" }).state).toBe(idle);
  });

  it("tab with a value locks the field and advances", () => {
    const step = liveDimStep(focused("length"), {
      kind: "tab",
      fields: FIELDS,
      back: false,
      value: 50,
    });
    expect(step.state).toEqual({ focus: "angle", locks: { length: 50 }, text: "" });
    expect(step.locked).toBe("length");
    expect(step.commitGesture).toBeUndefined();
  });

  it("tab with a null value advances WITHOUT locking", () => {
    const step = liveDimStep(focused("length"), {
      kind: "tab",
      fields: FIELDS,
      back: false,
      value: null,
    });
    expect(step.state).toEqual({ focus: "angle", locks: {}, text: "" });
    expect(step.locked).toBeUndefined();
  });

  it("tab wraps around and Shift+Tab wraps backwards", () => {
    const fwd = liveDimStep(focused("angle"), { kind: "tab", fields: FIELDS, back: false, value: null });
    expect(fwd.state.focus).toBe("length");
    const back = liveDimStep(focused("length"), { kind: "tab", fields: FIELDS, back: true, value: null });
    expect(back.state.focus).toBe("angle");
  });

  it("tab is a no-op with nothing focused", () => {
    const idle = liveDimInit();
    expect(liveDimStep(idle, { kind: "tab", fields: FIELDS, back: false, value: 5 }).state).toBe(idle);
  });

  it("enter locks, unfocuses AND commits the gesture", () => {
    const step = liveDimStep(focused("angle", { length: 50 }), { kind: "enter", value: 30 });
    expect(step.state).toEqual({ focus: null, locks: { length: 50, angle: 30 }, text: "" });
    expect(step.locked).toBe("angle");
    expect(step.commitGesture).toBe(true);
  });

  it("enter on an unparseable value changes nothing (chip flashes)", () => {
    const s: LiveDimState = { focus: "length", locks: {}, text: "abc" };
    const step = liveDimStep(s, { kind: "enter", value: null });
    expect(step.state).toBe(s);
    expect(step.commitGesture).toBeUndefined();
  });

  it("blurCommit locks without committing the gesture", () => {
    const step = liveDimStep(focused("length"), { kind: "blurCommit", value: 50 });
    expect(step.state).toEqual({ focus: null, locks: { length: 50 }, text: "" });
    expect(step.locked).toBe("length");
    expect(step.commitGesture).toBeUndefined();
  });

  it("blurCommit with an unparseable value only unfocuses", () => {
    const step = liveDimStep(focused("length", { angle: 90 }), { kind: "blurCommit", value: null });
    expect(step.state).toEqual({ focus: null, locks: { angle: 90 }, text: "" });
    expect(step.locked).toBeUndefined();
  });

  it("escField drops ONLY the focused field's lock", () => {
    const step = liveDimStep(focused("angle", { length: 50, angle: 30 }), { kind: "escField" });
    expect(step.state).toEqual({ focus: null, locks: { length: 50 }, text: "" });
  });

  it("reset clears everything", () => {
    expect(liveDimStep(focused("angle", { length: 50 }), { kind: "reset" }).state).toEqual(
      liveDimInit(),
    );
  });
});

describe("liveDimStep — diameter ↔ radius alias", () => {
  const aliasFields: DimFieldId[] = ["diameter", "radius"];

  it("locking radius clears a diameter lock", () => {
    const step = liveDimStep(focused("radius", { diameter: 25 }), { kind: "enter", value: 8 });
    expect(step.state.locks).toEqual({ radius: 8 });
  });

  it("locking diameter clears a radius lock", () => {
    const step = liveDimStep(focused("diameter", { radius: 8 }), {
      kind: "tab",
      fields: aliasFields,
      back: false,
      value: 25,
    });
    expect(step.state.locks).toEqual({ diameter: 25 });
  });

  it("leaves unrelated locks alone", () => {
    const step = liveDimStep(focused("radius", { diameter: 25, length: 4 }), {
      kind: "enter",
      value: 8,
    });
    expect(step.state.locks).toEqual({ radius: 8, length: 4 });
  });
});

// ── descriptors ───────────────────────────────────────────────────────────────

const stubFrame = (anchor: Point2): DimMeasureFrame => ({
  fields: [
    { field: "length", label: "L", domain: "length", drives: true, anchor: () => anchor },
    { field: "angle", label: "∠", domain: "angle", drives: false, anchor: () => anchor },
    { field: "radius", label: "R", domain: "length", drives: true, anchor: () => ({ x: 99, y: 99 }) },
  ],
  measure: () => ({ length: 10, angle: 45, radius: 3 }),
});

describe("describeDims", () => {
  it("reads measured values and carries the field spec through", () => {
    const dims = describeDims(stubFrame({ x: 1, y: 2 }), { x: 0, y: 0 }, {});
    expect(dims.map((d) => d.field)).toEqual(["length", "angle", "radius"]);
    expect(dims.map((d) => d.value)).toEqual([10, 45, 3]);
    expect(dims.map((d) => d.locked)).toEqual([false, false, false]);
    expect(dims.map((d) => d.drives)).toEqual([true, false, true]);
    expect(dims[0].label).toBe("L");
  });

  it("a locked field reads its LOCKED value verbatim, never a re-measurement", () => {
    const dims = describeDims(stubFrame({ x: 0, y: 0 }), { x: 0, y: 0 }, { length: 50 });
    expect(dims[0]).toMatchObject({ field: "length", value: 50, locked: true });
    expect(dims[1]).toMatchObject({ field: "angle", value: 45, locked: false });
  });

  it("one chip per frame field, whatever its anchor (separation is the driver's job)", () => {
    const dims = describeDims(stubFrame({ x: 1, y: 2 }), { x: 0, y: 0 }, {});
    // length + angle share an anchor; radius sits elsewhere — the chip set still
    // exposes all three, since the overlay driver keeps each its own anchor.
    expect(dims.map((d) => d.field)).toEqual(["length", "angle", "radius"]);
  });

  it("keeps each field's own anchor (a collision is resolved per frame, not merged)", () => {
    const frame: DimMeasureFrame = {
      fields: [
        { field: "width", label: "W", domain: "length", drives: true, anchor: () => ({ x: 5, y: 5 }) },
        {
          field: "height",
          label: "H",
          domain: "length",
          drives: true,
          anchor: () => ({ x: 5 + 1e-12, y: 5 - 1e-12 }),
        },
      ],
      measure: () => ({ width: 1, height: 2 }),
    };
    const dims = describeDims(frame, { x: 0, y: 0 }, {});
    expect(dims.map((d) => d.field)).toEqual(["width", "height"]);
    expect(dims[1].anchor).toEqual({ x: 5 + 1e-12, y: 5 - 1e-12 }); // untouched float noise
  });

  it("falls back to 0 for a field the frame did not measure", () => {
    const frame: DimMeasureFrame = {
      fields: [{ field: "sides", label: "n", domain: "count", drives: false, anchor: () => ({ x: 0, y: 0 }) }],
      measure: () => ({}),
    };
    expect(describeDims(frame, { x: 0, y: 0 }, {})[0].value).toBe(0);
  });
});
