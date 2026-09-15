/*
 * `config.install`: the authoritative provider projection, and the execution
 * gate it opens (`docs/assistant/wire-protocol.md` §5).
 *
 * The defect these cover is the one a mock cannot see. `buildApp` seeds nothing,
 * AgentKit's `TurnRunner` resolves the provider from ITS OWN store, and so a
 * fresh install reached `no_provider` on the first message no matter what Rust
 * had registered. Every case here therefore drives the REAL route over the REAL
 * store, and the end-to-end case drives the REAL binary — the one thing none of
 * them may do is put a provider row in the database by hand, which would be a
 * test of the bypass rather than of the fix.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { Buffer } from "node:buffer";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createRestHandler } from "agentkit/transport-http";
import type {
  AiProviderCapabilities,
  AiProviderConfig,
  AiProviderModel,
  AiRunEvent,
} from "agentkit/contracts";
import type { AiChatRequest, AiProviderClient } from "agentkit/core";
import { MockProviderClient } from "agentkit/testing";
import { buildApp, type App } from "../src/composition/app.js";
import {
  createConfigInstallRoute,
  parseConfigInstall,
  PROJECTED_BASE_URL,
  VERB_CONFIG_INSTALL,
  type ConfigInstallPayload,
  type ConfigInstallResult,
} from "../src/composition/configuration.js";
import { BridgeRequestError, type VerbRoute } from "../src/bridge/peer.js";
import { SYNTHETIC_ORIGIN } from "../src/bridge/verbs.js";
import { resolveDbPath } from "../src/main.js";
import { HostMock, Pipe, bytes, text } from "./harness.js";

const MAIN = new URL("../src/main.ts", import.meta.url).pathname;

const dirs: string[] = [];
const apps: App[] = [];
const cleanups: Array<() => void> = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  for (const app of apps.splice(0)) await app.stop();
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function tempAppDataDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "onecad-assistant-config-"));
  dirs.push(dir);
  return dir;
}

function refusingProviderFactory(): never {
  throw new Error("a provider client was built during a test that should not need one");
}

/** The production composition, with the provider seam left to the caller. */
async function boot(options: {
  appDataDir?: string;
  providerFactory?: (config: AiProviderConfig) => AiProviderClient;
}): Promise<App> {
  const appDataDir = options.appDataDir ?? tempAppDataDir();
  const dbPath = resolveDbPath(appDataDir);
  mkdirSync(dirname(dbPath), { recursive: true });
  const app = await buildApp({
    dbPath,
    providerFactory: options.providerFactory ?? refusingProviderFactory,
  });
  apps.push(app);
  return app;
}

/** Calls the verb the way the bridge does, so the parse and the route run. */
async function install(route: VerbRoute, payload: unknown): Promise<ConfigInstallResult> {
  const result = await route.handle({
    id: 0,
    principal: "ui",
    payload,
    signal: new AbortController().signal,
  });
  return result.payload as ConfigInstallResult;
}

function projection(generation: number, model: string): ConfigInstallPayload {
  return {
    generation,
    provider: {
      id: "local",
      model,
      capabilities: { streaming: true, toolCalling: true, modelList: true },
    },
  };
}

describe("config.install — the payload is a boundary", () => {
  test("a positive generation and either a provider or null are the whole shape", () => {
    expect(parseConfigInstall({ generation: 3, provider: null })).toEqual({
      generation: 3,
      provider: null,
    });
    expect(parseConfigInstall(projection(1, "qwen3:8b")).provider?.model).toBe("qwen3:8b");
  });

  test("a malformed payload is a named refusal rather than a guess", () => {
    const cases: unknown[] = [
      null,
      { provider: null },
      { generation: 0, provider: null },
      { generation: 1.5, provider: null },
      { generation: 1, provider: { id: "", model: "m", capabilities: {} } },
      { generation: 1, provider: { id: "local", model: "", capabilities: {} } },
      { generation: 1, provider: { id: "local", model: "m" } },
      {
        generation: 1,
        provider: { id: "local", model: "m", capabilities: { streaming: true } },
      },
    ];
    for (const payload of cases) {
      expect(() => parseConfigInstall(payload)).toThrow(BridgeRequestError);
    }
  });
});

describe("config.install — the projection", () => {
  test("installs one enabled provider, its model, and the default-provider setting", async () => {
    const app = await boot({});
    const route = createConfigInstallRoute(app.configuration);
    expect(app.configuration.installedGeneration).toBeNull();
    expect(app.configuration.executing).toBe(false);

    const result = await install(route, projection(1, "qwen3:8b"));
    expect(result).toEqual({ generation: 1, providerId: "local", executing: true });

    const providers = await app.store.providers.listProviders();
    expect(providers).toHaveLength(1);
    expect(providers[0]).toMatchObject({
      id: "local",
      defaultModel: "qwen3:8b",
      enabled: true,
      // No endpoint crosses (§5): what is stored is a name that cannot resolve.
      baseUrl: PROJECTED_BASE_URL,
    });
    expect(providers[0]?.apiKey ?? null).toBeNull();
    expect((await app.store.settings.getSettings()).defaultProviderId).toBe("local");
    expect(await app.store.providers.getCapabilities("local")).toMatchObject({
      streaming: true,
      toolCalling: true,
      modelList: true,
    });
  });

  test("a later generation replaces the model, and the gate stays open", async () => {
    const app = await boot({});
    const route = createConfigInstallRoute(app.configuration);
    await install(route, projection(1, "qwen3:8b"));
    const second = await install(route, projection(2, "llama3.2:3b"));

    expect(second).toEqual({ generation: 2, providerId: "local", executing: true });
    expect(await app.store.providers.listProviders()).toHaveLength(1);
    expect((await app.store.providers.getProvider("local"))?.defaultModel).toBe("llama3.2:3b");
  });

  test("a null provider clears the projection and closes the gate", async () => {
    const app = await boot({});
    const route = createConfigInstallRoute(app.configuration);
    await install(route, projection(1, "qwen3:8b"));

    const cleared = await install(route, { generation: 2, provider: null });
    expect(cleared).toEqual({ generation: 2, providerId: null, executing: false });
    // Unreachable by either resolution path: no default names it, and nothing
    // enabled is left for the fallback to find.
    expect((await app.store.settings.getSettings()).defaultProviderId).toBeUndefined();
    expect((await app.store.providers.listProviders()).filter((p) => p.enabled)).toEqual([]);
  });

  test("a stale generation is refused and changes nothing", async () => {
    const app = await boot({});
    const route = createConfigInstallRoute(app.configuration);
    await install(route, projection(7, "qwen3:8b"));

    for (const stale of [6, 7]) {
      await expect(install(route, projection(stale, "superseded"))).rejects.toThrow(
        /is not newer than the installed 7/,
      );
    }
    expect((await app.store.providers.getProvider("local"))?.defaultModel).toBe("qwen3:8b");
    expect(app.configuration.installedGeneration).toBe(7);
  });

  test("the refusal names its own error code, so the host can tell it apart", async () => {
    const app = await boot({});
    const route = createConfigInstallRoute(app.configuration);
    await install(route, projection(2, "qwen3:8b"));
    const caught = await install(route, projection(1, "old")).catch((err: unknown) => err);
    expect(caught).toBeInstanceOf(BridgeRequestError);
    expect((caught as BridgeRequestError).code).toBe("stale_generation");
  });

  test("only the trusted webview surface may configure this process", () => {
    const app = { configuration: null } as unknown as Parameters<
      typeof createConfigInstallRoute
    >[0];
    expect(createConfigInstallRoute(app).principals).toEqual(["ui"]);
  });
});

describe("the execution gate", () => {
  test("a new connection claims nothing, even over a database that still holds one", async () => {
    // The restart case, exactly: the sqlite file SURVIVES, so the provider row
    // is still there on the next boot. A row is not an acknowledgement — the
    // host that owns the configuration has not spoken on this connection yet.
    const appDataDir = tempAppDataDir();
    const first = await boot({ appDataDir });
    await install(createConfigInstallRoute(first.configuration), projection(1, "qwen3:8b"));
    expect(first.configuration.executing).toBe(true);
    await first.stop();
    apps.splice(apps.indexOf(first), 1);

    const second = await boot({ appDataDir });
    expect(await second.store.providers.listProviders()).toHaveLength(1);
    expect(second.configuration.installedGeneration).toBeNull();
    expect(second.configuration.executing).toBe(false);

    // And the host's re-install on the new connection opens it again.
    await install(createConfigInstallRoute(second.configuration), projection(1, "qwen3:8b"));
    expect(second.configuration.executing).toBe(true);
  });

  test("an administrative request is served while the gate is closed", async () => {
    const app = await boot({});
    const restFetch = createRestHandler(app.deps);
    const version = await restFetch(
      new Request(new URL("/v1/version", SYNTHETIC_ORIGIN).toString()),
    );
    expect(version.status).toBe(200);
    expect(app.configuration.executing).toBe(false);
  });
});

// ───────────────────────────────────────────────────────────────────────────
// The mid-flight policy (see `configuration.ts` — "Mid-flight settings edits").
// ───────────────────────────────────────────────────────────────────────────

/**
 * A scripted client that can be parked inside `streamChat`, so a settings edit
 * can be made to land while a run demonstrably holds the provider — rather than
 * racing a sleep against it.
 */
class ParkableProvider implements AiProviderClient {
  readonly id = "local";
  readonly kind = "openai-compatible";
  /** The model each call was made with, in call order. */
  readonly models: string[] = [];
  readonly entered: Promise<void>;
  private enter!: () => void;
  private gate: Promise<void> | undefined;
  private open: (() => void) | undefined;

  constructor(private readonly inner: MockProviderClient) {
    this.entered = new Promise<void>((resolve) => {
      this.enter = resolve;
    });
  }

  park(): void {
    this.gate = new Promise<void>((resolve) => {
      this.open = resolve;
    });
  }

  release(): void {
    this.open?.();
  }

  capabilities(): Promise<AiProviderCapabilities> {
    return this.inner.capabilities();
  }

  listModels(): Promise<AiProviderModel[]> {
    return this.inner.listModels();
  }

  async *streamChat(input: AiChatRequest): AsyncIterable<AiRunEvent> {
    this.models.push(input.model);
    this.enter();
    const gate = this.gate;
    this.gate = undefined;
    if (gate !== undefined) await gate;
    yield* this.inner.streamChat(input);
  }
}

describe("a settings edit across a run in flight", () => {
  test("binds the model at submit: the in-flight run keeps it, the next one takes the new", async () => {
    const inner = new MockProviderClient();
    inner.setScript([
      { steps: [{ kind: "text", content: "first" }] },
      { steps: [{ kind: "text", content: "second" }] },
    ]);
    const provider = new ParkableProvider(inner);
    const app = await boot({ providerFactory: () => provider });
    const route = createConfigInstallRoute(app.configuration);
    const restFetch = createRestHandler(app.deps);
    const call = async (
      method: string,
      path: string,
      body?: unknown,
      headers: Record<string, string> = {},
    ): Promise<Response> =>
      restFetch(
        new Request(new URL(path, SYNTHETIC_ORIGIN).toString(), {
          method,
          headers: { "content-type": "application/json", ...headers },
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        }),
      );

    await install(route, projection(1, "model-a"));
    const chatId = ((await (await call("POST", "/v1/chats", {})).json()) as { id: string }).id;

    provider.park();
    const first = (await (
      await call("POST", `/v1/chats/${chatId}/messages`, { content: "one" }, {
        "idempotency-key": "mid-flight-1",
      })
    ).json()) as { runId: string };
    await provider.entered;

    // The edit lands while the run holds the provider. It must NOT park behind
    // the completion — that is the whole reason the gate is not quiesced here.
    const applied = await install(route, projection(2, "model-b"));
    expect(applied).toMatchObject({ generation: 2, executing: true });

    provider.release();
    const settled = await waitForRun(call, first.runId);
    expect(settled.status).toBe("completed");

    const second = (await (
      await call("POST", `/v1/chats/${chatId}/messages`, { content: "two" }, {
        "idempotency-key": "mid-flight-2",
      })
    ).json()) as { runId: string };
    expect((await waitForRun(call, second.runId)).status).toBe("completed");

    // The run that was in flight spoke to the model it was submitted under; the
    // one submitted after the edit spoke to the new one. Nothing changed under a
    // run that had already started.
    expect(provider.models).toEqual(["model-a", "model-b"]);
  }, 30_000);
});

async function waitForRun(
  call: (method: string, path: string) => Promise<Response>,
  runId: string,
): Promise<{ status: string; error?: string }> {
  const deadline = Date.now() + 20_000;
  for (;;) {
    const run = (await (await call("GET", `/v1/runs/${runId}`)).json()) as {
      status: string;
      error?: string;
    };
    if (!["queued", "running"].includes(run.status)) return run;
    if (Date.now() > deadline) throw new Error(`run did not settle: ${JSON.stringify(run)}`);
    await Bun.sleep(25);
  }
}

// ───────────────────────────────────────────────────────────────────────────
// The acceptance case: one completed turn, from an empty database, over the
// real binary, with nothing hand-seeded.
// ───────────────────────────────────────────────────────────────────────────

/** One streamed OpenAI-compatible completion, as a scripted loopback answer. */
function completionSse(content: string): string {
  return [
    `data: ${JSON.stringify({ choices: [{ delta: { content }, finish_reason: null }] })}`,
    "",
    `data: ${JSON.stringify({ choices: [{ delta: {}, finish_reason: "stop" }] })}`,
    "",
    "data: [DONE]",
    "",
    "",
  ].join("\n");
}

interface ProviderCall {
  providerId: string;
  path: string;
  model: string;
}

describe("a turn, end to end, from an empty database", () => {
  test(
    "the production configuration path is the only thing that makes a turn possible",
    async () => {
      const dir = mkdtempSync(join(tmpdir(), "onecad-assistant-turn-"));
      cleanups.push(() => rmSync(dir, { recursive: true, force: true }));

      const child = Bun.spawn(["bun", MAIN, "--app-data-dir", dir, "--log-level", "error"], {
        stdin: "pipe",
        stdout: "pipe",
        stderr: "pipe",
      });
      cleanups.push(() => child.kill());

      const toSidecar = new Pipe();
      const fromSidecar = new Pipe();
      void (async () => {
        for await (const chunk of child.stdout as unknown as AsyncIterable<Uint8Array>) {
          await fromSidecar.write(chunk);
        }
        fromSidecar.close();
      })();
      void (async () => {
        for await (const chunk of toSidecar) {
          child.stdin.write(chunk);
          await child.stdin.flush();
        }
      })();

      const host = new HostMock(toSidecar, fromSidecar);
      await host.waitFor(-1, (f) => f.envelope.t === "hello");

      // The host side of the gateway: a scripted loopback provider. It answers
      // the OPERATION PATH the sidecar names against the PROVIDER ID it names,
      // which is the whole point — no URL ever crosses.
      const calls: ProviderCall[] = [];
      host.routes.set("provider.fetch", async (payload, id) => {
        const call = payload as {
          providerId: string;
          path: string;
          bodyBase64?: string;
        };
        const body = JSON.parse(
          Buffer.from(call.bodyBase64 ?? "", "base64").toString("utf8"),
        ) as { model?: string };
        calls.push({
          providerId: call.providerId,
          path: call.path,
          model: body.model ?? "",
        });
        await host.send({
          t: "res",
          id,
          ok: true,
          payload: {
            status: 200,
            statusText: "OK",
            headers: { "content-type": "text/event-stream" },
          },
        });
        await host.send({ t: "chunk", id, seq: 0 }, bytes(completionSse("Hello from OneCAD.")));
        await host.send({ t: "end", id, ok: true });
      });

      const rest = async (
        method: string,
        path: string,
        body?: unknown,
        headers: Record<string, string> = {},
      ): Promise<{ status: number; json: unknown }> => {
        const payload: Record<string, unknown> = {
          method,
          path,
          headers: body === undefined ? headers : { "content-type": "application/json", ...headers },
        };
        if (body !== undefined) {
          payload.bodyBase64 = Buffer.from(JSON.stringify(body), "utf8").toString("base64");
        }
        const answer = await host.call("agentkit.fetch", payload, { stream: true });
        const envelope = answer.res.envelope;
        if (envelope.t !== "res" || !envelope.ok) throw new Error("agentkit.fetch failed");
        const head = envelope.payload as { status: number };
        const raw = text(answer.chunks);
        return { status: head.status, json: raw === "" ? null : JSON.parse(raw) };
      };

      // Nothing is seeded: this is what the sidecar knows before the host speaks.
      const before = await rest("GET", "/v1/providers");
      expect(before.status).toBe(200);
      expect(before.json).toEqual([]);

      // THE production configuration path. The generation is Rust's to mint; the
      // payload carries an id, a model and capabilities, and nothing else.
      const installed = await host.call(VERB_CONFIG_INSTALL, projection(1, "qwen3:8b"));
      expect(installed.res.envelope).toMatchObject({
        ok: true,
        payload: { generation: 1, providerId: "local", executing: true },
      });

      const chat = await rest("POST", "/v1/chats", {});
      expect(chat.status).toBe(201);
      const chatId = (chat.json as { id: string }).id;

      const submitted = await rest(
        "POST",
        `/v1/chats/${chatId}/messages`,
        { content: "Say hello" },
        { "idempotency-key": "acceptance-1" },
      );
      expect(submitted.status).toBe(201);
      const turn = submitted.json as { runId: string; assistantMessageId: string };

      // Poll the durable record rather than the SSE stream: the assertion is
      // that the turn SETTLED, and the run row carries that whether or not
      // anyone was subscribed when it landed.
      const deadline = Date.now() + 25_000;
      let run: { status: string; error?: string } = { status: "unknown" };
      for (;;) {
        run = (await rest("GET", `/v1/runs/${turn.runId}`)).json as typeof run;
        if (!["queued", "running"].includes(run.status)) break;
        if (Date.now() > deadline) throw new Error(`run did not settle: ${JSON.stringify(run)}`);
        await Bun.sleep(100);
      }
      expect(run).toMatchObject({ status: "completed" });

      // One call, to the id the host registered, on the operation path — and at
      // the model the projection named, which is the proof the turn ran on the
      // configuration rather than on something this process chose.
      expect(calls).toHaveLength(1);
      expect(calls[0]).toEqual({
        providerId: "local",
        path: "/chat/completions",
        model: "qwen3:8b",
      });

      const messages = await rest("GET", `/v1/chats/${chatId}/messages`);
      const items = (messages.json as { items: Array<{ id: string; content: string }> }).items;
      expect(items.find((m) => m.id === turn.assistantMessageId)?.content).toBe(
        "Hello from OneCAD.",
      );

      const bye = await host.call("shutdown", {});
      expect(bye.res.envelope).toMatchObject({ ok: true });
      expect(await child.exited).toBe(0);
    },
    60_000,
  );
});
