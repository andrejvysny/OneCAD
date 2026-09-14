/**
 * Fake native-input helper: speaks the real JSON-line protocol and posts nothing.
 *
 * The failure modes that matter (a verb that hangs, a helper that dies mid-drag, a verb that
 * rejects) only happen on a real machine by accident, so they are knobs here instead. Every
 * request line is appended to `--log` so a test can assert what the client actually sent —
 * including the recovery `release_all` the client is supposed to issue on its own.
 *
 *   bun fake-helper.ts --log=<path> [--fail=<verb>:<code>] [--hang=<verb>] [--exit=<verb>]
 */
import { appendFileSync } from "node:fs";

function flag(name: string): string | undefined {
  const hit = process.argv.find((a) => a.startsWith(`--${name}=`));
  return hit === undefined ? undefined : hit.slice(name.length + 3);
}

const logPath = flag("log");
const hangVerb = flag("hang");
const exitVerb = flag("exit");
const [failVerb, failCode] = (flag("fail") ?? ":").split(":");

function write(obj: Record<string, unknown>): void {
  process.stdout.write(`${JSON.stringify(obj)}\n`);
}

function result(verb: string): Record<string, unknown> {
  switch (verb) {
    case "cursor":
      return { x: 100, y: 200 };
    case "release_all":
      return { releasedButtons: [], releasedMods: [] };
    case "version":
      return { version: "fake", protocol: 2 };
    case "windows":
      return { windows: [] };
    case "frontmost":
    case "focus":
      return { frontmost: true };
    case "permissions":
      return { accessibility: true, screenRecording: true };
    default:
      return {};
  }
}

async function handle(line: string): Promise<void> {
  if (logPath !== undefined) appendFileSync(logPath, `${line}\n`);
  const req = JSON.parse(line) as Record<string, unknown>;
  const verb = String(req.verb);
  if (verb === exitVerb) process.exit(9);
  if (verb === hangVerb) return;
  const wait = Number(req.durationMs ?? 0) + Number(req.holdMs ?? 0) + Number(req.dwellMs ?? 0);
  if (wait > 0) await Bun.sleep(Math.min(wait, 30_000));
  if (verb === failVerb) {
    write({ id: req.id, ok: false, code: failCode, message: `fake failure for '${verb}'` });
    return;
  }
  write({ id: req.id, ok: true, result: result(verb) });
}

process.stderr.write(`fake-helper ready pid=${process.pid}\n`);
const reader = Bun.stdin.stream().getReader();
const decoder = new TextDecoder();
let buf = "";
for (;;) {
  const { done, value } = await reader.read();
  if (done) break;
  buf += decoder.decode(value, { stream: true });
  let nl: number;
  while ((nl = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (line) await handle(line);
  }
}
