import { existsSync, readFileSync } from "node:fs";
import { dirname, isAbsolute, join, parse as parsePath, resolve } from "node:path";
import { z } from "zod";
import { AgentError } from "../errors.ts";
import type { Rect } from "../geometry/types.ts";
import { log } from "../log.ts";

export interface AgentConfig {
  app: { bundleId: string; windowLabel: string; title: string; processPatterns: string[] };
  launch: { dev: { command: string[]; readyTimeoutMs: number }; bundled: { appPath: string } };
  webdriver: { port: number };
  devServer: { port: number };
  artifacts: { dir: string };
  screenshots: { policy: "never" | "on_failure" | "state_changing" | "always"; previewMaxPx: number };
  settle: { frameMs: number; quietMs: number; timeoutMs: number };
  calibration: { probeTestId: string };
  input: { wheelLinesPerNotch: number; dwellMs: number; clickIntervalMs: number };
  logs: { devJsonl: string | null };
  nativeOcclusion: { rects: Rect[] };
}

export interface ResolvedConfig {
  root: string;
  config: AgentConfig;
  configPath: string;
}

export const CONFIG_FILENAME = "tauri-agent.config.json";

const port = z.number().int().min(1).max(65535);

/**
 * Typed as the geometry `Rect` so a key rename cannot pass tsc: the occlusion gate
 * reads `width`/`height`, and a config that shipped `w`/`h` turned every comparison
 * into NaN — the traffic-light guard became a silent no-op.
 */
const occlusionRect: z.ZodType<Rect> = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number(),
  height: z.number(),
});

export const AgentConfigSchema = z
  .object({
    app: z
      .object({
        bundleId: z.string().default("com.andrejvysny.onecad"),
        windowLabel: z.string().default("main"),
        title: z.string().default("OneCAD"),
        processPatterns: z.array(z.string()).default(["target/debug/onecad", "onecad-worker-"]),
      })
      .default({}),
    launch: z
      .object({
        dev: z
          .object({
            command: z.array(z.string()).min(1).default(["bun", "run", "tauri:agent"]),
            readyTimeoutMs: z.number().int().positive().default(600_000),
          })
          .default({}),
        bundled: z
          .object({
            appPath: z.string().default("src-tauri/target/release/bundle/macos/onecad.app"),
          })
          .default({}),
      })
      .default({}),
    webdriver: z.object({ port: port.default(4445) }).default({}),
    devServer: z.object({ port: port.default(1420) }).default({}),
    artifacts: z.object({ dir: z.string().default(".tauri-agent/artifacts") }).default({}),
    screenshots: z
      .object({
        policy: z.enum(["never", "on_failure", "state_changing", "always"]).default("state_changing"),
        previewMaxPx: z.number().int().positive().default(1600),
      })
      .default({}),
    settle: z
      .object({
        frameMs: z.number().int().positive().default(16),
        quietMs: z.number().int().positive().default(120),
        timeoutMs: z.number().int().positive().default(1500),
      })
      .default({}),
    calibration: z.object({ probeTestId: z.string().default("document-title") }).default({}),
    input: z
      .object({
        wheelLinesPerNotch: z.number().positive().default(3),
        dwellMs: z.number().int().nonnegative().default(600),
        clickIntervalMs: z.number().int().nonnegative().default(80),
      })
      .default({}),
    logs: z.object({ devJsonl: z.string().nullable().default("logs/dev.jsonl") }).default({}),
    nativeOcclusion: z
      .object({
        rects: z.array(occlusionRect).default([{ x: 0, y: 0, width: 80, height: 28 }]),
      })
      .default({}),
  })
  .strict();

function findUp(from: string, predicate: (dir: string) => boolean): string | null {
  let dir = resolve(from);
  const root = parsePath(dir).root;
  for (;;) {
    if (predicate(dir)) return dir;
    if (dir === root) return null;
    dir = dirname(dir);
  }
}

function discoverRoot(): string {
  const fromEnv = process.env.TAURI_AGENT_ROOT;
  if (fromEnv && fromEnv.length > 0) return resolve(fromEnv);
  const byConfig = findUp(process.cwd(), (d) => existsSync(join(d, CONFIG_FILENAME)));
  if (byConfig) return byConfig;
  const byGit = findUp(dirname(import.meta.dir), (d) => existsSync(join(d, ".git")));
  if (byGit) return byGit;
  throw new AgentError(
    "INTERNAL",
    `Could not locate the project root: no TAURI_AGENT_ROOT, no ${CONFIG_FILENAME} above ${process.cwd()}, no .git above ${import.meta.dir}`,
  );
}

export function parseConfig(raw: unknown, configPath: string): AgentConfig {
  const parsed = AgentConfigSchema.safeParse(raw);
  if (!parsed.success) {
    throw new AgentError("INTERNAL", `Invalid ${configPath}`, {
      remediation: `Fix the fields listed in details.issues in ${configPath}, or delete the file to fall back to defaults.`,
      details: { issues: parsed.error.issues },
    });
  }
  return parsed.data;
}

export function loadConfig(): ResolvedConfig {
  const root = discoverRoot();
  const configPath = join(root, CONFIG_FILENAME);
  if (!existsSync(configPath)) {
    log.warn("config file not found, using defaults", { configPath });
    return { root, config: parseConfig({}, configPath), configPath };
  }
  let raw: unknown;
  try {
    raw = JSON.parse(readFileSync(configPath, "utf8")) as unknown;
  } catch (cause) {
    throw new AgentError("INTERNAL", `${configPath} is not valid JSON`, {
      remediation: `Repair the JSON syntax in ${configPath}.`,
      cause,
    });
  }
  return { root, config: parseConfig(raw, configPath), configPath };
}

/** Every path inside AgentConfig is relative to `root`. */
export function resolveFromRoot(root: string, p: string): string {
  return isAbsolute(p) ? p : join(root, p);
}
