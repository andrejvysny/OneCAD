/**
 * The native MCP surface: discovery tools, `{axRef}` as a target, and the native settle.
 *
 * Two things are being proven here and they are not the same. The discovery tools must READ and
 * never act — a query that quietly posted input would be indistinguishable from a click in the
 * journal. And a native target must reach the SAME CGEvent the webview lane uses, through a
 * different gate: `checkGlobalPoint` over every window the app owns, not `checkPoint` over the
 * one calibrated window, because a Save panel legitimately hangs off the bottom of it.
 *
 * Everything runs against the canned `FakeAx`; nothing here talks to the real helper.
 */
import { describe, expect, test } from "bun:test";
import { AgentError } from "../src/errors.ts";
import { axRefStoreFrom } from "../src/native/resolve.ts";
import { axNode, axPoint, FakeAx, nativeWindow } from "./fixtures/fakeAx.ts";
import { fakeNode, GEOM, harness } from "./fixtures/fakeEnv.ts";

/** The calibrated window: content box global (100,50)–(900,550), CGWindowID 77. */
const MAIN = nativeWindow(GEOM.windowId, { x: 100, y: 50, width: 800, height: 500 });
/** A Save panel that overhangs the main window's bottom edge, as a real one does. */
const PANEL = nativeWindow(78, { x: 300, y: 520, width: 600, height: 400 });
/** Inside PANEL, outside MAIN: admitted by checkGlobalPoint, refused by checkPoint. */
const IN_PANEL = { x: 600, y: 700 };

function nativeHarness(ax: FakeAx, windows = [MAIN, PANEL]): ReturnType<typeof harness> {
  return harness({}, { ax, windows });
}

/** A pool exactly as `native_snapshot` would have left it. */
function poolAt(h: ReturnType<typeof harness>, generation: number, refs: string[]): void {
  h.session.axRefs = axRefStoreFrom(
    generation,
    refs.map((ref) => axNode({ ref })),
  );
}

describe("native discovery tools", () => {
  test("native_snapshot renders the tree, keeps the pool, and posts nothing", async () => {
    const ax = new FakeAx();
    ax.snapshotGeneration = 5;
    ax.snapshotNodes = [
      axNode({ ref: "@a5e1", role: "AXWindow", title: "Save", depth: 0, actions: [] }),
      axNode({ ref: "@a5e2", role: "AXTextField", title: "Save As:", value: "Untitled.ocad", depth: 2, actions: [] }),
      axNode({ ref: "@a5e3", role: "AXButton", title: "Save", depth: 2 }),
    ];
    const h = nativeHarness(ax);
    const env = await h.call("native_snapshot", {});

    expect(env.status).toBe("ok");
    expect(env.surface).toBe("native");
    expect(env.backend).toBe("none");
    // A query is always safe to repeat, and must never claim it drove the machine.
    expect(env.delivery).toMatchObject({ phase: "not_started", inputStarted: false, retrySafe: true });
    expect(h.input.calls).toEqual([]);

    const data = env.data as { generation: number; count: number; text: string };
    expect(data.generation).toBe(5);
    expect(data.count).toBe(3);
    // Readable lines, not a wall of JSON: ref, role, title, rect, actions.
    expect(data.text).toContain('@a5e3 AXButton "Save"');
    expect(data.text).toContain("actions=AXPress");
    expect(data.text).toContain('value="Untitled.ocad"');

    // The pool is the one a later pointer_click {axRef} resolves against.
    expect(h.session.axRefs?.generation).toBe(5);
    expect(h.session.axRefs?.get("@a5e2")?.title).toBe("Save As:");
  });

  test("native_find replaces the pool with the generation its own walk minted", async () => {
    const ax = new FakeAx();
    ax.findGeneration = 9;
    ax.findNodes = [axNode({ ref: "@a9e4", title: "Cancel" })];
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e1"]);

    const env = await h.call("native_find", { role: "AXButton", title: "Cancel" });
    expect(env.status).toBe("ok");
    expect(env.surface).toBe("native");
    expect((env.data as { count: number }).count).toBe(1);
    // A find IS a snapshot — it bumps the generation — so the pool is replaced, never merged.
    expect(h.session.axRefs?.generation).toBe(9);
    expect(h.session.axRefs?.get("@a5e1")).toBeUndefined();
    expect(ax.calls.map((c) => c.verb)).toEqual(["ax_find"]);
  });

  test("native_find with no criterion refuses instead of walking the whole tree", async () => {
    const ax = new FakeAx();
    const h = nativeHarness(ax);
    const env = await h.call("native_find", {});
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("INVALID_TARGET");
    expect(ax.calls).toEqual([]);
  });

  test("native_inspect reports where the element is and that the gate admits it", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL, title: "Save" });
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);

    const env = await h.call("native_inspect", { axRef: "@a5e3" });
    expect(env.status).toBe("ok");
    expect(env.surface).toBe("native");
    expect(env.target).toMatchObject({ ref: "@a5e3", role: "AXButton", name: "Save" });
    expect(env.data).toMatchObject({ centre: IN_PANEL, gateAdmitsPoint: true, source: "ref" });
    expect(env.delivery.inputStarted).toBe(false);
    expect(h.input.calls).toEqual([]);
  });

  test("native_focused_window, native_modal and native_menu_snapshot are read-only queries", async () => {
    const ax = new FakeAx();
    const h = nativeHarness(ax);
    for (const tool of ["native_focused_window", "native_modal", "native_menu_snapshot"] as const) {
      const env = await h.call(tool, {});
      expect(env.status).toBe("ok");
      expect(env.surface).toBe("native");
      expect(env.backend).toBe("none");
      expect(env.delivery.inputStarted).toBe(false);
      expect(env.data).toBeDefined();
    }
    expect(ax.calls.map((c) => c.verb)).toEqual(["ax_focused_window", "ax_modal", "ax_menu"]);
    expect(h.input.calls).toEqual([]);
  });

  test("every native tool is registered", () => {
    const names = harness().names();
    for (const tool of [
      "native_snapshot",
      "native_find",
      "native_inspect",
      "native_focused_window",
      "native_modal",
      "native_menu_snapshot",
    ]) {
      expect(names).toContain(tool);
    }
  });
});

describe("{axRef} as a pointer target", () => {
  test("a click on a native element posts a real CGEvent at the accessibility centre", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL });
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);

    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(env.status).toBe("ok");
    // The interaction is identical to a webview click; only the way the point was found differs.
    expect(env.mode).toBe("real_user");
    expect(env.backend).toBe("cgevent");
    expect(env.surface).toBe("native");
    expect(env.target).toMatchObject({ ref: "@a5e3", role: "AXButton" });
    expect(env.resolvedPoint?.global).toEqual(IN_PANEL);

    const click = h.input.calls.find((c) => c.verb === "click");
    expect(click?.point).toEqual(IN_PANEL);
    // No AXAction: accessibility located it, the mouse did the work.
    expect(ax.calls.some((c) => c.verb === "ax_point")).toBe(true);
    // The webview's own gates never ran: there is no DOM element to check or re-hit-test.
    expect(h.bridge.scripts).not.toContain("resolve.forInput");
    expect(h.bridge.scripts).not.toContain("pointer.recheckHit");
  });

  test("a point in a panel that overhangs the main window is admitted", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL });
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);
    const native = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(native.status).toBe("ok");

    // The same point through the webview gate is refused, which is exactly why the native lane
    // needs its own: gating a Save panel on the main window makes it unreachable.
    const webview = await h.call("pointer_drag_path", {
      points: [
        { x: 500, y: 650 },
        { x: 501, y: 650 },
      ],
      space: "webview",
    });
    expect(webview.error?.code).toBe("POINT_OUTSIDE_WINDOW");
  });

  test("a point in another application's window is refused and nothing is posted", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: { x: 1400, y: 900 } });
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);

    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("POINT_OUTSIDE_WINDOW");
    expect(env.delivery).toMatchObject({ phase: "not_started", retrySafe: true });
    expect(h.input.calls.filter((c) => c.verb !== "releaseAll")).toEqual([]);
  });

  test("a stale ref refuses without walking for a replacement", async () => {
    const ax = new FakeAx();
    ax.findNodes = [axNode({ ref: "@a9e1" })];
    const h = nativeHarness(ax);
    poolAt(h, 9, ["@a9e1"]);

    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("ELEMENT_STALE");
    // Re-finding by role and title would click whichever element now occupies that slot.
    expect(ax.calls).toEqual([]);
    expect(h.input.calls.filter((c) => c.verb !== "releaseAll")).toEqual([]);
  });

  test("a malformed accessibility ref is rejected by the schema", async () => {
    const h = nativeHarness(new FakeAx());
    const env = await h.call("pointer_click", { target: { axRef: "@s1e1" } });
    expect(env.status).toBe("error");
    expect(h.input.calls).toEqual([]);
  });

  test("an offset is re-gated, so it cannot walk the click out of every window", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL });
    // The settle walks too, and its walk replaces the pool. Answering with the same generation
    // and the same node keeps @a5e3 live across both clicks, which is what the helper would do
    // for a panel that did not change.
    ax.snapshotGeneration = 5;
    ax.snapshotNodes = [axNode({ ref: "@a5e3" })];
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);

    const inside = await h.call("pointer_click", { target: { axRef: "@a5e3" }, offset: { x: 10, y: 10 } });
    expect(inside.status).toBe("ok");
    expect(inside.resolvedPoint?.global).toEqual({ x: 610, y: 710 });

    const outside = await h.call("pointer_click", { target: { axRef: "@a5e3" }, offset: { x: 5000, y: 0 } });
    expect(outside.status).toBe("error");
    expect(outside.error?.code).toBe("POINT_OUTSIDE_WINDOW");
  });

  test("a session with no pid refuses before posting anything", async () => {
    const ax = new FakeAx();
    const h = nativeHarness(ax);
    h.session.pid = undefined;
    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("APP_NOT_RUNNING");
    expect(env.delivery).toMatchObject({ phase: "not_started", retrySafe: true });
    expect(ax.calls).toEqual([]);
    expect(h.input.calls.filter((c) => c.verb !== "releaseAll")).toEqual([]);
  });

  test("the webview lane refuses a native target rather than dispatching at nothing", async () => {
    const ax = new FakeAx();
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);
    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" }, mode: "webview" });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("INVALID_TARGET");
    expect(env.error?.message).toContain("webview mode");
    expect(ax.calls).toEqual([]);
  });

  test("a hover on a native element is a native action too", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL });
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);
    const env = await h.call("pointer_hover", { target: { axRef: "@a5e3" } });
    expect(env.surface).toBe("native");
    expect(h.input.calls.find((c) => c.verb === "move")?.point).toEqual(IN_PANEL);
  });
});

describe("native settle", () => {
  test("a native action settles on the accessibility tree, not on the DOM idle tuple", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL });
    ax.snapshotGeneration = 5;
    ax.snapshotNodes = [axNode({ ref: "@a5e3" })];
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);

    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(env.state?.settled).toBe(true);
    // The DOM was never polled, so there is no after-revision to report — claiming the
    // before-revision would say the page was observed unchanged when it was not looked at.
    expect(env.state?.afterRevision).toBeUndefined();
    expect(h.order).not.toContain("idle");
    expect(h.bridge.scripts).not.toContain("ui_idle");
    expect(ax.calls.filter((c) => c.verb === "ax_snapshot").length).toBeGreaterThanOrEqual(2);
    expect((env.data as { nativeSettle: { nodeCount: number } }).nativeSettle.nodeCount).toBe(1);
  });

  test("the settle's own walks carry the ref pool forward, the way a ui_snapshot refresh does", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL });
    ax.snapshotGeneration = 6;
    ax.bumpOnSnapshot = true;
    ax.snapshotNodes = [axNode({ ref: "@a6e1" })];
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);

    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(env.status).toBe("ok");
    const walks = ax.calls.filter((c) => c.verb === "ax_snapshot").length;
    // Each walk mints a generation; the pool tracks the LAST one, so it never lags behind the
    // helper and refuses a ref that is actually live.
    expect(h.session.axRefs?.generation).toBe(6 + walks - 1);
    expect((env.data as { nativeSettle: { generation: number } }).nativeSettle.generation).toBe(6 + walks - 1);
  });

  test("a webview action still settles on the DOM and never walks the accessibility tree", async () => {
    const ax = new FakeAx();
    const h = nativeHarness(ax);
    h.session.refs.set("@s1e1", fakeNode({ ref: "@s1e1" }));
    const env = await h.call("pointer_click", { target: { ref: "@s1e1" } });
    expect(env.surface).toBe("webview");
    expect(env.state?.afterRevision).toBe(1);
    expect(h.order).toContain("idle");
    expect(ax.calls).toEqual([]);
  });

  test("a settle that cannot read the tree warns, and the action is still reported delivered", async () => {
    const ax = new FakeAx();
    ax.pointReply = axPoint({ ref: "@a5e3", center: IN_PANEL });
    ax.snapshot = async (): Promise<never> => {
      throw new AgentError("HELPER_FAILED", "the helper died mid-walk");
    };
    const h = nativeHarness(ax);
    poolAt(h, 5, ["@a5e3"]);

    const env = await h.call("pointer_click", { target: { axRef: "@a5e3" } });
    expect(env.status).toBe("warning");
    expect(env.error).toBeUndefined();
    expect(env.state?.settled).toBe(false);
    expect(env.delivery.inputCompleted).toBe(true);
    expect(env.warnings.some((w) => w.includes("proved NOTHING") && w.includes("the helper died mid-walk"))).toBe(true);
    expect(h.input.calls.some((c) => c.verb === "click")).toBe(true);
  });
});

describe("keyboard on a native surface", () => {
  test('surface:"native" skips the WebView focus check and settles on the tree', async () => {
    const ax = new FakeAx();
    ax.snapshotNodes = [axNode({ ref: "@a1e1" })];
    const h = nativeHarness(ax);
    // Nothing editable is focused in the page — with a Save panel up, that is the normal state
    // and refusing on it would block the one flow this lane exists for.
    h.bridge.canned["keyboard.activeElement"] = { editable: false, tag: "body", type: null };

    const env = await h.call("keyboard_type_text", { text: "part-1.ocad", surface: "native" });
    expect(env.status).toBe("ok");
    expect(env.surface).toBe("native");
    expect(env.backend).toBe("cgevent");
    expect(h.input.calls.find((c) => c.verb === "type")).toBeDefined();
    expect(h.bridge.scripts).not.toContain("keyboard.activeElement");
    expect(h.order).not.toContain("idle");
  });

  test("without it, typing into a page that cannot accept text is still refused", async () => {
    const h = nativeHarness(new FakeAx());
    h.bridge.canned["keyboard.activeElement"] = { editable: false, tag: "body", type: null };
    const env = await h.call("keyboard_type_text", { text: "part-1.ocad" });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("INVALID_TARGET");
    expect(h.input.calls.filter((c) => c.verb === "type")).toEqual([]);
  });

  test('a shortcut declared native reports surface "native" and skips the sketch check', async () => {
    const ax = new FakeAx();
    ax.snapshotNodes = [axNode({ ref: "@a1e1" })];
    const h = nativeHarness(ax);
    const env = await h.call("keyboard_shortcut", { combo: "Enter", surface: "native" });
    expect(env.surface).toBe("native");
    expect(h.bridge.scripts).not.toContain("keyboard.sketchActive");
    expect(h.input.calls.find((c) => c.verb === "press")).toBeDefined();
  });

  test("a native settle with no pid says so rather than claiming a quiet it never measured", async () => {
    const h = nativeHarness(new FakeAx());
    h.session.pid = undefined;
    const env = await h.call("keyboard_shortcut", { combo: "Enter", surface: "native" });
    // The keystroke WAS posted, so this is a warning about the evidence, never an error.
    expect(env.status).toBe("warning");
    expect(env.error).toBeUndefined();
    expect(env.state?.settled).toBe(false);
    expect(env.warnings.some((w) => w.includes("native settle did not run"))).toBe(true);
    expect(h.input.calls.some((c) => c.verb === "press")).toBe(true);
  });

  test("keyboard_release_all claims no surface at all", async () => {
    const h = nativeHarness(new FakeAx());
    const env = await h.call("keyboard_release_all", {});
    expect(env.status).toBe("ok");
    expect(env.surface).toBeUndefined();
    expect(env.delivery.inputStarted).toBe(true);
  });
});
