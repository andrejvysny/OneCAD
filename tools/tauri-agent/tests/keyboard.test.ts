import { describe, expect, test } from "bun:test";
import { CROSS_MODE_WARNING, parseCombo } from "../src/mcp/tools/keyboard.ts";
import { isAgentError } from "../src/errors.ts";
import { harness } from "./fixtures/fakeEnv.ts";

const EDITABLE = { editable: true, tag: "input", type: "text" };
const NOT_EDITABLE = { editable: false, tag: "div", type: null };

function keyboardHarness(opts: { editable?: boolean; sketch?: boolean } = {}): ReturnType<typeof harness> {
  const h = harness();
  h.bridge.canned["keyboard.activeElement"] = opts.editable === false ? NOT_EDITABLE : EDITABLE;
  h.bridge.canned["keyboard.sketchActive"] = opts.sketch === true;
  return h;
}

describe("parseCombo", () => {
  test("splits modifiers from the key and normalises aliases", () => {
    expect(parseCombo("Primary+Shift+S")).toEqual({ key: "S", mods: ["Primary", "Shift"] });
    expect(parseCombo("cmd+z")).toEqual({ key: "z", mods: ["Command"] });
    expect(parseCombo("Ctrl+Alt+Delete")).toEqual({ key: "Delete", mods: ["Control", "Option"] });
    expect(parseCombo("Escape")).toEqual({ key: "Escape", mods: [] });
  });

  test("refuses a modifier it does not know", () => {
    try {
      parseCombo("Hyper+S");
      throw new Error("should have refused");
    } catch (e) {
      expect(isAgentError(e) && e.code).toBe("INVALID_TARGET");
    }
  });
});

describe("keyboard_type_text", () => {
  test("types into an editable element", async () => {
    const h = keyboardHarness();
    const env = await h.call("keyboard_type_text", { text: "hello" });
    expect(env.status).toBe("ok");
    expect(h.input.calls.find((c) => c.verb === "type")?.opts).toMatchObject({ text: "hello" });
  });

  test("refuses when nothing editable is focused, and posts nothing", async () => {
    const h = keyboardHarness({ editable: false });
    const env = await h.call("keyboard_type_text", { text: "hello" });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("INVALID_TARGET");
    expect(env.error?.message).toContain("<div>");
    expect(h.input.calls).toEqual([]);
  });

  test("allowShortcuts types anyway", async () => {
    const h = keyboardHarness({ editable: false });
    const env = await h.call("keyboard_type_text", { text: "e", allowShortcuts: true });
    expect(env.status).toBe("ok");
    expect(h.input.calls.some((c) => c.verb === "type")).toBe(true);
  });
});

describe("keyboard_press", () => {
  test("warns about the cross-mode fallback for a bare tool letter inside a sketch", async () => {
    const h = keyboardHarness({ sketch: true });
    const env = await h.call("keyboard_press", { key: "e" });
    expect(env.warnings).toContain(CROSS_MODE_WARNING);
    expect(env.status).toBe("warning");
  });

  test("does not warn outside a sketch, nor for a chord, nor for a non-tool key", async () => {
    const outside = await keyboardHarness({ sketch: false }).call("keyboard_press", { key: "e" });
    expect(outside.warnings).toEqual([]);
    const chord = await keyboardHarness({ sketch: true }).call("keyboard_press", { key: "s", mods: ["Primary"] });
    expect(chord.warnings).toEqual([]);
    const other = await keyboardHarness({ sketch: true }).call("keyboard_press", { key: "Escape" });
    expect(other.warnings).toEqual([]);
  });

  test("repeat presses the key that many times", async () => {
    const h = keyboardHarness();
    await h.call("keyboard_press", { key: "ArrowUp", repeat: 3 });
    expect(h.input.calls.filter((c) => c.verb === "press").length).toBe(3);
  });
});

describe("keyboard_shortcut", () => {
  test('"Primary+S" reaches the adapter as Command on macOS', async () => {
    const h = keyboardHarness();
    const env = await h.call("keyboard_shortcut", { combo: "Primary+S" });
    expect(env.status).toBe("ok");
    const expected = process.platform === "darwin" ? ["Command"] : ["Control"];
    expect(h.input.calls.find((c) => c.verb === "press")?.opts).toEqual({ key: "S", mods: expected });
  });
});

describe("keyboard_release_all", () => {
  test("succeeds even with no session at all", async () => {
    const h = harness();
    const env = await h.call("keyboard_release_all", {});
    expect(env.status).toBe("ok");
    expect(h.input.calls.map((c) => c.verb)).toEqual(["releaseAll"]);
  });

  test("reports rather than fails when the platform refuses", async () => {
    const h = harness();
    h.session.requirePlatform = () => {
      throw new Error("no platform adapter in this session");
    };
    const env = await h.call("keyboard_release_all", {});
    expect(env.status).toBe("warning");
    expect(env.warnings[0]).toContain("nothing was released");
  });
});
