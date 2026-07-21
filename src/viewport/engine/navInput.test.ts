import { describe, it, expect } from "vitest";
import {
  navInit,
  navReduce,
  normalizeWheel,
  definitiveDevice,
  clampFactor,
  LINE_HEIGHT_PX,
  SEGMENT_IDLE_MS,
  POST_GESTURE_SUPPRESS_MS,
  TRACKPAD_PAN_GAIN,
  TRACKPAD_ORBIT_GAIN,
  type NavEvent,
  type NavState,
  type NavOp,
  type DevicePref,
} from "./navInput";

const CTX = { pref: "auto" as DevicePref, viewportH: 800 };

function wheel(p: Partial<Extract<NavEvent, { type: "wheel" }>> = {}): NavEvent {
  return {
    type: "wheel",
    deltaX: 0,
    deltaY: 0,
    deltaMode: 0,
    ctrlKey: false,
    shiftKey: false,
    clientX: 100,
    clientY: 100,
    t: 0,
    ...p,
  };
}

/** Feed a sequence, returning every op and the final state. */
function run(
  events: NavEvent[],
  pref: DevicePref = "auto",
  state: NavState = navInit(),
): { ops: (NavOp | null)[]; state: NavState; device: string; detected: string } {
  const ops: (NavOp | null)[] = [];
  let s = state;
  let device = "mouse";
  let detected = "mouse";
  for (const ev of events) {
    const r = navReduce(s, ev, { ...CTX, pref });
    s = r.state;
    device = r.device;
    detected = r.detected;
    ops.push(r.op);
  }
  return { ops, state: s, device, detected };
}

/** A trackpad-shaped scroll stream: fractional, fast, some horizontal drift. */
function trackpadScroll(count: number, t0 = 0, extra: Partial<Extract<NavEvent, { type: "wheel" }>> = {}) {
  return Array.from({ length: count }, (_, i) =>
    wheel({ deltaX: 0.5, deltaY: 2.5 + i * 0.25, t: t0 + i * 16, ...extra }),
  );
}

/** A mouse-shaped stream: big integer notches, slow cadence, vertical only. */
function mouseWheel(count: number, t0 = 0, extra: Partial<Extract<NavEvent, { type: "wheel" }>> = {}) {
  return Array.from({ length: count }, (_, i) =>
    wheel({ deltaY: 120, t: t0 + i * 200, ...extra }),
  );
}

describe("normalizeWheel", () => {
  it("passes DOM_DELTA_PIXEL through untouched", () => {
    expect(normalizeWheel({ deltaX: 3, deltaY: -7.5, deltaMode: 0, shiftKey: false }, 800)).toEqual({
      dx: 3,
      dy: -7.5,
    });
  });

  it("expands DOM_DELTA_LINE by the line height", () => {
    const n = normalizeWheel({ deltaX: 0, deltaY: 3, deltaMode: 1, shiftKey: false }, 800);
    expect(n.dy).toBe(3 * LINE_HEIGHT_PX);
  });

  it("expands DOM_DELTA_PAGE by the viewport height", () => {
    const n = normalizeWheel({ deltaX: 0, deltaY: 1, deltaMode: 2, shiftKey: false }, 640);
    expect(n.dy).toBe(640);
  });

  it("guards a zero viewport height", () => {
    const n = normalizeWheel({ deltaX: 0, deltaY: 2, deltaMode: 2, shiftKey: false }, 0);
    expect(Number.isFinite(n.dy)).toBe(true);
    expect(n.dy).toBe(2);
  });
});

describe("definitiveDevice", () => {
  it("treats any non-pixel deltaMode as a mouse", () => {
    expect(definitiveDevice({ deltaMode: 1, ctrlKey: false, deltaY: 3 })).toBe("mouse");
    expect(definitiveDevice({ deltaMode: 2, ctrlKey: false, deltaY: 1 })).toBe("mouse");
  });

  it("treats a small ctrl-wheel delta as a trackpad pinch", () => {
    expect(definitiveDevice({ deltaMode: 0, ctrlKey: true, deltaY: -4 })).toBe("trackpad");
  });

  it("does NOT treat a big integer ctrl-wheel delta as a trackpad", () => {
    // A mouse user holding Ctrl to zoom. Misreading this would make their next
    // plain scroll pan instead of zoom.
    expect(definitiveDevice({ deltaMode: 0, ctrlKey: true, deltaY: 120 })).toBeNull();
  });

  it("returns null when there is no definitive signal", () => {
    expect(definitiveDevice({ deltaMode: 0, ctrlKey: false, deltaY: 2.5 })).toBeNull();
  });
});

describe("clampFactor", () => {
  it("bounds the per-event zoom", () => {
    expect(clampFactor(1000)).toBe(5);
    expect(clampFactor(0.0001)).toBe(0.2);
    expect(clampFactor(1.5)).toBe(1.5);
  });

  it("falls back to a no-op on degenerate input", () => {
    expect(clampFactor(NaN)).toBe(1);
    expect(clampFactor(0)).toBe(1);
    expect(clampFactor(-2)).toBe(1);
  });
});

describe("mouse routing", () => {
  it("zooms on a plain wheel and never pans", () => {
    const { ops } = run(mouseWheel(3));
    expect(ops.every((o) => o?.kind === "zoom")).toBe(true);
  });

  it("zooms toward the cursor position", () => {
    const { ops } = run([wheel({ deltaY: 120, clientX: 42, clientY: 77 })]);
    expect(ops[0]).toMatchObject({ kind: "zoom", clientX: 42, clientY: 77 });
  });

  it("scrolling down dollies out, up dollies in", () => {
    const down = run([wheel({ deltaY: 120 })]).ops[0] as Extract<NavOp, { kind: "zoom" }>;
    const up = run([wheel({ deltaY: -120 })]).ops[0] as Extract<NavOp, { kind: "zoom" }>;
    expect(down.factor).toBeGreaterThan(1);
    expect(up.factor).toBeLessThan(1);
  });

  it("still zooms when shift is held (shift is a trackpad-only modifier)", () => {
    const { ops } = run(mouseWheel(2, 0, { shiftKey: true }));
    expect(ops.every((o) => o?.kind === "zoom")).toBe(true);
  });

  it("emits nothing for a zero delta", () => {
    expect(run([wheel({ deltaY: 0 })]).ops[0]).toBeNull();
  });

  it("zooms on a shift+wheel that the platform folded onto deltaX", () => {
    // Verified in Chromium/macOS: shift + wheel arrives as deltaX=N, deltaY=0.
    // Reading only deltaY made shift+wheel a no-op.
    const op = run([wheel({ deltaX: 240, deltaY: 0, shiftKey: true })]).ops[0];
    expect(op?.kind).toBe("zoom");
    expect((op as Extract<NavOp, { kind: "zoom" }>).factor).toBeGreaterThan(1);
  });

  it("keeps the sign when zooming off the horizontal axis", () => {
    const up = run([wheel({ deltaX: -240, deltaY: 0, shiftKey: true })]).ops[0];
    expect((up as Extract<NavOp, { kind: "zoom" }>).factor).toBeLessThan(1);
  });

  it("prefers deltaY when both axes carry motion", () => {
    const a = run([wheel({ deltaX: 10, deltaY: 120 })]).ops[0] as Extract<NavOp, { kind: "zoom" }>;
    const b = run([wheel({ deltaX: 0, deltaY: 120 })]).ops[0] as Extract<NavOp, { kind: "zoom" }>;
    expect(a.factor).toBeCloseTo(b.factor, 9);
  });
});

describe("trackpad routing", () => {
  it("pans on a two-finger scroll", () => {
    const { ops } = run(trackpadScroll(4), "trackpad");
    expect(ops.every((o) => o?.kind === "pan")).toBe(true);
  });

  it("orbits when shift is held", () => {
    const { ops } = run(trackpadScroll(4, 0, { shiftKey: true }), "trackpad");
    expect(ops.every((o) => o?.kind === "orbit")).toBe(true);
  });

  it("carries horizontal motion into the pan", () => {
    const op = run([wheel({ deltaX: 8, deltaY: 0 })], "trackpad").ops[0];
    expect(op).toEqual({ kind: "pan", dx: 8 * TRACKPAD_PAN_GAIN, dy: 0 });
  });

  it("applies the orbit gain, not the pan gain", () => {
    const op = run([wheel({ deltaX: 10, deltaY: 20, shiftKey: true })], "trackpad").ops[0];
    expect(op).toEqual({
      kind: "orbit",
      dx: 10 * TRACKPAD_ORBIT_GAIN,
      dy: 20 * TRACKPAD_ORBIT_GAIN,
    });
  });

  it("emits nothing when both axes are zero", () => {
    expect(run([wheel({ deltaX: 0, deltaY: 0 })], "trackpad").ops[0]).toBeNull();
  });
});

describe("modifier latching across a momentum tail", () => {
  it("keeps orbiting after shift is released mid-segment", () => {
    // Fingers lift; macOS keeps delivering inertial events. Releasing shift
    // during that tail must NOT flip the gesture to pan.
    const events = [
      ...trackpadScroll(2, 0, { shiftKey: true }),
      ...trackpadScroll(3, 32, { shiftKey: false }),
    ];
    const { ops } = run(events, "trackpad");
    expect(ops.every((o) => o?.kind === "orbit")).toBe(true);
  });

  it("picks up the new modifier only on the next segment", () => {
    const events = [
      ...trackpadScroll(2, 0, { shiftKey: true }),
      // Gap longer than the idle timeout ⇒ a genuinely new gesture.
      ...trackpadScroll(2, 1000, { shiftKey: false }),
    ];
    const { ops } = run(events, "trackpad");
    expect(ops.slice(0, 2).every((o) => o?.kind === "orbit")).toBe(true);
    expect(ops.slice(2).every((o) => o?.kind === "pan")).toBe(true);
  });

  it("latches shift ON only at a segment boundary", () => {
    const events = [
      ...trackpadScroll(2, 0, { shiftKey: false }),
      ...trackpadScroll(2, 32, { shiftKey: true }), // same segment
    ];
    const { ops } = run(events, "trackpad");
    expect(ops.every((o) => o?.kind === "pan")).toBe(true);
  });
});

describe("device classification", () => {
  it("defaults to mouse so an existing user's first scroll still zooms", () => {
    expect(run([wheel({ deltaY: 120 })]).ops[0]?.kind).toBe("zoom");
  });

  it("routes a fractional diagonal first flick as a trackpad pan immediately", () => {
    // Strong evidence on the very first event avoids a wrong-feeling first gesture.
    expect(run([wheel({ deltaX: 1.5, deltaY: 3.25 })]).ops[0]?.kind).toBe("pan");
  });

  it("converges to trackpad and stays there across segments", () => {
    const r = run(trackpadScroll(6));
    expect(r.device).toBe("trackpad");
    const next = run(trackpadScroll(2, 5000), "auto", r.state);
    expect(next.ops.every((o) => o?.kind === "pan")).toBe(true);
  });

  it("flips back to mouse when a mouse is plugged in mid-session", () => {
    // The regression the lifetime latch would have caused: dock a MacBook, and
    // every wheel tick pans forever.
    const pad = run(trackpadScroll(6));
    expect(pad.device).toBe("trackpad");
    const mouse = run(mouseWheel(4, 10_000), "auto", pad.state);
    expect(mouse.device).toBe("mouse");
    expect(mouse.ops[mouse.ops.length - 1]?.kind).toBe("zoom");
  });

  it("lets definitive evidence override mid-segment", () => {
    const events = [
      ...trackpadScroll(3),
      // A line-mode event inside the same time window is unambiguously a mouse.
      wheel({ deltaY: 3, deltaMode: 1, t: 48 }),
    ];
    const { ops, device } = run(events);
    expect(device).toBe("mouse");
    expect(ops[ops.length - 1]?.kind).toBe("zoom");
  });

  it("does not flip on weak evidence mid-segment", () => {
    const events = [
      ...trackpadScroll(3),
      wheel({ deltaY: 120, t: 48 }), // big but same segment
    ];
    const { ops } = run(events);
    expect(ops[ops.length - 1]?.kind).toBe("pan");
  });

  it("respects a manual override against contrary evidence", () => {
    expect(run(mouseWheel(3), "trackpad").ops.every((o) => o?.kind === "pan")).toBe(true);
    expect(run(trackpadScroll(3), "mouse").ops.every((o) => o?.kind === "zoom")).toBe(true);
  });

  it("reports the detected device even while an override is active", () => {
    // The UI shows this under "Auto"; it must reflect detection, not the override.
    const r = run(trackpadScroll(6), "mouse");
    expect(r.detected).toBe("trackpad");
    expect(r.device).toBe("mouse"); // routing still obeys the override
    expect(r.ops.every((o) => o?.kind === "zoom")).toBe(true);
  });

  it("surfaces detection mid-segment, not only at a segment boundary", () => {
    // A continuous scroll is one segment; the label must not wait for it to end.
    const r = run(trackpadScroll(3));
    expect(r.detected).toBe("trackpad");
  });
});

describe("segment boundaries", () => {
  it("treats a gap over the idle timeout as a new gesture", () => {
    const events = [
      wheel({ deltaX: 1.5, deltaY: 2.5, shiftKey: true, t: 0 }),
      wheel({ deltaX: 1.5, deltaY: 2.5, shiftKey: false, t: SEGMENT_IDLE_MS + 1 }),
    ];
    const { ops } = run(events, "trackpad");
    expect(ops[0]?.kind).toBe("orbit");
    expect(ops[1]?.kind).toBe("pan");
  });

  it("treats a gap under the idle timeout as the same gesture", () => {
    const events = [
      wheel({ deltaX: 1.5, deltaY: 2.5, shiftKey: true, t: 0 }),
      wheel({ deltaX: 1.5, deltaY: 2.5, shiftKey: false, t: SEGMENT_IDLE_MS - 1 }),
    ];
    const { ops } = run(events, "trackpad");
    expect(ops.every((o) => o?.kind === "orbit")).toBe(true);
  });
});

describe("pinch arbitration", () => {
  const gStart = (scale: number, t: number): NavEvent => ({
    type: "gesturestart",
    scale,
    clientX: 50,
    clientY: 60,
    t,
  });
  const gChange = (scale: number, t: number): NavEvent => ({
    type: "gesturechange",
    scale,
    clientX: 50,
    clientY: 60,
    t,
  });
  const gEnd = (t: number): NavEvent => ({ type: "gestureend", t });

  it("zooms from cumulative WebKit scale, toward the gesture origin", () => {
    const { ops } = run([gStart(1, 0), gChange(2, 16)]);
    expect(ops[1]).toMatchObject({ kind: "zoom", clientX: 50, clientY: 60 });
    // scale doubled ⇒ fingers apart ⇒ closer ⇒ factor < 1
    expect((ops[1] as Extract<NavOp, { kind: "zoom" }>).factor).toBeCloseTo(0.5, 6);
  });

  it("treats scale as cumulative, not incremental", () => {
    const { ops } = run([gStart(1, 0), gChange(2, 16), gChange(4, 32)]);
    const a = (ops[1] as Extract<NavOp, { kind: "zoom" }>).factor;
    const b = (ops[2] as Extract<NavOp, { kind: "zoom" }>).factor;
    // Each step doubles again, so each factor is 0.5 — not 0.5 then 0.25.
    expect(a).toBeCloseTo(0.5, 6);
    expect(b).toBeCloseTo(0.5, 6);
  });

  it("ignores a ctrl-wheel once a WebKit gesture has claimed pinch", () => {
    const { ops } = run([
      gStart(1, 0),
      gChange(1.5, 16),
      wheel({ deltaY: -5, ctrlKey: true, t: 20 }),
    ]);
    expect(ops[2]).toBeNull();
  });

  it("ignores a ctrl-wheel tail arriving after gestureend", () => {
    // The exact double-zoom the source latch plus the grace window exist to stop.
    const { ops } = run([
      gStart(1, 0),
      gChange(1.5, 16),
      gEnd(32),
      wheel({ deltaY: -5, ctrlKey: true, t: 40 }),
    ]);
    expect(ops[3]).toBeNull();
  });

  it("suppresses a ctrl-wheel tail even without a prior gesturestart", () => {
    let s = navInit();
    s = navReduce(s, gStart(1, 0), CTX).state;
    s = navReduce(s, gEnd(10), CTX).state;
    const r = navReduce(s, wheel({ deltaY: -5, ctrlKey: true, t: 10 + POST_GESTURE_SUPPRESS_MS - 1 }), CTX);
    expect(r.op).toBeNull();
  });

  it("uses ctrl-wheel for pinch when no gesture events ever arrive", () => {
    const { ops } = run([wheel({ deltaY: -5, ctrlKey: true, t: 0 })]);
    expect(ops[0]?.kind).toBe("zoom");
    expect((ops[0] as Extract<NavOp, { kind: "zoom" }>).factor).toBeLessThan(1);
  });

  it("ctrl-wheel pinch out zooms in, pinch in zooms out", () => {
    const out = run([wheel({ deltaY: -5, ctrlKey: true })]).ops[0] as Extract<NavOp, { kind: "zoom" }>;
    const inn = run([wheel({ deltaY: 5, ctrlKey: true })]).ops[0] as Extract<NavOp, { kind: "zoom" }>;
    expect(out.factor).toBeLessThan(1);
    expect(inn.factor).toBeGreaterThan(1);
  });

  it("is idempotent on a duplicate gestureend", () => {
    // WebKit fires gestureend twice; the second must not resurrect a zoom.
    const { ops } = run([gStart(1, 0), gChange(1.5, 16), gEnd(32), gEnd(32)]);
    expect(ops[3]).toBeNull();
  });

  it("recovers from a gesturestart with no preceding gestureend", () => {
    const { ops } = run([gStart(1, 0), gChange(2, 16), gStart(1, 32), gChange(2, 48)]);
    // The second gesture restarts from its own scale rather than leaking the first's.
    expect((ops[3] as Extract<NavOp, { kind: "zoom" }>).factor).toBeCloseTo(0.5, 6);
  });

  it("ignores a gesturechange with no active gesture", () => {
    expect(run([gChange(2, 0)]).ops[0]).toBeNull();
  });

  it("survives a degenerate zero scale", () => {
    const { ops } = run([gStart(1, 0), gChange(0, 16)]);
    expect(ops[1]).toBeNull();
  });

  it("lets a plain scroll immediately follow a pinch", () => {
    const { ops } = run([
      gStart(1, 0),
      gChange(1.5, 16),
      gEnd(32),
      ...trackpadScroll(2, 300),
    ]);
    expect(ops.slice(3).every((o) => o?.kind === "pan")).toBe(true);
  });

  it("does not let a ctrl-wheel pinch bleed into the next pan segment", () => {
    const { ops } = run([
      wheel({ deltaY: -5, ctrlKey: true, t: 0 }),
      ...trackpadScroll(2, 16),
    ]);
    expect(ops[0]?.kind).toBe("zoom");
    expect(ops.slice(1).every((o) => o?.kind === "pan")).toBe(true);
  });
});

describe("purity", () => {
  it("does not mutate the state it is given", () => {
    const s0 = navInit();
    const snapshot = JSON.stringify(s0);
    navReduce(s0, wheel({ deltaY: 120 }), CTX);
    expect(JSON.stringify(s0)).toBe(snapshot);
  });

  it("is deterministic for the same inputs", () => {
    const s = navInit();
    const ev = wheel({ deltaX: 2.5, deltaY: 3.5 });
    expect(navReduce(s, ev, CTX).op).toEqual(navReduce(s, ev, CTX).op);
  });
});
