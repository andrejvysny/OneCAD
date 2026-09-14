import { afterAll, describe, expect, test } from "bun:test";
import { appendFileSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { countSince, devJsonlPath, logCursor, tailLogs } from "../src/observe/logs.ts";

const dir = mkdtempSync(join(tmpdir(), "tauri-agent-logs-"));
const path = join(dir, "dev.jsonl");

afterAll(() => rmSync(dir, { recursive: true, force: true }));

function line(level: string, target: string, msg: string): string {
  return `${JSON.stringify({ level, target, fields: { message: msg } })}\n`;
}

writeFileSync(
  path,
  line("INFO", "onecad_lib::document_runtime", "regen: ok") +
    line("ERROR", "worker", "OCCT blew up") +
    line("DEBUG", "fe", "pointer down") +
    line("WARN", "onecad_protocol::frames", "tx slow"),
);

describe("dev.jsonl tail", () => {
  test("reads everything from offset 0 and returns a usable cursor", async () => {
    const all = await tailLogs(path);
    expect(all.lines.length).toBe(4);
    expect(all.missing).toBe(false);
    const cursor = all.cursor;

    appendFileSync(path, line("ERROR", "fe", "boom"));
    const delta = await tailLogs(path, { since: cursor });
    expect(delta.lines.length).toBe(1);
    expect(delta.lines[0]?.raw).toContain("boom");
  });

  test("filters by minimum level, lane prefix and grep", async () => {
    expect((await tailLogs(path, { level: "ERROR" })).lines.map((l) => l.target)).toEqual(["worker", "fe"]);
    expect((await tailLogs(path, { level: "WARN" })).lines.length).toBe(3);
    expect((await tailLogs(path, { lane: "onecad_" })).lines.length).toBe(2);
    expect((await tailLogs(path, { lane: "onecad_protocol" })).lines.length).toBe(1);
    expect((await tailLogs(path, { grep: "OCCT" })).lines.length).toBe(1);
  });

  test("limit keeps the newest lines and reports the rest as dropped", async () => {
    const tail = await tailLogs(path, { limit: 2 });
    expect(tail.lines.length).toBe(2);
    expect(tail.matched).toBe(5);
    expect(tail.dropped).toBe(3);
    expect(tail.lines[1]?.raw).toContain("boom");
  });

  test("countSince counts ERROR lines after a cursor", async () => {
    const cursor = await logCursor(path);
    expect(await countSince(path, cursor, "ERROR")).toBe(0);
    appendFileSync(path, line("ERROR", "worker", "second failure") + line("INFO", "fe", "fine"));
    expect(await countSince(path, cursor, "ERROR")).toBe(1);
  });

  test("a truncated file (app restart) is re-read from the top rather than reported empty", async () => {
    const far = String(1_000_000);
    const tail = await tailLogs(path, { since: far });
    expect(tail.lines.length).toBeGreaterThan(0);
  });

  test("a partial trailing line is left for the next call", async () => {
    const p2 = join(dir, "partial.jsonl");
    writeFileSync(p2, `${line("INFO", "fe", "complete")}{"level":"ERROR"`);
    const tail = await tailLogs(p2);
    expect(tail.lines.length).toBe(1);
    appendFileSync(p2, `,"target":"worker"}\n`);
    const next = await tailLogs(p2, { since: tail.cursor });
    expect(next.lines.length).toBe(1);
    expect(next.lines[0]?.target).toBe("worker");
  });

  test("a missing file is reported, not thrown", async () => {
    const tail = await tailLogs(join(dir, "nope.jsonl"));
    expect(tail.missing).toBe(true);
    expect(tail.lines).toEqual([]);
    expect(await logCursor(join(dir, "nope.jsonl"))).toBe("0");
  });

  test("the path follows who owns the app", () => {
    expect(devJsonlPath({ root: "/r", devJsonl: "logs/dev.jsonl", journalDir: "/a/s", launched: true })).toBe(
      "/a/s/app-logs/dev.jsonl",
    );
    expect(devJsonlPath({ root: "/r", devJsonl: "logs/dev.jsonl", journalDir: "/a/s", launched: false })).toBe(
      "/r/logs/dev.jsonl",
    );
    expect(devJsonlPath({ root: "/r", devJsonl: null, journalDir: "/a/s", launched: false })).toBeNull();
  });
});
