/**
 * Process inspection for the session lifecycle: who owns a port, what a pid's command
 * line is, which processes match a pattern.
 *
 * Every command is an argv ARRAY — nothing is ever interpolated into a shell string.
 * `stop()` kills by pid based on what these return, so a quoting bug here would be a
 * kill of the wrong process.
 */
import { AgentError } from "../errors.ts";

const LSOF = "/usr/sbin/lsof";
const PS = "/bin/ps";
const PGREP = "/usr/bin/pgrep";
const RUN_TIMEOUT_MS = 5_000;

export interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}

export type Runner = (argv: string[]) => Promise<RunResult>;

export const spawnRunner: Runner = async (argv) => {
  const proc = Bun.spawn(argv, { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  const timer = setTimeout(() => proc.kill("SIGKILL"), RUN_TIMEOUT_MS);
  try {
    const [code, stdout, stderr] = await Promise.all([
      proc.exited,
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    return { code, stdout, stderr };
  } finally {
    clearTimeout(timer);
  }
};

export function parsePids(stdout: string): number[] {
  const out: number[] = [];
  for (const line of stdout.split("\n")) {
    const n = Number.parseInt(line.trim(), 10);
    if (Number.isInteger(n) && n > 0 && !out.includes(n)) out.push(n);
  }
  return out;
}

/** pids LISTENing on a TCP port. Empty (not an error) when the port is free — lsof exits 1. */
export async function listeningPids(port: number, run: Runner): Promise<number[]> {
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new AgentError("INTERNAL", `not a usable TCP port: ${String(port)}`);
  }
  const r = await run([LSOF, "-nP", `-iTCP:${port}`, "-sTCP:LISTEN", "-t"]);
  return parsePids(r.stdout);
}

/** Full command line of a pid, or null when it is gone. Reporting only — never ownership. */
export async function processCommand(pid: number, run: Runner): Promise<string | null> {
  if (!Number.isInteger(pid) || pid <= 0) return null;
  const r = await run([PS, "-o", "command=", "-p", String(pid)]);
  const cmd = r.stdout.trim();
  return r.code === 0 && cmd.length > 0 ? cmd : null;
}

/**
 * Executable path of a pid, or null when it is gone.
 *
 * `comm=` is the executable itself, with no arguments appended, so a checkout path that
 * contains a space survives it — deriving argv[0] from the command line by splitting on
 * whitespace does not, and that false negative would silently un-own a real survivor.
 */
export async function processExecutable(pid: number, run: Runner): Promise<string | null> {
  if (!Number.isInteger(pid) || pid <= 0) return null;
  const r = await run([PS, "-o", "comm=", "-p", String(pid)]);
  const exe = r.stdout.trim();
  return r.code === 0 && exe.length > 0 ? exe : null;
}

/** `ps -o comm=`, falling back to argv[0] of `command` so ownership never silently weakens. */
export async function executablePath(pid: number, command: string, run: Runner): Promise<string> {
  return (await processExecutable(pid, run)) ?? (command.trim().split(/\s+/)[0] ?? "");
}

/** pids whose full command line matches `pattern` (a pgrep ERE). Never includes this process. */
export async function pgrepFull(pattern: string, run: Runner): Promise<number[]> {
  if (pattern.length === 0) return [];
  const r = await run([PGREP, "-f", pattern]);
  return parsePids(r.stdout).filter((pid) => pid !== process.pid);
}

export function matchesAny(command: string, patterns: string[]): boolean {
  return patterns.some((p) => p.length > 0 && command.includes(p));
}

export type KillFn = (pid: number, signal: NodeJS.Signals | number) => void;

export function isAlive(pid: number, kill: KillFn): boolean {
  try {
    kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Best-effort signal: a process that already exited is not an error. */
export function signal(pid: number, sig: NodeJS.Signals, kill: KillFn): boolean {
  try {
    kill(pid, sig);
    return true;
  } catch {
    return false;
  }
}

/**
 * Survivors of a stop, restricted to processes that belong to THIS project.
 *
 * `processPatterns` alone is not a safe kill list — `onecad-worker-` would match a
 * colleague's checkout running in another window. A pid is only reported when its
 * command line also contains one of `owners` (the project root, or the bundled app
 * path), so an unrelated process is never a candidate.
 */
export async function findSurvivors(
  patterns: string[],
  owners: string[],
  run: Runner,
): Promise<Array<{ pid: number; command: string }>> {
  const found = new Map<number, string>();
  for (const pattern of patterns) {
    for (const pid of await pgrepFull(pattern, run)) {
      if (found.has(pid)) continue;
      const command = await processCommand(pid, run);
      if (command === null) continue;
      // Ownership is decided on the EXECUTABLE path; the full command line is recorded
      // for the report only, because arguments are not evidence of what is running.
      if (!isOwnedExecutable(await executablePath(pid, command, run), pattern, owners)) continue;
      found.set(pid, command);
    }
  }
  return [...found].map(([pid, command]) => ({ pid, command }));
}


/**
 * The pattern must sit in the EXECUTABLE PATH, not merely somewhere in the arguments: a
 * shell running `grep target/debug/onecad` inside this checkout carries both the pattern
 * and the project root in its command line and must never be a kill candidate.
 *
 * `exe` is an executable path (`processExecutable`), never a command line — the split
 * this function used to do mis-derived argv[0] whenever the path contained a space.
 */
export function isOwnedExecutable(exe: string, pattern: string, owners: string[]): boolean {
  if (pattern.length === 0 || !exe.includes(pattern)) return false;
  return owners.some((o) => o.length > 0 && exe.includes(o));
}
