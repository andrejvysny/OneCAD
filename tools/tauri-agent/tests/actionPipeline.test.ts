import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { AgentError } from "../src/errors.ts";
import { fakeNode, harness } from "./fixtures/fakeEnv.ts";

const TARGET = { role: "button", name: "Extrude" };

function withNode(h: ReturnType<typeof harness>): ReturnType<typeof harness> {
  h.session.refs.set("@s1e1", fakeNode({ ref: "@s1e1" }));
  return h;
}

describe("action pipeline", () => {
  test("runs ready → focus → calibrate → resolve → move → recheck → click → settle", async () => {
    const h = withNode(harness());
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("ok");
    expect(env.mode).toBe("real_user");
    expect(env.backend).toBe("cgevent");
    expect(h.order.slice(0, 8)).toEqual([
      "ready",
      "focus",
      "calibrate",
      "resolve",
      "revision",
      "move",
      "recheck",
      "click",
    ]);
    expect(env.target?.ref).toBe("@s1e1");
    // global = innerPosition/scaleFactor + css centre = (100,50) + (90,70)
    expect(env.resolvedPoint).toEqual({ global: { x: 190, y: 120 }, css: { x: 90, y: 70 } });
    expect(env.state?.settled).toBe(true);
  });

  test("a window that is not frontmost posts no input at all", async () => {
    const h = withNode(harness());
    h.session.frontmost = false;
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("WINDOW_NOT_FOREGROUND");
    expect(h.input.calls).toEqual([]);
  });

  test("something opening under the cursor during the move refuses the click", async () => {
    const h = withNode(harness());
    h.bridge.hitIsSelf = false;
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("ELEMENT_OCCLUDED");
    expect(env.error?.details?.reason).toBe("post-move-occlusion");
    expect(h.input.calls.map((c) => c.verb)).toEqual(["move", "releaseAll"]);
  });

  test("a throw after input started releases every held button and modifier", async () => {
    const h = withNode(harness());
    h.input.failOn = "click";
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("error");
    expect(h.input.calls.at(-1)?.verb).toBe("releaseAll");
  });

  test("a wedged bridge rebuilds the session once and does not retry the input", async () => {
    const h = withNode(harness());
    h.bridge.canned["ui_revision"] = () => {
      throw new AgentError("BRIDGE_WEDGED", "two consecutive bridged scripts exceeded their budget");
    };
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("BRIDGE_WEDGED");
    expect(h.session.reconnects).toBe(1);
    expect(h.input.calls.filter((c) => c.verb === "click").length).toBe(0);
  });

  test("effects report the console, log, window and geometry deltas", async () => {
    const h = withNode(harness());
    h.bridge.consoleErrors = 2;
    h.session.moved = true;
    let first = true;
    h.bridge.canned["effects.consoleErrors"] = () => {
      const v = first ? 2 : 5;
      first = false;
      return v;
    };
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.effects).toEqual({ consoleErrors: 3, logErrors: 0, newWindows: 0, windowMoved: true });
    expect(env.status).toBe("warning");
    expect(env.warnings.some((w) => w.includes("3 console error"))).toBe(true);
  });
});

describe("screenshot policy", () => {
  test('"state_changing" captures a click but not a hover', async () => {
    const h = withNode(harness());
    const click = await h.call("pointer_click", { target: TARGET });
    expect(click.screenshot?.path).toContain("-after.png");
    expect(existsSync(click.screenshot?.path ?? "")).toBe(true);
    const hover = await h.call("pointer_hover", { target: TARGET });
    expect(hover.screenshot).toBeUndefined();
  });

  test("screenshot:true overrides the policy for a hover", async () => {
    const h = withNode(harness());
    const hover = await h.call("pointer_hover", { target: TARGET, screenshot: true });
    expect(hover.screenshot?.path).toContain("-after.png");
  });

  test('"never" captures nothing, not even on failure', async () => {
    const h = withNode(harness({ screenshots: { policy: "never", previewMaxPx: 800 } }));
    const ok = await h.call("pointer_click", { target: TARGET });
    expect(ok.screenshot).toBeUndefined();
    h.input.failOn = "click";
    const bad = await h.call("pointer_click", { target: TARGET });
    expect(bad.status).toBe("error");
    expect(bad.screenshot).toBeUndefined();
  });

  test('"on_failure" captures only the failure', async () => {
    const h = withNode(harness({ screenshots: { policy: "on_failure", previewMaxPx: 800 } }));
    expect((await h.call("pointer_click", { target: TARGET })).screenshot).toBeUndefined();
    h.input.failOn = "click";
    const bad = await h.call("pointer_click", { target: TARGET });
    expect(bad.screenshot?.path).toContain("-failure.png");
  });

  test('"always" captures a hover too', async () => {
    const h = withNode(harness({ screenshots: { policy: "always", previewMaxPx: 800 } }));
    expect((await h.call("pointer_hover", { target: TARGET })).screenshot?.path).toContain("-after.png");
  });
});

describe("pointer verbs", () => {
  test("drag interpolates between the two resolved points and holds the button", async () => {
    const h = withNode(harness());
    h.session.refs.set("@s1e2", fakeNode({ ref: "@s1e2", name: "Canvas", css: "#canvas", rect: { x: 200, y: 200, width: 400, height: 300 } }));
    const env = await h.call("pointer_drag", {
      from: { ref: "@s1e1" },
      to: { ref: "@s1e2" },
      button: "right",
      mods: ["Shift"],
      steps: 4,
    });
    expect(env.status).toBe("ok");
    const call = h.input.calls.find((c) => c.verb === "path");
    expect(call?.button).toBe("right");
    const opts = call?.opts as { points: Array<{ x: number; y: number }>; mods: string[] };
    expect(opts.points.length).toBe(5);
    expect(opts.mods).toEqual(["Shift"]);
  });

  test("drag_path maps every point in its declared space and refuses one outside the window", async () => {
    const h = withNode(harness());
    const ok = await h.call("pointer_drag_path", {
      points: [
        { x: 100, y: 100 },
        { x: 220, y: 100 },
      ],
      space: "webview",
      button: "right",
      mods: ["Shift"],
    });
    expect((ok.data as { points: Array<{ x: number }> }).points[0]?.x).toBe(200);

    const outside = await h.call("pointer_drag_path", {
      points: [
        { x: 100, y: 100 },
        { x: 5000, y: 100 },
      ],
      space: "webview",
    });
    expect(outside.status).toBe("error");
    expect(outside.error?.code).toBe("POINT_OUTSIDE_WINDOW");
  });

  test("scroll turns notches into spaced wheel events and warns on a failed wheel probe", async () => {
    const h = withNode(harness());
    h.session.wheelProbe = { ok: false, device: "trackpad", linesPerNotch: 3 };
    const env = await h.call("pointer_scroll", { target: TARGET, dy: 3 });
    expect(h.order).toContain("wheelProbe");
    const scrolls = h.input.calls.filter((c) => c.verb === "scroll");
    expect(scrolls.length).toBe(3);
    expect((scrolls[0]?.opts as { delta: { dy: number } }).delta.dy).toBe(3);
    expect(env.warnings.some((w) => w.includes("mouse wheel"))).toBe(true);
  });

  test("notches:false sends the raw line delta once", async () => {
    const h = withNode(harness());
    await h.call("pointer_scroll", { target: TARGET, dy: -40, notches: false });
    const scrolls = h.input.calls.filter((c) => c.verb === "scroll");
    expect(scrolls.length).toBe(1);
    expect((scrolls[0]?.opts as { delta: { dy: number } }).delta.dy).toBe(-40);
  });

  test("the verbs with no webview lane refuse instead of faking one", async () => {
    const h = withNode(harness());
    for (const [tool, args] of [
      ["pointer_hover", { target: TARGET, mode: "webview" }],
      ["pointer_scroll", { target: TARGET, dy: 1, mode: "webview" }],
      ["pointer_drag", { from: { ref: "@s1e1" }, to: { ref: "@s1e1" }, mode: "webview" }],
    ] as const) {
      const env = await h.call(tool, args as Record<string, unknown>);
      expect(env.error?.code).toBe("INVALID_TARGET");
      expect(env.error?.message).toContain("not supported in webview mode");
    }
    expect(h.input.calls).toEqual([]);
  });

  test("webview mode is labelled as the webdriver backend", async () => {
    const h = withNode(harness());
    h.bridge.canned["webview.pointer"] = { ok: true, tag: "button" };
    const env = await h.call("pointer_click", { target: TARGET, mode: "webview" });
    expect(env.mode).toBe("webview");
    expect(env.backend).toBe("webdriver");
    expect(h.input.calls.filter((c) => c.verb === "click")).toEqual([]);
    expect(h.order).not.toContain("focus");
  });
});
