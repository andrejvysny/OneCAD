import { remote } from "webdriverio";
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { Pt } from "../geometry/types.ts";

type RemoteBrowser = Awaited<ReturnType<typeof remote>>;

/** Default fast-fail budget for one bridged script. */
const DEFAULT_BUDGET_MS = 15_000;
/** Snapshot walks the whole DOM; it gets its own, larger budget. */
export const SNAPSHOT_BUDGET_MS = 20_000;
export const INIT_TIMEOUT_MS = 15_000;
const INIT_POLL_BUDGET_MS = 3_000;
const INIT_POLL_INTERVAL_MS = 250;
/** devicePixelRatio vs tauri's scaleFactor; they are the same quantity read twice. */
const DPR_TOL = 0.01;
/** innerSize/scaleFactor vs innerWidth: one css px of rounding, no more. */
const SIZE_TOL_PX = 1;

export interface ExecuteOpts {
  readOnly: boolean;
  budgetMs?: number;
}

/** The surface `snapshot.ts` / `resolve.ts` need, so they are testable with a fake. */
export interface BridgeLike {
  execute<T>(
    name: string,
    fn: (...a: never[]) => T | Promise<T>,
    args: unknown[],
    opts: ExecuteOpts,
  ): Promise<T>;
}

export interface WindowGeomReading {
  innerPositionPx: Pt;
  innerSizePx: { width: number; height: number };
  scaleFactor: number;
  focused: boolean;
  /** calibration inputs — must agree with scaleFactor / 1 respectively */
  dpr: number;
  vvScale: number;
  innerWidth: number;
  innerHeight: number;
}

class BudgetTimeout extends Error {}

async function withBudget<T>(name: string, ms: number, run: () => Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      run(),
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new BudgetTimeout(`${name} did not return within ${ms}ms`)),
          ms,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

const TRANSPORT_DEAD = /invalid session id|session ?not ?created|no such session|ECONNREFUSED|socket hang up|not reachable|Failed to fetch/i;

function wrapScriptError(name: string, err: unknown): AgentError {
  const message = err instanceof Error ? err.message : String(err);
  if (TRANSPORT_DEAD.test(message)) {
    return new AgentError("WEBDRIVER_UNAVAILABLE", `${name}: ${message}`, { cause: err });
  }
  return new AgentError("INTERNAL", `${name}: ${message}`, { cause: err });
}

/**
 * The bridged `execute` channel can WEDGE — the app stays healthy while a script
 * response never arrives (see e2e-tauri/specs/composition.e2e.ts:94-157 and the
 * 2026-08-16 CI failure it documents). So: every script carries a NAMED budget,
 * read-only scripts get exactly one retry, and mutations get none — a re-sent
 * mutation may double-apply. Two consecutive budget failures mean the channel,
 * not the script, is broken; the bridge latches `wedged` and refuses further work
 * rather than letting the caller keep firing native input at a blind session.
 */
export class Bridge {
  readonly #browser: RemoteBrowser;
  #wedged = false;
  #consecutiveBudgetFailures = 0;

  private constructor(browser: RemoteBrowser) {
    this.#browser = browser;
  }

  static async connect(port: number): Promise<Bridge> {
    let browser: RemoteBrowser;
    try {
      browser = await remote({
        hostname: "127.0.0.1",
        port,
        path: "/",
        capabilities: { browserName: "tauri" },
        logLevel: "warn",
        connectionRetryCount: 0,
      });
    } catch (err) {
      throw new AgentError(
        "WEBDRIVER_UNAVAILABLE",
        `no WebDriver session on 127.0.0.1:${port}: ${err instanceof Error ? err.message : String(err)}`,
        { cause: err, details: { port } },
      );
    }
    const bridge = new Bridge(browser);
    // Bound the DRIVER's own script wait to the bridge budget. Left at the W3C
    // default (30 s) the driver is still waiting on a script this side already
    // gave up on, and the read-only retry re-enters it on top of the first.
    try {
      await browser.setTimeout({ script: SNAPSHOT_BUDGET_MS });
    } catch (err) {
      log.warn("could not set the session script timeout; a retry may re-enter a live script", {
        error: err instanceof Error ? err.message : String(err),
      });
    }
    await bridge.#waitForPluginInit();
    return bridge;
  }

  get wedged(): boolean {
    return this.#wedged;
  }

  async execute<T>(
    name: string,
    fn: (...a: never[]) => T | Promise<T>,
    args: unknown[],
    opts: ExecuteOpts,
  ): Promise<T> {
    if (this.#wedged) {
      throw new AgentError("BRIDGE_WEDGED", `bridge is wedged; ${name} refused`, {
        details: { script: name },
      });
    }
    const budgetMs = opts.budgetMs ?? DEFAULT_BUDGET_MS;
    const attempts = opts.readOnly ? 2 : 1;
    let last: AgentError | undefined;
    for (let attempt = 1; attempt <= attempts; attempt += 1) {
      try {
        const value = await withBudget(name, budgetMs, () =>
          // webdriverio types the script as a variadic browser-side function; the
          // agent's scripts are authored against DOM globals, not its element
          // transforms, so the call is bridged through an unknown-arg signature.
          (this.#browser.execute as unknown as (
            s: unknown,
            ...a: unknown[]
          ) => Promise<unknown>)(fn, ...args),
        );
        this.#consecutiveBudgetFailures = 0;
        return value as T;
      } catch (err) {
        if (!(err instanceof BudgetTimeout)) {
          // The channel answered — a script fault is not evidence of a wedge.
          this.#consecutiveBudgetFailures = 0;
          throw wrapScriptError(name, err);
        }
        this.#consecutiveBudgetFailures += 1;
        last = new AgentError("ACTION_TIMEOUT", err.message, {
          details: { script: name, budgetMs, attempt, attempts, readOnly: opts.readOnly },
        });
        if (this.#consecutiveBudgetFailures >= 2) {
          this.#wedged = true;
          throw new AgentError(
            "BRIDGE_WEDGED",
            `two consecutive bridged scripts exceeded their budget (last: ${name})`,
            { details: { script: name, budgetMs }, cause: err },
          );
        }
      }
    }
    throw last ?? new AgentError("INTERNAL", `${name}: unreachable`);
  }

  invoke<T>(
    command: string,
    args: Record<string, unknown> = {},
    opts: { readOnly?: boolean } = {},
  ): Promise<T> {
    return this.execute<T>(
      `invoke:${command}`,
      ((c: string, a: Record<string, unknown>) =>
        (window as unknown as TauriGlobal).__TAURI__.core.invoke(c, a)) as never,
      [command, args],
      { readOnly: opts.readOnly ?? false },
    );
  }

  async windowGeom(): Promise<WindowGeomReading> {
    const raw = await this.execute<WindowGeomReading>(
      "windowGeom",
      (async () => {
        const w = (window as unknown as TauriGlobal).__TAURI__.window.getCurrentWindow();
        const [p, s, sf, f] = await Promise.all([
          w.innerPosition(),
          w.innerSize(),
          w.scaleFactor(),
          w.isFocused(),
        ]);
        return {
          innerPositionPx: { x: p.x, y: p.y },
          innerSizePx: { width: s.width, height: s.height },
          scaleFactor: sf,
          focused: f,
          dpr: devicePixelRatio,
          vvScale: visualViewport?.scale ?? 1,
          innerWidth,
          innerHeight,
        };
      }) as never,
      [],
      { readOnly: true },
    );
    return validateGeom(raw);
  }

  async close(): Promise<void> {
    try {
      await withBudget("deleteSession", 5_000, () => this.#browser.deleteSession());
    } catch (err) {
      log.warn("bridge close failed; session may already be gone", {
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }

  /**
   * The wdio tauri plugin exposes `window.wdioTauri` once its frontend half has
   * booted; scripts sent before that can land in a half-initialised page.
   * Absence is tolerated (a build without the plugin still answers plain DOM
   * scripts) — only a present-but-never-resolving `waitForInit` is fatal.
   */
  async #waitForPluginInit(): Promise<void> {
    const state = await pollPluginInit({
      probe: (budgetMs) =>
        this.execute<"ready" | "absent">(
          "wdioTauri.waitForInit",
          (async () => {
            const w = (window as unknown as WdioGlobal).wdioTauri;
            if (!w || typeof w.waitForInit !== "function") return "absent";
            await w.waitForInit();
            return "ready";
          }) as never,
          [],
          { readOnly: true, budgetMs },
        ),
      transient: (err) => {
        const code = err instanceof AgentError ? err.code : "";
        if (code !== "ACTION_TIMEOUT" && code !== "BRIDGE_WEDGED") return false;
        // A deliberately short poll budget is not wedge evidence: the app is still
        // booting, which is precisely what waitForInit exists to absorb. Clear the
        // latch so the first real script gets its full budget and can re-latch.
        this.#consecutiveBudgetFailures = 0;
        this.#wedged = false;
        return true;
      },
      sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
      now: () => Date.now(),
    });
    if (state === "absent") {
      log.debug("window.wdioTauri is absent; this build answers plain DOM scripts", {});
    } else if (state === "timeout") {
      log.warn("window.wdioTauri.waitForInit never reported ready; continuing", {
        timeoutMs: INIT_TIMEOUT_MS,
      });
    }
  }
}

/** Injected so the poll is testable without a driver or a real clock. */
export interface InitPollDeps {
  probe(budgetMs: number): Promise<"ready" | "absent">;
  /** true when the failure means "still booting", false when it must propagate. */
  transient(err: unknown): boolean;
  sleep(ms: number): Promise<void>;
  now(): number;
}

/**
 * Polls the plugin's init signal until it reports, the deadline passes, or the
 * probe fails in a way that is not just a slow boot.
 *
 * `"absent"` is an answer, not a miss: the build has no plugin, and re-asking for
 * 15 s burns the caller's first action on a verdict that cannot change.
 */
export async function pollPluginInit(d: InitPollDeps): Promise<"ready" | "absent" | "timeout"> {
  const deadline = d.now() + INIT_TIMEOUT_MS;
  while (d.now() < deadline) {
    const budgetMs = Math.min(INIT_POLL_BUDGET_MS, Math.max(500, deadline - d.now()));
    try {
      const state = await d.probe(budgetMs);
      if (state === "ready" || state === "absent") return state;
    } catch (err) {
      if (!d.transient(err)) throw err;
    }
    await d.sleep(INIT_POLL_INTERVAL_MS);
  }
  return "timeout";
}

interface TauriGlobal {
  __TAURI__: {
    core: { invoke(command: string, args?: Record<string, unknown>): Promise<unknown> };
    window: {
      getCurrentWindow(): {
        innerPosition(): Promise<{ x: number; y: number }>;
        innerSize(): Promise<{ width: number; height: number }>;
        scaleFactor(): Promise<number>;
        isFocused(): Promise<boolean>;
      };
    };
  };
}

interface WdioGlobal {
  wdioTauri?: { waitForInit?: () => Promise<void> };
}

/** Shape gate: a reading whose numbers did not survive the call is a transport fault. */
function geomIsUsable(raw: WindowGeomReading): boolean {
  const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
  return (
    raw !== null &&
    typeof raw === "object" &&
    typeof raw.innerPositionPx === "object" &&
    finite(raw.innerPositionPx?.x) &&
    finite(raw.innerPositionPx?.y) &&
    finite(raw.innerSizePx?.width) &&
    finite(raw.innerSizePx?.height) &&
    finite(raw.scaleFactor) &&
    raw.scaleFactor > 0 &&
    typeof raw.focused === "boolean" &&
    finite(raw.dpr) &&
    finite(raw.vvScale) &&
    finite(raw.innerWidth) &&
    finite(raw.innerHeight)
  );
}

/**
 * The same quantity, read twice from two different APIs. They must agree, because
 * `cssToGlobal` divides by exactly one of them.
 */
function geomMismatch(raw: WindowGeomReading): string | null {
  const sf = raw.scaleFactor;
  if (raw.vvScale !== 1) {
    return `visualViewport.scale is ${raw.vvScale}, not 1: the page is zoomed and css px are not layout px`;
  }
  if (Math.abs(raw.dpr - sf) > DPR_TOL) {
    return `devicePixelRatio ${raw.dpr} disagrees with the window scaleFactor ${sf}`;
  }
  if (Math.abs(raw.innerSizePx.width / sf - raw.innerWidth) > SIZE_TOL_PX) {
    return `innerSize.width ${raw.innerSizePx.width} / ${sf} disagrees with innerWidth ${raw.innerWidth}`;
  }
  if (Math.abs(raw.innerSizePx.height / sf - raw.innerHeight) > SIZE_TOL_PX) {
    return `innerSize.height ${raw.innerSizePx.height} / ${sf} disagrees with innerHeight ${raw.innerHeight}`;
  }
  return null;
}

/**
 * Boundary validation, not defensive padding: a NaN here becomes a click in
 * another application, and a scale disagreement becomes a click in the wrong part
 * of this one.
 *
 * The embedded driver DOES await an async script — tauri-plugin-wdio-webdriver's
 * `executor.rs` wraps the script in an async IIFE and awaits it, and the whole
 * bridge depends on that — so a blank reading means the window API itself answered
 * with nothing usable, not that a promise was dropped.
 */
export function validateGeom(raw: WindowGeomReading): WindowGeomReading {
  if (!geomIsUsable(raw)) {
    throw new AgentError(
      "WEBDRIVER_UNAVAILABLE",
      `windowGeom returned an unusable reading: ${JSON.stringify(raw)}`,
      {
        remediation:
          "`window.__TAURI__` is missing or answered with blanks. Confirm the app was built with withGlobalTauri and the tauri-e2e feature, then reconnect.",
        details: { raw },
      },
    );
  }
  const mismatch = geomMismatch(raw);
  if (mismatch !== null) {
    throw new AgentError("CALIBRATION_FAILED", `the window geometry is inconsistent: ${mismatch}`, {
      remediation:
        "Reset the webview zoom to 100% and re-run session_calibrate; the window's scale factor and the page's own must agree before any point can be mapped.",
      details: { raw },
    });
  }
  return raw;
}
