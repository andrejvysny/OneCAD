import { describe, expect, test } from "bun:test";
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { harness } from "./fixtures/fakeEnv.ts";

function present(visible: boolean): { present: boolean; visible: boolean; attr: string | null } {
  return { present: true, visible, attr: null };
}

describe("wait_for", () => {
  test("element resolves as soon as the selector is visible", async () => {
    const h = harness();
    h.bridge.canned["wait.element"] = present(true);
    const env = await h.call("wait_for", { condition: { kind: "element", target: { testId: "viewport-canvas" } } });
    expect(env.status).toBe("ok");
    expect((env.data as { kind: string }).kind).toBe("element");
  });

  test("element polls until it appears", async () => {
    const h = harness();
    let calls = 0;
    h.bridge.canned["wait.element"] = () => {
      calls += 1;
      return calls < 3 ? { present: false, visible: false, attr: null } : present(true);
    };
    const env = await h.call("wait_for", { condition: { kind: "element", target: { testId: "history-item" } } });
    expect(env.status).toBe("ok");
    expect(calls).toBe(3);
  });

  test("element_hidden waits for it to go away", async () => {
    const h = harness();
    h.bridge.canned["wait.element"] = { present: false, visible: false, attr: null };
    const env = await h.call("wait_for", {
      condition: { kind: "element_hidden", target: { testId: "regen-busy" } },
      timeoutMs: 500,
    });
    expect(env.status).toBe("ok");
  });

  test("a timeout is an ACTION_TIMEOUT envelope carrying what was last observed", async () => {
    const h = harness();
    h.bridge.canned["wait.element"] = { present: false, visible: false, attr: null };
    const env = await h.call("wait_for", {
      condition: { kind: "element", target: { testId: "never-appears" } },
      timeoutMs: 120,
    });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("ACTION_TIMEOUT");
    expect(env.error?.details?.lastObserved).toMatchObject({ present: false });
    expect((env.data as { observed: unknown }).observed).toBeDefined();
  });

  test("attribute compares the value when one is given", async () => {
    const h = harness();
    h.bridge.canned["wait.element"] = { present: true, visible: true, attr: "0" };
    const ok = await h.call("wait_for", {
      condition: { kind: "attribute", target: { testId: "sketch-dof" }, name: "data-dof", value: "0" },
      timeoutMs: 200,
    });
    expect(ok.status).toBe("ok");
    const wrong = await h.call("wait_for", {
      condition: { kind: "attribute", target: { testId: "sketch-dof" }, name: "data-dof", value: "3" },
      timeoutMs: 120,
    });
    expect(wrong.status).toBe("error");
  });

  test("text and text_gone read the visible page text", async () => {
    const h = harness();
    h.bridge.bodyText = "Extrude 1";
    expect((await h.call("wait_for", { condition: { kind: "text", text: "Extrude 1" } })).status).toBe("ok");
    expect(
      (await h.call("wait_for", { condition: { kind: "text_gone", text: "Extrude 1" }, timeoutMs: 120 })).status,
    ).toBe("error");
  });

  test("revision_stable waits out a moving revision", async () => {
    const h = harness();
    let rev = 0;
    h.bridge.canned["ui_revision"] = () => {
      rev += rev < 3 ? 1 : 0;
      return { rev, lastMutationAt: 0, now: 0 };
    };
    const env = await h.call("wait_for", {
      condition: { kind: "revision_stable", quietMs: 50 },
      timeoutMs: 3000,
    });
    expect(env.status).toBe("ok");
    expect((env.data as { observed: { revision: number } }).observed.revision).toBe(3);
  });

  test("log_line greps the app log the session launched", async () => {
    const h = harness();
    const dir = join(h.journal.dir, "app-logs");
    mkdirSync(dir, { recursive: true });
    const path = join(dir, "dev.jsonl");
    writeFileSync(path, `${JSON.stringify({ level: "INFO", target: "onecad_lib", fields: {} })}\n`);
    setTimeout(() => {
      appendFileSync(path, `${JSON.stringify({ level: "INFO", target: "onecad_lib", msg: "regen: ok" })}\n`);
    }, 150);
    const env = await h.call("wait_for", { condition: { kind: "log_line", grep: "regen: ok" }, timeoutMs: 3000 });
    expect(env.status).toBe("ok");
  });

  test("window waits for the app's on-screen windows", async () => {
    const h = harness();
    const env = await h.call("wait_for", { condition: { kind: "window", count: 1 }, timeoutMs: 120 });
    expect(env.status).toBe("error");
    expect(env.error?.details?.lastObserved).toMatchObject({ count: 0 });
  });

  test("delay simply waits", async () => {
    const h = harness();
    const started = Date.now();
    const env = await h.call("wait_for", { condition: { kind: "delay", ms: 150 } });
    expect(env.status).toBe("ok");
    expect(Date.now() - started).toBeGreaterThanOrEqual(140);
  });
});
