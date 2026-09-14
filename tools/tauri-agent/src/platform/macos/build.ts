/**
 * Builds the Swift CGEvent helper on first use and caches it by source hash, so editing
 * `helper/main.swift` transparently produces a new binary instead of reusing a stale one.
 */
import { chmod, mkdir, rename, unlink } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

import { AgentError } from "../../errors.ts";
import { log } from "../../log.ts";

const SWIFTC = "swiftc";
/** A cold optimised build of the helper takes seconds; two minutes means swiftc is stuck. */
const BUILD_TIMEOUT_MS = 120_000;

export function helperSourcePath(): string {
  return join(import.meta.dir, "helper", "main.swift");
}

let cacheDir: string | null = null;

/** Where the compiled helper lives: `~/.cache/tauri-agent`, or the temp dir if that is unusable. */
export function helperCacheDir(): string {
  return cacheDir ?? join(homedir(), ".cache", "tauri-agent");
}

async function ensureCacheDir(): Promise<string> {
  if (cacheDir !== null) return cacheDir;
  const candidates = [join(homedir(), ".cache", "tauri-agent"), join(tmpdir(), "tauri-agent")];
  const failures: Record<string, string> = {};
  for (const dir of candidates) {
    try {
      await mkdir(dir, { recursive: true });
      await Bun.write(join(dir, ".writable"), "");
      cacheDir = dir;
      return dir;
    } catch (e) {
      failures[dir] = String(e);
    }
  }
  throw new AgentError("HELPER_FAILED", "no writable cache directory for the macOS input helper", {
    details: { candidates, failures },
  });
}

let inFlight: Promise<string> | null = null;

/** Absolute path to a compiled helper binary matching the current `main.swift`. */
export function ensureHelper(): Promise<string> {
  inFlight ??= buildHelper().catch((e: unknown) => {
    inFlight = null;
    throw e;
  });
  return inFlight;
}

async function buildHelper(): Promise<string> {
  const src = helperSourcePath();
  const source = await Bun.file(src).arrayBuffer();
  const sha = new Bun.CryptoHasher("sha256").update(new Uint8Array(source)).digest("hex").slice(0, 12);
  const out = join(await ensureCacheDir(), `macos-helper-${sha}`);
  if (await Bun.file(out).exists()) return out;

  const tmp = `${out}.tmp.${process.pid}.${Math.random().toString(36).slice(2, 8)}`;
  log.info("building macOS input helper", { src, out });
  const started = Date.now();
  const stderrText = await compile(src, tmp, out);
  await chmod(tmp, 0o755);
  await rename(tmp, out);
  log.info("built macOS input helper", { out, durationMs: Date.now() - started });
  if (stderrText.trim().length > 0) log.debug("swiftc diagnostics", { tail: tail(stderrText) });
  return out;
}

/** Runs swiftc into `tmp`; resolves with its stderr, throws HELPER_FAILED with the compiler tail. */
async function compile(src: string, tmp: string, out: string): Promise<string> {
  let proc;
  let timedOut = false;
  try {
    proc = Bun.spawn([SWIFTC, "-O", "-framework", "Cocoa", "-framework", "Carbon", src, "-o", tmp], {
      stdin: "ignore",
      stdout: "pipe",
      stderr: "pipe",
    });
  } catch (cause) {
    throw new AgentError("HELPER_FAILED", `could not run ${SWIFTC} (is the full Xcode toolchain installed?)`, {
      details: { src, out },
      cause,
    });
  }
  const killer = setTimeout(() => {
    timedOut = true;
    proc.kill();
  }, BUILD_TIMEOUT_MS);
  const [code, stderrText] = await Promise.all([proc.exited, new Response(proc.stderr).text()]).finally(() =>
    clearTimeout(killer),
  );
  if (code !== 0) {
    await unlink(tmp).catch(() => {});
    const why = timedOut ? `did not finish within ${BUILD_TIMEOUT_MS}ms` : `exited ${code}`;
    throw new AgentError("HELPER_FAILED", `swiftc ${why} building the macOS input helper`, {
      details: { src, out, exitCode: code, timedOut, stderrTail: tail(stderrText) },
    });
  }
  return stderrText;
}

function tail(text: string, lines = 20): string {
  return text.trimEnd().split("\n").slice(-lines).join("\n");
}
