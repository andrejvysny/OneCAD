import { beforeEach, describe, expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { isAgentError } from "../src/errors.ts";
import { Resolver, refStoreFrom } from "../src/semantic/resolve.ts";
import { snapshotInPage } from "../src/semantic/snapshotScript.ts";
import type { PageSnapshot } from "../src/semantic/snapshotScript.ts";
import type { BridgeLike } from "../src/semantic/webdriver.ts";

/**
 * The Resolver against the REAL in-page script, in jsdom.
 *
 * `resolve.test.ts` cans the script's answers to exercise the verdict ladder; this
 * file runs the script itself, which is the only way to catch the failures that
 * live in how the page re-finds an element.
 */
const FIXTURE = `<!doctype html><html><body>
  <header data-tauri-drag-region data-testid="title-bar"><span data-testid="doc">Untitled</span></header>
  <div role="group" data-testid="rows">
    <button aria-label="Delete" data-rect="0,0,100,20"><span>x</span></button>
    <button aria-label="Delete" data-rect="0,30,100,20"><span>x</span></button>
    <button aria-label="Delete" data-rect="0,60,100,20"><span>x</span></button>
  </div>
  <button data-testid="icon-btn" aria-label="Extrude" data-rect="0,200,40,40">
    <span data-testid="icon" style="pointer-events:none">E</span>
  </button>
</body></html>`;

let dom: JSDOM;
let hit: Element | null = null;

function installStubs(win: JSDOM["window"]): void {
  win.Element.prototype.getBoundingClientRect = function stub(this: Element) {
    const raw = this.getAttribute("data-rect");
    const [x, y, width, height] = raw ? raw.split(",").map(Number) : [4, 8, 100, 24];
    return {
      x, y, width, height,
      left: x as number, top: y as number,
      right: (x as number) + (width as number),
      bottom: (y as number) + (height as number),
      toJSON: () => ({}),
    } as DOMRect;
  };
  (win.document as unknown as { elementFromPoint: (x: number, y: number) => Element | null })
    .elementFromPoint = () => hit;
}

/** Runs whatever script the Resolver sends, against the live jsdom document. */
const liveBridge: BridgeLike = {
  execute: <T,>(_name: string, fn: (...a: never[]) => T | Promise<T>, args: unknown[]): Promise<T> =>
    Promise.resolve((fn as (...a: unknown[]) => T | Promise<T>)(...args)),
};

function snap(): PageSnapshot {
  return snapshotInPage({ mode: "interactive", maxNodes: 300 });
}

function resolverFor(s: PageSnapshot): Resolver {
  return new Resolver(liveBridge, refStoreFrom(s.nodes));
}

async function codeOf(fn: () => Promise<unknown>): Promise<string> {
  try {
    await fn();
  } catch (e) {
    return isAgentError(e) ? e.code : `not-an-AgentError:${String(e)}`;
  }
  return "no-throw";
}

beforeEach(() => {
  dom = new JSDOM(FIXTURE, { pretendToBeVisual: true });
  installStubs(dom.window);
  const g = globalThis as unknown as Record<string, unknown>;
  g.window = dom.window;
  g.document = dom.window.document;
  g.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  g.MutationObserver = dom.window.MutationObserver;
  g.NodeFilter = dom.window.NodeFilter;
  hit = null;
});

describe("a held ref whose element was removed", () => {
  test("is ELEMENT_STALE, never the sibling that slid into its position", async () => {
    const s = snap();
    const rows = s.nodes.filter((n) => n.name === "Delete");
    expect(rows.length).toBe(3);
    const first = rows[0]!;
    const second = rows[1]!;
    // Same shape, same name: their fingerprints differ ONLY by the positional path,
    // so re-finding by that path makes row 2 pass row 1's identity check.
    expect(first.fp).not.toBe(second.fp);
    expect(first.css).toContain("nth-of-type(1)");

    dom.window.document.querySelectorAll("button[aria-label=Delete]")[0]!.remove();
    hit = dom.window.document.querySelectorAll("button[aria-label=Delete]")[0] ?? null;

    expect(await codeOf(() => resolverFor(s).resolve({ ref: first.ref }, { forInput: true }))).toBe(
      "ELEMENT_STALE",
    );
  });

  test("a ref from an older snapshot generation is stale, not a same-numbered new node", async () => {
    const first = snap();
    const stale = first.nodes.find((n) => n.name === "Delete")!;
    const second = snap();
    expect(second.nodes.some((n) => n.ref === stale.ref)).toBe(false);
    // The resolver still knows the old node, but the page's map no longer does.
    expect(await codeOf(() => resolverFor(first).resolve({ ref: stale.ref }, { forInput: true }))).toBe(
      "ELEMENT_STALE",
    );
    expect(second.generation).toBe(first.generation + 1);
  });
});

describe("hit testing", () => {
  test("an ancestor hit counts as self when the target cannot receive events", async () => {
    const s = snap();
    const icon = s.nodes.find((n) => n.testId === "icon")!;
    hit = dom.window.document.querySelector('[data-testid="icon-btn"]');
    const got = await resolverFor(s).resolve({ ref: icon.ref }, { forInput: true });
    expect(got.node?.testId).toBe("icon");
  });

  test("an ancestor hit is still occlusion when the target does receive events", async () => {
    const s = snap();
    const icon = s.nodes.find((n) => n.testId === "icon")!;
    (dom.window.document.querySelector('[data-testid="icon"]') as HTMLElement).style.pointerEvents = "auto";
    hit = dom.window.document.querySelector('[data-testid="icon-btn"]');
    expect(await codeOf(() => resolverFor(s).resolve({ ref: icon.ref }, { forInput: true }))).toBe(
      "ELEMENT_OCCLUDED",
    );
  });
});

describe("point targets", () => {
  test("a drag starting on a drag region is refused", async () => {
    const s = snap();
    hit = dom.window.document.querySelector('[data-testid="doc"]');
    expect(
      await codeOf(() => resolverFor(s).resolve({ point: { x: 5, y: 5, space: "webview" } }, { forDrag: true })),
    ).toBe("ELEMENT_OCCLUDED");
  });

  test("a click on the same point is allowed, and flagged", async () => {
    const s = snap();
    hit = dom.window.document.querySelector('[data-testid="doc"]');
    const got = await resolverFor(s).resolve({ point: { x: 5, y: 5, space: "webview" } }, { forInput: true });
    expect(got.css).toEqual({ x: 5, y: 5 });
    expect(got.dragRegion).toBe(true);
  });

  test("a point in the clear carries no flag", async () => {
    const s = snap();
    hit = dom.window.document.querySelector('[data-testid="icon-btn"]');
    const got = await resolverFor(s).resolve({ point: { x: 10, y: 210, space: "webview" } }, { forInput: true });
    expect(got.dragRegion ?? false).toBe(false);
  });
});
