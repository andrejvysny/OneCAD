import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { readdir } from "node:fs/promises";
import { createRestHandler } from "agentkit/transport-http";
import { buildApp, HOST_VERSION, type App } from "../src/composition/app.js";
import { resolveDbPath } from "../src/main.js";
import { SYNTHETIC_ORIGIN } from "../src/bridge/verbs.js";
import { TOOL_NAMESPACE } from "../src/tools/index.js";

const dirs: string[] = [];
const apps: App[] = [];

afterEach(async () => {
  for (const app of apps.splice(0)) await app.stop();
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function tempAppDataDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "onecad-assistant-"));
  dirs.push(dir);
  return dir;
}

/**
 * The factory a test supplies instead of a real provider client. It is not a
 * mock of a provider — it is a tripwire: nothing in these tests should reach a
 * provider at all, so being called is the failure.
 */
function refusingProviderFactory(): never {
  throw new Error("a provider client was built during a test that should not need one");
}

async function boot(): Promise<{ app: App; appDataDir: string }> {
  const appDataDir = tempAppDataDir();
  const dbPath = resolveDbPath(appDataDir);
  mkdirSync(dirname(dbPath), { recursive: true });
  const app = await buildApp({ dbPath, providerFactory: refusingProviderFactory });
  apps.push(app);
  return { app, appDataDir };
}

function get(path: string): Request {
  return new Request(new URL(path, SYNTHETIC_ORIGIN).toString(), { method: "GET" });
}

describe("composition root", () => {
  test("builds the graph over the host-chosen sqlite path", async () => {
    const { app, appDataDir } = await boot();
    expect(app.dbPath).toBe(join(appDataDir, "assistant", "agentkit.sqlite"));
    expect(existsSync(app.dbPath)).toBe(true);
    expect(await readdir(join(appDataDir, "assistant"))).toContain("agentkit.sqlite");
  });

  test("the REST handler answers from memory, with no server anywhere", async () => {
    const { app } = await boot();
    const restFetch = createRestHandler(app.deps);
    const response = await restFetch(get("/v1/version"));
    expect(response.status).toBe(200);
    const body = (await response.json()) as { packages?: Record<string, string> };
    expect(body.packages).toMatchObject({ "onecad-assistant-host": HOST_VERSION });
  });

  test("GET /v1/tools advertises exactly the read-only tool a turn would stage", async () => {
    const { app } = await boot();
    const restFetch = createRestHandler(app.deps);
    const response = await restFetch(get("/v1/tools"));
    expect(response.status).toBe(200);
    const tools = (await response.json()) as Array<{ name: string; effect: string }>;
    expect(tools.map((t) => t.name)).toEqual(["onecad_host_info"]);
    expect(tools.every((t) => t.effect === "read")).toBe(true);
  });

  test("the tool namespace is this module's own and not a reserved one", () => {
    expect(TOOL_NAMESPACE).toBe("onecad");
    expect(["agentkit", "chat", "mcp"]).not.toContain(TOOL_NAMESPACE);
  });

  test("no provider is seeded: configuration arrives from the host, not from env", async () => {
    const { app } = await boot();
    expect(await app.store.providers.listProviders()).toEqual([]);
  });

  test("stop is idempotent, so a second signal is safe", async () => {
    const { app } = await boot();
    await app.stop();
    await app.stop();
    apps.splice(apps.indexOf(app), 1);
  });
});

describe("no listener exists", () => {
  test("building the app and serving a request never binds a socket", async () => {
    // A trap rather than an inspection: if any line of the composition or of
    // AgentKit beneath it reached for a listening socket, this throws with a
    // stack that names the caller.
    const realServe = Bun.serve;
    const realListen = Bun.listen;
    const attempts: string[] = [];
    // Deliberately replacing a runtime global for the duration of the test.
    Bun.serve = (() => {
      attempts.push("Bun.serve");
      throw new Error("Bun.serve must never be called by the assistant host");
    }) as unknown as typeof Bun.serve;
    Bun.listen = (() => {
      attempts.push("Bun.listen");
      throw new Error("Bun.listen must never be called by the assistant host");
    }) as unknown as typeof Bun.listen;
    try {
      const { app } = await boot();
      const restFetch = createRestHandler(app.deps);
      expect((await restFetch(get("/v1/version"))).status).toBe(200);
      expect((await restFetch(get("/v1/chats"))).status).toBe(200);
    } finally {
      Bun.serve = realServe;
      Bun.listen = realListen;
    }
    expect(attempts).toEqual([]);
  });

  test("no source file reaches for a server, a listener, or MCP", async () => {
    // The trap above covers the paths a test happens to walk. This covers the
    // rest of the package, including code a test never reaches — and it is the
    // check that fails when someone adds `Bun.serve` to `main.ts` next year.
    const root = new URL("../src/", import.meta.url).pathname;
    const forbidden: Array<[RegExp, string]> = [
      [/\bBun\.serve\b/, "Bun.serve"],
      [/\bBun\.listen\b/, "Bun.listen"],
      [/\bserveRest\b/, "serveRest"],
      [/agentkit\/mcp-client/, "agentkit/mcp-client"],
      [/agentkit\/mcp-server/, "agentkit/mcp-server"],
      [/createMcpServerHandler|createMcpToolSetContributor|McpClientManager/, "an MCP symbol"],
    ];
    const offences: string[] = [];
    for (const file of await walk(root)) {
      // Comments are stripped first: this check is about what the code DOES,
      // and the composition root's doc comment explains at length which of
      // these it deliberately does not call.
      const source = stripComments(await Bun.file(file).text());
      for (const [pattern, label] of forbidden) {
        if (pattern.test(source)) offences.push(`${file.slice(root.length)}: ${label}`);
      }
    }
    expect(offences).toEqual([]);
  });
});

function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[^\n"'`]*\/\/.*$/gm, "");
}

async function walk(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...(await walk(full)));
    else if (entry.name.endsWith(".ts")) files.push(full);
  }
  return files;
}
