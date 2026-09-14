/**
 * The composition root: AgentKit's object graph for OneCAD's sidecar, in the
 * order `examples/desktop-host/src/wiring.ts` establishes, with three things
 * removed and nothing added.
 *
 *  1. Ambient ports — `Clock` / `IdGenerator` / `SecretStore`.
 *  2. Storage — one `SqliteAssistantStore` behind every port.
 *  3. Tools — one read-only contributor (`../tools/index.ts`).
 *  4. The write pipeline — `SessionWritePolicy` + `ProposalService` over a
 *     `NoopProposalApplier`. Nothing is ever staged, but `recoverOnBoot`
 *     needs a `ProposalService` to reconcile against.
 *  5. `SingleProcessTaskRunner` — the queue, built before `TurnRunner`.
 *  6. `TurnRunner` — the durable worker over the run loop.
 *  7. `ExecutorRegistry` + `ChatTurnExecutor`, via `createDispatchingWorker`.
 *  8. `recoverOnBoot` — clean up after the last crash, BEFORE claiming work.
 *  9. `taskRunner.startWorker` — start claiming.
 * 10. `RestHandlerDeps` — what `createRestHandler` needs. `main.ts` holds the
 *     resulting function in memory and feeds it frames.
 *
 * ## What is deliberately missing
 *
 * **No `Bun.serve`, and no socket of any kind.** The example's step 11 binds a
 * port; this one does not, because a listening socket is an unauthenticated
 * entry point into a process that holds the user's chat history and can spend
 * their provider credits. `createRestHandler` is a plain
 * `(Request) => Promise<Response>`, so the transport it is given can be the
 * OCAK1 bridge — which is reachable only by this process's parent, over a pipe
 * it owns.
 *
 * **No MCP, in either direction.** No `mcp-client` contributor (which spawns a
 * process of a stored config's choosing) and no `mcp-server` handler (which
 * hands tool execution to whoever holds a token). Neither is imported anywhere
 * in this package; the capability is absent from the binary, not switched off
 * by a flag someone can set.
 *
 * **No provider seeding from the environment.** The example reads
 * `AGENTKIT_*` env vars on first boot. Nothing here does: configuration comes
 * from the Rust host, which knows the app's data directory and the user's
 * settings. A sidecar that picks its own database path or its own model
 * endpoint is a sidecar whose behaviour the app cannot state.
 */
import { SqliteAssistantStore } from "agentkit/adapters-sqlite";
import { MemorySecretStore } from "agentkit/adapters-memory";
import type { AiProviderConfig } from "agentkit/contracts";
import type { AiProviderClient } from "agentkit/core";
import {
  ChatTurnExecutor,
  ExecutorRegistry,
  ProposalService,
  SessionWritePolicy,
  TaskService,
  TurnRunner,
  type Clock,
  type IdGenerator,
  type Logger,
  type SecretStore,
  type ToolGuard,
  type ToolSetContributor,
  createContributorToolCatalog,
  createDispatchingWorker,
  defaultClock,
  defaultIds,
  recoverOnBoot,
} from "agentkit/host";
import { SingleProcessTaskRunner } from "agentkit/runner-local";
import type { RestHandlerDeps } from "agentkit/transport-http";
import { NoopProposalApplier } from "./noopApplier.js";
import { createOneCadToolSetContributor } from "../tools/index.js";

/** Reported to the host in `hello`, and to a client by `GET /v1/version`. */
export const HOST_VERSION = "0.1.0";

/** The owner id `SingleProcessTaskRunner` stamps on the leases it claims. */
const WORKER_OWNER_ID = "onecad-assistant-host";

/** How many turns this process runs at once. One user, one window; two is slack. */
const WORKER_CONCURRENCY = 2;

export interface BuildAppOptions {
  /** Absolute. Chosen by the Rust host from the app data directory, never here. */
  dbPath: string;
  /**
   * REQUIRED, with no default.
   *
   * The example defaults to `OpenAiCompatibleClient.fromConfig(config)`, which
   * uses the global `fetch` — i.e. the network. There is no such default here
   * on purpose: making the caller supply the factory means no code path in this
   * package can reach a provider except through the one the caller wired, and
   * `main.ts` wires the OCAK1 gateway (`../bridge/verbs.ts`). A test supplies a
   * scripted client and is therefore offline by construction, not by discipline.
   */
  providerFactory(config: AiProviderConfig): AiProviderClient;
  /**
   * Defaults to a process-lifetime, in-memory store. Adequate because this
   * build's provider credentials live on the Rust side of the bridge — the
   * gateway injects them, so nothing durable is expected here.
   */
  secrets?: SecretStore;
  clock?: Clock;
  ids?: IdGenerator;
  logger?: Logger;
}

export interface App {
  store: SqliteAssistantStore;
  dbPath: string;
  turnRunner: TurnRunner;
  taskService: TaskService;
  proposals: ProposalService;
  taskRunner: SingleProcessTaskRunner;
  /** What `createRestHandler` needs. `main.ts` never turns it into a server. */
  deps: RestHandlerDeps;
  /** Stops the worker, disposes contributors, closes the DB. Idempotent. */
  stop(): Promise<void>;
}

export async function buildApp(options: BuildAppOptions): Promise<App> {
  const clock = options.clock ?? defaultClock;
  const ids = options.ids ?? defaultIds;
  const secrets = options.secrets ?? new MemorySecretStore();
  const logger = options.logger;

  // 2. Storage. The caller creates the parent directory — `SqliteAssistantStore`
  // opens a file and does not make one.
  const store = new SqliteAssistantStore(options.dbPath, { clock, ids });

  // 3. Tools: read-only, and the same array every consumer below stages.
  const contributors: ToolSetContributor[] = [createOneCadToolSetContributor()];

  // The guard chain, defined ONCE and shared. Empty today; the point of the
  // single array is that `TurnRunner` and the catalogue route cannot advertise
  // a different tool set from the one a turn actually runs.
  const toolGuards: ToolGuard[] = [];

  // 4. The write pipeline. See `./noopApplier.ts` — the applier throwing is
  // this build's no-mutation guarantee, not a placeholder to be quietly filled.
  const policy = new SessionWritePolicy({ clock });
  const proposals = new ProposalService({
    store,
    applier: new NoopProposalApplier(),
    policy,
    clock,
    ids,
    logger,
  });

  // 5. The queue. Built before `TurnRunner`, which takes it as a dependency.
  const taskRunner = new SingleProcessTaskRunner({ store, clock, logger });

  // 6. The durable worker over core's run loop.
  const turnRunner = new TurnRunner({
    store,
    taskRunner,
    providerFactory: options.providerFactory,
    secrets,
    contributors,
    toolGuards,
    clock,
    ids,
    logger,
  });
  const taskService = new TaskService({ store, taskRunner, ids, clock });

  // 7. Dispatch through the registry so every executor gets `spawnChild` wired.
  const registry = new ExecutorRegistry();
  registry.register(new ChatTurnExecutor(turnRunner));

  // 8. Clean up after the last crash BEFORE claiming anything. The supervisor
  // restarts this process on a malformed frame, so "the last crash" is a
  // routine event here rather than an exceptional one.
  await recoverOnBoot({ taskRunner, proposals, logger });

  // 9. Start claiming.
  const handle = await taskRunner.startWorker(
    createDispatchingWorker(registry, { store, clock, logger, taskService }),
    { concurrency: WORKER_CONCURRENCY, ownerId: WORKER_OWNER_ID },
  );

  // 10. What the transport needs.
  //
  // No `basePath`: the paths arriving over `agentkit.fetch` are minted by the
  // Rust host from `REST_ROUTES`, so there is no mount prefix to strip and no
  // second place that has to agree about one.
  //
  // No `cors`, no `authenticate`, no `authorize`: this handler is not reachable
  // from a browser or from the network. Its only caller is the bridge, and §4
  // of the wire protocol is where authority is decided — the Rust reader stamps
  // the principal, and `agentkit.fetch` is `ui`-only.
  const deps: RestHandlerDeps = {
    store,
    turns: turnRunner,
    tasks: taskService,
    proposals,
    secrets,
    // `GET /v1/tools` answers 200 because of this line: the catalogue stages
    // the SAME contributors a turn does, so what the route advertises and what
    // a run receives cannot drift.
    toolCatalog: createContributorToolCatalog({
      contributors,
      guards: toolGuards,
      logger,
    }),
    packages: { "onecad-assistant-host": HOST_VERSION },
    logger,
  };

  return {
    store,
    dbPath: options.dbPath,
    turnRunner,
    taskService,
    proposals,
    taskRunner,
    deps,
    async stop(): Promise<void> {
      await handle.stop();
      await turnRunner.disposeContributors();
      store.close();
    },
  };
}
