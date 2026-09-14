import { describe, expect, test } from "bun:test";
import type { Runner } from "../src/session/procs.ts";
import {
  findSurvivors,
  isAlive,
  listeningPids,
  matchesAny,
  parsePids,
  processCommand,
} from "../src/session/procs.ts";

interface Recorded {
  calls: string[][];
  runner: Runner;
}

function recorder(reply: (argv: string[]) => { code?: number; stdout?: string }): Recorded {
  const calls: string[][] = [];
  return {
    calls,
    runner: async (argv) => {
      calls.push(argv);
      const r = reply(argv);
      return { code: r.code ?? 0, stdout: r.stdout ?? "", stderr: "" };
    },
  };
}

describe("parsePids", () => {
  test("keeps positive integers and drops duplicates and noise", () => {
    expect(parsePids("123\n456\n123\n\nnot-a-pid\n-4\n")).toEqual([123, 456]);
  });
});

describe("listeningPids", () => {
  test("passes an argv array — nothing is ever interpolated into a shell", async () => {
    const rec = recorder(() => ({ stdout: "4242\n" }));
    expect(await listeningPids(4445, rec.runner)).toEqual([4242]);
    expect(rec.calls[0]).toEqual(["/usr/sbin/lsof", "-nP", "-iTCP:4445", "-sTCP:LISTEN", "-t"]);
  });

  test("a free port is an empty list, not an error (lsof exits 1)", async () => {
    const rec = recorder(() => ({ code: 1, stdout: "" }));
    expect(await listeningPids(1420, rec.runner)).toEqual([]);
  });

  test("refuses a port outside the TCP range", async () => {
    const rec = recorder(() => ({}));
    await expect(listeningPids(0, rec.runner)).rejects.toThrow(/not a usable TCP port/);
  });
});

describe("processCommand", () => {
  test("returns the command line", async () => {
    const rec = recorder(() => ({ stdout: "/repo/target/debug/onecad\n" }));
    expect(await processCommand(77, rec.runner)).toBe("/repo/target/debug/onecad");
    expect(rec.calls[0]).toEqual(["/bin/ps", "-o", "command=", "-p", "77"]);
  });

  test("a dead pid is null", async () => {
    const rec = recorder(() => ({ code: 1, stdout: "" }));
    expect(await processCommand(77, rec.runner)).toBeNull();
  });
});

describe("findSurvivors", () => {
  const MINE = "/Users/me/OneCAD";

  function sweepRunner(): Runner {
    const commands: Record<number, string> = {
      10: `${MINE}/src-tauri/target/debug/onecad`,
      11: "/Users/other/OneCAD/src-tauri/target/debug/onecad",
      12: `${MINE}/src-tauri/binaries/onecad-worker-aarch64-apple-darwin`,
      13: `/bin/zsh -c grep -rn target/debug/onecad ${MINE}/src`,
      14: `/usr/bin/pgrep -f target/debug/onecad|onecad-worker- ${MINE}`,
    };
    return async (argv) => {
      if (argv[0] === "/usr/bin/pgrep") {
        const pattern = argv[2] as string;
        const hits = Object.entries(commands)
          .filter(([, cmd]) => cmd.includes(pattern))
          .map(([pid]) => pid);
        return { code: hits.length > 0 ? 0 : 1, stdout: hits.join("\n"), stderr: "" };
      }
      const pid = Number(argv[4]);
      const cmd = commands[pid];
      return { code: cmd ? 0 : 1, stdout: cmd ?? "", stderr: "" };
    };
  }

  test("only reports processes whose command line also names this project", async () => {
    const found = await findSurvivors(
      ["target/debug/onecad", "onecad-worker-"],
      [MINE],
      sweepRunner(),
    );
    expect(found.map((s) => s.pid).sort()).toEqual([10, 12]);
    // pid 11 matches the pattern but belongs to another checkout.
    expect(found.some((s) => s.pid === 11)).toBe(false);
    // pids 13/14 carry the pattern AND the project root in their ARGUMENTS only.
    expect(found.some((s) => s.pid === 13 || s.pid === 14)).toBe(false);
  });
});

describe("matchesAny / isAlive", () => {
  test("matchesAny is a substring test over the configured patterns", () => {
    expect(matchesAny("/x/target/debug/onecad --flag", ["target/debug/onecad"])).toBe(true);
    expect(matchesAny("/usr/bin/vim", ["target/debug/onecad"])).toBe(false);
    expect(matchesAny("anything", [""])).toBe(false);
  });

  test("isAlive turns the signal-0 probe into a boolean", () => {
    expect(isAlive(1, () => undefined)).toBe(true);
    expect(
      isAlive(1, () => {
        throw new Error("ESRCH");
      }),
    ).toBe(false);
  });
});
