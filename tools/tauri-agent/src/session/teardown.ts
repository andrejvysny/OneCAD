/**
 * Teardown: killing what the session started, and proving nothing of ours survived.
 *
 * The app intercepts its own close (a frontend confirm dialog), so closing the window is
 * not a way to stop it. The launched tree is signalled by PROCESS GROUP, and anything still
 * matching the app's process patterns afterwards is killed only when its command line also
 * names this project — never a process belonging to some other checkout.
 */
import { log } from "../log.ts";
import type { LaunchHandle } from "./launch.ts";
import type { KillFn, Runner } from "./procs.ts";
import { findSurvivors, isAlive, signal } from "./procs.ts";

const TERM_GRACE_MS = 3_000;
const RECHECK_MS = 500;

export interface Survivor {
  pid: number;
  command: string;
}

export async function killLauncherGroup(launcher: LaunchHandle, kill: KillFn): Promise<void> {
  if (launcher.exited) return;
  // The negative pid signals the whole group: `tauri dev`, cargo, vite and the app.
  signal(-launcher.pgid, "SIGTERM", kill);
  if (await launcher.waitExit(TERM_GRACE_MS)) return;
  log.warn("launcher group ignored SIGTERM; sending SIGKILL", { pgid: launcher.pgid });
  signal(-launcher.pgid, "SIGKILL", kill);
  await launcher.waitExit(TERM_GRACE_MS);
}

export interface SweepOpts {
  patterns: string[];
  owners: string[];
  runner: Runner;
  kill: KillFn;
  sleep: (ms: number) => Promise<void>;
  /** false only reports; it never signals anything. */
  killThem: boolean;
}

export async function sweepSurvivors(opts: SweepOpts): Promise<Survivor[]> {
  const first = await findSurvivors(opts.patterns, opts.owners, opts.runner);
  if (first.length === 0 || !opts.killThem) return first;
  for (const s of first) {
    log.warn("killing survivor", s);
    signal(s.pid, "SIGKILL", opts.kill);
  }
  await opts.sleep(RECHECK_MS);
  const again = await findSurvivors(opts.patterns, opts.owners, opts.runner);
  return again.filter((s) => isAlive(s.pid, opts.kill));
}
