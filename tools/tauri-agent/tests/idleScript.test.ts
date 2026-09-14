/**
 * The in-page idle reading. Two things are worth pinning here: that a missing signal comes
 * back as an explicit `null` (never a value that reads as "idle"), and that a throw anywhere
 * inside the page function cannot fail the action that is settling.
 */
import { beforeEach, describe, expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { readIdleInPage } from "../src/semantic/idleScript.ts";

const FIXTURE = `<!doctype html><html><head><title>t</title></head><body>
  <div id="overlay"></div>
</body></html>`;

interface TestWindow {
  __tauriAgentRev?: { rev: number; lastMutationAt: number };
  __tauriAgentChromeRemoved?: boolean;
  __stores?: Record<string, { getState(): Record<string, unknown> } | undefined>;
  __vpEngine?: unknown;
}

let dom: JSDOM;

function freshPage(): TestWindow {
  dom = new JSDOM(FIXTURE, { pretendToBeVisual: true });
  const g = globalThis as unknown as Record<string, unknown>;
  g.window = dom.window;
  g.document = dom.window.document;
  return dom.window as unknown as TestWindow;
}

function store(state: Record<string, unknown>): { getState(): Record<string, unknown> } {
  return { getState: () => state };
}

/** Registers a pill exactly the way `ViewportEngine.setupDebugOverlay` does. */
function addPill(): HTMLElement {
  const el = dom.window.document.createElement("div");
  el.textContent = "origin";
  el.dataset.vpDebugLabel = "1";
  dom.window.document.getElementById("overlay")?.appendChild(el);
  return el as unknown as HTMLElement;
}

describe("readIdleInPage", () => {
  beforeEach(() => {
    freshPage();
  });

  test("an unprepared page reports every signal as unavailable, never as idle", () => {
    const r = readIdleInPage();
    expect(r).toEqual({
      rev: -1,
      regenBusy: null,
      geometryPending: null,
      documentRevision: null,
      frames: null,
      camera: null,
    });
  });

  test("reads the regen counter, the geometry flag, the document revision and the frame count", () => {
    const w = freshPage();
    w.__tauriAgentRev = { rev: 12, lastMutationAt: 0 };
    w.__stores = {
      document: store({ regenBusy: 2, revision: 9 }),
      viewport: store({ geometryPending: true }),
    };
    w.__vpEngine = {
      frameCount: 41,
      debugSnapshot: () => ({ camPos: [1, 2, 3], target: [4, 5, 6], distance: 7 }),
    };

    expect(readIdleInPage()).toEqual({
      rev: 12,
      regenBusy: 2,
      geometryPending: true,
      documentRevision: 9,
      frames: 41,
      camera: { x: 1, y: 2, z: 3, tx: 4, ty: 5, tz: 6, distance: 7 },
    });
  });

  /**
   * `__stores` is installed asynchronously (a `Promise.all` of dynamic modules in
   * `src/main.tsx`), so it is briefly absent after load and a store may lack a field. A
   * partial page must degrade field by field, not lose the whole reading.
   */
  test("a store that is missing a field reports that field null and keeps the rest", () => {
    const w = freshPage();
    w.__tauriAgentRev = { rev: 3, lastMutationAt: 0 };
    w.__stores = { document: store({ revision: 4 }) };
    const r = readIdleInPage();
    expect(r.rev).toBe(3);
    expect(r.documentRevision).toBe(4);
    expect(r.regenBusy).toBeNull();
    expect(r.geometryPending).toBeNull();
  });

  test("a throwing debugSnapshot costs the camera only, never the reading", () => {
    const w = freshPage();
    w.__tauriAgentRev = { rev: 5, lastMutationAt: 0 };
    w.__vpEngine = {
      frameCount: 8,
      debugSnapshot: () => {
        throw new Error("scene walk blew up");
      },
    };
    const r = readIdleInPage();
    expect(r.frames).toBe(8);
    expect(r.camera).toBeNull();
    expect(r.rev).toBe(5);
  });

  test("a throwing store getState costs that store only", () => {
    const w = freshPage();
    w.__stores = {
      document: {
        getState: () => {
          throw new Error("store not ready");
        },
      },
      viewport: store({ geometryPending: false }),
    };
    const r = readIdleInPage();
    expect(r.regenBusy).toBeNull();
    expect(r.documentRevision).toBeNull();
    expect(r.geometryPending).toBe(false);
  });

  test("a partial camera snapshot is null rather than a half-read pose", () => {
    const w = freshPage();
    w.__vpEngine = { frameCount: 1, debugSnapshot: () => ({ camPos: [1, 2, 3], target: null, distance: 7 }) };
    expect(readIdleInPage().camera).toBeNull();
  });

  describe("the ?vpdebug origin pill", () => {
    /**
     * The pill is anchored at the world origin, which in sketch mode is exactly where the
     * user draws, so it would sit in every screenshot this harness captures as evidence.
     * Unregistering is the mechanism, not a style change: `HtmlOverlayDriver.update` rewrites
     * `display` on every registered item on every frame.
     */
    test("is unregistered through the public overlay API and detached from the DOM", () => {
      const w = freshPage();
      addPill();
      const unregistered: string[] = [];
      w.__vpEngine = { frameCount: 1, overlay: { unregister: (id: string) => unregistered.push(id) } };

      readIdleInPage();

      expect(unregistered).toEqual(["__debug_origin"]);
      expect(dom.window.document.querySelectorAll("[data-vp-debug-label]").length).toBe(0);
      expect(w.__tauriAgentChromeRemoved).toBe(true);
    });

    /** Detaching matters because the engine re-registers the pill when a sketch session closes. */
    test("a re-register after removal puts nothing back on screen", () => {
      const w = freshPage();
      const pill = addPill();
      w.__vpEngine = { frameCount: 1, overlay: { unregister: () => {} } };
      readIdleInPage();
      expect(pill.isConnected).toBe(false);
    });

    /**
     * Unregistering stops the driver writing to the element but leaves the last frame's
     * transform and `display` on it, so latching on the unregister alone would give up while
     * a pill was still visible.
     */
    test("does not latch until a pill was actually removed", () => {
      const w = freshPage();
      w.__vpEngine = { frameCount: 1, overlay: { unregister: () => {} } };
      readIdleInPage();
      expect(w.__tauriAgentChromeRemoved).toBeUndefined();

      addPill();
      readIdleInPage();
      expect(w.__tauriAgentChromeRemoved).toBe(true);
    });

    test("is attempted on every reading until the engine exists, then never again", () => {
      const w = freshPage();
      addPill();
      // The viewport mounts after the bridge is built, so the first readings find no engine.
      readIdleInPage();
      readIdleInPage();
      expect(w.__tauriAgentChromeRemoved).toBeUndefined();
      expect(dom.window.document.querySelectorAll("[data-vp-debug-label]").length).toBe(1);

      const unregistered: string[] = [];
      w.__vpEngine = { frameCount: 1, overlay: { unregister: (id: string) => unregistered.push(id) } };
      readIdleInPage();
      readIdleInPage();
      expect(unregistered).toEqual(["__debug_origin"]);
    });

    test("an overlay that throws does not cost the reading", () => {
      const w = freshPage();
      w.__tauriAgentRev = { rev: 2, lastMutationAt: 0 };
      w.__vpEngine = {
        frameCount: 3,
        overlay: {
          unregister: () => {
            throw new Error("overlay gone");
          },
        },
      };
      const r = readIdleInPage();
      expect(r.rev).toBe(2);
      expect(r.frames).toBe(3);
      expect(w.__tauriAgentChromeRemoved).toBeUndefined();
    });
  });

  /**
   * webdriverio serializes the function with `Function.prototype.toString` and evaluates the
   * text in the page. A closure over anything in the module becomes a ReferenceError in the
   * webview with no stack worth reading, so rebuilding it here turns that into a test failure.
   */
  test("is plain, self-contained JavaScript when serialized", () => {
    const src = readIdleInPage.toString();
    expect(src).not.toMatch(/\brequire\(|\bimport\b/);
    expect(() => new Function(`return (${src})`)).not.toThrow();
  });
});
