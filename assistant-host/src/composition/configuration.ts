/**
 * `config.install` — the authoritative provider projection, applied
 * (`docs/assistant/wire-protocol.md` §5, "Host → sidecar, configuration").
 *
 * ## Why this exists
 *
 * AgentKit resolves the provider for a turn from ITS OWN store
 * (`TurnRunner.resolveProvider` reads `settings.defaultProviderId`, then
 * `store.providers`). Nothing in this package seeds that store — the composition
 * root deliberately reads no environment — so a freshly installed OneCAD reached
 * `no_provider` on the first message no matter what the Rust side had
 * registered. Rust is the configuration authority; the row in AgentKit's
 * provider table is a PROJECTION of it, installed over the bridge and never
 * invented here.
 *
 * ## What does not cross
 *
 * No endpoint and no credential. The projection carries `{ id, model,
 * capabilities }`, and the `baseUrl` written into the store is
 * {@link PROJECTED_BASE_URL} — a name in the reserved `.invalid` TLD that cannot
 * resolve. `createGatewayFetch` (`../bridge/verbs.ts`) only uses `baseUrl` to
 * check that the client stayed inside its own provider and to compute the
 * OPERATION PATH; the real endpoint is resolved on the Rust side from the
 * provider id. So this process still cannot name a URL, which is the property
 * ADR-0017 buys.
 *
 * ## The execution gate
 *
 * A connection starts with no acknowledged generation, and the worker is not
 * claiming. Administrative requests (`/v1/version`, `/v1/chats`, the tool
 * catalogue) are served throughout; provider work starts only once a generation
 * carrying a non-null provider has been applied. That is what makes "the user
 * has not configured a model yet" a visible waiting-for-configuration state
 * rather than a queued turn that fails `no_provider` once per retry until the
 * attempt budget is gone.
 *
 * The gate is closed on every new connection EVEN THOUGH the sqlite file
 * survives a restart and may still hold the previous projection. A row in the
 * database is not an acknowledgement: the host that owns the configuration has
 * not spoken yet, and acting on a stale row is how a restarted child answers
 * with a model the user removed.
 *
 * ## Mid-flight settings edits — the policy
 *
 * **A run binds its provider and model when the turn starts.** `runTurn`
 * resolves the provider and the model exactly once and carries both through
 * every pass of that turn, so a projection applied while a run is in flight
 * cannot change the model that run is speaking to. This module therefore
 * applies a new projection IMMEDIATELY and neither cancels nor quiesces
 * in-flight work: quiescing would park a settings edit behind a completion that
 * can legitimately take minutes, and cancelling would throw away an answer the
 * user is already reading.
 *
 * The one place the binding is re-taken is a RETRIED attempt, which re-enters
 * `runTurn` and resolves again. That is a new execution of the turn, not a
 * mid-flight swap, and it is the behaviour AgentKit's retry contract already
 * has.
 *
 * A null projection is the exception: it stops claiming FIRST and only then
 * removes the row, because a turn claimed after the provider is gone would fail
 * `RecordNotFoundError` and burn an attempt for a condition that is not an
 * error at all.
 */
import type { AiProviderCapabilities, AiProviderConfig } from "agentkit/contracts";
import type { AssistantStore, Logger, WorkerHandle } from "agentkit/host";
import {
  BridgeRequestError,
  type HandlerResult,
  type InboundRequest,
  type VerbRoute,
} from "../bridge/peer.js";

/** The verb this module serves (§5). */
export const VERB_CONFIG_INSTALL = "config.install";

/**
 * The `baseUrl` the projection writes into AgentKit's provider row.
 *
 * `.invalid` is reserved by RFC 2606 and can never resolve, so if any code path
 * in this process ever bypassed the gateway `fetch` and went to the network, it
 * fails immediately and loudly instead of quietly reaching a real server.
 */
export const PROJECTED_BASE_URL = "http://provider.invalid";

/** Every provider Rust projects is spoken to over the OpenAI-compatible shape. */
const PROJECTED_KIND = "openai-compatible";

/** The provider half of a `config.install` payload. */
export interface ProjectedProvider {
  /** The logical id the sidecar names in `provider.fetch`. Never a URL. */
  id: string;
  model: string;
  capabilities: AiProviderCapabilities;
}

/** The `config.install` request payload (§5). */
export interface ConfigInstallPayload {
  generation: number;
  provider: ProjectedProvider | null;
}

/** The `res` payload: the applied generation, echoed, plus what it did. */
export interface ConfigInstallResult {
  /** The generation now installed — what gates execution on the host side. */
  generation: number;
  /** The projected provider id, or `null` when the projection was cleared. */
  providerId: string | null;
  /** Whether provider work is being claimed as of this answer. */
  executing: boolean;
}

/**
 * Claiming, as one switch.
 *
 * `stop()` is deliberately NOT awaited by its caller: `SingleProcessTaskRunner`
 * clears its worker slot synchronously and only then awaits in-flight work, so
 * "nothing more will be claimed" is true the moment the call is made, while
 * "everything in flight has settled" can take as long as a completion does.
 * Answering the host after the second would mean a user who switches their
 * assistant off waits out the run they are switching off.
 */
export interface ExecutionGate {
  readonly open: boolean;
  start(): Promise<void>;
  stop(): void;
  /** Resolves when work in flight at the last {@link stop} has settled. */
  settle(): Promise<void>;
}

export function createExecutionGate(deps: {
  start(): Promise<WorkerHandle>;
  logger?: Logger | undefined;
}): ExecutionGate {
  let handle: WorkerHandle | undefined;
  let settling: Promise<void> = Promise.resolve();
  return {
    get open(): boolean {
      return handle !== undefined;
    },
    async start(): Promise<void> {
      if (handle !== undefined) return;
      handle = await deps.start();
      deps.logger?.info("assistant execution gate open");
    },
    stop(): void {
      if (handle === undefined) return;
      settling = handle.stop();
      handle = undefined;
      deps.logger?.info("assistant execution gate closed");
    },
    settle(): Promise<void> {
      return settling;
    },
  };
}

function bad(message: string): BridgeRequestError {
  return new BridgeRequestError("bad_request", message);
}

function requireBoolean(source: Record<string, unknown>, key: string): boolean {
  const value = source[key];
  if (typeof value !== "boolean") throw bad(`capabilities.${key} must be a boolean`);
  return value;
}

/**
 * The wire is a system boundary, so the payload is checked rather than trusted.
 * A malformed one is a named refusal on this request; it is not fatal to the
 * connection, because the frame itself was well formed.
 */
export function parseConfigInstall(payload: unknown): ConfigInstallPayload {
  if (typeof payload !== "object" || payload === null) throw bad("payload is not an object");
  const record = payload as Record<string, unknown>;
  const generation = record.generation;
  if (typeof generation !== "number" || !Number.isSafeInteger(generation) || generation < 1) {
    throw bad("generation must be a positive safe integer");
  }
  const provider = record.provider;
  if (provider === null || provider === undefined) return { generation, provider: null };
  if (typeof provider !== "object") throw bad("provider must be an object or null");
  const source = provider as Record<string, unknown>;
  const id = source.id;
  const model = source.model;
  if (typeof id !== "string" || id === "") throw bad("provider.id must be a non-empty string");
  if (typeof model !== "string" || model === "") {
    throw bad("provider.model must be a non-empty string");
  }
  const capabilities = source.capabilities;
  if (typeof capabilities !== "object" || capabilities === null) {
    throw bad("provider.capabilities must be an object");
  }
  const caps = capabilities as Record<string, unknown>;
  return {
    generation,
    provider: {
      id,
      model,
      capabilities: {
        streaming: requireBoolean(caps, "streaming"),
        toolCalling: requireBoolean(caps, "toolCalling"),
        modelList: requireBoolean(caps, "modelList"),
      },
    },
  };
}

/** The installed projection and the gate it controls. One per connection. */
export class Configuration {
  private installed: number | null = null;
  private projected: string | null = null;
  /** Installs are serialised: two overlapping ones must not interleave writes. */
  private queue: Promise<unknown> = Promise.resolve();

  constructor(
    private readonly deps: {
      store: AssistantStore;
      gate: ExecutionGate;
      logger?: Logger | undefined;
    },
  ) {}

  /** The acknowledged generation, or `null` while none has been installed. */
  get installedGeneration(): number | null {
    return this.installed;
  }

  /** Whether provider work is being claimed. */
  get executing(): boolean {
    return this.deps.gate.open;
  }

  install(payload: ConfigInstallPayload): Promise<ConfigInstallResult> {
    const run = this.queue.then(
      () => this.apply(payload),
      () => this.apply(payload),
    );
    this.queue = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async apply(payload: ConfigInstallPayload): Promise<ConfigInstallResult> {
    if (this.installed !== null && payload.generation <= this.installed) {
      // §5: a stale generation is refused, so a settings edit that lost the race
      // cannot overwrite the newer one that already landed.
      throw new BridgeRequestError(
        "stale_generation",
        `generation ${payload.generation} is not newer than the installed ${this.installed}`,
      );
    }
    const { store, gate } = this.deps;
    const provider = payload.provider;
    if (provider === null) {
      gate.stop();
      await store.settings.updateSettings({ defaultProviderId: undefined });
      if (this.projected !== null) {
        // DISABLED, not deleted. `resolveProvider` takes the default provider
        // when settings names one and otherwise the first ENABLED row, so a
        // disabled row with no default is unreachable by either path — and
        // `SqliteProviderStore.deleteProvider` removes the `providers` row
        // before the `provider_capabilities` row that references it, which its
        // own foreign key refuses. Disabling says the same thing without
        // depending on that ordering.
        const current = await store.providers.getProvider(this.projected);
        if (current !== null) await store.providers.upsertProvider({ ...current, enabled: false });
      }
      this.projected = null;
    } else {
      const config: AiProviderConfig = {
        id: provider.id,
        // The projection carries no label, because the contract carries none.
        // The id is what the host named and what `provider.fetch` will name.
        label: provider.id,
        kind: PROJECTED_KIND,
        baseUrl: PROJECTED_BASE_URL,
        defaultModel: provider.model,
        enabled: true,
        metadata: {},
      };
      await store.providers.upsertProvider(config);
      await store.providers.saveCapabilities(provider.id, provider.capabilities);
      await store.settings.updateSettings({ defaultProviderId: provider.id });
      this.projected = provider.id;
      await gate.start();
    }
    this.installed = payload.generation;
    this.deps.logger?.info("assistant configuration installed", {
      generation: payload.generation,
      providerId: this.projected,
      executing: gate.open,
    });
    return {
      generation: payload.generation,
      providerId: this.projected,
      executing: gate.open,
    };
  }
}

/** §5 host → sidecar: install the authoritative provider projection. */
export function createConfigInstallRoute(configuration: Configuration): VerbRoute {
  return {
    // §4: configuration comes from the trusted `#[tauri::command]` surface. A
    // request this process originated must never be able to configure itself.
    principals: ["ui"],
    async handle(request: InboundRequest): Promise<HandlerResult> {
      const result = await configuration.install(parseConfigInstall(request.payload));
      return { kind: "unary", payload: result };
    },
  };
}
