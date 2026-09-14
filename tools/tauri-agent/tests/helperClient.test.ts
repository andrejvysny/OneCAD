/**
 * HelperClient failure-path contract, driven against `tests/fixtures/fake-helper.ts` over the
 * real JSON-line protocol. Nothing here posts an HID event, so the drills that matter — a verb
 * that hangs, a helper that dies mid-drag, a verb that rejects after the button went down —
 * can be run on a developer's desktop without touching whatever is frontmost.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { AgentError, isAgentError } from "../src/errors.ts";
import { HelperClient, MacInput, mapHelperError, verbTimeout } from "../src/platform/macos/input.ts";

const FIXTURE = join(import.meta.dir, "fixtures", "fake-helper.ts");
const clients: HelperClient[] = [];

function fakeClient(knobs: string[] = []): { client: HelperClient; log: string } {
  const log = join(mkdtempSync(join(tmpdir(), "tauri-agent-fake-")), "requests.jsonl");
  const client = new HelperClient(() => Promise.resolve([process.execPath, FIXTURE, `--log=${log}`, ...knobs]));
  clients.push(client);
  return { client, log };
}

function sent(log: string): Array<Record<string, unknown>> {
  let text = "";
  try {
    text = readFileSync(log, "utf8");
  } catch {
    return [];
  }
  return text
    .split("\n")
    .filter((l) => l.trim().length > 0)
    .map((l) => JSON.parse(l) as Record<string, unknown>);
}

async function reject(p: Promise<unknown>): Promise<AgentError> {
  const e = await p.then(
    () => null,
    (err: unknown) => err,
  );
  expect(isAgentError(e)).toBe(true);
  return e as AgentError;
}

afterEach(async () => {
  while (clients.length > 0) await clients.pop()?.dispose();
});

describe("HelperClient", () => {
  test("verbTimeout budgets the motion a verb was asked to perform", () => {
    expect(verbTimeout("cursor", {})).toBe(5_000);
    expect(verbTimeout("move", { durationMs: 1_200 })).toBe(6_200);
    expect(verbTimeout("path", { durationMs: 1_000, holdMs: 200, dwellMs: 100 })).toBe(6_300);
  });

  test("mapHelperError maps rejected arguments to INVALID_TARGET, not INTERNAL", () => {
    expect(mapHelperError("keydown", "INVALID_ARGS", "unknown key").code).toBe("INVALID_TARGET");
    expect(mapHelperError("nope", "INVALID_VERB", "unknown verb").code).toBe("INVALID_TARGET");
    expect(mapHelperError("focus", "WINDOW_NOT_FOUND", "no pid").code).toBe("WINDOW_NOT_FOUND");
    expect(mapHelperError("move", "SOMETHING_NEW", "?").code).toBe("INTERNAL");
  });

  test("a verb waiting behind a slow one does not spend its timeout in the queue", async () => {
    const { client } = fakeClient();
    const slow = client.request("path", { button: "left", points: [], durationMs: 900 }, 3_000);
    const quick = client.request("cursor", {}, 400);
    expect(await quick).toEqual({ x: 100, y: 200 });
    await slow;
  }, 15_000);

  test("a hung verb times out, restarts the helper and drains OS-held input", async () => {
    const { client, log } = fakeClient(["--hang=cursor"]);
    const err = await reject(client.request("cursor", {}, 300));
    expect(err.code).toBe("ACTION_TIMEOUT");
    expect(await client.request("version")).toEqual({ version: "fake", protocol: 2 });
    const drains = sent(log).filter((r) => r.verb === "release_all" && r.osState === true);
    expect(drains.length).toBeGreaterThanOrEqual(1);
  }, 15_000);

  test("an unplanned helper exit rejects the in-flight verb and drains before the next one", async () => {
    const { client, log } = fakeClient(["--exit=boom"]);
    const err = await reject(client.request("boom"));
    expect(err.code).toBe("HELPER_FAILED");
    expect(await client.request("version")).toEqual({ version: "fake", protocol: 2 });
    const lines = sent(log);
    const boomAt = lines.findIndex((r) => r.verb === "boom");
    expect(lines.slice(boomAt + 1).some((r) => r.verb === "release_all" && r.osState === true)).toBe(true);
  }, 15_000);

  test("releaseAll releases tracked state only, never the user's own buttons", async () => {
    const { client, log } = fakeClient();
    expect(await client.releaseAll()).toEqual({ releasedButtons: [], releasedMods: [] });
    const release = sent(log).find((r) => r.verb === "release_all");
    expect(release?.osState).toBeUndefined();
  }, 15_000);

  test("a path that fails after the button went down releases before rethrowing", async () => {
    const { client, log } = fakeClient(["--fail=path:INVALID_ARGS"]);
    const err = await reject(
      new MacInput(client).path("left", [
        { x: 10, y: 10 },
        { x: 20, y: 20 },
      ]),
    );
    expect(err.code).toBe("INVALID_TARGET");
    const lines = sent(log);
    expect(lines[lines.length - 1]?.verb).toBe("release_all");
    expect(lines[lines.length - 1]?.osState).toBeUndefined();
  }, 15_000);

  test("an out-of-range duration is refused before the helper ever sees it", async () => {
    const { client, log } = fakeClient();
    const err = await reject(new MacInput(client).move({ x: 1, y: 1 }, { durationMs: 1e20 }));
    expect(err.code).toBe("INVALID_TARGET");
    expect(sent(log).some((r) => r.verb === "move")).toBe(false);
    expect(await client.request("cursor")).toEqual({ x: 100, y: 200 });
  }, 15_000);

  test("dispose releases tracked input before the helper goes away", async () => {
    const { client, log } = fakeClient();
    await client.request("version");
    await client.dispose();
    expect(sent(log).some((r) => r.verb === "release_all")).toBe(true);
    const err = await reject(client.request("version"));
    expect(err.code).toBe("HELPER_FAILED");
  }, 15_000);
});
