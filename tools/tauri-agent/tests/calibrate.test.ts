/**
 * Calibration must be OBSERVATION: session_start, recalibration and the lazy wheel probe
 * may not change document, camera, selection, history or application state.
 *
 * Two halves. The jsdom half runs the real in-page arm script against a fixture that
 * registers the app's own wheel listener exactly as `CadOrbitControls` does
 * (`src/viewport/engine/CadOrbitControls.ts`: bubble phase, on the canvas element,
 * `{ passive: false }`) and proves the probe notch never reaches it. The second half drives
 * `hoverProbe` / `wheelProbe` against fake deps and proves no probe posts native input at a
 * point that has not passed `checkPoint` — the same gate the action pipeline uses.
 */
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { AgentError } from "../src/errors.ts";
import { checkPoint, cssToGlobal, globalToCss, pointInWindow } from "../src/geometry/mapping.ts";
import type { Pt, Rect, WindowGeom } from "../src/geometry/types.ts";
import {
  armWheelInPage,
  disarmWheelInPage,
  hoverProbe,
  hoverRan,
  stepOffGlobal,
  wheelProbe,
} from "../src/session/calibrate.ts";
import type { ProbeDeps, ProbeWindow } from "../src/session/calibrate.ts";
import type { NativeInput } from "../src/platform/adapter.ts";
import type { SessionBridge } from "../src/session/types.ts";

// --- the in-page arm script, under jsdom ------------------------------------

/**
 * Faithful to the app: `data-testid="viewport-canvas"` is the CONTAINER
 * (`src/viewport/ViewportRoot.tsx`), and `CadOrbitControls` binds its wheel handler to the
 * `<canvas>` inside it (`ViewportEngine.ts` passes `element: this.canvas`). So the real
 * event target is the canvas and the app's listener is at-target, bubble phase — which a
 * capture-phase listener on `window` still precedes.
 */
const PAGE = `<!doctype html><html><body>
  <div data-testid="viewport-canvas"><canvas width="10" height="10"></canvas></div>
</body></html>`;

const PROBE: Pt = { x: 320, y: 240 };
const CANVAS_RECT = { x: 220, y: 140, width: 200, height: 200 };

interface PageFixture {
  dom: JSDOM;
  /** The app's own handler: bubble phase, on the canvas, exactly as CadOrbitControls binds it. */
  appWheels: Pt[];
  wheelAt(p: Pt, over?: { deltaY?: number; deltaMode?: number }): boolean;
  sample(): { deltaMode: number; deltaY: number } | null | undefined;
  armed(): boolean;
}

function makePage(): PageFixture {
  const dom = new JSDOM(PAGE, { pretendToBeVisual: true });
  const win = dom.window;
  win.Element.prototype.getBoundingClientRect = function stub(this: Element) {
    const r = CANVAS_RECT;
    return {
      ...r,
      left: r.x,
      top: r.y,
      right: r.x + r.width,
      bottom: r.y + r.height,
      toJSON: () => ({}),
    } as DOMRect;
  };
  const g = globalThis as unknown as Record<string, unknown>;
  g.window = win;
  g.document = win.document;
  const canvas = win.document.querySelector("canvas") as Element;
  const appWheels: Pt[] = [];
  canvas.addEventListener(
    "wheel",
    (e) => {
      const ev = e as WheelEvent;
      appWheels.push({ x: ev.clientX, y: ev.clientY });
    },
    { passive: false },
  );
  return {
    dom,
    appWheels,
    wheelAt(p, over = {}) {
      const ev = new win.WheelEvent("wheel", {
        clientX: p.x,
        clientY: p.y,
        deltaY: over.deltaY ?? 3,
        deltaMode: over.deltaMode ?? 1,
        bubbles: true,
        cancelable: true,
      });
      canvas.dispatchEvent(ev);
      return ev.defaultPrevented;
    },
    sample: () => (win as unknown as ProbeWindow).__tauriAgentWheel,
    armed: () => (win as unknown as ProbeWindow).__tauriAgentWheelDisarm !== undefined,
  };
}

let page: PageFixture;

beforeEach(() => {
  page = makePage();
});

afterEach(() => {
  disarmWheelInPage();
  page.dom.window.close();
});

describe("armWheelInPage", () => {
  test("locates the viewport without registering anything when no probe point is given", () => {
    const arm = armWheelInPage(null, 4, 5_000);
    expect(arm).toEqual({ present: true, rect: CANVAS_RECT, armed: false });
    expect(page.armed()).toBe(false);
    page.wheelAt(PROBE);
    // Nothing armed, so the app sees its own wheel and the probe records nothing.
    expect(page.appWheels).toHaveLength(1);
    expect(page.sample()).toBeNull();
  });

  test("the probe notch is classified and then swallowed before the app's handler sees it", () => {
    expect(armWheelInPage(PROBE, 4, 5_000).armed).toBe(true);
    const prevented = page.wheelAt(PROBE, { deltaY: 3, deltaMode: 1 });

    expect(page.appWheels).toEqual([]);
    expect(prevented).toBe(true);
    expect(page.sample()).toMatchObject({ deltaMode: 1, deltaY: 3, deltaX: 0 });
    // One notch is all it wanted: the listener took itself down.
    expect(page.armed()).toBe(false);
  });

  test("a wheel away from the probe point passes through and leaves the probe armed", () => {
    armWheelInPage(PROBE, 4, 5_000);
    const far = { x: PROBE.x + 60, y: PROBE.y };
    const prevented = page.wheelAt(far);

    expect(page.appWheels).toEqual([far]);
    expect(prevented).toBe(false);
    expect(page.sample()).toBeNull();
    expect(page.armed()).toBe(true);

    // Still armed, so the probe's own notch is still captured.
    page.wheelAt(PROBE);
    expect(page.appWheels).toEqual([far]);
    expect(page.sample()).toMatchObject({ deltaY: 3 });
  });

  test("a wheel just inside the tolerance is still ours", () => {
    armWheelInPage(PROBE, 4, 5_000);
    page.wheelAt({ x: PROBE.x + 4, y: PROBE.y - 4 });
    expect(page.appWheels).toEqual([]);
    expect(page.sample()).toMatchObject({ deltaY: 3 });
  });

  test("the in-page guard timer disarms, so a dropped bridge cannot eat the user's wheel", async () => {
    armWheelInPage(PROBE, 4, 20);
    expect(page.armed()).toBe(true);
    await new Promise((r) => setTimeout(r, 80));

    expect(page.armed()).toBe(false);
    page.wheelAt(PROBE);
    expect(page.appWheels).toEqual([PROBE]);
    expect(page.sample()).toBeNull();
  });

  test("disarmWheelInPage removes the listener and is idempotent", () => {
    armWheelInPage(PROBE, 4, 5_000);
    expect(disarmWheelInPage()).toEqual({ disarmed: true });
    expect(disarmWheelInPage()).toEqual({ disarmed: false });

    page.wheelAt(PROBE);
    expect(page.appWheels).toEqual([PROBE]);
    expect(page.sample()).toBeNull();
  });

  test("re-arming takes the previous listener down instead of stacking one", () => {
    armWheelInPage(PROBE, 4, 5_000);
    armWheelInPage(PROBE, 4, 5_000);
    disarmWheelInPage();

    page.wheelAt(PROBE);
    expect(page.appWheels).toEqual([PROBE]);
  });
});

// --- the probes, against fake deps ------------------------------------------

/** 1200x800 css window whose content box starts at (100, 50) on a 2x display. */
const GEOM: WindowGeom = {
  innerPositionPx: { x: 200, y: 100 },
  innerSizePx: { width: 2400, height: 1600 },
  scaleFactor: 2,
  nativeBoundsPt: { x: 100, y: 50, width: 1200, height: 800 },
  windowId: 7,
};

/** A tiny window with legal pixels only in the vertical strip x in [23, 37). */
const BOXED_GEOM: WindowGeom = {
  innerPositionPx: { x: 0, y: 0 },
  innerSizePx: { width: 60, height: 40 },
  scaleFactor: 1,
  nativeBoundsPt: { x: 0, y: 0, width: 60, height: 40 },
  windowId: 9,
};
const BOXED_OCCLUSION: Rect[] = [
  { x: 0, y: 0, width: 23, height: 40 },
  { x: 37, y: 0, width: 23, height: 40 },
];

interface InputCall {
  verb: "move" | "scroll";
  p: Pt;
}

function recordingInput(): { input: NativeInput; calls: InputCall[] } {
  const calls: InputCall[] = [];
  const noop = async (): Promise<void> => undefined;
  const input = {
    move: async (p: Pt) => {
      calls.push({ verb: "move", p });
    },
    scroll: async (p: Pt) => {
      calls.push({ verb: "scroll", p });
    },
    down: noop,
    up: noop,
    click: noop,
    path: noop,
    keyDown: noop,
    keyUp: noop,
    press: noop,
    type: noop,
    releaseAll: noop,
    cursor: async () => ({ x: 0, y: 0 }),
    permissions: async () => ({ accessibility: true, screenRecording: true }),
    dispose: noop,
  } as unknown as NativeInput;
  return { input, calls };
}

interface BridgeOpts {
  probeRect?: Rect;
  canvasRect?: Rect | null;
  /** null models a viewport that never reports a wheel event. */
  wheelSample?: { deltaMode: number; deltaX: number; deltaY: number; gapMs: number } | null;
}

function fakeBridge(opts: BridgeOpts): { bridge: SessionBridge; scripts: string[] } {
  const scripts: string[] = [];
  const bridge: SessionBridge = {
    wedged: false,
    windowGeom: async () => {
      throw new Error("unused");
    },
    close: async () => undefined,
    invoke: (async () => undefined) as SessionBridge["invoke"],
    execute: (async (name: string, _fn: unknown, args: unknown[]) => {
      scripts.push(name);
      if (name === "calibrate.pickProbe") {
        return { found: true, usedFallback: false, description: "span", rect: opts.probeRect ?? null };
      }
      if (name === "calibrate.hover") {
        return { connected: true, hit: true, hover: true, rect: opts.probeRect ?? null };
      }
      if (name === "calibrate.wheelArm") {
        const rect = opts.canvasRect === undefined ? null : opts.canvasRect;
        if (rect === null) return { present: false, rect: null, armed: false };
        return { present: true, rect, armed: args[0] !== null };
      }
      if (name === "calibrate.wheelRead") return opts.wheelSample ?? null;
      if (name === "calibrate.wheelDisarm") return { disarmed: true };
      throw new Error(`unexpected script ${name}`);
    }) as SessionBridge["execute"],
  };
  return { bridge, scripts };
}

function probeDeps(
  bridge: SessionBridge,
  input: NativeInput,
  geom: WindowGeom,
  occlusion: Rect[],
): ProbeDeps {
  return { bridge, input, geom, occlusion, sleep: async () => undefined };
}

function centerOf(r: Rect): Pt {
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
}

describe("hoverProbe step-off", () => {
  const cases: { edge: string; rect: Rect }[] = [
    { edge: "left", rect: { x: 0, y: 392, width: 20, height: 16 } },
    { edge: "right", rect: { x: 1180, y: 392, width: 20, height: 16 } },
    { edge: "top", rect: { x: 390, y: 0, width: 20, height: 10 } },
    { edge: "bottom", rect: { x: 390, y: 790, width: 20, height: 10 } },
  ];

  for (const { edge, rect } of cases) {
    test(`stays inside the window for a probe on the ${edge} edge`, async () => {
      const { bridge } = fakeBridge({ probeRect: rect });
      const { input, calls } = recordingInput();
      const probe = await hoverProbe(probeDeps(bridge, input, GEOM, []), "");
      // A probe that RAN is the precondition of everything below; the union's other variant
      // means it was skipped, which for these deps would be a defect, not a pass.
      if (!hoverRan(probe)) throw new Error(`probe was skipped: ${probe.skipped}`);

      const moves = calls.filter((c) => c.verb === "move");
      expect(moves).toHaveLength(2);
      const stepOff = moves[0]?.p as Pt;
      // The point actually handed to the native layer must clear the same gate every
      // action point clears — window bounds AND the native occlusion rects.
      expect(() => checkPoint(globalToCss(stepOff, GEOM), GEOM, [])).not.toThrow();
      expect(pointInWindow(stepOff, GEOM)).toBe(true);
      // It really did step off, and it came back to the probe point.
      expect(stepOff).not.toEqual(probe.global);
      expect(moves[1]?.p).toEqual(probe.global);
      expect(probe.ok).toBe(true);
    });
  }

  test("the old fixed +40,+40 step-off was outside the window on the right and bottom edges", () => {
    for (const rect of [cases[1]?.rect, cases[3]?.rect]) {
      const c = centerOf(rect as Rect);
      expect(() => checkPoint({ x: c.x + 40, y: c.y + 40 }, GEOM, [])).toThrow(AgentError);
    }
  });

  test("a step-off over the traffic lights is rejected in favour of one that is not", async () => {
    const occlusion: Rect[] = [{ x: 0, y: 0, width: 100, height: 40 }];
    const rect: Rect = { x: 110, y: 12, width: 20, height: 16 };
    const { bridge } = fakeBridge({ probeRect: rect });
    const { input, calls } = recordingInput();
    await hoverProbe(probeDeps(bridge, input, GEOM, occlusion), "");

    const stepOff = calls.filter((c) => c.verb === "move")[0]?.p as Pt;
    // Candidate 1 (-40, 0) lands at css (80, 20), under the occlusion rect; candidate 2 wins.
    expect(stepOff).toEqual(cssToGlobal({ x: 160, y: 20 }, GEOM));
    expect(() => checkPoint(globalToCss(stepOff, GEOM), GEOM, occlusion)).not.toThrow();
  });

  test("no legal candidate fails CALIBRATION_FAILED with the candidates tried, and posts nothing", async () => {
    const { bridge } = fakeBridge({ probeRect: { x: 20, y: 10, width: 20, height: 20 } });
    const { input, calls } = recordingInput();
    const deps = probeDeps(bridge, input, BOXED_GEOM, BOXED_OCCLUSION);

    // The probe point itself is legal; it is the neighbourhood that is not.
    expect(() => checkPoint({ x: 30, y: 20 }, BOXED_GEOM, BOXED_OCCLUSION)).not.toThrow();

    const err = (await hoverProbe(deps, "").then(
      () => null,
      (e: unknown) => e,
    )) as AgentError | null;
    expect(err).toBeInstanceOf(AgentError);
    expect(err?.code).toBe("CALIBRATION_FAILED");
    expect(err?.message).toContain("step-off");
    const tried = (err?.details as { tried?: Pt[] }).tried;
    expect(tried).toHaveLength(8);
    expect(calls).toEqual([]);
  });

  test("stepOffGlobal refuses rather than guessing when every candidate is illegal", () => {
    expect(() => stepOffGlobal({ geom: BOXED_GEOM, occlusion: BOXED_OCCLUSION }, { x: 30, y: 20 })).toThrow(
      AgentError,
    );
  });

  test("a probe element under a native control is refused before the cursor moves", async () => {
    const rect: Rect = { x: 10, y: 4, width: 40, height: 20 };
    const { bridge } = fakeBridge({ probeRect: rect });
    const { input, calls } = recordingInput();
    const err = (await hoverProbe(probeDeps(bridge, input, GEOM, [{ x: 0, y: 0, width: 100, height: 40 }]), "").then(
      () => null,
      (e: unknown) => e,
    )) as AgentError | null;

    expect(err?.code).toBe("CALIBRATION_FAILED");
    expect(err?.message).toContain("legal point");
    expect(calls).toEqual([]);
  });
});

describe("wheelProbe", () => {
  const CANVAS: Rect = { x: 0, y: 0, width: 200, height: 100 };
  const MOUSE_SAMPLE = { deltaMode: 1, deltaX: 0, deltaY: 3, gapMs: 1e6 };

  test("scrolls at the viewport centre and disarms the page afterwards", async () => {
    const { bridge, scripts } = fakeBridge({ canvasRect: CANVAS, wheelSample: MOUSE_SAMPLE });
    const { input, calls } = recordingInput();
    const probe = await wheelProbe(probeDeps(bridge, input, GEOM, []), 3);

    expect(probe).toEqual({ ok: true, device: "mouse", linesPerNotch: 3, attempts: 1, sample: MOUSE_SAMPLE });
    expect(calls).toEqual([{ verb: "scroll", p: cssToGlobal({ x: 100, y: 50 }, GEOM) }]);
    // Locate, arm, disarm — the disarm is what leaves the page clean.
    expect(scripts.filter((s) => s === "calibrate.wheelArm")).toHaveLength(2);
    expect(scripts).toContain("calibrate.wheelDisarm");
  });

  test("the viewport centre must clear checkPoint, not merely pointInWindow", async () => {
    // Centre css (100, 50) -> global (200, 100): inside the window, but under a native control.
    const occlusion: Rect[] = [{ x: 0, y: 0, width: 120, height: 80 }];
    expect(pointInWindow(cssToGlobal({ x: 100, y: 50 }, GEOM), GEOM)).toBe(true);

    const { bridge, scripts } = fakeBridge({ canvasRect: CANVAS, wheelSample: MOUSE_SAMPLE });
    const { input, calls } = recordingInput();
    const probe = await wheelProbe(probeDeps(bridge, input, GEOM, occlusion), 3);

    expect(probe).toEqual({
      skipped: expect.stringContaining("not a legal point") as unknown as string,
      // Refused before anything was armed or posted.
      posted: false,
    });
    expect(calls).toEqual([]);
    // Located only; never armed, so there is nothing to disarm.
    expect(scripts).toEqual(["calibrate.wheelArm"]);
  });

  test("a viewport that never reports a wheel still leaves the page disarmed", async () => {
    const { bridge, scripts } = fakeBridge({ canvasRect: CANVAS, wheelSample: null });
    const { input } = recordingInput();
    const probe = await wheelProbe(probeDeps(bridge, input, GEOM, []), 3);

    // `posted: true` is the load-bearing half: the sample is written by the same listener that
    // swallows the notch, so no sample means that notch was NOT swallowed and reached the app.
    expect(probe).toEqual({ skipped: "the viewport received no wheel event", posted: true });
    expect(scripts.at(-1)).toBe("calibrate.wheelDisarm");
  });

  test("no viewport arms nothing and asks for no disarm", async () => {
    const { bridge, scripts } = fakeBridge({ canvasRect: null });
    const { input, calls } = recordingInput();
    const probe = await wheelProbe(probeDeps(bridge, input, GEOM, []), 3);

    expect(probe).toEqual({ skipped: "no viewport", posted: false });
    expect(calls).toEqual([]);
    expect(scripts).toEqual(["calibrate.wheelArm"]);
  });

  test("a bridge failure mid-probe still disarms the page", async () => {
    const { bridge, scripts } = fakeBridge({ canvasRect: CANVAS, wheelSample: MOUSE_SAMPLE });
    const { input } = recordingInput();
    const deps = probeDeps(bridge, input, GEOM, []);
    const exploding: ProbeDeps = {
      ...deps,
      input: {
        ...deps.input,
        scroll: async () => {
          throw new Error("helper died mid-notch");
        },
      },
    };

    await expect(wheelProbe(exploding, 3)).rejects.toThrow("helper died mid-notch");
    expect(scripts.at(-1)).toBe("calibrate.wheelDisarm");
  });

  /**
   * Adversarial-review finding: every test above calls these page functions DIRECTLY, so none
   * of them crosses the boundary that actually matters. webdriverio serializes a page function
   * with `Function.prototype.toString` and evaluates the text in the webview, so a reference to
   * anything in this module becomes a ReferenceError in the real page — and would still pass
   * every jsdom test here. Rebuilding them from their own source turns that into a failure.
   */
  test("the wheel page functions are plain, self-contained JavaScript when serialized", () => {
    for (const [name, fn] of [
      ["armWheelInPage", armWheelInPage],
      ["disarmWheelInPage", disarmWheelInPage],
    ] as const) {
      const src = fn.toString();
      expect(src, name).not.toMatch(/\brequire\(|\bimport\b/);
      expect(() => new Function(`return (${src})`), name).not.toThrow();
    }
  });
});
