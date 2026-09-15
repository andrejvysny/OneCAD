/**
 * Accessibility actuation — the three verbs that changed a stated invariant.
 *
 * The rule this file replaces read "AX reads, never AX acts". The rule it replaces it with is
 * narrower and still binding: CGEvent is the only acceptance-grade actuator. So the tests here
 * check two different things. First, that the actuation is HONEST — `mode:"accessibility"`,
 * never `real_user`, and `delivery` saying the input landed so nothing re-sends it. Second, that
 * it refuses rather than reporting a success the application did not give it: an action the
 * element does not advertise, a value the field would not store, a menu path that names nothing.
 */
import { describe, expect, test } from "bun:test";
import { AgentError } from "../src/errors.ts";
import { GEOM, harness } from "./fixtures/fakeEnv.ts";
import { FakeAx, axMenuItem, axNode, axWindow, nativeWindow } from "./fixtures/fakeAx.ts";

function withAx(): { h: ReturnType<typeof harness>; ax: FakeAx } {
  const ax = new FakeAx();
  ax.windowList = [axWindow()];
  ax.findNodes = [axNode({ ref: "@a2e1", title: "Save" })];
  const h = harness({}, { windows: [nativeWindow(7, GEOM.nativeBoundsPt)], ax });
  return { h, ax };
}

describe("native_press", () => {
  test("presses, and labels itself accessibility rather than real_user", async () => {
    const { h, ax } = withAx();
    const env = await h.call("native_press", { ref: "@a2e1" });
    expect(env.status).toBe("ok");
    expect(env.mode).toBe("accessibility");
    expect(env.backend).toBe("ax");
    expect(env.surface).toBe("native");
    expect(ax.calls.filter((c) => c.verb === "ax_press")).toHaveLength(1);
    // Nothing physical happened, and nothing needed to be in front for it.
    expect(h.input.calls).toEqual([]);
    expect(h.order).not.toContain("focus");
  });

  test("the action landed, so the call is not retry-safe", async () => {
    const { h } = withAx();
    const env = await h.call("native_press", { ref: "@a2e1" });
    expect(env.delivery.inputCompleted).toBe(true);
    expect(env.delivery.retrySafe).toBe(false);
    expect(env.delivery.mayHaveSideEffects).toBe(true);
  });

  test("an unadvertised action refuses instead of reporting a press that did nothing", async () => {
    const { h, ax } = withAx();
    ax.pressError = new AgentError("INVALID_TARGET", "'@a2e1' (AXStaticText) does not offer AXPress");
    const env = await h.call("native_press", { ref: "@a2e1", action: "AXPress" });
    expect(env.error?.code).toBe("INVALID_TARGET");
  });

  /**
   * The window-chrome gate lives in the helper (it reads `AXSubrole`), so what is checkable here is
   * that the opt-in actually reaches it — a parameter dropped in the TypeScript layer would make
   * the guard permanently un-overridable and look like a helper bug.
   */
  test("acceptDisruption is forwarded to the helper, not dropped on the way", async () => {
    const { h, ax } = withAx();
    await h.call("native_press", { ref: "@a2e1", acceptDisruption: "yes" });
    const call = ax.calls.find((c) => c.verb === "ax_press");
    expect((call?.arg as { acceptDisruption?: string }).acceptDisruption).toBe("yes");

    // And it is absent by default, so the guard is on unless a caller opts out of it.
    const { h: h2, ax: ax2 } = withAx();
    await h2.call("native_press", { ref: "@a2e1" });
    const plain = ax2.calls.find((c) => c.verb === "ax_press");
    expect((plain?.arg as { acceptDisruption?: string }).acceptDisruption).toBeUndefined();
  });

  test("a stale ref refuses rather than pressing whatever now occupies the slot", async () => {
    const { h, ax } = withAx();
    ax.pressError = new AgentError("ELEMENT_STALE", "ref '@a2e1' is from snapshot generation 2");
    const env = await h.call("native_press", { ref: "@a2e1" });
    expect(env.error?.code).toBe("ELEMENT_STALE");
  });
});

describe("native_set_value", () => {
  test("writes the value and reports it clean when the field kept it", async () => {
    const { h, ax } = withAx();
    const env = await h.call("native_set_value", { ref: "@a2e1", value: "part.onecad" });
    expect(env.status).toBe("ok");
    expect(env.mode).toBe("accessibility");
    expect((env.data as { matched: boolean }).matched).toBe(true);
    expect(ax.calls.some((c) => c.verb === "ax_set_value")).toBe(true);
  });

  /**
   * The case this exists to catch: the write is accepted and the field stores something else.
   * Silently reporting "ok" here is how a test passes against a filename that was never set.
   */
  test("a value the field changed comes back as a warning naming both strings", async () => {
    const { h, ax } = withAx();
    ax.setValueReadBack = "part";
    const env = await h.call("native_set_value", { ref: "@a2e1", value: "part.onecad" });
    expect(env.status).toBe("warning");
    expect(env.warnings.join(" ")).toContain("part.onecad");
    expect((env.data as { matched: boolean }).matched).toBe(false);
  });

  test("a non-settable attribute refuses instead of dropping the write", async () => {
    const { h, ax } = withAx();
    ax.setValueError = new AgentError("INVALID_TARGET", "AXValue is not settable on '@a2e1'");
    const env = await h.call("native_set_value", { ref: "@a2e1", value: "x" });
    expect(env.error?.code).toBe("INVALID_TARGET");
    expect(env.delivery.retrySafe).toBe(true);
  });
});

describe("native_menu_invoke", () => {
  test("presses the item by title path, with no keyboard involved at all", async () => {
    const { h } = withAx();
    const env = await h.call("native_menu_invoke", { path: ["File", "Save"] });
    expect(env.status).toBe("ok");
    expect(env.mode).toBe("accessibility");
    expect((env.data as { path: string[] }).path).toEqual(["File", "Save"]);
    // This is the whole point over keyboard_shortcut: no key is sent, so no keyboard layout
    // can turn Cmd+Z into Cmd+Y on the way.
    expect(h.input.calls).toEqual([]);
  });

  test("an unknown path refuses and the refusal carries the paths that exist", async () => {
    const { h, ax } = withAx();
    ax.menuPressError = new AgentError(
      "ELEMENT_NOT_FOUND",
      "no menu item at path File > Save as…; available: File > Save | File > Save As…",
    );
    const env = await h.call("native_menu_invoke", { path: ["File", "Save as…"] });
    expect(env.error?.code).toBe("ELEMENT_NOT_FOUND");
    expect(env.error?.message).toContain("Save As…");
  });

  test("a bare menu-bar title is refused by the schema, before it reaches the helper", async () => {
    const { h, ax } = withAx();
    const env = await h.call("native_menu_invoke", { path: ["File"] });
    expect(env.status).toBe("error");
    // Refused at the boundary, so the helper never saw it and nothing opened on screen.
    expect(ax.calls.some((c) => c.verb === "ax_menu_press")).toBe(false);
  });

  test("an ambiguous path refuses rather than pressing the first match", async () => {
    const { h, ax } = withAx();
    ax.menuPressError = new AgentError("INVALID_TARGET", "2 menu items share the path Window > Zoom");
    const env = await h.call("native_menu_invoke", { path: ["Window", "Zoom"] });
    expect(env.error?.code).toBe("INVALID_TARGET");
  });
});

/**
 * The rendering contract between the two menu tools.
 *
 * `native_menu_snapshot` tells the caller to feed its paths to `native_menu_invoke`, so the string
 * it prints must be the string that verb resolves. The fixture deliberately shapes `path` the way
 * the helper does — ENDING with the item's own title — because a fixture that leaves the title out
 * makes a renderer which appends it look correct.
 */
describe("menu paths round-trip between snapshot and invoke", () => {
  function withMenu(): ReturnType<typeof harness> {
    const ax = new FakeAx();
    ax.windowList = [axWindow()];
    ax.menuItems = [
      axMenuItem({ path: ["File"] }),
      axMenuItem({ path: ["File", "Save"] }),
      axMenuItem({ path: ["File", "Get Info"], enabled: false }),
      axMenuItem({ path: ["Edit", "Undo"] }),
    ];
    return harness({}, { windows: [nativeWindow(7, GEOM.nativeBoundsPt)], ax });
  }

  test("a rendered path is exactly the path, with no segment repeated", async () => {
    const h = withMenu();
    const env = await h.call("native_menu_snapshot", {});
    const text = (env.data as { text: string }).text;
    expect(text).toContain("File > Save");
    // The defect this pins: the item's own title appended a second time.
    expect(text).not.toContain("File > Save > Save");
    expect(text).not.toContain("Edit > Undo > Undo");
  });

  test("the separator is the one native_menu_invoke parses, not a typographic lookalike", async () => {
    const h = withMenu();
    const env = await h.call("native_menu_snapshot", {});
    const text = (env.data as { text: string }).text;
    expect(text).not.toContain("\u203a");
  });

  test("a path read out of the snapshot resolves when handed straight back", async () => {
    const h = withMenu();
    const snap = await h.call("native_menu_snapshot", {});
    const line = (snap.data as { text: string }).text
      .split("\n")
      .find((l) => l.startsWith("File > Save"));
    expect(line).toBeDefined();
    const path = (line as string).split("  ")[0]?.split(" > ") ?? [];
    const env = await h.call("native_menu_invoke", { path });
    expect(env.status).toBe("ok");
    expect((env.data as { path: string[] }).path).toEqual(["File", "Save"]);
  });
});

describe("the actuation tools are registered alongside the reads", () => {
  test("all nine native tools exist", () => {
    const { h } = withAx();
    const names = h.names();
    for (const n of [
      "native_snapshot",
      "native_find",
      "native_inspect",
      "native_focused_window",
      "native_modal",
      "native_menu_snapshot",
      "native_press",
      "native_set_value",
      "native_menu_invoke",
    ]) {
      expect(names, n).toContain(n);
    }
  });
});
