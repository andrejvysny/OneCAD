import { describe, expect, test } from "bun:test";
import type { Runner } from "../src/session/procs.ts";
import {
  findSurvivors,
  isAlive,
  isOwnedExecutable,
  listeningPids,
  matchesAny,
  parsePids,
  processCommand,
  processExecutable,
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

describe("processExecutable", () => {
  test("reads `ps -o comm=` — the executable path, with no arguments appended", async () => {
    const rec = recorder(() => ({ stdout: "/repo/target/debug/onecad\n" }));
    expect(await processExecutable(77, rec.runner)).toBe("/repo/target/debug/onecad");
    expect(rec.calls[0]).toEqual(["/bin/ps", "-o", "comm=", "-p", "77"]);
  });

  test("a dead pid is null", async () => {
    const rec = recorder(() => ({ code: 1, stdout: "" }));
    expect(await processExecutable(77, rec.runner)).toBeNull();
  });
});

describe("isOwnedExecutable", () => {
  test("accepts an executable path containing a space", () => {
    const root = "/Users/me/My Projects/OneCAD";
    expect(isOwnedExecutable(`${root}/src-tauri/target/debug/onecad`, "target/debug/onecad", [root])).toBe(true);
  });

  test("the pattern alone is not ownership — another checkout is refused", () => {
    expect(
      isOwnedExecutable("/Users/other/OneCAD/src-tauri/target/debug/onecad", "target/debug/onecad", [
        "/Users/me/OneCAD",
      ]),
    ).toBe(false);
  });

  test("an empty pattern never owns anything", () => {
    expect(isOwnedExecutable("/Users/me/OneCAD/x", "", ["/Users/me/OneCAD"])).toBe(false);
  });
});

describe("findSurvivors", () => {
  const MINE = "/Users/me/OneCAD";
  const SPACED = "/Users/me/My Projects/OneCAD";

  interface Proc {
    command: string;
    /** `ps -o comm=`: the executable path only. */
    comm: string;
  }

  const PROCS: Record<number, Proc> = {
    10: { command: `${MINE}/src-tauri/target/debug/onecad`, comm: `${MINE}/src-tauri/target/debug/onecad` },
    11: {
      command: "/Users/other/OneCAD/src-tauri/target/debug/onecad",
      comm: "/Users/other/OneCAD/src-tauri/target/debug/onecad",
    },
    12: {
      command: `${MINE}/src-tauri/binaries/onecad-worker-aarch64-apple-darwin`,
      comm: `${MINE}/src-tauri/binaries/onecad-worker-aarch64-apple-darwin`,
    },
    13: { command: `/bin/zsh -c grep -rn target/debug/onecad ${MINE}/src`, comm: "/bin/zsh" },
    14: { command: `/usr/bin/pgrep -f target/debug/onecad|onecad-worker- ${MINE}`, comm: "/usr/bin/pgrep" },
    15: {
      command: `${SPACED}/src-tauri/target/debug/onecad --flag`,
      comm: `${SPACED}/src-tauri/target/debug/onecad`,
    },
  };

  function sweepRunner(): Runner {
    return async (argv) => {
      if (argv[0] === "/usr/bin/pgrep") {
        const pattern = argv[2] as string;
        const hits = Object.entries(PROCS)
          .filter(([, p]) => p.command.includes(pattern))
          .map(([pid]) => pid);
        return { code: hits.length > 0 ? 0 : 1, stdout: hits.join("\n"), stderr: "" };
      }
      const proc = PROCS[Number(argv[4])];
      const out = proc === undefined ? "" : argv[2] === "comm=" ? proc.comm : proc.command;
      return { code: out.length > 0 ? 0 : 1, stdout: out, stderr: "" };
    };
  }

  const PATTERNS = ["target/debug/onecad", "onecad-worker-"];

  test("only reports processes whose EXECUTABLE also names this project", async () => {
    const found = await findSurvivors(PATTERNS, [MINE], sweepRunner());
    expect(found.map((s) => s.pid).sort()).toEqual([10, 12]);
    // pid 11 matches the pattern but belongs to another checkout.
    expect(found.some((s) => s.pid === 11)).toBe(false);
    // pids 13/14 carry the pattern AND the project root in their ARGUMENTS only.
    expect(found.some((s) => s.pid === 13 || s.pid === 14)).toBe(false);
    // pid 15 is the same app in a DIFFERENT checkout as far as this sweep is concerned.
    expect(found.some((s) => s.pid === 15)).toBe(false);
  });

  test("a checkout path containing a space is still owned, and reports its full command line", async () => {
    const found = await findSurvivors(PATTERNS, [SPACED], sweepRunner());
    expect(found).toEqual([{ pid: 15, command: `${SPACED}/src-tauri/target/debug/onecad --flag` }]);
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
