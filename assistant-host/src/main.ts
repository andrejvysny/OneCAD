#!/usr/bin/env bun
/**
 * `onecad-assistant-host` — the supervised assistant sidecar's entry point.
 *
 * This process is started by the OneCAD Rust host, speaks OCAK1 on stdin/stdout
 * (`docs/assistant/wire-protocol.md`), logs to stderr, and binds nothing. Every
 * decision it could make wrongly has been taken away from it:
 *
 *  - **Where its data lives** is `--app-data-dir`, passed by the host. There is
 *    no fallback and no default, because a sidecar that invents a path writes
 *    the user's chat history somewhere the app cannot find, back up or delete.
 *  - **Which provider it talks to** is decided by the host's gateway; see
 *    `bridge/verbs.ts`. This process holds no API key and cannot name a URL.
 *  - **What it may do to a document** is nothing; see
 *    `composition/noopApplier.ts`.
 *
 * ## Why the imports below are dynamic
 *
 * §8 of the wire protocol: stdout carries frames only, and one `console.log`
 * from any dependency desynchronises the host's frame reader. {@link claimStdout}
 * takes the real stdout writer and rebinds `process.stdout.write` and the five
 * `console` methods to stderr — but ES module imports are hoisted, so a static
 * import of AgentKit would be EVALUATED BEFORE any statement in this file ran,
 * and a package that prints at module scope would print onto the frame stream
 * before the guard existed. Everything below `claimStdout()` is therefore an
 * `await import(...)`, and only type-only imports (which erase completely) are
 * static. This is the one file in the package where import style is a
 * correctness property rather than a preference.
 */
import { claimStdout, createLogger, isLogLevel, LOG_LEVELS, type LogLevel } from "./log.js";
// `node:path` is a builtin with no module-scope output, so it is safe above the
// stdout claim for the same reason `./log.js` is. Everything the doc comment
// above is about — a dependency printing during its own evaluation — needs a
// dependency; this has none.
import nodePath from "node:path";

/**
 * Claimed only when this module IS the process, so that importing it from a
 * test does not hijack the test runner's stdout. The compiled binary always
 * takes this branch; `tests/stdout.test.ts` proves the guard itself in a
 * spawned child, which is the only place the claim can be observed honestly.
 */
const writeFrame = import.meta.main ? claimStdout() : undefined;

import type { AiProviderConfig } from "agentkit/contracts";
import type { BridgePeer as BridgePeerType } from "./bridge/peer.js";

/**
 * Where the store goes under the host's data directory, as path SEGMENTS. Fixed
 * here rather than passed, so the host names a directory it owns and this
 * process names the one file inside it that belongs to the assistant.
 *
 * Segments rather than a `"a/b"` string because the separator is the platform's,
 * and `path.join` is what knows which one that is.
 */
const DB_RELATIVE_SEGMENTS = ["assistant", "agentkit.sqlite"] as const;

/**
 * The subset of `node:path` this module needs, so a test can hand it
 * `path.win32` or `path.posix` and check the OTHER platform's behaviour from
 * whichever platform it happens to be running on.
 */
export interface PathImpl {
  isAbsolute(value: string): boolean;
  join(...parts: string[]): string;
  readonly sep: string;
}

export interface HostArgs {
  appDataDir: string;
  logLevel: LogLevel;
}

export class ArgumentError extends Error {
  override readonly name = "ArgumentError";
}

/**
 * Whether a path is one this process will accept as its data directory, and if
 * not, why.
 *
 * `startsWith("/")` was the whole test here, and on Windows it rejects
 * `C:\Users\Andrej\AppData\Roaming\OneCAD` — a perfectly ordinary absolute path,
 * and the exact one the Rust supervisor passes as a platform-native `PathBuf`.
 * That is a startup blocker, not a display bug: the process exits 2 before the
 * handshake and the supervisor reads it as a crash-looping sidecar.
 *
 * `path.isAbsolute` is the platform's own answer and is the primary gate. It is
 * not the whole policy, though, because on Windows it is true of two forms this
 * process must still refuse:
 *
 * * **Root-relative** (`\OneCAD\data`) — absolute on the CURRENT DRIVE, which is
 *   process state, so the path names a different directory depending on where
 *   the process was started. Unreproducible is exactly what the original check
 *   was trying to prevent.
 * * **Device namespace** (`\\?\C:\…`, `\\.\PIPE\…`) — not a filesystem path in
 *   the ordinary sense, and the escape-hatch semantics differ per API.
 *
 * Drive-relative (`C:data`) is refused by `isAbsolute` itself, which reports it
 * as relative. UNC (`\\server\share\…`) is ACCEPTED and requires both a server
 * and a share: `\\server` alone names no directory.
 *
 * A regular expression is deliberately not the gate. The platform's own
 * definition of "absolute" is the one the platform's own file APIs will use, and
 * a pattern that agrees with it today is a pattern that has to be maintained
 * against it forever.
 */
export function assertAbsoluteDataDir(value: string, impl: PathImpl = nodePath): void {
  const windows = impl.sep === "\\";
  if (!impl.isAbsolute(value)) {
    // Relative would resolve against whatever cwd the supervisor happened to
    // have, which is not something the app can state or reproduce. On Windows a
    // drive-relative path (`C:data`) lands here too, and for the same reason:
    // the drive's current directory is process state.
    throw new ArgumentError(`--app-data-dir must be absolute, got ${value}`);
  }
  if (!windows) return;

  const separator = /^[\\/]/;
  const uncPrefix = /^[\\/]{2}/;
  if (uncPrefix.test(value)) {
    const rest = value.slice(2);
    if (/^[?.][\\/]/.test(rest) || rest === "?" || rest === ".") {
      throw new ArgumentError(
        `--app-data-dir must be a drive or UNC path, not a device path, got ${value}`,
      );
    }
    const [server, share] = rest.split(/[\\/]/);
    if (!server || !share) {
      throw new ArgumentError(
        `--app-data-dir must name a UNC server AND share (\\\\server\\share\\...), got ${value}`,
      );
    }
    return;
  }
  if (separator.test(value)) {
    throw new ArgumentError(
      `--app-data-dir must name a drive or a UNC share, not the current drive's root, ` +
        `got ${value}`,
    );
  }
  // `isAbsolute` said yes and it is neither UNC nor root-relative, so it is
  // drive-qualified (`C:\...`), which is the form the host passes.
}

/**
 * Argument handling is the one place where a bad input must produce a refusal
 * to start rather than a guess — a sidecar that silently picked a default data
 * directory would look healthy while writing to the wrong place.
 */
export function parseArgs(argv: readonly string[], impl: PathImpl = nodePath): HostArgs {
  let appDataDir: string | undefined;
  let logLevel: LogLevel = "info";
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    switch (flag) {
      case "--app-data-dir":
        if (value === undefined) throw new ArgumentError("--app-data-dir needs a path");
        appDataDir = value;
        i += 1;
        break;
      case "--log-level":
        if (value === undefined || !isLogLevel(value)) {
          throw new ArgumentError(`--log-level must be one of ${LOG_LEVELS.join(", ")}`);
        }
        logLevel = value;
        i += 1;
        break;
      default:
        throw new ArgumentError(`unknown argument ${JSON.stringify(flag)}`);
    }
  }
  if (appDataDir === undefined) {
    throw new ArgumentError("--app-data-dir is required; the host chooses where data lives");
  }
  assertAbsoluteDataDir(appDataDir, impl);
  return { appDataDir, logLevel };
}

/**
 * The store file inside the host's data directory.
 *
 * `path.join` rather than string concatenation: it is what normalises a trailing
 * separator, and it is what writes the separator the platform actually uses.
 */
export function resolveDbPath(appDataDir: string, impl: PathImpl = nodePath): string {
  return impl.join(appDataDir, ...DB_RELATIVE_SEGMENTS);
}

async function main(write: (bytes: Uint8Array) => Promise<void>): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const logger = createLogger(args.logLevel);

  const { randomUUID } = await import("node:crypto");
  const { mkdirSync } = await import("node:fs");
  const { dirname } = await import("node:path");
  const { createRestHandler } = await import("agentkit/transport-http");
  const { OpenAiCompatibleClient } = await import("agentkit/core");
  const { BridgePeer } = await import("./bridge/peer.js");
  const {
    createAgentkitFetchRoute,
    createGatewayFetch,
    createShutdownRoute,
    VERB_AGENTKIT_FETCH,
    VERB_SHUTDOWN,
  } = await import("./bridge/verbs.js");
  const { createConfigInstallRoute, VERB_CONFIG_INSTALL } = await import(
    "./composition/configuration.js"
  );
  const { buildApp, HOST_VERSION } = await import("./composition/app.js");

  const dbPath = resolveDbPath(args.appDataDir);
  // `SqliteAssistantStore` opens a file; it does not create the directory it
  // sits in, and a missing parent surfaces as an opaque sqlite error.
  mkdirSync(dirname(dbPath), { recursive: true });

  // Declared before `buildApp` because the provider factory closes over it.
  //
  // It being `undefined` here is no longer something a turn can observe:
  // `buildApp` COMPOSES the graph and starts no worker, so the first thing that
  // can claim a turn is the execution gate, and the only thing that opens the
  // gate is a `config.install` — which cannot arrive before `accept`, which
  // cannot happen before this binding is assigned. The throw below is an
  // assertion about that ordering, not a case that can occur.
  let peer: BridgePeerType | undefined;

  const app = await buildApp({
    dbPath,
    logger,
    providerFactory: (config: AiProviderConfig) => {
      if (!peer) throw new Error("provider requested before the assistant bridge was open");
      return OpenAiCompatibleClient.fromConfig(config, createGatewayFetch(peer, config));
    },
  });

  // Held as a function, never served. See `composition/app.ts`.
  const restFetch = createRestHandler(app.deps);

  let stopping = false;
  const stop = async (reason: string, code: number): Promise<void> => {
    if (stopping) return;
    stopping = true;
    logger.info("assistant host stopping", { reason });
    peer?.close(new Error(reason));
    await app.stop();
    process.exit(code);
  };

  peer = new BridgePeer({
    input: Bun.stdin.stream() as unknown as AsyncIterable<Uint8Array>,
    write,
    logger,
    identity: { hostVersion: HOST_VERSION, sessionNonce: randomUUID() },
    routes: {
      [VERB_AGENTKIT_FETCH]: createAgentkitFetchRoute(restFetch),
      [VERB_SHUTDOWN]: createShutdownRoute(() => void stop("shutdown requested", 0)),
      [VERB_CONFIG_INSTALL]: createConfigInstallRoute(app.configuration),
    },
  });

  // `hello` doubles as this process's readiness signal for ADMINISTRATIVE work:
  // the graph above is already built, so the first `agentkit.fetch` after
  // `accept` is served rather than refused. Provider work is separately gated —
  // this process claims nothing until the host installs a generation carrying a
  // provider (`composition/configuration.ts`).
  const accept = await peer.start();
  logger.info("assistant bridge open", {
    appVersion: accept.appVersion,
    bridgeVersion: accept.bridgeVersion,
    dbPath,
    executing: app.configuration.executing,
  });

  process.on("SIGINT", () => void stop("SIGINT", 0));
  process.on("SIGTERM", () => void stop("SIGTERM", 0));

  // The pipe closing is the supervisor going away — there is nobody left to
  // answer, so exiting non-zero tells it this was not a requested shutdown.
  await peer.waitForClose();
  await stop("bridge closed", 1);
}

if (import.meta.main) {
  try {
    await main(writeFrame as (bytes: Uint8Array) => Promise<void>);
  } catch (err) {
    // No frame can be written for this: either the arguments were wrong (the
    // handshake never happened) or the graph failed to build. stderr and a
    // non-zero exit are the whole report, and the supervisor reads both.
    process.stderr.write(
      `onecad-assistant-host: ${err instanceof Error ? err.message : String(err)}\n`,
    );
    process.exit(2);
  }
}
