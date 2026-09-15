/**
 * `AxResolver` — the native half of the resolution ladder.
 *
 * The refusals are the subject. Everything here is driven against in-memory fakes, because what
 * has to be proven is the DECISION the resolver makes with a given reply: that a held ref is never
 * turned back into a query, that a refusal from the helper reaches the caller as itself, and that
 * a point the helper was perfectly happy with is still refused when it is not inside a window this
 * application owns.
 */
import { describe, expect, test } from "bun:test";
import type { AgentError } from "../src/errors.ts";
import { AgentError as Err, isAgentError } from "../src/errors.ts";
import type { GlobalOcclusion } from "../src/geometry/mapping.ts";
import type { WindowGeom } from "../src/geometry/types.ts";
import { AxResolver, axRefStoreFrom } from "../src/native/resolve.ts";
import type { AxTarget } from "../src/native/resolve.ts";
import { axNode, axPoint, FakeAx, fakeWindows, nativeWindow } from "./fixtures/fakeAx.ts";

const PID = 4242;
/** The calibrated main window: global (200,100) 800x600, CGWindowID 42. */
const GEOM: WindowGeom = {
  innerPositionPx: { x: 200, y: 100 },
  innerSizePx: { width: 800, height: 600 },
  scaleFactor: 1,
  nativeBoundsPt: { x: 200, y: 100, width: 800, height: 600 },
  windowId: 42,
};
const MAIN = nativeWindow(42, { x: 200, y: 100, width: 800, height: 600 });
/** A panel that overhangs the main window's bottom edge, as an NSSavePanel does. */
const PANEL = nativeWindow(43, { x: 300, y: 500, width: 600, height: 400 });
const OCCLUSION: GlobalOcclusion = { geom: GEOM, rects: [{ x: 0, y: 0, width: 80, height: 28 }] };

function resolverFor(
  ax: FakeAx,
  windows = [MAIN],
  refs?: ReturnType<typeof axRefStoreFrom>,
): { resolver: AxResolver; ax: FakeAx; windows: ReturnType<typeof fakeWindows> } {
  const w = fakeWindows(windows);
  return {
    resolver: new AxResolver({ ax, windows: w, pid: PID, ...(refs === undefined ? {} : { refs }) }),
    ax,
    windows: w,
  };
}

async function reject(p: Promise<unknown>): Promise<AgentError> {
  const e = await p.then(
    () => null,
    (err: unknown) => err,
  );
  expect(isAgentError(e)).toBe(true);
  return e as AgentError;
}

describe("AxResolver — held refs", () => {
  test("a live ref resolves to the helper's centre, verbatim", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a1e1", center: { x: 500, y: 300 } });
    const { resolver } = resolverFor(ax);
    const r = await resolver.resolve({ ref: "@a1e1" });
    expect(r.source).toBe("ref");
    expect(r.global).toEqual({ x: 500, y: 300 });
    expect(r.point.center).toEqual({ x: 500, y: 300 });
    // No walk happened: a held ref is resolved from the handle, never re-found.
    expect(ax.calls.map((c) => c.verb)).toEqual(["ax_point"]);
  });

  test("a ref from an older generation refuses and does NOT fall back to a find", async () => {
    const ax = new FakeAx();
    ax.findNodes = [axNode({ ref: "@a2e1" })];
    const pool = axRefStoreFrom(2, [axNode({ ref: "@a2e1" })]);
    const { resolver } = resolverFor(ax, [MAIN], pool);
    const e = await reject(resolver.resolve({ ref: "@a1e1" }));
    expect(e.code).toBe("ELEMENT_STALE");
    expect(e.details).toMatchObject({ ref: "@a1e1", generation: 1, poolGeneration: 2 });
    // The whole point: re-finding by role and title would address whichever element now occupies
    // that slot and report success.
    expect(ax.calls).toEqual([]);
  });

  test("an unknown index in the CURRENT generation is ELEMENT_NOT_FOUND, a different fact", async () => {
    const ax = new FakeAx();
    const pool = axRefStoreFrom(2, [axNode({ ref: "@a2e1" })]);
    const { resolver } = resolverFor(ax, [MAIN], pool);
    const e = await reject(resolver.resolve({ ref: "@a2e9" }));
    expect(e.code).toBe("ELEMENT_NOT_FOUND");
    expect(ax.calls).toEqual([]);
  });

  test("a malformed ref never reaches the wire", async () => {
    const ax = new FakeAx();
    const { resolver } = resolverFor(ax);
    const e = await reject(resolver.resolve({ ref: "button#save" }));
    expect(e.code).toBe("INVALID_TARGET");
    expect(ax.calls).toEqual([]);
  });

  test("with no local pool the helper is still the authority, and its refusal is re-dressed", async () => {
    const ax = new FakeAx();
    ax.pointReply = new Err("ELEMENT_MOVING", "'@a1e1' moved between two samples 50 ms apart", {
      details: { verb: "ax_point", helperCode: "ELEMENT_MOVING" },
    });
    const { resolver } = resolverFor(ax);
    const e = await reject(resolver.resolve({ ref: "@a1e1" }));
    expect(e.code).toBe("ELEMENT_MOVING");
    expect(e.remediation).toContain("still animating");
    // The helper's own code survives the re-dressing, so a report can still name the exact refusal.
    expect(e.details).toMatchObject({ helperCode: "ELEMENT_MOVING", ref: "@a1e1", pid: PID });
  });

  test("each helper refusal keeps its code and gains accessibility remediation", async () => {
    for (const code of ["ELEMENT_STALE", "ELEMENT_NOT_FOUND", "ELEMENT_MOVING", "POINT_OUTSIDE_WINDOW"] as const) {
      const ax = new FakeAx();
      ax.pointReply = new Err(code, `helper says ${code}`, { details: { helperCode: code } });
      const { resolver } = resolverFor(ax);
      const e = await reject(resolver.resolve({ ref: "@a1e1" }));
      expect(e.code).toBe(code);
      expect(e.remediation).not.toBe("");
      expect(e.details).toMatchObject({ helperCode: code });
    }
  });

  test("a refusal this layer has no better words for passes through untouched", async () => {
    const ax = new FakeAx();
    ax.pointReply = new Err("NATIVE_INPUT_PERMISSION_DENIED", "no Accessibility grant");
    const { resolver } = resolverFor(ax);
    const e = await reject(resolver.resolve({ ref: "@a1e1" }));
    expect(e.code).toBe("NATIVE_INPUT_PERMISSION_DENIED");
  });
});

describe("AxResolver — queries", () => {
  test("a unique match resolves, and the pool is replaced with the generation find minted", async () => {
    const ax = new FakeAx();
    ax.findGeneration = 5;
    ax.findNodes = [axNode({ ref: "@a5e1", title: "Save" })];
    ax.pointReply = axPoint({ ref: "@a5e1", generation: 5, center: { x: 500, y: 300 } });
    const { resolver } = resolverFor(ax);
    const r = await resolver.resolve({ role: "AXButton", title: "Sav" });
    expect(r.source).toBe("find");
    expect(r.node?.ref).toBe("@a5e1");
    expect(r.generation).toBe(5);
    expect(r.global).toEqual({ x: 500, y: 300 });
    // A find IS a snapshot: it bumps the generation, so the pool it leaves behind must be the
    // new one — a store still claiming the old generation would refuse the ref just minted.
    expect(resolver.refs?.generation).toBe(5);
    expect(ax.calls.map((c) => c.verb)).toEqual(["ax_find", "ax_point"]);
  });

  test("no match is ELEMENT_NOT_FOUND and reports what the walk saw", async () => {
    const ax = new FakeAx();
    ax.findNodes = [];
    const { resolver } = resolverFor(ax);
    const e = await reject(resolver.resolve({ role: "AXButton", title: "Nope" }));
    expect(e.code).toBe("ELEMENT_NOT_FOUND");
    expect(e.details).toMatchObject({ query: { role: "AXButton", title: "Nope" } });
  });

  test("several matches refuse rather than pick one", async () => {
    const ax = new FakeAx();
    ax.findNodes = [axNode({ ref: "@a2e1" }), axNode({ ref: "@a2e2" })];
    const { resolver } = resolverFor(ax);
    const e = await reject(resolver.resolve({ role: "AXButton" }));
    expect(e.code).toBe("INVALID_TARGET");
    expect((e.details as { candidates: unknown[] }).candidates).toHaveLength(2);
    // The pool is still replaced: those refs are real and the caller can now name one.
    expect(resolver.refs?.all()).toHaveLength(2);
  });

  test("an empty query is refused before it can match everything", async () => {
    const ax = new FakeAx();
    const { resolver } = resolverFor(ax);
    const e = await reject(resolver.resolve({} as AxTarget));
    expect(e.code).toBe("INVALID_TARGET");
    expect(ax.calls).toEqual([]);
  });
});

describe("AxResolver — the window gate", () => {
  test("a point in a panel that overhangs the main window is admitted", async () => {
    const ax = new FakeAx();
    // (600, 850) is below the main window's bottom edge; only the panel contains it.
    ax.pointReply = axPoint({ ref: "@a1e1", center: { x: 600, y: 850 } });
    const { resolver } = resolverFor(ax, [PANEL, MAIN]);
    const r = await resolver.resolve({ ref: "@a1e1" }, { occlusion: OCCLUSION });
    expect(r.global).toEqual({ x: 600, y: 850 });
  });

  test("a point in another application's window is refused, and the details say why", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({
      ref: "@a1e1",
      center: { x: 1400, y: 900 },
      window: { known: true, id: 777, source: "axPrivate" },
      frontmost: false,
    });
    const { resolver } = resolverFor(ax, [MAIN]);
    const e = await reject(resolver.resolve({ ref: "@a1e1" }));
    expect(e.code).toBe("POINT_OUTSIDE_WINDOW");
    // A known AX window id that is absent from this app's window list is direct evidence.
    expect(e.details).toMatchObject({ ref: "@a1e1", pid: PID, axWindowId: 777, frontmost: false });
  });

  test("an uncorrelated AX window id reports null, never a number", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a1e1", center: { x: 1400, y: 900 }, window: { known: false, source: "ambiguous" } });
    const { resolver } = resolverFor(ax, [MAIN]);
    const e = await reject(resolver.resolve({ ref: "@a1e1" }));
    expect(e.details).toMatchObject({ axWindowId: null, axWindowIdSource: "ambiguous" });
  });

  test("windows are re-read on every resolve, because a panel may have just opened", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a1e1", center: { x: 500, y: 300 } });
    const { resolver, windows } = resolverFor(ax);
    await resolver.resolve({ ref: "@a1e1" });
    await resolver.resolve({ ref: "@a1e1" });
    expect(windows.listCalls).toBe(2);
  });

  test("an off-screen window cannot contain a clickable point", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a1e1", center: { x: 500, y: 300 } });
    const hidden = nativeWindow(42, { x: 200, y: 100, width: 800, height: 600 }, { onscreen: false });
    const { resolver } = resolverFor(ax, [hidden]);
    const e = await reject(resolver.resolve({ ref: "@a1e1" }));
    expect(e.code).toBe("WINDOW_NOT_FOUND");
  });

  test("by default the native-occlusion rects are not applied: the control IS the target", async () => {
    const ax = new FakeAx();
    // global (220, 114) is css (20, 14) in the main window — squarely on a traffic light, which
    // is a legitimate accessibility target and exactly what this layer exists to reach.
    ax.pointReply = axPoint({ ref: "@a1e1", center: { x: 220, y: 114 }, subrole: "AXCloseButton" });
    const { resolver } = resolverFor(ax);
    expect((await resolver.resolve({ ref: "@a1e1" })).global).toEqual({ x: 220, y: 114 });
    // ...and a caller that IS mapping a webview-derived point can still ask for the strict gate.
    const e = await reject(resolver.resolve({ ref: "@a1e1" }, { occlusion: OCCLUSION }));
    expect(e.code).toBe("ELEMENT_OCCLUDED");
  });
});
