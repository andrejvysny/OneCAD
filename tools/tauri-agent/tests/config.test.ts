import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, realpathSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { pointInNativeOcclusion } from "../src/geometry/mapping.ts";
import { CONFIG_FILENAME, loadConfig, parseConfig } from "../src/session/config.ts";

/** The repo root, which is where the committed tauri-agent.config.json lives. */
const REPO_ROOT = fileURLToPath(new URL("../../../", import.meta.url));

const cwd = process.cwd();
const envRoot = process.env.TAURI_AGENT_ROOT;
const dirs: string[] = [];

function tempRoot(config?: unknown): string {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "tauri-agent-config-")));
  dirs.push(dir);
  if (config !== undefined) writeFileSync(join(dir, CONFIG_FILENAME), JSON.stringify(config), "utf8");
  return dir;
}

afterEach(() => {
  process.chdir(cwd);
  if (envRoot === undefined) delete process.env.TAURI_AGENT_ROOT;
  else process.env.TAURI_AGENT_ROOT = envRoot;
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

describe("loadConfig", () => {
  test("walks up from cwd to the directory holding the config file", () => {
    delete process.env.TAURI_AGENT_ROOT;
    const root = tempRoot({ webdriver: { port: 4500 } });
    const nested = join(root, "a", "b");
    mkdirSync(nested, { recursive: true });
    process.chdir(nested);

    const resolved = loadConfig();
    expect(resolved.root).toBe(root);
    expect(resolved.configPath).toBe(join(root, CONFIG_FILENAME));
    expect(resolved.config.webdriver.port).toBe(4500);
    // Everything the file omitted falls back to the documented defaults.
    expect(resolved.config.app.bundleId).toBe("com.andrejvysny.onecad");
    expect(resolved.config.artifacts.dir).toBe(".tauri-agent/artifacts");
  });

  test("TAURI_AGENT_ROOT wins over cwd discovery", () => {
    const viaEnv = tempRoot({ app: { title: "FromEnv" } });
    const viaCwd = tempRoot({ app: { title: "FromCwd" } });
    process.env.TAURI_AGENT_ROOT = viaEnv;
    process.chdir(viaCwd);

    const resolved = loadConfig();
    expect(resolved.root).toBe(viaEnv);
    expect(resolved.config.app.title).toBe("FromEnv");
  });

  test("a root with no config file still yields full defaults", () => {
    process.env.TAURI_AGENT_ROOT = tempRoot();
    const { config } = loadConfig();
    expect(config).toEqual(parseConfig({}, "<defaults>"));
    expect(config.launch.dev.command).toEqual(["bun", "run", "tauri:agent"]);
  });

  test("defaults match the committed tauri-agent.config.json", () => {
    const d = parseConfig({}, "<defaults>");
    expect(d.app).toEqual({
      bundleId: "com.andrejvysny.onecad",
      windowLabel: "main",
      title: "OneCAD",
      processPatterns: ["target/debug/onecad", "onecad-worker-"],
    });
    expect(d.launch).toEqual({
      dev: { command: ["bun", "run", "tauri:agent"], readyTimeoutMs: 600_000 },
      bundled: { appPath: "src-tauri/target/release/bundle/macos/onecad.app" },
    });
    expect(d.webdriver.port).toBe(4445);
    expect(d.devServer.port).toBe(1420);
    expect(d.screenshots).toEqual({ policy: "state_changing", previewMaxPx: 1600 });
    expect(d.settle).toEqual({ frameMs: 16, quietMs: 120, timeoutMs: 1500 });
    expect(d.calibration.probeTestId).toBe("document-title");
    expect(d.input).toEqual({ wheelLinesPerNotch: 3, dwellMs: 600, clickIntervalMs: 80 });
    expect(d.logs.devJsonl).toBe("logs/dev.jsonl");
    expect(d.nativeOcclusion.rects).toEqual([{ x: 0, y: 0, width: 80, height: 28 }]);
  });

  test("the committed config's occlusion rects are the geometry Rect shape", () => {
    // Key drift here is invisible: {x,y,w,h} makes every comparison NaN and the
    // native-control gate stops refusing clicks on the macOS close button.
    process.env.TAURI_AGENT_ROOT = REPO_ROOT;
    const { config } = loadConfig();
    expect(pointInNativeOcclusion({ x: 20, y: 14 }, config.nativeOcclusion.rects)).toBe(true);
    expect(pointInNativeOcclusion({ x: 200, y: 14 }, config.nativeOcclusion.rects)).toBe(false);
  });

  test("rejects an invalid config with the offending field in details", () => {
    process.env.TAURI_AGENT_ROOT = tempRoot({ webdriver: { port: 0 } });
    expect(() => loadConfig()).toThrow(/Invalid/);
  });
});
