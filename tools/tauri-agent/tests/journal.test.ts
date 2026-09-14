import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Journal, type JournalEntry } from "../src/trace/journal.ts";

const dirs: string[] = [];

function tempJournal(): Journal {
  const dir = mkdtempSync(join(tmpdir(), "tauri-agent-journal-"));
  dirs.push(dir);
  return new Journal(join(dir, "s-1"));
}

function entry(actionId: string, tool = "session_status"): JournalEntry {
  return {
    ts: new Date().toISOString(),
    sessionId: "s-1",
    actionId,
    tool,
    input: {},
    result: { actionId, status: "ok" },
    durationMs: 1,
  };
}

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

describe("Journal", () => {
  test("mints zero-padded sequential action ids", () => {
    const j = tempJournal();
    expect([j.nextActionId(), j.nextActionId(), j.nextActionId()]).toEqual(["A-001", "A-002", "A-003"]);
  });

  test("since() on an empty journal returns nothing", async () => {
    expect(await tempJournal().since()).toEqual([]);
  });

  test("appends one JSON line per entry and reads them back", async () => {
    const j = tempJournal();
    await j.append(entry("A-001"));
    await j.append(entry("A-002", "observe_trace"));
    const lines = readFileSync(j.path, "utf8").trim().split("\n");
    expect(lines.length).toBe(2);
    const all = await j.since();
    expect(all.map((e) => e.actionId)).toEqual(["A-001", "A-002"]);
    expect(all[1]?.tool).toBe("observe_trace");
  });

  test("since(actionId) is exclusive and limit keeps the most recent", async () => {
    const j = tempJournal();
    for (let i = 1; i <= 5; i += 1) await j.append(entry(`A-00${i}`));
    expect((await j.since("A-002")).map((e) => e.actionId)).toEqual(["A-003", "A-004", "A-005"]);
    expect((await j.since(undefined, 2)).map((e) => e.actionId)).toEqual(["A-004", "A-005"]);
    expect(await j.since("A-005")).toEqual([]);
  });

  test("concurrent appends do not interleave", async () => {
    const j = tempJournal();
    await Promise.all([1, 2, 3, 4, 5, 6].map((i) => j.append(entry(`A-00${i}`))));
    const all = await j.since();
    expect(all.length).toBe(6);
    expect(new Set(all.map((e) => e.actionId)).size).toBe(6);
  });

  test("artifactPath is relative to the session dir", () => {
    const j = tempJournal();
    expect(j.artifactPath("A-001-after.png")).toBe(join(j.dir, "A-001-after.png"));
  });
});
