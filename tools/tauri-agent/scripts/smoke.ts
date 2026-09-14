/**
 * Live acceptance for the session lifecycle: `bun tools/tauri-agent/scripts/smoke.ts`.
 *
 * Drives the orchestrator directly (no MCP) through launch → calibrate → one native window
 * screenshot → stop → survivor check. It moves the REAL cursor once (the hover probe) and
 * brings the app to the front; that is the point — a calibration that is not proven against
 * the real window server is not proven at all.
 *
 * Everything goes to stderr: this is not the MCP server, but stdout stays clean by habit.
 */
import { randomBytes } from "node:crypto";
import { loadConfig, resolveFromRoot } from "../src/session/config.ts";
import { SessionOrchestrator } from "../src/session/orchestrator.ts";
import { tailFile } from "../src/session/launch.ts";
import { spawnRunner } from "../src/session/procs.ts";
import { Journal } from "../src/trace/journal.ts";

function say(label: string, value?: unknown): void {
  const text = value === undefined ? label : `${label} ${JSON.stringify(value, null, 2)}`;
  process.stderr.write(`${text}\n`);
}

async function survivors(patterns: string[]): Promise<string> {
  const r = await spawnRunner(["/usr/bin/pgrep", "-fl", patterns.join("|")]);
  return r.stdout.trim();
}

const resolved = loadConfig();
const sessionId = `smoke-${new Date().toISOString().replace(/[:.]/g, "-")}-${randomBytes(2).toString("hex")}`;
const journal = new Journal(resolveFromRoot(resolved.root, `${resolved.config.artifacts.dir}/${sessionId}`));
const session = new SessionOrchestrator({ sessionId, journal, config: resolved });

say(`smoke: artifacts in ${journal.dir}`);
let failure: unknown;
try {
  const status = await session.start({ mode: "launch", launch: "dev" });
  say("STATUS", status);

  const geom = session.requireGeom();
  const shotPath = journal.artifactPath("smoke-window.png");
  const capture = session.requirePlatform().capture;
  const shot = await capture.window(geom.windowId, shotPath, geom.nativeBoundsPt);
  await capture.preview(shotPath, journal.artifactPath("smoke-window-preview.png"), 1600);
  say("SCREENSHOT", { path: shotPath, ...shot });
} catch (e) {
  failure = e;
  say("START FAILED", { error: String(e), details: (e as { details?: unknown }).details });
  say("LAUNCHER LOG TAIL", await tailFile(journal.artifactPath("launcher.log"), 40));
}

try {
  say("STOP", await session.stop());
} catch (e) {
  failure ??= e;
  say("STOP FAILED", { error: String(e), details: (e as { details?: unknown }).details });
}

say("SURVIVORS", await survivors(resolved.config.app.processPatterns));
process.exit(failure === undefined ? 0 : 1);
