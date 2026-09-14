import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const PKG_DIR = dirname(import.meta.dir);
const SERVER = join(PKG_DIR, "src", "mcp", "server.ts");
const root = mkdtempSync(join(tmpdir(), "tauri-agent-smoke-"));

afterAll(() => rmSync(root, { recursive: true, force: true }));

interface RpcResponse {
  id?: number;
  result?: {
    tools?: Array<{ name: string; description?: string }>;
    isError?: boolean;
    content?: Array<{ type: string; text?: string }>;
  };
}

// The SDK's stdio transport frames messages as newline-delimited JSON, no Content-Length.
async function* jsonLines(stream: ReadableStream<Uint8Array>): AsyncGenerator<RpcResponse> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    buf += decoder.decode(value, { stream: true });
    let nl = buf.indexOf("\n");
    while (nl >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (line.length > 0) yield JSON.parse(line) as RpcResponse;
      nl = buf.indexOf("\n");
    }
  }
}

test("serves initialize + tools/list over stdio", async () => {
  const proc = Bun.spawn(["bun", SERVER], {
    cwd: PKG_DIR,
    env: { ...process.env, TAURI_AGENT_ROOT: root, TAURI_AGENT_LOG: "error" },
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  try {
    const send = (msg: unknown): void => {
      proc.stdin.write(`${JSON.stringify(msg)}\n`);
      proc.stdin.flush();
    };
    send({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "smoke", version: "0" },
      },
    });
    send({ jsonrpc: "2.0", method: "notifications/initialized" });
    send({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} });
    send({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "session_status", arguments: {} } });

    const seen = new Map<number, RpcResponse>();
    const read = (async () => {
      for await (const msg of jsonLines(proc.stdout)) {
        if (typeof msg.id === "number") seen.set(msg.id, msg);
        if (seen.has(3)) return;
      }
    })();
    await Promise.race([
      read,
      Bun.sleep(20_000).then(() => {
        throw new Error("timed out waiting for tools/list");
      }),
    ]);

    expect(seen.get(1)?.result).toBeDefined();
    const tools = seen.get(2)?.result?.tools ?? [];
    const names = tools.map((t) => t.name);
    expect(names).toContain("session_status");
    expect(names).toContain("observe_trace");

    const call = seen.get(3)?.result;
    expect(call?.isError).toBe(false);
    const envelope = JSON.parse(call?.content?.[0]?.text ?? "{}") as {
      actionId: string;
      status: string;
      data: { state: string };
    };
    expect(envelope.actionId).toBe("A-001");
    expect(envelope.status).toBe("ok");
    expect(envelope.data.state).toBe("Idle");
  } finally {
    proc.kill();
    await proc.exited;
  }
}, 30_000);
