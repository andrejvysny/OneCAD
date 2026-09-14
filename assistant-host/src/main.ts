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
 * Where the store goes under the host's data directory. Fixed here rather than
 * passed, so the host names a directory it owns and this process names the one
 * file inside it that belongs to the assistant.
 */
const DB_RELATIVE_PATH = "assistant/agentkit.sqlite";

export interface HostArgs {
  appDataDir: string;
  logLevel: LogLevel;
}

export class ArgumentError extends Error {
  override readonly name = "ArgumentError";
}

/**
 * Argument handling is the one place where a bad input must produce a refusal
 * to start rather than a guess — a sidecar that silently picked a default data
 * directory would look healthy while writing to the wrong place.
 */
export function parseArgs(argv: readonly string[]): HostArgs {
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
  if (!appDataDir.startsWith("/")) {
    // Relative would resolve against whatever cwd the supervisor happened to
    // have, which is not something the app can state or reproduce.
    throw new ArgumentError(`--app-data-dir must be absolute, got ${appDataDir}`);
  }
  return { appDataDir, logLevel };
}

export function resolveDbPath(appDataDir: string): string {
  return `${appDataDir.replace(/\/+$/, "")}/${DB_RELATIVE_PATH}`;
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
  const { buildApp, HOST_VERSION } = await import("./composition/app.js");

  const dbPath = resolveDbPath(args.appDataDir);
  // `SqliteAssistantStore` opens a file; it does not create the directory it
  // sits in, and a missing parent surfaces as an opaque sqlite error.
  mkdirSync(dirname(dbPath), { recursive: true });

  // Declared before `buildApp` because the provider factory closes over it, and
  // assigned before any turn can run, because a turn needs a request from a
  // host that has already seen `hello`.
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
    },
  });

  // `hello` doubles as this process's readiness signal: the graph above is
  // already built, so the first `agentkit.fetch` after `accept` is served
  // rather than refused.
  const accept = await peer.start();
  logger.info("assistant bridge open", {
    appVersion: accept.appVersion,
    bridgeVersion: accept.bridgeVersion,
    dbPath,
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
