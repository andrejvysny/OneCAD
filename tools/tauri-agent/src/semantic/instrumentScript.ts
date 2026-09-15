/**
 * Page instrumentation, installed once per bridge rather than as a side effect of a snapshot.
 *
 * `settle()` watches a DOM-mutation revision and the action pipeline counts console errors, so
 * BOTH probes have to exist before the first action — not before the first `ui_snapshot`. They
 * used to be installed only by `snapshotInPage`, which left `session_start` → `keyboard_shortcut`
 * settling on `-1 === -1` (instant, proving nothing) and reporting zero console errors whatever
 * the page logged.
 *
 * Like `checkScript.ts`, the exported function is serialized with `Function.prototype.toString`
 * and evaluated in the webview, so it MUST NOT close over anything in this module: every helper
 * and constant lives inside its own body. Only types cross the boundary, and types are erased.
 *
 * The installers are duplicated in `snapshotScript.ts` on purpose. That file has to keep working
 * standalone under the same serialization rule, and there is no way to share a helper across two
 * independently-serialized page functions. `tests/instrumentScript.test.ts` asserts the two paths
 * produce identical globals so they cannot drift apart unnoticed.
 */
import type { AgentConsoleEntry } from "./snapshotScript.ts";

interface InstrumentWindow extends Window {
  __tauriAgentRev?: { rev: number; lastMutationAt: number };
  __tauriAgentConsole?: { entries: AgentConsoleEntry[]; errors: number };
}

export interface InstrumentReport {
  /** False when this call found the probe already installed — the call is a no-op, not a reset. */
  revInstalled: boolean;
  consoleInstalled: boolean;
  rev: number;
  consoleErrors: number;
}

/** Idempotent: a second call re-reports the live state and never resets a counter or re-wraps console. */
export function installInstrumentation(): InstrumentReport {
  const w = window as unknown as InstrumentWindow;
  const CONSOLE_RING = 200;

  let revInstalled = false;
  if (!w.__tauriAgentRev) {
    const rev = { rev: 0, lastMutationAt: performance.now() };
    w.__tauriAgentRev = rev;
    new MutationObserver(() => {
      rev.rev += 1;
      rev.lastMutationAt = performance.now();
    }).observe(document.documentElement, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true,
    });
    revInstalled = true;
  }

  let consoleInstalled = false;
  if (!w.__tauriAgentConsole) {
    const ring: { entries: AgentConsoleEntry[]; errors: number } = { entries: [], errors: 0 };
    w.__tauriAgentConsole = ring;
    const push = (level: string, text: string): void => {
      ring.entries.push({ level, text: String(text).slice(0, 500), t: performance.now() });
      if (ring.entries.length > CONSOLE_RING) ring.entries.shift();
      if (level === "error") ring.errors += 1;
    };
    const wrap = (level: "error" | "warn"): void => {
      const orig = console[level];
      console[level] = function wrapped(...a: unknown[]): void {
        push(level, a.map((v) => String(v)).join(" "));
        orig.apply(console, a);
      };
    };
    wrap("error");
    wrap("warn");
    window.addEventListener("error", (e) => push("error", e.message || "error"));
    window.addEventListener("unhandledrejection", (e) =>
      push("error", "unhandledrejection: " + String((e as PromiseRejectionEvent).reason)),
    );
    consoleInstalled = true;
  }

  return {
    revInstalled,
    consoleInstalled,
    rev: w.__tauriAgentRev ? w.__tauriAgentRev.rev : -1,
    consoleErrors: w.__tauriAgentConsole ? w.__tauriAgentConsole.errors : 0,
  };
}
