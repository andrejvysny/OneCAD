/**
 * Builds the real Swift helper and drives it over the real JSON-line protocol.
 *
 * Deliberately read-only with respect to the machine: it reads the cursor, reads
 * permissions WITHOUT prompting, lists windows, and moves the cursor to where it
 * already is. It never clicks, presses a key, or scrolls — those would land in whatever
 * application happens to be frontmost.
 */
import { afterAll, describe, expect, test } from "bun:test";

import { AgentError, isAgentError } from "../src/errors.ts";
import { ensureHelper, helperCacheDir } from "../src/platform/macos/build.ts";
import { getHelperClient, MacInput } from "../src/platform/macos/input.ts";
import { MacWindows } from "../src/platform/macos/windows.ts";

const swiftc = Bun.which("swiftc");
const runnable = process.platform === "darwin" && swiftc !== null;
const why = `needs macOS with swiftc on PATH (platform=${process.platform}, swiftc=${swiftc ?? "missing"})`;
// Mirror of ONECAD_REQUIRE_WORKER: a gate that silently skips is a gate that proves nothing.
if (!runnable && process.env.TAURI_AGENT_REQUIRE_HELPER === "1") {
  throw new Error(`[helperProtocol] TAURI_AGENT_REQUIRE_HELPER=1 but the real helper cannot run: ${why}`);
}
if (!runnable) console.error(`[helperProtocol] skipped: ${why}.`);

const input = new MacInput();
const windows = new MacWindows();
const client = getHelperClient();

afterAll(async () => {
  if (runnable) await input.dispose();
});

/** Permission-independent: the verb either works or reports the missing grant. */
async function tolerate(fn: () => Promise<void>): Promise<string | null> {
  try {
    await fn();
    return null;
  } catch (e) {
    if (isAgentError(e) && e.code === "NATIVE_INPUT_PERMISSION_DENIED") return e.code;
    throw e;
  }
}

describe.skipIf(!runnable)("macOS helper build + protocol", () => {
  test("ensureHelper compiles a cached binary named by source hash", async () => {
    const bin = await ensureHelper();
    expect(bin.startsWith(`${helperCacheDir()}/macos-helper-`)).toBe(true);
    expect(await Bun.file(bin).exists()).toBe(true);
    expect(await ensureHelper()).toBe(bin);
  }, 180_000);

  test("--selftest reports permissions, its own windows and a cursor round-trip", async () => {
    const bin = await ensureHelper();
    const proc = Bun.spawn([bin, "--selftest"], { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
    const [code, out] = await Promise.all([proc.exited, new Response(proc.stdout).text()]);
    console.error(`[helperProtocol] --selftest exit=${code} ${out.trim()}`);

    const report = JSON.parse(out) as Record<string, unknown>;
    expect(typeof report.ok).toBe("boolean");
    expect(report.version).toBe("1.1.0");
    expect(typeof report.pid).toBe("number");
    const perms = report.permissions as Record<string, unknown>;
    expect(typeof perms.accessibility).toBe("boolean");
    expect(typeof perms.screenRecording).toBe("boolean");
    // The helper is a faceless CLI, so an empty window list is the expected shape.
    expect(Array.isArray(report.windows)).toBe(true);
    const cursor = report.cursor as Record<string, { x: number; y: number }>;
    expect(typeof cursor.before.x).toBe("number");
    expect(typeof cursor.after.y).toBe("number");
    expect(typeof report.moveOk).toBe("boolean");
    expect(code).toBe(report.ok === true ? 0 : 1);
  }, 180_000);

  test("--version prints the protocol banner", async () => {
    const bin = await ensureHelper();
    const proc = Bun.spawn([bin, "--version"], { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
    const [code, out] = await Promise.all([proc.exited, new Response(proc.stdout).text()]);
    expect(code).toBe(0);
    expect(JSON.parse(out)).toEqual({ version: "1.1.0", protocol: 2 });
  }, 180_000);

  test("cursor() returns finite global points", async () => {
    const p = await input.cursor();
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  }, 180_000);

  test("permissions() does not prompt and returns two booleans", async () => {
    const perms = await input.permissions({ prompt: false });
    console.error(`[helperProtocol] permissions ${JSON.stringify(perms)}`);
    expect(typeof perms.accessibility).toBe("boolean");
    expect(typeof perms.screenRecording).toBe("boolean");
  });

  test("windows.list() for this pid returns well-formed rows (usually none)", async () => {
    const rows = await windows.list(process.pid);
    expect(Array.isArray(rows)).toBe(true);
    for (const row of rows) {
      expect(typeof row.windowId).toBe("number");
      expect(typeof row.bounds.width).toBe("number");
      expect(typeof row.bounds.height).toBe("number");
      expect(typeof row.onscreen).toBe("boolean");
    }
  });

  test("move() to the current cursor position is a no-op that exercises the input path", async () => {
    const before = await input.cursor();
    const denied = await tolerate(() => input.move(before, { durationMs: 0, steps: 1 }));
    if (denied) {
      console.error("[helperProtocol] Accessibility not granted; move path reported the grant instead.");
      return;
    }
    const after = await input.cursor();
    expect(Math.abs(after.x - before.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(1);
  });

  test("releaseAll() never throws and the helper actually answered it", async () => {
    await expect(input.releaseAll()).resolves.toBeUndefined();
    // A swallowed failure also resolves, so assert the reply shape the helper alone produces.
    const reply = await client.releaseAll();
    expect(Array.isArray(reply.releasedButtons)).toBe(true);
    expect(Array.isArray(reply.releasedMods)).toBe(true);
  });

  test("a bad argument maps to an INVALID_TARGET AgentError", async () => {
    const err = await input.keyDown("no-such-key").then(
      () => null,
      (e: unknown) => e,
    );
    expect(isAgentError(err)).toBe(true);
    expect((err as AgentError).code).toBe("INVALID_TARGET");
  });

  test("an out-of-range pid is rejected instead of trapping the helper", async () => {
    const err = await client.request("frontmost", { pid: 1e10 }).then(
      () => null,
      (e: unknown) => e,
    );
    expect(isAgentError(err)).toBe(true);
    expect((err as AgentError).code).toBe("INVALID_TARGET");
    // The helper survived the hostile argument: it still answers.
    expect(Number.isFinite((await input.cursor()).x)).toBe(true);
  });

  test("an absurd durationMs is clamped by the helper rather than trapping it", async () => {
    const before = await input.cursor();
    // steps:1 keeps this a single no-op move; only the numeric conversion is under test.
    const denied = await tolerate(async () => {
      await client.request("move", { x: before.x, y: before.y, durationMs: 1e20, steps: 1 });
    });
    if (denied) return;
    const after = await input.cursor();
    expect(Math.abs(after.x - before.x)).toBeLessThanOrEqual(1);
  });

  test("isFrontmost() reports without activating anything", async () => {
    expect(await windows.isFrontmost(process.pid)).toBe(false);
  });

  test("MacInput refuses an out-of-range duration before the helper sees it", async () => {
    const before = await input.cursor();
    const err = await input.move(before, { durationMs: 1e20 }).then(
      () => null,
      (e: unknown) => e,
    );
    expect((err as AgentError).code).toBe("INVALID_TARGET");
    expect(Number.isFinite((await input.cursor()).y)).toBe(true);
  });
});
