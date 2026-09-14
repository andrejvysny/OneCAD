/** `bun scripts/mcp-call.ts <tool> '<json args>'` → prints the envelope (and saves the inline image). */
const [tool, argJson] = [process.argv[2], process.argv[3] ?? "{}"];
const sock = process.env.TAURI_AGENT_SHELL_SOCK ?? "/tmp/tauri-agent-shell.sock";
if (!tool) { process.stderr.write("usage: mcp-call.ts <tool> '<json>'\n"); process.exit(2); }
let buf = "";
await new Promise<void>((done) => {
  Bun.connect({
    unix: sock,
    socket: {
      open(s) { s.write(JSON.stringify({ name: tool, arguments: JSON.parse(argJson) })); },
      data(_s, d) { buf += d.toString(); },
      close() { done(); },
      error(_s, e) { process.stderr.write(`socket error ${e}\n`); done(); },
    },
  });
});
const res = JSON.parse(await Bun.file(buf.trim()).text()) as { content?: Array<{ type: string; text?: string; data?: string }>; isError?: boolean; mcpError?: unknown };
if (res.mcpError) { process.stdout.write(`MCP ERROR ${JSON.stringify(res.mcpError)}\n`); process.exit(1); }
const text = res.content?.find((c) => c.type === "text")?.text ?? "";
process.stdout.write(`${text}\n`);
const img = res.content?.find((c) => c.type === "image");
if (img?.data) { await Bun.write("/tmp/tauri-agent-last.png", Buffer.from(img.data, "base64")); process.stdout.write("[inline image saved to /tmp/tauri-agent-last.png]\n"); }
process.exit(res.isError ? 1 : 0);
