import { describe, expect, test } from "bun:test";
import type { IdleReading } from "../src/semantic/idleScript.ts";
import type { BridgeLike, ExecuteOpts } from "../src/semantic/webdriver.ts";
import type { AxNode, AxSnapshotResult } from "../src/platform/adapter.ts";
import type { AxSettleDeps, SettleSignals } from "../src/mcp/tools/settle.ts";
import { settle, settleNative, UNINSTRUMENTED_WARNING } from "../src/mcp/tools/settle.ts";
import { axNode } from "./fixtures/fakeAx.ts";

const ALL_SIGNALS: SettleSignals = {
  regenBusy: true,
  geometryPending: true,
  documentRevision: true,
  frames: true,
};

/** Nothing in flight, everything published — the reading a quiet app produces. */
function idle(over: Partial<IdleReading> = {}): IdleReading {
  return {
    rev: 7,
    regenBusy: 0,
    geometryPending: false,
    documentRevision: 1,
    frames: 1,
    camera: null,
    ...over,
  };
}

/** Replays a scripted sequence of idle readings; the last one repeats forever. */
class IdleBridge implements BridgeLike {
  reads = 0;
  constructor(private readonly readings: IdleReading[]) {}
  async execute<T>(_name: string, _fn: unknown, _args: unknown[], _opts: ExecuteOpts): Promise<T> {
    const r = this.readings[Math.min(this.reads, this.readings.length - 1)] as IdleReading;
    this.reads += 1;
    return r as T;
  }
}

/** Revisions only, everything else quiet — the shape the old DOM-only settle watched. */
function revBridge(revs: number[]): IdleBridge {
  return new IdleBridge(revs.map((rev) => idle({ rev })));
}

const nap = async (): Promise<void> => {};
const CFG = { frameMs: 1, quietMs: 1, timeoutMs: 30, signals: ALL_SIGNALS };
/** The native lane takes the same three knobs and no signal block. */
const NATIVE_CFG = { frameMs: 1, quietMs: 1, timeoutMs: 30 };

describe("settle", () => {
  /**
   * -1 is what `readRevision` answers on a page with no mutation observer. Two of those compare
   * equal, so before this guard an uninstrumented page reported a clean, instant settle having
   * watched nothing whatsoever — the exact false negative that made an action look verified.
   */
  test("an uninstrumented page is NOT reported as quiet", async () => {
    const bridge = revBridge([-1, -1]);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(false);
    expect(out.afterRevision).toBe(-1);
    expect(out.warning).toBe(UNINSTRUMENTED_WARNING);
  });

  test("the uninstrumented warning says the action was still sent", async () => {
    const out = await settle(revBridge([-1, -1]), CFG, nap);
    expect(out.warning).toMatch(/WAS sent/);
    expect(out.warning).toMatch(/instrumentWarning/);
  });

  test("settles when two consecutive reads agree", async () => {
    const bridge = revBridge([7, 7]);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(true);
    expect(out.afterRevision).toBe(7);
    expect(out.warning).toBeUndefined();
    expect(bridge.reads).toBe(2);
  });

  test("keeps polling while the revision keeps moving", async () => {
    const bridge = revBridge([1, 2, 3, 3]);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(true);
    expect(out.afterRevision).toBe(3);
    expect(bridge.reads).toBe(4);
  });

  test("warns instead of throwing when the UI never goes quiet", async () => {
    let rev = 0;
    const bridge: BridgeLike = {
      execute: async <T,>() => {
        rev += 1;
        return idle({ rev }) as T;
      },
    };
    const out = await settle(bridge, { ...CFG, timeoutMs: 5 }, (ms) => Bun.sleep(ms));
    expect(out.settled).toBe(false);
    expect(out.warning).toContain("still changing");
    expect(out.afterRevision).toBeGreaterThan(0);
  });

  /**
   * A click on Extrude runs frontend → Rust → the OCCT worker → mesh generation → a Three.js
   * upload. The DOM goes quiet long before that finishes, so a revision-only settle declared
   * the action done mid-transaction; `regenBusy` is the app's own count of in-flight regens.
   */
  test("keeps polling while regenBusy > 0, and settles once it reaches 0 with the tuple stable", async () => {
    const bridge = new IdleBridge([
      idle({ regenBusy: 2 }),
      idle({ regenBusy: 2 }),
      idle({ regenBusy: 2 }),
      idle({ regenBusy: 0 }),
      idle({ regenBusy: 0 }),
    ]);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(true);
    // A stable-but-busy reading is NOT quiet: reads 2 and 3 agreed and still did not settle.
    expect(bridge.reads).toBe(5);
    expect(out.idle?.regenBusy).toBe(0);
    expect(out.warning).toBeUndefined();
  });

  /**
   * The orbit case. Rendering is on-demand, so the camera drag repaints WebGL with ZERO DOM
   * mutations — a revision-only settle would have stopped at the second read having watched
   * nothing that was actually moving.
   */
  test("an orbit that changes only the frame count is not reported settled early", async () => {
    const readings = [idle({ frames: 1 }), idle({ frames: 2 }), idle({ frames: 3 }), idle({ frames: 3 })];
    const bridge = new IdleBridge(readings);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(true);
    expect(bridge.reads).toBe(4);
    expect(out.idle?.frames).toBe(3);

    // Same sequence, `frames` switched off: it settles at the second read, which is exactly
    // the false quiet this member exists to prevent.
    const blind = new IdleBridge(readings);
    const blindOut = await settle(blind, { ...CFG, signals: { ...ALL_SIGNALS, frames: false } }, nap);
    expect(blindOut.settled).toBe(true);
    expect(blind.reads).toBe(2);
  });

  test("geometryPending blocks a settle even when every other member is stable", async () => {
    const bridge = new IdleBridge([idle({ geometryPending: true })]);
    const out = await settle(bridge, { ...CFG, timeoutMs: 5 }, (ms) => Bun.sleep(ms));
    expect(out.settled).toBe(false);
    expect(out.moving).toEqual(["geometryPending"]);
    expect(out.warning).toContain("geometryPending=true");
  });

  /**
   * Degrading to DOM-only settling is the defect being fixed, so an absent signal must not
   * become a silent pass: the settle still returns (blocking on it would hang every action
   * on a build that lacks it) but says plainly what it could not watch.
   */
  test("a null frames still settles, and says the signal was unavailable", async () => {
    const bridge = new IdleBridge([idle({ frames: null })]);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(true);
    expect(out.unavailable).toEqual(["frames"]);
    expect(out.warning).toContain("frames");
    expect(out.warning).toContain("?vpdebug");
  });

  test("a signal that is switched off is not reported as unavailable", async () => {
    const bridge = new IdleBridge([idle({ frames: null })]);
    const out = await settle(bridge, { ...CFG, signals: { ...ALL_SIGNALS, frames: false } }, nap);
    expect(out.settled).toBe(true);
    expect(out.unavailable).toBeUndefined();
    expect(out.warning).toBeUndefined();
  });

  /**
   * "It did not settle" without saying WHAT moved is the uninformative failure this package
   * exists to remove — the member name is the whole diagnosis.
   */
  test("the timeout warning names the member that was still moving", async () => {
    let frames = 0;
    const bridge: BridgeLike = {
      execute: async <T,>() => {
        frames += 1;
        return idle({ frames }) as T;
      },
    };
    const out = await settle(bridge, { ...CFG, timeoutMs: 5 }, (ms) => Bun.sleep(ms));
    expect(out.settled).toBe(false);
    expect(out.moving).toEqual(["frames"]);
    expect(out.warning).toContain("still moving: frames");
    // The DOM revision never moved, so naming it would be a lie.
    expect(out.warning).not.toContain("rev ");
  });

  test("the timeout warning names a busy member as well as a moving one", async () => {
    let n = 0;
    const bridge: BridgeLike = {
      execute: async <T,>() => {
        n += 1;
        return idle({ rev: n, regenBusy: 3 }) as T;
      },
    };
    const out = await settle(bridge, { ...CFG, timeoutMs: 5 }, (ms) => Bun.sleep(ms));
    expect(out.settled).toBe(false);
    expect(out.moving).toContain("rev");
    expect(out.moving).toContain("regenBusy");
    expect(out.warning).toContain("still busy: regenBusy=3");
  });
});

/**
 * The native lane. A Save panel changes nothing in the DOM, so the idle tuple is stable from the
 * first read and the webview settle would report a quiet it never observed. These prove the two
 * properties that carry over: it goes quiet on a STABLE accessibility subtree, and when it does
 * not, the warning names what was still moving rather than saying "it did not settle".
 */
describe("settleNative", () => {
  const PID = 4242;

  /** Replays scripted walks; the last one repeats forever, as IdleBridge does for the DOM. */
  function axFake(walks: AxNode[][], opts: { truncated?: boolean } = {}): AxSettleDeps & { reads: number } {
    const self = {
      reads: 0,
      pid: PID,
      ax: {
        snapshot: async (): Promise<AxSnapshotResult> => {
          const nodes = walks[Math.min(self.reads, walks.length - 1)] as AxNode[];
          self.reads += 1;
          return {
            pid: PID,
            // Every walk mints a generation, exactly as the helper does.
            generation: self.reads,
            nodes,
            total: nodes.length,
            truncated: opts.truncated === true,
            stopReason: opts.truncated === true ? "maxNodes" : "complete",
            windowSource: "focused",
            window: null,
          };
        },
      },
    };
    return self;
  }

  const panel = (y: number): AxNode[] => [
    axNode({ ref: "@a1e1", role: "AXWindow", title: "Save", bounds: { x: 300, y, width: 600, height: 400 } }),
    axNode({ ref: "@a1e2", role: "AXButton", title: "Save", bounds: { x: 800, y: y + 360, width: 80, height: 24 } }),
  ];

  test("goes quiet when two consecutive walks hash the same", async () => {
    const deps = axFake([panel(520)]);
    const out = await settleNative(deps, NATIVE_CFG, nap);
    expect(out.settled).toBe(true);
    expect(out.warning).toBeUndefined();
    expect(deps.reads).toBe(2);
    expect(out.ax).toMatchObject({ nodeCount: 2, windowSource: "focused" });
    // The DOM was never read, so there is no revision to report.
    expect(out.afterRevision).toBeUndefined();
  });

  test("a ref generation that changes every walk does NOT stop it settling", async () => {
    // The refs differ between walks by construction (each walk mints a generation); hashing them
    // would mean nothing ever goes quiet.
    const deps = axFake([[axNode({ ref: "@a1e1" })], [axNode({ ref: "@a2e1" })], [axNode({ ref: "@a3e1" })]]);
    const out = await settleNative(deps, NATIVE_CFG, nap);
    expect(out.settled).toBe(true);
  });

  test("keeps polling while a sheet is still sliding in", async () => {
    const deps = axFake([panel(300), panel(400), panel(520), panel(520)]);
    const out = await settleNative(deps, NATIVE_CFG, nap);
    expect(out.settled).toBe(true);
    expect(deps.reads).toBe(4);
  });

  test("the timeout warning names what was still moving, and is never fatal", async () => {
    let y = 300;
    const deps: AxSettleDeps = {
      pid: PID,
      ax: {
        snapshot: async (): Promise<AxSnapshotResult> => {
          y += 20;
          const nodes = panel(y);
          return {
            pid: PID,
            generation: 1,
            nodes,
            total: nodes.length,
            truncated: false,
            stopReason: "complete",
            windowSource: "focused",
            window: null,
          };
        },
      },
    };
    const out = await settleNative(deps, { ...NATIVE_CFG, timeoutMs: 5 }, (ms) => Bun.sleep(ms));
    expect(out.settled).toBe(false);
    expect(out.moving?.length).toBeGreaterThan(0);
    expect(out.warning).toContain("still changing");
    // The element that moved is named with its role, title and both rects.
    expect(out.warning).toContain("AXWindow");
    expect(out.warning).toContain("no longer present");
    expect(out.warning).toContain("WAS sent");
  });

  test("a node appearing is named as such, and the node count is reported", async () => {
    let extra = 0;
    const deps: AxSettleDeps = {
      pid: PID,
      ax: {
        snapshot: async (): Promise<AxSnapshotResult> => {
          extra += 1;
          const nodes = [...panel(520), ...Array.from({ length: extra }, (_, i) => axNode({ ref: `@a1e${i + 3}`, title: `row ${i}` }))];
          return {
            pid: PID,
            generation: 1,
            nodes,
            total: nodes.length,
            truncated: false,
            stopReason: "complete",
            windowSource: "focused",
            window: null,
          };
        },
      },
    };
    const out = await settleNative(deps, { ...NATIVE_CFG, timeoutMs: 5 }, (ms) => Bun.sleep(ms));
    expect(out.settled).toBe(false);
    expect(out.warning).toMatch(/node count \d+→\d+/);
    expect(out.warning).toContain("now present");
  });

  /** Degrading silently is the defect; a walk that hit its cap hashed only part of the tree. */
  test("a truncated walk settles, and says the quiet it found is partial", async () => {
    const out = await settleNative(axFake([panel(520)], { truncated: true }), NATIVE_CFG, nap);
    expect(out.settled).toBe(true);
    expect(out.ax?.truncated).toBe(true);
    expect(out.warning).toContain("only the first");
    expect(out.warning).toContain("maxNodes");
  });

  test("a tree that cannot be read at all warns rather than throwing", async () => {
    const deps: AxSettleDeps = {
      pid: PID,
      ax: {
        snapshot: async (): Promise<AxSnapshotResult> => {
          throw new Error("accessibility is not permitted");
        },
      },
    };
    const out = await settleNative(deps, NATIVE_CFG, nap);
    expect(out.settled).toBe(false);
    expect(out.warning).toContain("proved NOTHING");
    expect(out.warning).toContain("accessibility is not permitted");
  });

  test("every walk is handed to the caller, so an AX ref pool can follow the generation", async () => {
    const seen: number[] = [];
    const deps = { ...axFake([panel(520)]), onWalk: (w: AxSnapshotResult) => seen.push(w.generation) };
    const out = await settleNative(deps, NATIVE_CFG, nap);
    expect(out.settled).toBe(true);
    expect(seen).toEqual([1, 2]);
  });
});
