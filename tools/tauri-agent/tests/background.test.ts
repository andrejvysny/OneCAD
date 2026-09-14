/**
 * The background policy's promise, tested as a promise rather than as a code path.
 *
 * The claim is: a session started with `interaction:"background"` never moves the cursor, never
 * activates the application and never posts an OS event. A test that only checked the tools would
 * prove the tools behave — the next verb anyone writes would not be covered. So the first group
 * here goes at the SEAM (`platform/refusing.ts`), which is what makes the claim structural, and
 * the second drives the real tool handlers and asserts the helper was never asked to do anything.
 */
import { describe, expect, test } from "bun:test";
import { AgentError, isAgentError } from "../src/errors.ts";
import type { NativeInput, NativeWindows } from "../src/platform/adapter.ts";
import { refusingFocus, refusingInput, backgroundAdapter } from "../src/platform/refusing.ts";
import { GEOM, fakeNode, fakePlatform, harness } from "./fixtures/fakeEnv.ts";
import { FakeAx, axNode, axWindow, nativeWindow } from "./fixtures/fakeAx.ts";

const TARGET = { ref: "@s1e1" };

function codeOf(e: unknown): string {
  return isAgentError(e) ? e.code : `(not an AgentError: ${String(e)})`;
}

describe("the refusing seam", () => {
  /**
   * Every verb that posts an OS event, enumerated from the interface rather than from a list
   * someone maintains by hand — a new input verb that forgets to refuse fails here.
   */
  const POSTING: Array<[keyof NativeInput, unknown[]]> = [
    ["move", [{ x: 1, y: 1 }]],
    ["down", ["left", { x: 1, y: 1 }]],
    ["up", ["left", { x: 1, y: 1 }]],
    ["click", ["left", { x: 1, y: 1 }]],
    ["path", ["left", [{ x: 1, y: 1 }]]],
    ["scroll", [{ x: 1, y: 1 }, { dy: 1 }]],
    ["keyDown", ["a"]],
    ["keyUp", ["a"]],
    ["press", ["a"]],
    ["type", ["hello"]],
  ];

  test("refuses every verb that would post an OS event", async () => {
    const real = fakePlatform([], [], new FakeAx()).input;
    const guarded = refusingInput(real);
    for (const [verb, args] of POSTING) {
      const fn = guarded[verb] as (...a: unknown[]) => Promise<unknown>;
      const code = await fn(...args).then(
        () => "(did not throw)",
        (e: unknown) => codeOf(e),
      );
      expect(code, String(verb)).toBe("BACKGROUND_CAPABILITY_UNAVAILABLE");
    }
    // Nothing reached the real helper: not one verb, not even a partially applied one.
    expect(real.calls).toEqual([]);
  });

  test("the enumerated list is the whole posting surface of NativeInput", () => {
    const real = fakePlatform([], [], new FakeAx()).input;
    // Anything on the interface that is NOT enumerated above must be one of the reads or the
    // teardown verb. A new posting verb added without a refusal shows up here as an unknown name.
    const allowed = new Set(["releaseAll", "cursor", "permissions", "layout", "dispose"]);
    const posting = new Set(POSTING.map(([v]) => v as string));
    for (const key of Object.keys(real) as string[]) {
      if (typeof (real as unknown as Record<string, unknown>)[key] !== "function") continue;
      if (allowed.has(key) || posting.has(key)) continue;
      throw new Error(`NativeInput.${key} is neither refused nor listed as safe in background mode`);
    }
  });

  test("passes through the reads, teardown and release — which can only REDUCE held state", async () => {
    const real = fakePlatform([], [], new FakeAx()).input;
    const guarded = refusingInput(real);
    // Each of these must reach the real adapter and answer, not refuse. `releaseAll` is the
    // one that is recorded, and is the interesting case: it is allowed because it can only
    // hand a key BACK, never take one.
    expect(await guarded.cursor()).toEqual({ x: 500, y: 300 });
    expect((await guarded.permissions()).accessibility).toBe(true);
    await guarded.releaseAll();
    await guarded.dispose();
    expect(real.calls.map((c) => c.verb)).toContain("releaseAll");
  });

  test("refuses focus but keeps both window reads", async () => {
    const real: NativeWindows = {
      list: async () => [nativeWindow(7, GEOM.nativeBoundsPt)],
      isFrontmost: async () => true,
      focus: async () => true,
    };
    const guarded = refusingFocus(real);
    expect(await guarded.list(1)).toHaveLength(1);
    expect(await guarded.isFrontmost(1)).toBe(true);
    expect(await guarded.focus(1).then(() => "(did not throw)", codeOf)).toBe(
      "BACKGROUND_CAPABILITY_UNAVAILABLE",
    );
  });

  test("capture and accessibility are the SAME objects — a background session still sees", () => {
    const real = fakePlatform([], [], new FakeAx());
    const guarded = backgroundAdapter(real);
    // Identity, not equality: wrapping either of these would be a place for a refusal to creep in.
    expect(guarded.capture).toBe(real.capture);
    expect(guarded.ax).toBe(real.ax);
    expect(guarded.name).toBe(real.name);
  });
});

function bg(): ReturnType<typeof harness> {
  const ax = new FakeAx();
  ax.windowList = [axWindow()];
  ax.findNodes = [axNode({ ref: "@a2e1" })];
  const h = harness({}, { windows: [nativeWindow(7, GEOM.nativeBoundsPt)], ax });
  h.session.refs.set("@s1e1", fakeNode({ ref: "@s1e1" }));
  h.session.interactionPolicy = "background";
  h.bridge.canned["webview.pointer"] = { ok: true, tag: "button" };
  h.bridge.canned["webview.wheel"] = { ok: true, tag: "canvas", defaultPrevented: true };
  h.bridge.canned["webview.key"] = { ok: true, tag: "body", defaultPrevented: true };
  return h;
}

describe("background sessions through the real tools", () => {

  /** The headline: drive everything the lane supports and assert the desktop was untouched. */
  test("a whole background run posts no OS event and never activates the app", async () => {
    const h = bg();
    const calls = [
      ["pointer_move", { target: TARGET }],
      ["pointer_hover", { target: TARGET, dwellMs: 0 }],
      ["pointer_click", { target: TARGET }],
      ["pointer_scroll", { target: TARGET, dy: 2 }],
      ["keyboard_press", { key: "Escape" }],
      ["keyboard_shortcut", { combo: "Primary+S" }],
      ["keyboard_down", { key: "Shift" }],
      ["keyboard_up", { key: "Shift" }],
    ] as const;
    for (const [tool, args] of calls) {
      const env = await h.call(tool, args as Record<string, unknown>);
      expect(env.status, tool).not.toBe("error");
      expect(env.mode, tool).toBe("webview");
      expect(env.backend, tool).toBe("webdriver");
      expect(env.interaction.policy, tool).toBe("background");
      expect(env.interaction.foregroundChanged, tool).toBe(false);
      expect(env.interaction.cursorMoved, tool).toBe(false);
    }
    // The two facts that matter to the person at the keyboard.
    expect(h.input.calls).toEqual([]);
    expect(h.order).not.toContain("focus");
  });

  /**
   * The tools whose entire reason for existing is this policy were missing from the honesty
   * check above, which is exactly how a systemic mislabelling shipped green: every bare
   * `okResult` inherited a "foreground" default, so a background session's own proof block said
   * it was a foreground session.
   */
  test("EVERY tool reports the session's real policy, including the ones that skip the pipeline", async () => {
    const h = bg();
    const calls = [
      ["session_status", {}],
      ["window_list", {}],
      ["ui_find", { target: TARGET }],
      ["native_snapshot", {}],
      ["native_modal", {}],
      ["native_menu_snapshot", {}],
      ["native_press", { ref: "@a2e1" }],
      ["native_set_value", { ref: "@a2e1", value: "x" }],
      ["native_menu_invoke", { path: ["File", "Save"] }],
    ] as const;
    for (const [tool, args] of calls) {
      const env = await h.call(tool, args as Record<string, unknown>);
      expect(env.interaction.policy, tool).toBe("background");
      expect(env.interaction.foregroundChanged, tool).toBe(false);
      expect(env.interaction.cursorMoved, tool).toBe(false);
    }
    expect(h.input.calls).toEqual([]);
    expect(h.order).not.toContain("focus");
  });

  test("the accessibility lane is the way past native chrome in background", async () => {
    const h = bg();
    const press = await h.call("native_press", { ref: "@a2e1" });
    expect(press.status).toBe("ok");
    expect(press.mode).toBe("accessibility");
    expect(press.backend).toBe("ax");
    // It acted, so it must not be re-sent — and it needed nothing in front to do it.
    expect(press.delivery.retrySafe).toBe(false);
    expect(h.order).not.toContain("focus");
  });

  test("an explicit mode:\"real_user\" is refused, not quietly downgraded", async () => {
    const h = bg();
    const env = await h.call("pointer_click", { target: TARGET, mode: "real_user" });
    expect(env.error?.code).toBe("BACKGROUND_CAPABILITY_UNAVAILABLE");
    // Refused BEFORE anything was sent, so the caller may safely re-send it elsewhere.
    expect(env.delivery.retrySafe).toBe(true);
    expect(env.delivery.inputStarted).toBe(false);
    expect(h.input.calls).toEqual([]);
  });

  test("the press-and-hold verbs refuse with the reason, naming the verbs that do work", async () => {
    const h = bg();
    for (const [tool, args] of [
      ["pointer_down", { target: TARGET }],
      ["pointer_up", { target: TARGET }],
      ["pointer_drag", { from: TARGET, to: TARGET }],
      ["pointer_drag_path", { points: [{ x: 10, y: 10 }, { x: 20, y: 20 }], space: "webview" }],
    ] as const) {
      const env = await h.call(tool, args as Record<string, unknown>);
      expect(env.error?.code, tool).toBe("BACKGROUND_CAPABILITY_UNAVAILABLE");
      expect(env.delivery.retrySafe, tool).toBe(true);
      const details = env.error?.details as { supportedInBackground?: string[] } | undefined;
      expect(details?.supportedInBackground, tool).toContain("pointer_click");
    }
    expect(h.input.calls).toEqual([]);
  });

  /** The fidelity limits are in the envelope, where a report will find them. */
  test("the lane states what it did NOT prove", async () => {
    const h = bg();
    const hover = await h.call("pointer_hover", { target: TARGET, dwellMs: 0 });
    expect(hover.warnings.some((w) => w.includes("CSS :hover"))).toBe(true);

    const scroll = await h.call("pointer_scroll", { target: TARGET, dy: 1 });
    expect(scroll.warnings.some((w) => w.includes("lines-per-notch"))).toBe(true);

    const key = await h.call("keyboard_shortcut", { combo: "Primary+S" });
    expect(key.warnings.some((w) => w.includes("menu accelerator"))).toBe(true);
  });

  test("keys going to a native panel are refused: the page is not that surface", async () => {
    const h = bg();
    const env = await h.call("keyboard_type_text", { text: "x", surface: "native" });
    expect(env.error?.code).toBe("INVALID_TARGET");
    expect(h.input.calls).toEqual([]);
  });

  test("a foreground session still reports foreground on the pipeline-skipping tools too", async () => {
    const ax = new FakeAx();
    ax.windowList = [axWindow()];
    ax.findNodes = [axNode({ ref: "@a2e1" })];
    const h = harness({}, { windows: [nativeWindow(7, GEOM.nativeBoundsPt)], ax });
    const env = await h.call("native_press", { ref: "@a2e1" });
    expect(env.interaction.policy).toBe("foreground");
  });

  test("a foreground session is completely unchanged by all of this", async () => {
    const h = harness();
    h.session.refs.set("@s1e1", fakeNode({ ref: "@s1e1" }));
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.mode).toBe("real_user");
    expect(env.backend).toBe("cgevent");
    expect(env.interaction.policy).toBe("foreground");
    expect(h.order).toContain("focus");
    expect(h.input.calls.some((c) => c.verb === "click")).toBe(true);
  });

  test("the pipeline reports an activation it actually caused", async () => {
    const h = harness();
    h.session.refs.set("@s1e1", fakeNode({ ref: "@s1e1" }));
    // Models the app having been behind: the gate brings it forward, and the envelope says so.
    h.session.activatesOnFocus = true;
    const env = await h.call("pointer_click", { target: TARGET });
    expect(env.interaction.foregroundChanged).toBe(true);
    expect(env.interaction.cursorMoved).toBe(true);
  });
});

/**
 * The refusals a background session produces must be honest in `delivery`, because that is the
 * field every caller's retry logic reads — including the repo's own `smoke-actions.ts`, which
 * aborts a run on anything that is not retry-safe.
 */
describe("background refusals are retry-safe, because they touched nothing", () => {
  test("keyboard_release_all names itself as the one OS-event exception", async () => {
    const h = bg();
    const env = await h.call("keyboard_release_all", {});
    expect(env.warnings.some((w) => w.includes("one verb"))).toBe(true);
    // It really is allowed through: releasing can only hand a key back, never press one.
    expect(h.input.calls.map((c) => c.verb)).toContain("releaseAll");
  });
});

describe("the background refusal is actionable", () => {
  test("names the background verbs and the foreground escape hatch", () => {
    const e = new AgentError("BACKGROUND_CAPABILITY_UNAVAILABLE", "x");
    expect(e.remediation).toContain("native_menu_invoke");
    expect(e.remediation).toContain('interaction:"foreground"');
    // And it warns what choosing the escape hatch costs, so it is a decision, not a reflex.
    expect(e.remediation).toContain("take over the pointer");
  });
});
