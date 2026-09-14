import { describe, expect, test } from "bun:test";
import { isAgentError } from "../src/errors.ts";
import type { WindowGeom } from "../src/geometry/types.ts";
import { Resolver, refStoreFrom } from "../src/semantic/resolve.ts";
import type { Target } from "../src/semantic/resolve.ts";
import type { SnapNode } from "../src/semantic/snapshot.ts";
import type { BridgeLike, ExecuteOpts } from "../src/semantic/webdriver.ts";

type Canned = Record<string, unknown>;

class FakeBridge implements BridgeLike {
  readonly calls: Array<{ name: string; args: unknown[]; readOnly: boolean }> = [];

  constructor(private readonly canned: Canned) {}

  async execute<T>(
    name: string,
    _fn: (...a: never[]) => T | Promise<T>,
    args: unknown[],
    opts: ExecuteOpts,
  ): Promise<T> {
    this.calls.push({ name, args, readOnly: opts.readOnly });
    const v = this.canned[name];
    if (v === undefined) throw new Error(`unexpected bridged script: ${name}`);
    return (typeof v === "function" ? (v as (a: unknown[]) => unknown)(args) : v) as T;
  }
}

function node(over: Partial<SnapNode> & { ref: string }): SnapNode {
  return {
    fp: `fp-${over.ref}`,
    role: "button",
    name: "Untitled",
    rect: { x: 10, y: 10, width: 40, height: 20 },
    depth: 0,
    state: {},
    css: `body > #${over.ref.slice(1)}`,
    dragRegion: false,
    disabled: false,
    ...over,
  };
}

const NODES: SnapNode[] = [
  node({ ref: "@e1", name: "Sketch", testId: "tool-sketch" }),
  node({ ref: "@e2", name: "Save" }),
  node({ ref: "@e3", name: "Save" }),
  node({ ref: "@e4", role: "textbox", name: "Part name", testId: "part-name" }),
  node({ ref: "@e5", name: "sketch region", testId: "region-btn" }),
  node({ ref: "@e6", role: "status", name: "3 DoF", testId: "sketch-dof", dragRegion: true }),
  node({ ref: "@e7", name: "Row", css: "body > div > button:nth-of-type(2)" }),
];

/** What the in-page check reports for a healthy, still, unobstructed element. */
function healthy(args: unknown[]): Record<string, unknown> {
  const ref = args[0] as string | null;
  return {
    found: true,
    fp: ref === null ? null : `fp-${ref}`,
    fpAvailable: true,
    rect0: { x: 10, y: 10, width: 40, height: 20 },
    rect1: { x: 10, y: 10, width: 40, height: 20 },
    dragRegion: false,
    hitIsSelf: true,
    occluder: null,
  };
}

function resolver(canned: Canned = {}): { r: Resolver; bridge: FakeBridge } {
  const bridge = new FakeBridge(canned);
  return { r: new Resolver(bridge, refStoreFrom(NODES)), bridge };
}

async function codeOf(fn: () => Promise<unknown>): Promise<string> {
  try {
    await fn();
  } catch (e) {
    return isAgentError(e) ? e.code : `not-an-AgentError:${String(e)}`;
  }
  return "no-throw";
}

describe("target priority", () => {
  test("ref wins over every other key", async () => {
    const { r, bridge } = resolver();
    const t = { ref: "@e2", testId: "tool-sketch", role: "button", name: "Sketch" } as unknown as Target;
    const got = await r.resolve(t);
    expect(got.source).toBe("ref");
    expect(got.node?.ref).toBe("@e2");
    expect(bridge.calls).toEqual([]);
  });

  test("role+name wins over testId and text", async () => {
    const { r } = resolver();
    const t = { role: "button", name: "Sketch", testId: "part-name", text: "Save" } as unknown as Target;
    const got = await r.resolve(t);
    expect(got.source).toBe("role");
    expect(got.node?.ref).toBe("@e1");
  });

  test("testId wins over text", async () => {
    const { r } = resolver();
    const got = await r.resolve({ testId: "part-name", text: "Save" } as unknown as Target);
    expect(got.source).toBe("testId");
    expect(got.node?.ref).toBe("@e4");
  });

  test("the centre of the snapshot rect, plus the offset, is the point", async () => {
    const { r } = resolver();
    expect((await r.resolve({ ref: "@e1" })).css).toEqual({ x: 30, y: 20 });
    expect((await r.resolve({ ref: "@e1" }, { offset: { x: 5, y: -2 } })).css).toEqual({ x: 35, y: 18 });
  });
});

describe("name matching", () => {
  test("exact, then case-insensitive, then prefix — the first non-empty tier wins", async () => {
    const { r } = resolver();
    expect((await r.resolve({ role: "button", name: "Sketch" })).node?.ref).toBe("@e1");
    expect((await r.resolve({ role: "button", name: "sketch" })).node?.ref).toBe("@e1");
    expect((await r.resolve({ role: "textbox", name: "part na" })).node?.ref).toBe("@e4");
  });

  test("an ambiguous name refuses and lists the candidates", async () => {
    const { r } = resolver();
    expect(await codeOf(() => r.resolve({ role: "button", name: "Save" }))).toBe("INVALID_TARGET");
    try {
      await r.resolve({ role: "button", name: "Save" });
    } catch (e) {
      if (!isAgentError(e)) throw e;
      const candidates = e.details?.candidates as Array<{ ref: string }>;
      expect(candidates.map((c) => c.ref)).toEqual(["@e2", "@e3"]);
    }
  });

  test("a prefix that matches two elements refuses rather than picking the first", async () => {
    const { r } = resolver();
    expect(await codeOf(() => r.resolve({ role: "button", name: "Sk" }))).toBe("INVALID_TARGET");
  });

  test("role alone refuses when the role is not unique", async () => {
    const { r } = resolver();
    expect(await codeOf(() => r.resolve({ role: "button" }))).toBe("INVALID_TARGET");
    expect((await r.resolve({ role: "textbox" })).node?.ref).toBe("@e4");
  });

  test("text falls through to substring, and a miss is ELEMENT_NOT_FOUND", async () => {
    const { r } = resolver();
    expect((await r.resolve({ text: "Part name" })).source).toBe("text");
    expect((await r.resolve({ text: "region" })).node?.ref).toBe("@e5");
    expect(await codeOf(() => r.resolve({ text: "Nothing here" }))).toBe("ELEMENT_NOT_FOUND");
    expect(await codeOf(() => r.resolve({ ref: "@e99" }))).toBe("ELEMENT_NOT_FOUND");
    expect(await codeOf(() => r.resolve({ testId: "absent" }))).toBe("ELEMENT_NOT_FOUND");
  });
});

describe("css and point targets", () => {
  test("a css target reads its rect from the page", async () => {
    const { r, bridge } = resolver({ "resolve.css": { x: 0, y: 0, width: 200, height: 100 } });
    const got = await r.resolve({ css: "#panel" });
    expect(got.source).toBe("css");
    expect(got.css).toEqual({ x: 100, y: 50 });
    expect(bridge.calls[0]?.args).toEqual(["#panel"]);
    expect(bridge.calls[0]?.readOnly).toBe(true);
  });

  test("a css selector matching nothing is ELEMENT_NOT_FOUND", async () => {
    const { r } = resolver({ "resolve.css": null });
    expect(await codeOf(() => r.resolve({ css: "#gone" }))).toBe("ELEMENT_NOT_FOUND");
  });

  test("webview and window points pass through, and an unchecked resolve stays off the bridge", async () => {
    const { r, bridge } = resolver();
    expect((await r.resolve({ point: { x: 12, y: 34, space: "webview" } })).css).toEqual({ x: 12, y: 34 });
    expect((await r.resolve({ point: { x: 12, y: 34, space: "window" } })).css).toEqual({ x: 12, y: 34 });
    expect(bridge.calls).toEqual([]);
  });

  /**
   * A raw point skipped every gate, so `pointer_drag` could start on the title bar
   * and drag the WINDOW across the desktop while the test believed it was dragging
   * content. One probe, at the exact point the backend will press.
   */
  test("a point target is drag-region gated with one bridged probe", async () => {
    const dragging = resolver({ "resolve.pointDragRegion": true });
    expect(
      await codeOf(() => dragging.r.resolve({ point: { x: 12, y: 34, space: "webview" } }, { forDrag: true })),
    ).toBe("ELEMENT_OCCLUDED");
    expect(dragging.bridge.calls[0]?.name).toBe("resolve.pointDragRegion");
    expect(dragging.bridge.calls[0]?.args).toEqual([12, 34]);
    expect(dragging.bridge.calls[0]?.readOnly).toBe(true);
    try {
      await dragging.r.resolve({ point: { x: 12, y: 34, space: "webview" } }, { forDrag: true });
    } catch (e) {
      if (!isAgentError(e)) throw e;
      expect(e.details?.reason).toBe("tauri-drag-region");
    }

    const clicking = resolver({ "resolve.pointDragRegion": true });
    const clicked = await clicking.r.resolve(
      { point: { x: 12, y: 34, space: "webview" } },
      { forInput: true, offset: { x: 3, y: 4 } },
    );
    expect(clicked.dragRegion).toBe(true);
    expect(clicking.bridge.calls.length).toBe(1);
    // The probe must read the point that will actually be pressed, offset included.
    expect(clicking.bridge.calls[0]?.args).toEqual([15, 38]);

    const clear = resolver({ "resolve.pointDragRegion": false });
    const ok = await clear.r.resolve({ point: { x: 1, y: 2, space: "webview" } }, { forDrag: true });
    expect(ok.dragRegion ?? false).toBe(false);
    expect(ok.css).toEqual({ x: 1, y: 2 });
  });

  test("a global point needs the window geometry, and refuses without it", async () => {
    const { r } = resolver();
    expect(await codeOf(() => r.resolve({ point: { x: 250, y: 160, space: "global" } }))).toBe("INVALID_TARGET");
    const geom: WindowGeom = {
      innerPositionPx: { x: 400, y: 200 },
      innerSizePx: { width: 1600, height: 1200 },
      scaleFactor: 2,
      nativeBoundsPt: { x: 200, y: 100, width: 800, height: 600 },
      windowId: 7,
    };
    const got = await r.resolve({ point: { x: 250, y: 160, space: "global" } }, { geom });
    expect(got.css).toEqual({ x: 50, y: 60 });
  });
});

describe("forInput verification", () => {
  test("a healthy element resolves to the centre of the FRESH rect", async () => {
    const { r, bridge } = resolver({
      "resolve.forInput": (args: unknown[]) => ({
        ...healthy(args),
        rect0: { x: 40, y: 60, width: 80, height: 40 },
        rect1: { x: 40, y: 60, width: 80, height: 40 },
      }),
    });
    const got = await r.resolve({ ref: "@e1" }, { forInput: true, offset: { x: 2, y: 3 } });
    expect(got.css).toEqual({ x: 82, y: 83 });
    expect(got.rect).toEqual({ x: 40, y: 60, width: 80, height: 40 });
    expect(bridge.calls[0]?.name).toBe("resolve.forInput");
    expect(bridge.calls[0]?.args).toEqual(["@e1", null, 2, 3]);
  });

  /**
   * `node.css` is a POSITIONAL path. Re-finding a detached element with it hands
   * back whichever sibling slid into that position, and a positional fingerprint
   * then agrees — a click on the wrong row of a list, reported as success.
   */
  test("the page-side fallback selector is nominal, or absent", async () => {
    const canned = { "resolve.forInput": healthy };
    const byRef = resolver(canned);
    await byRef.r.resolve({ ref: "@e1" }, { forInput: true });
    expect(byRef.bridge.calls[0]?.args).toEqual(["@e1", null, 0, 0]);

    const byTestId = resolver(canned);
    await byTestId.r.resolve({ testId: "part-name" }, { forInput: true });
    expect(byTestId.bridge.calls[0]?.args).toEqual(["@e4", '[data-testid="part-name"]', 0, 0]);

    const byId = resolver(canned);
    await byId.r.resolve({ text: "Part name" }, { forInput: true });
    expect(byId.bridge.calls[0]?.args).toEqual(["@e4", '[data-testid="part-name"]', 0, 0]);

    const positional = resolver(canned);
    await positional.r.resolve({ text: "Row" }, { forInput: true });
    expect(positional.bridge.calls[0]?.args).toEqual(["@e7", null, 0, 0]);

    const byCss = resolver({ ...canned, "resolve.css": { x: 0, y: 0, width: 20, height: 20 } });
    await byCss.r.resolve({ css: "#panel" }, { forInput: true });
    expect(byCss.bridge.calls[1]?.args).toEqual([null, "#panel", 0, 0]);
  });

  test("a rect that did not survive the bridge is ELEMENT_MOVING, not a click at the origin", async () => {
    // The embedded driver serializes NaN and Infinity as `null`, which compares
    // equal to itself and subtracts to 0 — a blanked rect looked perfectly still.
    const blank = { x: null, y: null, width: null, height: null };
    const { r } = resolver({
      "resolve.forInput": (args: unknown[]) => ({ ...healthy(args), rect0: blank, rect1: blank }),
    });
    expect(await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_MOVING");

    const half = resolver({
      "resolve.forInput": (args: unknown[]) => ({
        ...healthy(args),
        rect1: { x: 10, y: 10, width: 40, height: null },
      }),
    });
    expect(await codeOf(() => half.r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_MOVING");
  });

  test("a css rect that did not survive the bridge never becomes a point", async () => {
    const { r } = resolver({ "resolve.css": { x: null, y: 0, width: 10, height: 10 } });
    expect(await codeOf(() => r.resolve({ css: "#panel" }))).toBe("INVALID_TARGET");
  });

  test("a fingerprint mismatch is ELEMENT_STALE, not a click", async () => {
    const { r } = resolver({
      "resolve.forInput": (args: unknown[]) => ({ ...healthy(args), fp: "fp-somethingelse" }),
    });
    expect(await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_STALE");
  });

  test("a page reload wipes the fingerprint helper, which is itself staleness", async () => {
    const { r } = resolver({
      "resolve.forInput": (args: unknown[]) => ({ ...healthy(args), fp: null, fpAvailable: false }),
    });
    expect(await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_STALE");
  });

  test("a detached element is ELEMENT_STALE", async () => {
    const { r } = resolver({
      "resolve.forInput": {
        found: false, fp: null, fpAvailable: true, rect0: null, rect1: null,
        dragRegion: false, hitIsSelf: false, occluder: null,
      },
    });
    expect(await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_STALE");
  });

  test("two rect samples more than 0.5 px apart are ELEMENT_MOVING", async () => {
    const { r } = resolver({
      "resolve.forInput": (args: unknown[]) => ({
        ...healthy(args),
        rect0: { x: 10, y: 10, width: 40, height: 20 },
        rect1: { x: 10, y: 14, width: 40, height: 20 },
      }),
    });
    expect(await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_MOVING");
  });

  test("sub-pixel jitter is not movement", async () => {
    const { r } = resolver({
      "resolve.forInput": (args: unknown[]) => ({
        ...healthy(args),
        rect1: { x: 10.4, y: 10, width: 40, height: 20 },
      }),
    });
    const got = await r.resolve({ ref: "@e1" }, { forInput: true });
    expect(got.css).toEqual({ x: 30.4, y: 20 });
  });

  test("something else at the point is ELEMENT_OCCLUDED and names the occluder", async () => {
    const { r } = resolver({
      "resolve.forInput": (args: unknown[]) => ({
        ...healthy(args),
        hitIsSelf: false,
        occluder: { tag: "div", testId: "modal-scrim", role: "presentation" },
      }),
    });
    expect(await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_OCCLUDED");
    try {
      await r.resolve({ ref: "@e1" }, { forInput: true });
    } catch (e) {
      if (!isAgentError(e)) throw e;
      expect(e.details?.occluder).toEqual({ tag: "div", testId: "modal-scrim", role: "presentation" });
    }
  });

  test("a drag that would start on a drag region is refused, but a click there is not", async () => {
    const canned = {
      "resolve.forInput": (args: unknown[]) => ({ ...healthy(args), dragRegion: true }),
    };
    const drag = resolver(canned);
    expect(await codeOf(() => drag.r.resolve({ ref: "@e6" }, { forInput: true, forDrag: true }))).toBe("ELEMENT_OCCLUDED");
    try {
      await drag.r.resolve({ ref: "@e6" }, { forInput: true, forDrag: true });
    } catch (e) {
      if (!isAgentError(e)) throw e;
      expect(e.details?.reason).toBe("tauri-drag-region");
    }
    const click = resolver(canned);
    expect((await click.r.resolve({ ref: "@e6" }, { forInput: true })).node?.ref).toBe("@e6");
  });

  test("identity is checked before movement, and movement before occlusion", async () => {
    const { r } = resolver({
      "resolve.forInput": (args: unknown[]) => ({
        ...healthy(args),
        fp: "fp-wrong",
        rect1: { x: 99, y: 99, width: 40, height: 20 },
        hitIsSelf: false,
        occluder: { tag: "div", testId: null, role: null },
      }),
    });
    expect(await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }))).toBe("ELEMENT_STALE");
  });

  test("a css target has no fingerprint to compare, but is still stability- and hit-checked", async () => {
    const { r } = resolver({
      "resolve.css": { x: 0, y: 0, width: 20, height: 20 },
      "resolve.forInput": (args: unknown[]) => ({ ...healthy(args), fp: "anything", hitIsSelf: false, occluder: null }),
    });
    expect(await codeOf(() => r.resolve({ css: "#panel" }, { forInput: true }))).toBe("ELEMENT_OCCLUDED");
  });
});

describe("bridged scripts", () => {
  /**
   * webdriverio ships `fn.toString()`, so a TypeScript construct bun failed to
   * strip, or a `require`/`import` sneaking in, becomes a page-side SyntaxError
   * with no stack worth reading. Rebuilding each captured script here turns that
   * into a test failure instead.
   */
  test("every script the Resolver sends is plain, self-contained JavaScript", async () => {
    const seen: Array<{ name: string; src: string }> = [];
    const capture: BridgeLike = {
      async execute<T>(name: string, fn: (...a: never[]) => T | Promise<T>): Promise<T> {
        seen.push({ name, src: fn.toString() });
        return null as T;
      },
    };
    const r = new Resolver(capture, refStoreFrom(NODES));
    await codeOf(() => r.resolve({ css: "#panel" }));
    await codeOf(() => r.resolve({ ref: "@e1" }, { forInput: true }));
    await codeOf(() => r.resolve({ point: { x: 1, y: 2, space: "webview" } }, { forDrag: true }));
    expect(seen.map((s) => s.name)).toEqual([
      "resolve.css",
      "resolve.forInput",
      "resolve.pointDragRegion",
    ]);
    for (const { name, src } of seen) {
      expect(src, name).not.toMatch(/\brequire\(|\bimport\b/);
      expect(() => new Function(`return (${src})`), name).not.toThrow();
    }
  });
});
