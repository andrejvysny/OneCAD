import { appendFile, mkdir, readFile } from "node:fs/promises";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

export interface JournalEntry {
  ts: string;
  sessionId: string;
  actionId: string;
  tool: string;
  input: unknown;
  result: unknown;
  durationMs: number;
}

const FILENAME = "journal.jsonl";
const DEFAULT_LIMIT = 100;

export class Journal {
  readonly dir: string;
  readonly path: string;
  private counter = 0;
  // Appends are chained so concurrent tool calls cannot interleave partial lines.
  private tail: Promise<void> = Promise.resolve();

  constructor(dir: string) {
    this.dir = dir;
    this.path = join(dir, FILENAME);
    mkdirSync(dir, { recursive: true });
  }

  nextActionId(): string {
    this.counter += 1;
    return `A-${String(this.counter).padStart(3, "0")}`;
  }

  append(entry: JournalEntry): Promise<void> {
    const line = `${safeStringify(entry)}\n`;
    this.tail = this.tail.then(async () => {
      await mkdir(this.dir, { recursive: true });
      await appendFile(this.path, line, "utf8");
    });
    return this.tail;
  }

  async since(actionId?: string, limit = DEFAULT_LIMIT): Promise<JournalEntry[]> {
    await this.tail;
    let text: string;
    try {
      text = await readFile(this.path, "utf8");
    } catch {
      return [];
    }
    const all = text
      .split("\n")
      .filter((l) => l.length > 0)
      .map(parseLine)
      .filter((e): e is JournalEntry => e !== null);
    // actionIds are zero-padded and monotonic, so lexicographic order is chronological.
    const filtered = actionId === undefined ? all : all.filter((e) => e.actionId > actionId);
    return limit > 0 && filtered.length > limit ? filtered.slice(filtered.length - limit) : filtered;
  }

  artifactPath(name: string): string {
    return join(this.dir, name);
  }
}

function parseLine(line: string): JournalEntry | null {
  try {
    return JSON.parse(line) as JournalEntry;
  } catch {
    return null;
  }
}

function safeStringify(entry: JournalEntry): string {
  try {
    return JSON.stringify(entry);
  } catch {
    return JSON.stringify({
      ts: entry.ts,
      sessionId: entry.sessionId,
      actionId: entry.actionId,
      tool: entry.tool,
      input: null,
      result: { serializationError: true },
      durationMs: entry.durationMs,
    });
  }
}
