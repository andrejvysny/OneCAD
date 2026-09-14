import { describe, expect, test } from "bun:test";
import { classifyWheel } from "../src/geometry/wheelClass.ts";
import type { WheelSample } from "../src/geometry/wheelClass.ts";
import { navInit, navReduce } from "../../../src/viewport/engine/navInput.ts";
import type { InputDevice, NavEvent, NavState } from "../../../src/viewport/engine/navInput.ts";

/**
 * The agent mirrors the app's wheel heuristic. A text/regex comparison of the two
 * copies proved nothing about behaviour, so this imports the app reducer directly
 * (navInput.ts is pure — no DOM, no timers) and compares VERDICTS over a grid.
 *
 * The probe notch always OPENS a segment, and navInput weighs an opening event
 * with `gapMs = Infinity`; anything the mirror does with a measured gap is a lie.
 */
const CTX = { pref: "auto", viewportH: 800 } as const;

function detected(sample: WheelSample, prior: InputDevice): InputDevice {
  const state: NavState = { ...navInit(), prior };
  const ev: NavEvent = {
    type: "wheel",
    deltaX: sample.deltaX,
    deltaY: sample.deltaY,
    deltaMode: sample.deltaMode,
    ctrlKey: false,
    shiftKey: false,
    clientX: 100,
    clientY: 100,
    t: 1_000,
  };
  return navReduce(state, ev, CTX).detected;
}

const MAGNITUDES = [3, 10, 39, 40, 99, 100, 120, 300];
const PRIORS: InputDevice[] = ["mouse", "trackpad"];

interface Row {
  label: string;
  sample: WheelSample;
}

function rows(): Row[] {
  const out: Row[] = [];
  for (const m of MAGNITUDES) {
    for (const [kind, v] of [["int", m], ["frac", m + 0.5]] as const) {
      for (const sign of [1, -1]) {
        const d = v * sign;
        out.push({ label: `dy=${d} ${kind}`, sample: { deltaMode: 0, deltaX: 0, deltaY: d } });
        out.push({ label: `dx=${d} ${kind}`, sample: { deltaMode: 0, deltaX: d, deltaY: 0 } });
        out.push({
          label: `diag=${d} ${kind}`,
          sample: { deltaMode: 0, deltaX: d, deltaY: d },
        });
      }
      // deltaMode != 0 is definitive mouse whatever the magnitude; the mirror must agree.
      out.push({ label: `lines dy=${v} ${kind}`, sample: { deltaMode: 1, deltaX: 0, deltaY: v } });
      out.push({ label: `pages dy=${v} ${kind}`, sample: { deltaMode: 2, deltaX: 0, deltaY: v } });
    }
  }
  return out;
}

describe("classifyWheel agrees with navInput.navReduce", () => {
  const grid = rows();

  test("the grid covers every magnitude, axis, sign and delta mode", () => {
    expect(grid.length).toBe(MAGNITUDES.length * 2 * (2 * 3 + 2));
  });

  for (const prior of PRIORS) {
    for (const row of grid) {
      test(`prior=${prior} ${row.label}`, () => {
        expect(classifyWheel(row.sample, prior)).toBe(detected(row.sample, prior));
      });
    }
  }
});

describe("classifyWheel", () => {
  test("one integer 100 px notch scores mouse, whatever the prior", () => {
    const notch: WheelSample = { deltaMode: 0, deltaX: 0, deltaY: 100 };
    expect(classifyWheel(notch, "trackpad")).toBe("mouse");
    expect(classifyWheel(notch, "mouse")).toBe("mouse");
    expect(classifyWheel({ ...notch, deltaY: -100 }, "trackpad")).toBe("mouse");
  });

  test("a small fractional delta scores trackpad", () => {
    expect(classifyWheel({ deltaMode: 0, deltaX: 0, deltaY: 3.5 }, "mouse")).toBe("trackpad");
    expect(classifyWheel({ deltaMode: 0, deltaX: -2.25, deltaY: 3.5 }, "mouse")).toBe("trackpad");
  });

  test("a horizontal component alone is trackpad evidence", () => {
    expect(classifyWheel({ deltaMode: 0, deltaX: 8, deltaY: 60 }, "mouse")).toBe("trackpad");
  });

  test("deltaMode != 0 is definitive mouse regardless of magnitude", () => {
    expect(classifyWheel({ deltaMode: 1, deltaX: 0, deltaY: 3 }, "trackpad")).toBe("mouse");
    expect(classifyWheel({ deltaMode: 2, deltaX: 0, deltaY: 1 }, "trackpad")).toBe("mouse");
  });

  test("an evidence tie falls back to the carried prior, as navInput does", () => {
    const ambiguous: WheelSample = { deltaMode: 0, deltaX: 0, deltaY: 50 };
    expect(classifyWheel(ambiguous, "mouse")).toBe("mouse");
    expect(classifyWheel(ambiguous, "trackpad")).toBe("trackpad");
  });

  test("a measured inter-event gap is ignored: the probe notch always opens a segment", () => {
    const fast = { deltaMode: 0, deltaX: 0, deltaY: 100, gapMs: 10 } as WheelSample;
    expect(classifyWheel(fast, "trackpad")).toBe("mouse");
    expect(classifyWheel(fast, "trackpad")).toBe(detected(fast, "trackpad"));
  });

  test("a sample with no motion proves nothing", () => {
    expect(classifyWheel({ deltaMode: 0, deltaX: 0, deltaY: 0 }, "mouse")).toBe("unknown");
    expect(classifyWheel({ deltaMode: 0, deltaX: 0, deltaY: Number.NaN }, "mouse")).toBe("unknown");
  });
});
