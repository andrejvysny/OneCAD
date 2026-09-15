/**
 * Start-path preconditions: TCC grants, who owns the ports, which pid is the app.
 *
 * Every one of these refuses rather than improvises. A port held by somebody else is
 * PORT_IN_USE, never a kill; the agent only ever signals processes it launched or that it
 * can prove belong to this project.
 */
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { NativeInput, Permissions } from "../platform/adapter.ts";
import type { CaptureCapability } from "./types.ts";
import type { LaunchHandle } from "./launch.ts";
import { tailFile } from "./launch.ts";
import type { Runner } from "./procs.ts";
import { executablePath, isOwnedExecutable, listeningPids, processCommand } from "./procs.ts";

const LAUNCHER_LOG_TAIL = 40;
const PID_TRIES = 6;
const PID_INTERVAL_MS = 500;

export async function checkPermissions(input: NativeInput, allowDegradedCapture: boolean): Promise<Permissions> {
  const perms = await input.permissions({ prompt: false });
  if (!perms.accessibility) {
    throw new AgentError("NATIVE_INPUT_PERMISSION_DENIED", "Accessibility is not granted to this process", {
      details: { permissions: perms },
    });
  }
  if (!perms.screenRecording && !allowDegradedCapture) {
    throw new AgentError("SCREEN_CAPTURE_PERMISSION_DENIED", "Screen Recording is not granted to this process", {
      details: { permissions: perms, hint: "pass allowDegradedCapture:true to start without usable screenshots" },
    });
  }
  return perms;
}

/**
 * What the session may claim about screenshots, from the grant it actually holds.
 *
 * `checkPermissions` has already refused the no-grant/no-flag case, so reaching here without
 * the grant means the caller asked for a degraded session on purpose — and a degraded session
 * must SAY so rather than discovering it one failed capture at a time.
 */
export function captureCapabilityOf(perms: Permissions): CaptureCapability {
  if (perms.screenRecording) return { available: true, authoritative: true };
  return { available: false, authoritative: false, reason: "SCREEN_RECORDING_PERMISSION_DENIED" };
}

/**
 * True when the WebDriver port is already served by our own app and reuse was requested.
 * Any other owner is fatal: killing it could take out a developer's own dev session.
 *
 * "Ours" is CHECKOUT-SCOPED. A bare pattern match is not enough — `target/debug/onecad`
 * also matches another checkout of this same project, and adopting that app would later
 * let this session tear down something it does not own. The executable path must carry
 * the pattern AND one of `owners`, exactly as the survivor sweep decides.
 */
export async function portOwnerIsOurs(
  port: number,
  patterns: string[],
  owners: string[],
  reuseExisting: boolean,
  run: Runner,
): Promise<boolean> {
  const pids = await listeningPids(port, run);
  if (pids.length === 0) return false;
  const pid = pids[0] as number;
  const command = (await processCommand(pid, run)) ?? "";
  const exe = await executablePath(pid, command, run);
  const ours = patterns.some((p) => isOwnedExecutable(exe, p, owners));
  if (ours && reuseExisting) {
    log.info("reusing the app already serving the WebDriver port", { pid, port });
    return true;
  }
  throw new AgentError("PORT_IN_USE", `TCP port ${port} is held by pid ${pid}`, {
    // `matchesApp` is the checkout-scoped verdict: false covers both "not this app"
    // and "this app, but built in another checkout".
    details: { port, pid, command, executable: exe, matchesApp: ours },
  });
}

export async function checkDevServerPort(port: number, run: Runner): Promise<void> {
  const pids = await listeningPids(port, run);
  if (pids.length === 0) return;
  const pid = pids[0] as number;
  throw new AgentError("DEV_SERVER_PORT_IN_USE", `the Vite dev port ${port} is held by pid ${pid}`, {
    details: { port, pid, command: await processCommand(pid, run) },
  });
}

/** The listener on the WebDriver port is the app binary, not the `bun`/`tauri` launcher. */
export async function resolveListenerPid(
  port: number,
  run: Runner,
  sleep: (ms: number) => Promise<void>,
): Promise<number> {
  for (let i = 0; i < PID_TRIES; i += 1) {
    const pids = await listeningPids(port, run);
    if (pids.length > 0) return pids[0] as number;
    await sleep(PID_INTERVAL_MS);
  }
  throw new AgentError("APP_NOT_RUNNING", `nothing owns TCP port ${port} although /status answered`, {
    details: { port },
  });
}

/**
 * Readiness check that aborts the moment the launcher dies — otherwise a build failure
 * would be reported as a 600 s timeout instead of the compiler error in launcher.log.
 */
export function launcherAbortCheck(launcher: LaunchHandle, logPath: string): () => Promise<void> {
  return async () => {
    if (!launcher.exited) return;
    throw new AgentError(
      "APP_NOT_RUNNING",
      `the launcher exited (code ${String(launcher.exitCode)}) before the app served WebDriver`,
      {
        details: {
          exitCode: launcher.exitCode,
          launcherLog: logPath,
          tail: await tailFile(logPath, LAUNCHER_LOG_TAIL),
        },
      },
    );
  };
}
