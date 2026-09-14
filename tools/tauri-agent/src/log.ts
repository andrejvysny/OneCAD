// The MCP stdio transport owns process.stdout; every diagnostic line goes to stderr.

export type LogLevel = "debug" | "info" | "warn" | "error";

const ORDER: Record<LogLevel, number> = { debug: 10, info: 20, warn: 30, error: 40 };

function currentLevel(): LogLevel {
  const raw = (process.env.TAURI_AGENT_LOG ?? "info").toLowerCase();
  return raw === "debug" || raw === "info" || raw === "warn" || raw === "error" ? raw : "info";
}

function emit(level: LogLevel, msg: string, data?: object): void {
  if (ORDER[level] < ORDER[currentLevel()]) return;
  const line = { ts: new Date().toISOString(), level, msg, ...(data ?? {}) };
  let text: string;
  try {
    text = JSON.stringify(line);
  } catch {
    text = JSON.stringify({ ts: line.ts, level, msg, dataError: "unserializable" });
  }
  process.stderr.write(`${text}\n`);
}

export const log = {
  debug(msg: string, data?: object): void {
    emit("debug", msg, data);
  },
  info(msg: string, data?: object): void {
    emit("info", msg, data);
  },
  warn(msg: string, data?: object): void {
    emit("warn", msg, data);
  },
  error(msg: string, data?: object): void {
    emit("error", msg, data);
  },
};
