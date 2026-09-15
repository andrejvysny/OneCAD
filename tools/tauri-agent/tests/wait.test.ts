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

  /**
   * The CAD half of the idle tuple. DOM quietness says nothing about a regen that is still
   * running in the OCCT worker, so these read the app's own counters — and an UNKNOWN reading
   * is never reported as idle, because a caller that asked for this fact specifically would
   * act on a lie.
   */
  test("worker_idle resolves when no regen is in flight", async () => {
    const h = harness();
    const env = await h.call("wait_for", { condition: { kind: "worker_idle" }, timeoutMs: 500 });
    expect(env.status).toBe("ok");
    expect((env.data as { observed: { regenBusy: number } }).observed.regenBusy).toBe(0);
  });

  test("worker_idle waits out an in-flight regen and reports it on timeout", async () => {
    const h = harness();
    h.bridge.idle = { ...h.bridge.idle, regenBusy: 2 };
    const env = await h.call("wait_for", { condition: { kind: "worker_idle" }, timeoutMs: 150 });
    expect(env.status).toBe("error");
    expect(env.error?.code).toBe("ACTION_TIMEOUT");
    expect(env.error?.details?.lastObserved).toMatchObject({ regenBusy: 2, geometryPending: false });
  });

  test("worker_idle refuses to call an unavailable signal idle", async () => {
    const h = harness();
    h.bridge.idle = { ...h.bridge.idle, regenBusy: null };
    const env = await h.call("wait_for", { condition: { kind: "worker_idle" }, timeoutMs: 150 });
    expect(env.status).toBe("error");
    expect(env.error?.details?.lastObserved).toMatchObject({ regenBusy: null });
  });

  test("render_idle resolves once the frame count stops moving", async () => {
    const h = harness();
    let frames = 0;
    h.bridge.canned["ui_idle"] = () => {
      frames += frames < 3 ? 1 : 0;
      return { rev: 1, regenBusy: 0, geometryPending: false, documentRevision: 1, frames, camera: null };
    };
    const env = await h.call("wait_for", { condition: { kind: "render_idle", quietMs: 50 }, timeoutMs: 3000 });
    expect(env.status).toBe("ok");
    expect((env.data as { observed: { frames: number } }).observed.frames).toBe(3);
  });

  test("render_idle reports the frame count it last saw when rendering never stops", async () => {
    const h = harness();
    let frames = 0;
    h.bridge.canned["ui_idle"] = () => {
      frames += 1;
      return { rev: 1, regenBusy: 0, geometryPending: false, documentRevision: 1, frames, camera: null };
    };
    const env = await h.call("wait_for", { condition: { kind: "render_idle", quietMs: 50 }, timeoutMs: 150 });
    expect(env.status).toBe("error");
    expect(env.error?.details?.lastObserved).toMatchObject({ available: true });
    expect((env.error?.details?.lastObserved as { frames: number }).frames).toBeGreaterThan(0);
  });

  test("render_frame_after resolves on a frame committed past the given count", async () => {
    const h = harness();
    h.bridge.idle = { ...h.bridge.idle, frames: 9 };
    const ok = await h.call("wait_for", { condition: { kind: "render_frame_after", frames: 5 }, timeoutMs: 500 });
    expect(ok.status).toBe("ok");
    const late = await h.call("wait_for", { condition: { kind: "render_frame_after", frames: 20 }, timeoutMs: 150 });
    expect(late.status).toBe("error");
    expect(late.error?.details?.lastObserved).toMatchObject({ frames: 9, after: 20 });
  });

  test("snapshot_at_least reads agent_status and reports the id it last saw", async () => {
    const h = harness();
    h.bridge.canned["invoke:agent_status"] = () => ({
      snapshotId: 7,
      pendingRenderExpectations: 0,
      pendingBodies: [],
    });
    const ok = await h.call("wait_for", { condition: { kind: "snapshot_at_least", id: 5 }, timeoutMs: 500 });
    expect(ok.status).toBe("ok");
    expect(h.bridge.invoked).toContain("agent_status");
    const late = await h.call("wait_for", { condition: { kind: "snapshot_at_least", id: 9 }, timeoutMs: 150 });
    expect(late.status).toBe("error");
    expect(late.error?.details?.lastObserved).toMatchObject({ snapshotId: 7, atLeast: 9 });
  });

  /** `snapshotId` is null while the document runtime is busy — unknown, never "reached". */
  test("snapshot_at_least treats a busy runtime as unknown, not as reached", async () => {
    const h = harness();
    h.bridge.canned["invoke:agent_status"] = () => ({
      snapshotId: null,
      pendingRenderExpectations: null,
      pendingBodies: null,
    });
    const env = await h.call("wait_for", { condition: { kind: "snapshot_at_least", id: 0 }, timeoutMs: 150 });
    expect(env.status).toBe("error");
    expect(env.error?.details?.lastObserved).toMatchObject({ snapshotId: null, runtimeBusy: true });
  });

  test("camera_stable resolves once the orbit stops and reports the pose on timeout", async () => {
    const h = harness();
    const pose = { x: 1, y: 2, z: 3, tx: 0, ty: 0, tz: 0, distance: 10 };
    h.bridge.idle = { ...h.bridge.idle, camera: pose };
    const ok = await h.call("wait_for", { condition: { kind: "camera_stable", quietMs: 50 }, timeoutMs: 3000 });
    expect(ok.status).toBe("ok");

    let n = 0;
    h.bridge.canned["ui_idle"] = () => {
      n += 1;
      return {
        rev: 1,
        regenBusy: 0,
        geometryPending: false,
        documentRevision: 1,
        frames: 1,
        camera: { ...pose, x: n },
      };
    };
    const late = await h.call("wait_for", { condition: { kind: "camera_stable", quietMs: 50 }, timeoutMs: 150 });
    expect(late.status).toBe("error");
    expect((late.error?.details?.lastObserved as { camera: { x: number } }).camera.x).toBeGreaterThan(0);
  });

  test("camera_stable does not call an unavailable camera stable", async () => {
    const h = harness();
    h.bridge.idle = { ...h.bridge.idle, camera: null };
    const env = await h.call("wait_for", { condition: { kind: "camera_stable", quietMs: 10 }, timeoutMs: 150 });
    expect(env.status).toBe("error");
    expect(env.error?.details?.lastObserved).toMatchObject({ camera: null, available: false });
  });

  test("delay simply waits", async () => {
    const h = harness();
    const started = Date.now();
    const env = await h.call("wait_for", { condition: { kind: "delay", ms: 150 } });
    expect(env.status).toBe("ok");
    expect(Date.now() - started).toBeGreaterThanOrEqual(140);
  });
});
