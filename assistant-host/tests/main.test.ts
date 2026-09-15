import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, posix, win32 } from "node:path";
import { ArgumentError, parseArgs, resolveDbPath } from "../src/main.js";
import { HostMock, Pipe, text } from "./harness.js";

const MAIN = new URL("../src/main.ts", import.meta.url).pathname;

const cleanups: Array<() => void> = [];
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
});

describe("host-supplied configuration", () => {
  test("--app-data-dir decides where the store lives", () => {
    const args = parseArgs(["--app-data-dir", "/var/onecad", "--log-level", "debug"]);
    expect(args).toEqual({ appDataDir: "/var/onecad", logLevel: "debug" });
    expect(resolveDbPath(args.appDataDir)).toBe("/var/onecad/assistant/agentkit.sqlite");
    expect(resolveDbPath("/var/onecad/")).toBe("/var/onecad/assistant/agentkit.sqlite");
  });

  test("the level defaults to info, and an unknown level is refused", () => {
    expect(parseArgs(["--app-data-dir", "/tmp/x"]).logLevel).toBe("info");
    expect(() => parseArgs(["--app-data-dir", "/tmp/x", "--log-level", "trace"])).toThrow(
      ArgumentError,
    );
  });

  test("a missing, relative, or unknown argument refuses to start", () => {
    expect(() => parseArgs([])).toThrow(/--app-data-dir is required/);
    expect(() => parseArgs(["--app-data-dir"])).toThrow(/needs a path/);
    expect(() => parseArgs(["--app-data-dir", "relative/dir"])).toThrow(/must be absolute/);
    expect(() => parseArgs(["--data-dir", "/tmp/x"])).toThrow(/unknown argument/);
  });
});

/**
 * F11. The check used to be `appDataDir.startsWith("/")`, which refuses
 * `C:\Users\Andrej\AppData\Roaming\OneCAD` — an ordinary Windows absolute path, and
 * the exact one the Rust supervisor passes as a platform-native `PathBuf`. The
 * sidecar exited 2 before the handshake, so the supervisor saw a crash loop.
 *
 * Both platform tables run on EVERY platform: `parseArgs` takes the path
 * implementation, so a Linux CI box checks Windows behaviour against
 * `path.win32` rather than against an idea of what Windows does.
 */
describe("the data directory, on each platform's own terms", () => {
  const windows = (value: string): string => parseArgs(["--app-data-dir", value], win32).appDataDir;
  const unix = (value: string): string => parseArgs(["--app-data-dir", value], posix).appDataDir;

  test("R01: the Windows paths the old predicate rejected are absolute and accepted", () => {
    for (const value of [
      "C:\\Users\\Andrej\\AppData\\Roaming\\OneCAD",
      "\\\\server\\share\\OneCAD",
    ]) {
      // The probe's claim, restated against the module that has to honour it.
      expect(win32.isAbsolute(value)).toBe(true);
      expect(value.startsWith("/")).toBe(false);
      expect(windows(value)).toBe(value);
    }
  });

  test("Windows accepts drive-qualified and UNC paths, spaces and Unicode included", () => {
    for (const value of [
      "C:\\Users\\Andrej\\AppData\\Roaming\\OneCAD",
      "C:/Users/Andrej/AppData/Roaming/OneCAD",
      "D:\\Program Files\\OneCAD data",
      "C:\\Users\\Ond\u0159ej\\\u6587\u4ef6\\OneCAD",
      "\\\\server\\share\\OneCAD",
      "\\\\server\\share",
      "//server/share/OneCAD",
    ]) {
      expect(windows(value)).toBe(value);
    }
  });

  test("Windows refuses every form that is not a reproducible directory", () => {
    for (const [value, why] of [
      ["relative\\dir", "relative"],
      ["C:data", "drive-relative: the drive's current directory is process state"],
      ["\\OneCAD\\data", "root-relative: absolute on whichever drive we started on"],
      ["/OneCAD/data", "root-relative, spelled the other way"],
      ["\\\\server", "a UNC server with no share names no directory"],
      ["\\\\server\\", "likewise an empty share"],
      ["\\\\?\\C:\\OneCAD", "the device namespace is not an ordinary path"],
      ["\\\\.\\PIPE\\onecad", "nor is a device name"],
      ["", "nothing at all"],
    ] as const) {
      expect(() => windows(value), why).toThrow(ArgumentError);
    }
  });

  test("macOS and Linux accept rooted paths and refuse everything else", () => {
    for (const value of [
      "/var/onecad",
      "/Users/andrej/Library/Application Support/OneCAD",
      "/home/andrej/\u00dcnicode/OneCAD",
    ]) {
      expect(unix(value)).toBe(value);
    }
    for (const [value, why] of [
      ["relative/dir", "relative"],
      ["C:\\Users\\Andrej", "a Windows path is not absolute here, and is not a fallback"],
      ["~/onecad", "the shell expands this; this process does not"],
      ["", "nothing at all"],
    ] as const) {
      expect(() => unix(value), why).toThrow(ArgumentError);
    }
  });

  test("the store path is joined with the platform's separator", () => {
    expect(resolveDbPath("C:\\Users\\Andrej\\OneCAD", win32)).toBe(
      "C:\\Users\\Andrej\\OneCAD\\assistant\\agentkit.sqlite",
    );
    // A trailing separator is normalised rather than doubled.
    expect(resolveDbPath("C:\\Users\\Andrej\\OneCAD\\", win32)).toBe(
      "C:\\Users\\Andrej\\OneCAD\\assistant\\agentkit.sqlite",
    );
    expect(resolveDbPath("\\\\server\\share", win32)).toBe(
      "\\\\server\\share\\assistant\\agentkit.sqlite",
    );
    expect(resolveDbPath("/var/onecad", posix)).toBe("/var/onecad/assistant/agentkit.sqlite");
    expect(resolveDbPath("/var/onecad/", posix)).toBe("/var/onecad/assistant/agentkit.sqlite");
  });

  test("the process exits non-zero instead of guessing a data directory", async () => {
    const child = Bun.spawn(["bun", MAIN], { stdout: "pipe", stderr: "pipe" });
    const [code, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ]);
    expect(code).toBe(2);
    expect(stdout).toBe("");
    expect(stderr).toContain("--app-data-dir is required");
  });
});

describe("the sidecar, end to end over a real pipe", () => {
  test(
    "handshakes, serves an AgentKit route, and exits 0 on shutdown",
    async () => {
      const dir = mkdtempSync(join(tmpdir(), "onecad-assistant-e2e-"));
      cleanups.push(() => rmSync(dir, { recursive: true, force: true }));

      const child = Bun.spawn(["bun", MAIN, "--app-data-dir", dir, "--log-level", "error"], {
        stdin: "pipe",
        stdout: "pipe",
        stderr: "pipe",
      });
      cleanups.push(() => child.kill());

      const toSidecar = new Pipe();
      const fromSidecar = new Pipe();
      void (async () => {
        for await (const chunk of child.stdout as unknown as AsyncIterable<Uint8Array>) {
          await fromSidecar.write(chunk);
        }
        fromSidecar.close();
      })();
      void (async () => {
        for await (const chunk of toSidecar) {
          child.stdin.write(chunk);
          await child.stdin.flush();
        }
      })();

      const host = new HostMock(toSidecar, fromSidecar);
      const hello = await host.waitFor(-1, (f) => f.envelope.t === "hello");
      expect(hello.envelope).toMatchObject({ t: "hello", protocolVersion: 1 });

      // `hello` is the readiness signal: the very next frame is served.
      const version = await host.call(
        "agentkit.fetch",
        { method: "GET", path: "/v1/version", headers: {} },
        { stream: true },
      );
      expect(version.res.envelope).toMatchObject({ ok: true, payload: { status: 200 } });
      expect(version.end?.envelope).toMatchObject({ t: "end", ok: true });
      const body = JSON.parse(text(version.chunks)) as { packages?: Record<string, string> };
      expect(body.packages).toHaveProperty("onecad-assistant-host");

      // A request the host is not allowed to make on this verb table.
      const forbidden = await host.call(
        "provider.fetch",
        {},
        { principal: "ui" },
      );
      expect(forbidden.res.envelope).toMatchObject({
        ok: false,
        error: { code: "unknown_verb" },
      });

      const bye = await host.call("shutdown", {});
      expect(bye.res.envelope).toMatchObject({ ok: true });
      expect(await child.exited).toBe(0);

      // §8 again, this time on the real binary's streams: everything the
      // decoder saw was a frame, and the log lane carried no frame magic.
      const stderr = await new Response(child.stderr).text();
      expect(stderr).not.toContain("OCAK");
    },
    30_000,
  );
});
