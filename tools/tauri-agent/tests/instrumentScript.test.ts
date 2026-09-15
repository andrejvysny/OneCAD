/**
 * The instrumentation probes must exist before the FIRST action, not before the first
 * `ui_snapshot`. Nothing in the MCP API forces a snapshot first, and an uninstrumented page
 * makes `settle` compare -1 to -1 and report a clean quiet having watched nothing.
 *
 * These probes are installed by two independently-serialized page functions — page scripts
 * cannot share a helper — so the drift between them is the thing worth pinning.
 */
import { beforeEach, describe, expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { installInstrumentation } from "../src/semantic/instrumentScript.ts";
import { snapshotInPage } from "../src/semantic/snapshotScript.ts";
import type { AgentWindow } from "../src/semantic/snapshotScript.ts";

const FIXTURE = `<!doctype html><html><head><title>t</title></head><body>
  <div data-testid="editor"><button aria-label="Go">Go</button></div>
</body></html>`;

let dom: JSDOM;

function freshPage(): AgentWindow {
  dom = new JSDOM(FIXTURE, { pretendToBeVisual: true });
  // jsdom performs no layout; the snapshot path needs a non-zero rect to consider a node visible.
  dom.window.Element.prototype.getBoundingClientRect = function stub(this: Element) {
    return { x: 4, y: 8, width: 100, height: 24, left: 4, top: 8, right: 104, bottom: 32, toJSON: () => ({}) };
  } as never;
  const g = globalThis as unknown as Record<string, unknown>;
  g.window = dom.window;
  g.document = dom.window.document;
  g.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  g.MutationObserver = dom.window.MutationObserver;
  g.NodeFilter = dom.window.NodeFilter;
  // `performance` is deliberately NOT swapped for jsdom's: jsdom's implementation delegates
  // back to the global one, so assigning it here makes `performance.now()` recurse forever.
  // The probes only need a monotonic number, and the runtime's own clock is one.
  return dom.window as unknown as AgentWindow;
}

/** The globals both installers are contracted to produce, compared structurally. */
function shape(w: AgentWindow): Record<string, unknown> {
  return {
    revKeys: w.__tauriAgentRev ? Object.keys(w.__tauriAgentRev).sort() : null,
    revIsNumber: typeof w.__tauriAgentRev?.rev === "number",
    consoleKeys: w.__tauriAgentConsole ? Object.keys(w.__tauriAgentConsole).sort() : null,
    entriesIsArray: Array.isArray(w.__tauriAgentConsole?.entries),
    errorsIsNumber: typeof w.__tauriAgentConsole?.errors === "number",
    consoleErrorWrapped: dom.window.console.error.name === "wrapped",
    consoleWarnWrapped: dom.window.console.warn.name === "wrapped",
  };
}

describe("installInstrumentation", () => {
  beforeEach(() => {
    freshPage();
  });

  test("installs both probes on a page that has none", () => {
    const w = globalThis.window as unknown as AgentWindow;
    expect(w.__tauriAgentRev).toBeUndefined();
    expect(w.__tauriAgentConsole).toBeUndefined();

    const report = installInstrumentation();

    expect(report.revInstalled).toBe(true);
    expect(report.consoleInstalled).toBe(true);
    expect(report.rev).toBe(0);
    expect(w.__tauriAgentRev).toBeDefined();
    expect(w.__tauriAgentConsole).toBeDefined();
  });

  test("the revision probe actually counts DOM mutations", async () => {
    installInstrumentation();
    const w = globalThis.window as unknown as AgentWindow;
    dom.window.document.body.appendChild(dom.window.document.createElement("div"));
    await new Promise((r) => setTimeout(r, 20));
    expect(w.__tauriAgentRev?.rev).toBeGreaterThan(0);
  });

  test("the console probe counts errors and keeps the original console working", () => {
    installInstrumentation();
    const w = globalThis.window as unknown as AgentWindow;
    dom.window.console.error("boom");
    dom.window.console.warn("careful");
    expect(w.__tauriAgentConsole?.errors).toBe(1);
    expect(w.__tauriAgentConsole?.entries.map((e) => e.level)).toEqual(["error", "warn"]);
  });

  test("is idempotent: a second call neither resets the counter nor re-wraps console", async () => {
    installInstrumentation();
    const firstError = dom.window.console.error;
    dom.window.console.error("boom");
    dom.window.document.body.appendChild(dom.window.document.createElement("div"));
    await new Promise((r) => setTimeout(r, 20));

    const w = globalThis.window as unknown as AgentWindow;
    const revBefore = w.__tauriAgentRev?.rev ?? -1;
    expect(revBefore).toBeGreaterThan(0);

    const second = installInstrumentation();

    expect(second.revInstalled).toBe(false);
    expect(second.consoleInstalled).toBe(false);
    expect(second.rev).toBe(revBefore);
    expect(second.consoleErrors).toBe(1);
    // A second wrap would double-count every later error.
    expect(dom.window.console.error).toBe(firstError);
  });

  test("runs after the snapshot script already installed the probes", () => {
    snapshotInPage({ mode: "interactive", maxNodes: 50 });
    const w = globalThis.window as unknown as AgentWindow;
    const revRef = w.__tauriAgentRev;

    const report = installInstrumentation();

    expect(report.revInstalled).toBe(false);
    expect(report.consoleInstalled).toBe(false);
    // Same object, so the snapshot path's observer is still the live one.
    expect(w.__tauriAgentRev).toBe(revRef);
  });

  test("produces the same globals as the snapshot script, so the two cannot drift", () => {
    installInstrumentation();
    const viaInstrument = shape(globalThis.window as unknown as AgentWindow);

    freshPage();
    snapshotInPage({ mode: "interactive", maxNodes: 50 });
    const viaSnapshot = shape(globalThis.window as unknown as AgentWindow);

    expect(viaInstrument).toEqual(viaSnapshot);
  });

  /**
   * webdriverio serializes the function with `Function.prototype.toString` and evaluates the
   * text in the page. A closure over anything in the module becomes a ReferenceError in the
   * webview with no stack worth reading, so rebuilding it here turns that into a test failure.
   */
  test("is plain, self-contained JavaScript when serialized", () => {
    const src = installInstrumentation.toString();
    expect(src).not.toMatch(/\brequire\(|\bimport\b/);
    expect(() => new Function(`return (${src})`)).not.toThrow();
  });
});
