/**
 * "Has the UI gone quiet?" — one frame, then a quiet window of unchanged DOM revisions.
 *
 * A settle timeout is never fatal: the input WAS posted, so refusing the envelope would
 * hide evidence the caller needs. A long regen legitimately keeps mutating for seconds;
 * the caller is told with a warning and pointed at `wait_for` instead.
 */
import { readRevision } from "../../semantic/snapshot.ts";
import type { BridgeLike } from "../../semantic/webdriver.ts";

export interface SettleConfig {
  frameMs: number;
  quietMs: number;
  timeoutMs: number;
}

export interface SettleOutcome {
  settled: boolean;
  afterRevision: number;
  warning?: string;
}

export type Sleep = (ms: number) => Promise<void>;

const realSleep: Sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function settle(
  bridge: BridgeLike,
  cfg: SettleConfig,
  sleep: Sleep = realSleep,
): Promise<SettleOutcome> {
  const deadline = Date.now() + cfg.timeoutMs;
  await sleep(cfg.frameMs);
  let last = (await readRevision(bridge)).rev;
  for (;;) {
    await sleep(cfg.quietMs);
    const rev = (await readRevision(bridge)).rev;
    if (rev === last) return { settled: true, afterRevision: rev };
    last = rev;
    if (Date.now() >= deadline) {
      return {
        settled: false,
        afterRevision: rev,
        warning: `the UI was still changing ${cfg.timeoutMs}ms after the input (DOM revision ${rev}); the action WAS sent — confirm the outcome with ui_snapshot, or wait_for {kind:"revision_stable"}`,
      };
    }
  }
}
