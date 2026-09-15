import { describe, expect, test } from "bun:test";
import { existsSync, writeFileSync } from "node:fs";
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

/**
 * A session without the Screen Recording grant must not ATTEMPT a picture it cannot take, and
 * must not hide that it took none. Every image it does return has to say whether it is evidence.
 */
describe("capture capability", () => {
  function degraded(h: ReturnType<typeof harness>): ReturnType<typeof harness> {
    h.session.capture = { available: false, authoritative: false, reason: "SCREEN_RECORDING_PERMISSION_DENIED" };
    return h;
  }

  test("a state-changing action skips the policy screenshot, says so, and never calls capture", async () => {
    const h = degraded(withNode(harness()));
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("warning");
    expect(env.error).toBeUndefined();
    expect(env.screenshot).toBeUndefined();
    expect(env.warnings.some((w) => w.includes("Screen Recording"))).toBe(true);
    // The click itself was delivered in full; only the picture is missing.
    expect(env.delivery.inputCompleted).toBe(true);
    expect(env.delivery.evidenceIncomplete).toBe(true);
    // The whole point: nothing was attempted, so there is no "capture failed" to explain.
    expect(h.order).not.toContain("capture");
  });

  test("an explicitly requested screenshot is refused before any input is posted", async () => {
    const h = degraded(withNode(harness()));
    const env = await h.call("pointer_hover", { target: TARGET, screenshot: true });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("SCREEN_CAPTURE_PERMISSION_DENIED");
    expect(env.delivery).toMatchObject({ phase: "not_started", retrySafe: true });
    // `releaseAll` is recover's cleanup and posts nothing; no pointer verb ever ran.
    expect(h.input.calls.map((c) => c.verb)).toEqual(["releaseAll"]);
    expect(h.order).not.toContain("capture");
  });

  test("ui_screenshot is refused outright rather than answering with a picture of nothing", async () => {
    const h = degraded(harness());
    const env = await h.call("ui_screenshot", {});
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("SCREEN_CAPTURE_PERMISSION_DENIED");
    expect(env.error?.remediation).toContain("Screen Recording");
    expect(h.order).not.toContain("capture");
  });

  test("with the grant, a captured image is marked authoritative", async () => {
    const h = withNode(harness());
    const click = await h.call("pointer_click", { target: TARGET });
    expect(click.screenshot?.authoritative).toBe(true);
    const shot = await h.call("ui_screenshot", {});
    expect(shot.screenshot?.authoritative).toBe(true);
  });

  test("a capture that reports degradation is never authoritative, grant or no grant", async () => {
    const h = withNode(harness());
    h.session.platform.capture.window = async (_id, outPath) => {
      writeFileSync(outPath, Buffer.from([0x89, 0x50, 0x4e, 0x47]));
      return {
        width: 2880,
        height: 1200,
        pixelScale: 2,
        warning: "captured 2880x1200px does not match the requested 1440x900pt",
        authoritative: false,
        reason: "bounds-mismatch",
      };
    };
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.screenshot?.path).toContain("-after.png");
    expect(env.screenshot?.authoritative).toBe(false);
    // The image is still returned; the caller is told why it is not evidence.
    expect(env.status).toBe("warning");
    expect(env.warnings.some((w) => w.includes("does not match the requested"))).toBe(true);
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

  /**
   * The refusal set is exactly the press-and-hold verbs, and the reason is specific rather than
   * "we did not get round to it": OneCAD calls `setPointerCapture` on pointerdown, a synthetic
   * pointer id cannot be captured, and the call throws inside the app's own handler. A faked
   * lane would report the drag delivered and move nothing.
   */
  test("the press-and-hold verbs refuse a webview lane instead of faking one", async () => {
    const h = withNode(harness());
    for (const [tool, args] of [
      ["pointer_down", { target: TARGET, mode: "webview" }],
      ["pointer_up", { target: TARGET, mode: "webview" }],
      ["pointer_drag", { from: { ref: "@s1e1" }, to: { ref: "@s1e1" }, mode: "webview" }],
      ["pointer_drag_path", { points: [{ x: 10, y: 10 }, { x: 20, y: 20 }], space: "webview", mode: "webview" }],
    ] as const) {
      const env = await h.call(tool, args as Record<string, unknown>);
      expect(env.error?.code, tool).toBe("INVALID_TARGET");
      expect(env.error?.message, tool).toContain("not supported in webview mode");
    }
    expect(h.input.calls).toEqual([]);
  });

  test("hover and scroll DO have a webview lane, and post no native input", async () => {
    const h = withNode(harness());
    h.bridge.canned["webview.pointer"] = { ok: true, tag: "button" };
    h.bridge.canned["webview.wheel"] = { ok: true, tag: "canvas", defaultPrevented: true };

    const hover = await h.call("pointer_hover", { target: TARGET, mode: "webview", dwellMs: 0 });
    expect(hover.mode).toBe("webview");
    expect(hover.backend).toBe("webdriver");
    // The CSS-:hover limit is stated in the envelope, not buried in a doc.
    expect(hover.warnings.some((w) => w.includes("CSS :hover"))).toBe(true);

    const scroll = await h.call("pointer_scroll", { target: TARGET, dy: 2, mode: "webview" });
    expect(scroll.mode).toBe("webview");
    expect(scroll.backend).toBe("webdriver");
    // Positive dy is "zoom in" for the tool and a NEGATIVE DOM deltaY; the sign must survive.
    expect((scroll.data as { deltaY: number }).deltaY).toBeLessThan(0);

    expect(h.input.calls).toEqual([]);
    expect(h.order).not.toContain("focus");
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

/**
 * The envelope must distinguish "the click was never sent" from "the click was sent and the
 * evidence failed". Getting this wrong makes a caller re-send a Save, a Delete or a drag.
 */
describe("delivery semantics", () => {
  test("a delivered click whose screenshot fails is a warning, not an error", async () => {
    const h = withNode(harness());
    h.session.platform.capture.window = async () => {
      throw new AgentError("SCREEN_CAPTURE_PERMISSION_DENIED", "fake capture failed");
    };
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("warning");
    expect(env.error).toBeUndefined();
    expect(env.screenshot).toBeUndefined();
    expect(env.delivery).toMatchObject({
      phase: "postconditions_completed",
      inputStarted: true,
      inputCompleted: true,
      mayHaveSideEffects: true,
      retrySafe: false,
      evidenceIncomplete: true,
    });
    expect(env.warnings.some((w) => w.includes("WAS delivered") && w.includes("SCREEN_CAPTURE_PERMISSION_DENIED"))).toBe(true);
    // The action itself still reports what it observed; only the picture is missing.
    expect(env.state?.settled).toBe(true);
    expect(env.effects).toBeDefined();
  });

  test("a delivered evidence failure is not an MCP protocol error", async () => {
    const h = withNode(harness());
    h.session.platform.capture.window = async () => {
      throw new AgentError("SCREEN_CAPTURE_PERMISSION_DENIED", "fake capture failed");
    };
    // isError would invite a client to auto-retry a click the window server already delivered.
    expect((await h.raw("pointer_click", { target: TARGET, inline: false })).isError).toBe(false);
  });

  test("a refusal before the input is retry-safe and posted nothing", async () => {
    const h = withNode(harness());
    h.session.frontmost = false;
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("error");
    expect(env.delivery).toMatchObject({
      phase: "not_started",
      inputStarted: false,
      inputCompleted: false,
      mayHaveSideEffects: false,
      retrySafe: true,
    });
    expect(env.delivery.evidenceIncomplete).toBeUndefined();
    expect(h.input.calls).toEqual([]);
  });

  test("a failure reading the before-state is retry-safe: the input never ran", async () => {
    const h = withNode(harness());
    h.bridge.canned["ui_revision"] = () => {
      throw new AgentError("BRIDGE_WEDGED", "two consecutive bridged scripts exceeded their budget");
    };
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("error");
    expect(env.delivery.phase).toBe("not_started");
    expect(env.delivery.retrySafe).toBe(true);
    expect(h.input.calls.filter((c) => c.verb === "click")).toEqual([]);
    expect(h.session.reconnects).toBe(1);
  });

  test("a throw mid-gesture may have landed, so it is never retry-safe", async () => {
    const h = withNode(harness());
    h.input.failOn = "click";
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("error");
    expect(env.delivery).toMatchObject({
      phase: "input_started",
      inputStarted: true,
      inputCompleted: false,
      mayHaveSideEffects: true,
      retrySafe: false,
    });
    expect(h.input.calls.at(-1)?.verb).toBe("releaseAll");
  });

  test("a settle failure after a delivered input reports the cause without an error field", async () => {
    const h = withNode(harness());
    // captureBefore reads `ui_revision`; settle reads the whole idle tuple via `ui_idle`.
    h.bridge.canned["ui_idle"] = () => {
      throw new AgentError("SETTLE_TIMEOUT", "fake settle read failed");
    };
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("warning");
    expect(env.error).toBeUndefined();
    expect(env.state).toBeUndefined();
    expect(env.effects).toBeUndefined();
    expect((env.data as { postconditionError: { code: string; message: string } }).postconditionError).toEqual({
      code: "SETTLE_TIMEOUT",
      message: "fake settle read failed",
    });
    expect(env.delivery).toMatchObject({
      phase: "input_completed",
      inputCompleted: true,
      mayHaveSideEffects: true,
      retrySafe: false,
      evidenceIncomplete: true,
    });
    expect(h.input.calls.at(-1)?.verb).toBe("releaseAll");
  });

  test("a wedged bridge AFTER a delivered input still releases and rebuilds, and never errors", async () => {
    const h = withNode(harness());
    h.bridge.canned["ui_idle"] = () => {
      throw new AgentError("BRIDGE_WEDGED", "two consecutive bridged scripts exceeded their budget");
    };
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("warning");
    expect(env.error).toBeUndefined();
    expect(h.session.reconnects).toBe(1);
    expect(env.warnings.some((w) => w.includes("bridge wedged and was rebuilt"))).toBe(true);
    // releaseAll runs FIRST in recover: before the reconnect and before the failure picture.
    expect(h.order.indexOf("releaseAll")).toBeGreaterThan(-1);
    expect(h.order.indexOf("releaseAll")).toBeLessThan(h.order.indexOf("capture"));
    expect(env.delivery.retrySafe).toBe(false);
  });

  test("a delivered failure keeps the tool's own payload beside the cause", async () => {
    const h = withNode(harness());
    h.bridge.canned["ui_idle"] = () => {
      throw new AgentError("SETTLE_TIMEOUT", "fake settle read failed");
    };
    const env = await h.call("pointer_drag_path", {
      points: [
        { x: 100, y: 100 },
        { x: 220, y: 100 },
      ],
      space: "webview",
    });
    const data = env.data as { points: Array<{ x: number }>; postconditionError: { code: string } };
    expect(data.points[0]?.x).toBe(200);
    expect(data.postconditionError.code).toBe("SETTLE_TIMEOUT");
  });

  test("a clean state-changing action is delivered and not retry-safe", async () => {
    const h = withNode(harness());
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.status).toBe("ok");
    expect(env.delivery).toEqual({
      phase: "postconditions_completed",
      inputStarted: true,
      inputCompleted: true,
      mayHaveSideEffects: true,
      retrySafe: false,
    });
  });

  test("a hover reports no side effects even though it posted input", async () => {
    const h = withNode(harness());
    const env = await h.call("pointer_hover", { target: TARGET });
    expect(env.status).toBe("ok");
    expect(env.delivery.mayHaveSideEffects).toBe(false);
    expect(env.delivery.retrySafe).toBe(false);
  });

  test("a query tool posts nothing and is always safe to repeat", async () => {
    const h = harness();
    h.bridge.canned["wait.element"] = { present: true, visible: true, attr: null };
    const env = await h.call("wait_for", {
      condition: { kind: "element", target: { testId: "viewport-canvas" } },
    });
    expect(env.status).toBe("ok");
    expect(env.delivery).toEqual({
      phase: "not_started",
      inputStarted: false,
      inputCompleted: false,
      mayHaveSideEffects: false,
      retrySafe: true,
    });
    expect(h.input.calls).toEqual([]);
  });
});
