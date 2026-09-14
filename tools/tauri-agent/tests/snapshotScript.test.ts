import { beforeAll, describe, expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { snapshotInPage } from "../src/semantic/snapshotScript.ts";
import type { AgentWindow, PageNode, PageSnapshot } from "../src/semantic/snapshotScript.ts";

const FIXTURE = `<!doctype html><html><head><title>Untitled — OneCAD</title></head><body>
  <header data-tauri-drag-region data-testid="title-bar">
    <span data-tauri-drag-region aria-hidden="true" class="spacer"></span>
    <span data-tauri-drag-region data-testid="document-title">Untitled.onecad</span>
  </header>
  <div role="toolbar" data-testid="model-toolbar">
    <button aria-label="Sketch" aria-pressed="false"><svg viewBox="0 0 1 1"></svg></button>
    <button aria-label="Extrude" aria-pressed="false" aria-disabled="true">E</button>
    <button data-testid="ghost-tool" aria-label="Ghost" style="display:none">G</button>
  </div>
  <div aria-hidden="true"><button aria-label="Decorative">x</button></div>
  <main data-testid="editor">
    <label for="part-name">Part name</label>
    <input id="part-name" type="text" value="Part1">
    <div data-testid="viewport-canvas"><canvas width="10" height="10"></canvas></div>
    <span data-testid="sketch-dof" data-dof="3" role="status">3 DoF</span>
  </main>
</body></html>`;

/**
 * jsdom performs no layout, so every rect is 0x0 and the production visibility
 * rule would reject the whole fixture. The rect source is stubbed; the rest of
 * the rule (computed display/visibility/opacity, the `hidden` attribute) runs
 * exactly as it does in the webview.
 */
function installRectStub(win: JSDOM["window"]): void {
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
}

let dom: JSDOM;

beforeAll(() => {
  dom = new JSDOM(FIXTURE, { pretendToBeVisual: true });
  installRectStub(dom.window);
  const g = globalThis as unknown as Record<string, unknown>;
  g.window = dom.window;
  g.document = dom.window.document;
  g.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  g.MutationObserver = dom.window.MutationObserver;
  g.NodeFilter = dom.window.NodeFilter;
});

function snap(overrides: Partial<Parameters<typeof snapshotInPage>[0]> = {}): PageSnapshot {
  return snapshotInPage({ mode: "interactive", maxNodes: 300, ...overrides });
}

function byName(s: PageSnapshot, name: string): PageNode | undefined {
  return s.nodes.find((n) => n.name === name);
}

function byTestId(s: PageSnapshot, testId: string): PageNode | undefined {
  return s.nodes.find((n) => n.testId === testId);
}

describe("snapshotInPage", () => {
  test("assigns sequential unique refs and populates the in-page ref map", () => {
    const s = snap();
    expect(s.rootFound).toBe(true);
    expect(s.nodes.length).toBeGreaterThan(5);
    expect(s.nodes.map((n) => n.ref)).toEqual(
      s.nodes.map((_, i) => `@s${s.generation}e${i + 1}`),
    );
    const refs = (dom.window as unknown as AgentWindow).__tauriAgentRefs;
    expect(refs).toBeInstanceOf(Map);
    expect(refs?.size).toBe(s.nodes.length);
    const sketch = byName(s, "Sketch");
    expect(refs?.get(sketch!.ref)?.getAttribute("aria-label")).toBe("Sketch");
  });

  test("names come off the ladder: aria-label, label[for], text content", () => {
    const s = snap();
    expect(byName(s, "Sketch")?.role).toBe("button");
    const input = s.nodes.find((n) => n.role === "textbox");
    expect(input?.name).toBe("Part name");
    expect(input?.state.value).toBe("Part1");
    expect(byTestId(s, "document-title")?.name).toBe("Untitled.onecad");
  });

  test("implicit roles: toolbar, button, textbox, and the viewport container", () => {
    const s = snap();
    expect(byTestId(s, "model-toolbar")?.role).toBe("toolbar");
    expect(byTestId(s, "editor")?.role).toBe("main");
    const viewport = byTestId(s, "viewport-canvas");
    expect(viewport?.role).toBe("region");
    expect(viewport?.name).toBe("Viewport");
  });

  test("landmarks drive indentation depth", () => {
    const s = snap();
    expect(byTestId(s, "model-toolbar")?.depth).toBe(0);
    expect(byName(s, "Sketch")?.depth).toBe(1);
    expect(byTestId(s, "editor")?.depth).toBe(0);
    expect(byTestId(s, "viewport-canvas")?.depth).toBe(1);
  });

  test("aria-disabled is kept and flagged, not dropped", () => {
    const s = snap();
    const extrude = byName(s, "Extrude");
    expect(extrude).toBeDefined();
    expect(extrude?.disabled).toBe(true);
    expect(extrude?.state.disabled).toBe(true);
    expect(byName(s, "Sketch")?.disabled).toBe(false);
  });

  test("drag regions are flagged so a drag never starts on one by accident", () => {
    const s = snap();
    expect(byTestId(s, "document-title")?.dragRegion).toBe(true);
    expect(byTestId(s, "title-bar")?.dragRegion).toBe(true);
    expect(byName(s, "Sketch")?.dragRegion).toBe(false);
  });

  test("hidden elements and aria-hidden subtrees are excluded", () => {
    const s = snap();
    expect(byTestId(s, "ghost-tool")).toBeUndefined();
    expect(byName(s, "Ghost")).toBeUndefined();
    expect(byName(s, "Decorative")).toBeUndefined();
    expect(s.nodes.some((n) => n.role === "button" && n.name === "")).toBe(false);
  });

  test("arbitrary state attributes ride along", () => {
    const s = snap();
    const dof = byTestId(s, "sketch-dof");
    expect(dof?.role).toBe("status");
    expect(dof?.state["data-dof"]).toBe("3");
    expect(byName(s, "Sketch")?.state.pressed).toBe("false");
  });

  test("fingerprints are stable across two runs of an unchanged DOM", () => {
    const a = snap();
    const b = snap();
    expect(b.nodes.map((n) => n.fp)).toEqual(a.nodes.map((n) => n.fp));
  });

  /**
   * Two snapshots of a same-shaped list used to reuse `@e2`, and a fingerprint
   * built from a positional path matches a surviving sibling — so a ref held
   * across a re-render addressed a DIFFERENT row and the click landed on it.
   */
  test("refs are namespaced per snapshot generation, and an older generation is gone", () => {
    const a = snap();
    const b = snap();
    expect(b.generation).toBe(a.generation + 1);
    expect(a.generation).toBeGreaterThan(0);
    for (const ref of a.nodes.map((n) => n.ref)) expect(ref).toMatch(/^@s\d+e\d+$/);
    const overlap = b.nodes.filter((n) => a.nodes.some((m) => m.ref === n.ref));
    expect(overlap).toEqual([]);
    const refs = (dom.window as unknown as AgentWindow).__tauriAgentRefs;
    expect(refs?.get(b.nodes[0]!.ref)).toBeDefined();
    expect(refs?.get(a.nodes[0]!.ref)).toBeUndefined();
  });

  /**
   * A live readout (the DoF counter) re-renders its own text constantly. Naming
   * its container from that text put it in the container's fingerprint, so every
   * tick reported ELEMENT_STALE for an element that never moved.
   */
  test("a landmark is never named by its own text content", () => {
    const before = snap();
    const dof = byTestId(before, "sketch-dof")!;
    const toolbar = byTestId(before, "model-toolbar")!;
    expect(dof.role).toBe("status");
    expect(dof.name).toBe("");
    expect(toolbar.name).toBe("");
    const el = dom.window.document.querySelector('[data-testid="sketch-dof"]') as Element;
    try {
      el.textContent = "1 DoF";
      const after = snap();
      expect(byTestId(after, "sketch-dof")?.fp).toBe(dof.fp);
      expect(byTestId(after, "sketch-dof")?.name).toBe("");
    } finally {
      el.textContent = "3 DoF";
    }
  });

  test("an explicit label still names a landmark", () => {
    const el = dom.window.document.querySelector('[data-testid="model-toolbar"]') as Element;
    try {
      el.setAttribute("aria-label", "Model tools");
      expect(byTestId(snap(), "model-toolbar")?.name).toBe("Model tools");
    } finally {
      el.removeAttribute("aria-label");
    }
  });

  test("a fingerprint changes when the accessible name changes, and only that one", () => {
    const before = snap();
    const button = dom.window.document.querySelector('[aria-label="Sketch"]') as Element;
    button.setAttribute("aria-label", "Sketch profile");
    const after = snap();
    try {
      const beforeSketch = byName(before, "Sketch");
      const afterSketch = byName(after, "Sketch profile");
      expect(afterSketch).toBeDefined();
      expect(afterSketch?.fp).not.toBe(beforeSketch?.fp);
      const unchanged = (s: PageSnapshot) =>
        s.nodes.filter((n) => n.testId !== undefined).map((n) => `${n.testId}:${n.fp}`);
      expect(unchanged(after)).toEqual(unchanged(before));
    } finally {
      button.setAttribute("aria-label", "Sketch");
    }
  });

  test("__tauriAgentFp recomputes the same fingerprint resolve() will compare against", () => {
    const s = snap();
    const w = dom.window as unknown as AgentWindow;
    expect(typeof w.__tauriAgentFp).toBe("function");
    const sketch = byName(s, "Sketch")!;
    expect(w.__tauriAgentFp!(w.__tauriAgentRefs!.get(sketch.ref)!)).toBe(sketch.fp);
  });

  test("maxNodes caps the payload but total still reports the real count", () => {
    const s = snap({ maxNodes: 3 });
    expect(s.nodes.length).toBe(3);
    expect(s.total).toBeGreaterThan(3);
    expect(s.truncated).toBe(true);
  });

  test("the serialized form is self-contained JavaScript, as webdriverio ships it", () => {
    const src = snapshotInPage.toString();
    expect(src).not.toMatch(/\bimport\b/);
    // Rebuilt with no module bindings in scope: a closure over module scope, or a
    // TypeScript-only construct bun did not strip, throws right here.
    const rebuilt = new Function(`return (${src})`)() as typeof snapshotInPage;
    const viaBridge = rebuilt({ mode: "interactive", maxNodes: 300 });
    expect(viaBridge.rootFound).toBe(true);
    const local = snap();
    // Refs carry the snapshot generation, which each call bumps; everything else
    // must be byte-identical between the module copy and the serialized one.
    expect(viaBridge.nodes.map((n) => `${n.role}|${n.name}|${n.fp}`)).toEqual(
      local.nodes.map((n) => `${n.role}|${n.name}|${n.fp}`),
    );
    expect(local.generation).toBe(viaBridge.generation + 1);
  });

  test("a missing root is reported, never guessed", () => {
    const s = snap({ rootCss: "#nope" });
    expect(s.rootFound).toBe(false);
    expect(s.nodes).toEqual([]);
  });

  test("rootCss scopes the walk", () => {
    const s = snap({ rootCss: '[role="toolbar"]' });
    expect(s.rootFound).toBe(true);
    expect(byName(s, "Sketch")).toBeDefined();
    expect(byTestId(s, "document-title")).toBeUndefined();
  });

  test("accessibility mode adds static text that interactive mode omits", () => {
    const interactive = snap();
    const a11y = snap({ mode: "accessibility" });
    expect(interactive.nodes.some((n) => n.role === "text")).toBe(false);
    expect(a11y.nodes.some((n) => n.role === "text")).toBe(true);
  });

  test("dom mode attaches attributes and a css path", () => {
    const s = snap({ mode: "dom" });
    const sketch = byName(s, "Sketch")!;
    expect(sketch.attrs?.["aria-label"]).toBe("Sketch");
    expect(sketch.css).toContain("button");
    expect(byTestId(s, "sketch-dof")?.css).toContain("span");
  });

  test("the revision and console probes install once and survive later snapshots", () => {
    const w = dom.window as unknown as AgentWindow;
    snap();
    const rev = w.__tauriAgentRev;
    const ring = w.__tauriAgentConsole;
    expect(rev).toBeDefined();
    expect(ring?.entries).toEqual([]);
    snap();
    expect(w.__tauriAgentRev).toBe(rev!);
    expect(w.__tauriAgentConsole).toBe(ring!);
  });
});
