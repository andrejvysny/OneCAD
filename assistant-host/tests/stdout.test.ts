import { describe, expect, test } from "bun:test";
import { createLogger } from "../src/log.js";

const CHILD = new URL("./fixtures/stdout-child.ts", import.meta.url).pathname;

describe("stdout hygiene (wire protocol §8)", () => {
  test("after claimStdout, only the returned writer reaches stdout", async () => {
    const child = Bun.spawn(["bun", CHILD], { stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ]);
    await child.exited;

    // The whole point: a stray print from anywhere in the process cannot appear
    // between two frames, because it is not on this stream at all.
    expect(stdout).toBe("FRAME_ONLY");

    for (const marker of [
      "POLLUTION_log",
      "POLLUTION_info",
      "POLLUTION_debug",
      "POLLUTION_warn",
      "POLLUTION_error",
      "POLLUTION_raw",
    ]) {
      expect(stderr).toContain(marker);
    }
  });

  test("the logger writes one JSON object per line and honours its level", () => {
    const lines: string[] = [];
    const logger = createLogger("warn", (line) => lines.push(line));
    logger.debug("dropped");
    logger.info("dropped");
    logger.warn("kept", { a: 1 });
    logger.error("kept too");
    expect(lines).toHaveLength(2);
    const first = JSON.parse(lines[0]!) as Record<string, unknown>;
    expect(first).toMatchObject({ level: "warn", target: "assistant", message: "kept", fields: { a: 1 } });
  });

  test("an unserialisable field does not turn a log call into a second failure", () => {
    const lines: string[] = [];
    const logger = createLogger("info", (line) => lines.push(line));
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    logger.error("boom", cyclic);
    expect(lines).toHaveLength(1);
    expect(JSON.parse(lines[0]!)).toMatchObject({ message: "boom", fields: "<unserialisable>" });
  });
});
