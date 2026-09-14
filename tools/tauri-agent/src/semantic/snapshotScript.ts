/**
 * The in-page half of `ui_snapshot`.
 *
 * `snapshotInPage` is serialized with `Function.prototype.toString` by
 * webdriverio and evaluated in the webview, so it MUST NOT close over anything
 * in this module: every constant and helper lives inside its own body. Only the
 * types below cross the boundary, and types are erased before serialization.
 */

export type SnapshotScriptMode = "interactive" | "accessibility" | "dom";

export interface SnapshotScriptOpts {
  mode: SnapshotScriptMode;
  rootCss?: string;
  maxNodes: number;
}

export interface PageRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PageNode {
  ref: string;
  fp: string;
  role: string;
  name: string;
  testId?: string;
  rect: PageRect;
  depth: number;
  state: Record<string, string | boolean | number>;
  css: string;
  dragRegion: boolean;
  disabled: boolean;
  /** mode "dom" only */
  attrs?: Record<string, string>;
}

export interface PageSnapshot {
  rootFound: boolean;
  /** Monotonic per page load. Refs are namespaced with it — see `emit`. */
  generation: number;
  revision: number;
  title: string;
  viewport: { width: number; height: number };
  nodes: PageNode[];
  /** candidates that passed every filter, including those dropped by maxNodes */
  total: number;
  truncated: boolean;
}

export interface AgentConsoleEntry {
  level: string;
  text: string;
  t: number;
}

export interface AgentWindow extends Window {
  __tauriAgentRev?: { rev: number; lastMutationAt: number };
  __tauriAgentConsole?: { entries: AgentConsoleEntry[]; errors: number };
  __tauriAgentRefs?: Map<string, Element>;
  /** Bumped by every snapshot so two snapshots can never issue the same ref. */
  __tauriAgentSnapGen?: number;
  /** installed with each snapshot so `resolve` recomputes fingerprints identically */
  __tauriAgentFp?: (el: Element) => string;
}

export function snapshotInPage(opts: SnapshotScriptOpts): PageSnapshot {
  const w = window as unknown as AgentWindow;
  const LANDMARKS = ["toolbar", "main", "dialog", "region", "navigation", "group", "status"];
  const INTERACTIVE = [
    "button", "link", "checkbox", "radio", "slider", "spinbutton", "textbox",
    "searchbox", "combobox", "listbox", "option", "menuitem", "menuitemcheckbox",
    "menuitemradio", "tab", "switch", "treeitem",
  ];
  const CONSOLE_RING = 200;
  const NAME_MAX = 60;

  function installRev(): void {
    if (w.__tauriAgentRev) return;
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
  }

  function installConsole(): void {
    if (w.__tauriAgentConsole) return;
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
  }

  /** FNV-1a 32-bit. Identity only needs stability, not collision resistance. */
  function hash(s: string): string {
    let h = 0x811c9dc5;
    for (let i = 0; i < s.length; i += 1) {
      h ^= s.charCodeAt(i);
      h = (h + ((h << 1) + (h << 4) + (h << 7) + (h << 8) + (h << 24))) >>> 0;
    }
    return ("00000000" + h.toString(16)).slice(-8);
  }

  function clip(s: string): string {
    const t = String(s).replace(/\s+/g, " ").trim();
    return t.length > NAME_MAX ? t.slice(0, NAME_MAX) : t;
  }

  function esc(id: string): string {
    const css = (window as unknown as { CSS?: { escape?: (s: string) => string } }).CSS;
    return css && css.escape ? css.escape(id) : id.replace(/["\\]/g, "\\$&");
  }

  function inputRole(type: string | null): string {
    const t = (type || "text").toLowerCase();
    if (t === "checkbox") return "checkbox";
    if (t === "radio") return "radio";
    if (t === "range") return "slider";
    if (t === "number") return "spinbutton";
    if (t === "search") return "searchbox";
    if (t === "button" || t === "submit" || t === "reset" || t === "image") return "button";
    if (t === "hidden") return "none";
    return "textbox";
  }

  function roleOf(el: Element): string {
    const explicit = el.getAttribute("role");
    if (explicit && explicit.trim()) return explicit.trim().split(/\s+/)[0] as string;
    // The WebGL canvas has no accessible identity of its own; the agent needs a
    // stable handle for viewport gestures, so its container is named here.
    if (el.getAttribute("data-testid") === "viewport-canvas") return "region";
    const tag = el.tagName.toLowerCase();
    if (tag === "button" || tag === "summary") return "button";
    if (tag === "a") return el.hasAttribute("href") ? "link" : "generic";
    if (tag === "select") return "combobox";
    if (tag === "textarea") return "textbox";
    if (tag === "option") return "option";
    if (tag === "dialog") return "dialog";
    if (tag === "nav") return "navigation";
    if (tag === "main") return "main";
    if (/^h[1-6]$/.test(tag)) return "heading";
    if (tag === "input") return inputRole(el.getAttribute("type"));
    if ((el as HTMLElement).isContentEditable) return "textbox";
    return "generic";
  }

  function labelTextFor(el: Element): string {
    if (el.id) {
      const l = document.querySelector('label[for="' + esc(el.id) + '"]');
      if (l && l.textContent) return l.textContent;
    }
    const anc = el.closest("label");
    if (anc && anc.textContent) return anc.textContent;
    return "";
  }

  function nameOf(el: Element, role: string): string {
    const aria = el.getAttribute("aria-label");
    if (aria && aria.trim()) return clip(aria);
    const lb = el.getAttribute("aria-labelledby");
    if (lb) {
      const parts: string[] = [];
      lb.split(/\s+/).forEach((id) => {
        const r = document.getElementById(id);
        if (r && r.textContent && r.textContent.trim()) parts.push(r.textContent.trim());
      });
      if (parts.length) return clip(parts.join(" "));
    }
    const lf = labelTextFor(el);
    if (lf.trim()) return clip(lf);
    const title = el.getAttribute("title");
    if (title && title.trim()) return clip(title);
    if (el.getAttribute("data-testid") === "viewport-canvas") return "Viewport";
    // A landmark wraps other people's text — a live readout, a whole panel. Naming
    // it from that text puts every child re-render into the container's identity,
    // which reads back as ELEMENT_STALE for an element that never moved.
    if (LANDMARKS.indexOf(role) >= 0) return "";
    return clip(el.textContent || "");
  }

  function isDisabled(el: Element): boolean {
    return el.hasAttribute("disabled") || el.getAttribute("aria-disabled") === "true";
  }

  function stateOf(el: Element): Record<string, string | boolean | number> {
    const s: Record<string, string | boolean | number> = {};
    const field = el as unknown as { checked?: unknown; value?: unknown };
    const pressed = el.getAttribute("aria-pressed");
    if (pressed !== null) s.pressed = pressed;
    const expanded = el.getAttribute("aria-expanded");
    if (expanded !== null) s.expanded = expanded;
    const checked = el.getAttribute("aria-checked");
    if (checked !== null) s.checked = checked;
    else if (typeof field.checked === "boolean") s.checked = field.checked;
    if (typeof field.value === "string" && field.value !== "") s.value = clip(field.value);
    const dof = el.getAttribute("data-dof");
    if (dof !== null) s["data-dof"] = dof;
    if (isDisabled(el)) s.disabled = true;
    return s;
  }

  function visible(el: Element, rect: PageRect): boolean {
    if (rect.width <= 0 || rect.height <= 0) return false;
    if (el.hasAttribute("hidden")) return false;
    const st = getComputedStyle(el);
    if (st.display === "none" || st.visibility === "hidden" || st.visibility === "collapse") {
      return false;
    }
    return st.opacity !== "0";
  }

  /**
   * No class names: this app's classes are Tailwind state utilities that churn on
   * every toggle and hover-render, and a fingerprint built on them would report
   * ELEMENT_STALE for an element that never moved.
   */
  function cssSeg(el: Element): string {
    const tag = el.tagName.toLowerCase();
    if (el.id) return tag + "#" + esc(el.id);
    const parent = el.parentElement;
    if (!parent) return tag;
    let n = 0;
    let idx = 0;
    for (let i = 0; i < parent.children.length; i += 1) {
      const ch = parent.children[i] as Element;
      if (ch.tagName === el.tagName) {
        n += 1;
        if (ch === el) idx = n;
      }
    }
    return n > 1 ? tag + ":nth-of-type(" + idx + ")" : tag;
  }

  function cssPath(el: Element, maxDepth: number): string {
    const segs: string[] = [];
    let cur: Element | null = el;
    while (cur && cur.nodeType === 1 && segs.length < maxDepth) {
      segs.unshift(cssSeg(cur));
      if (cur.id || cur === document.body) break;
      cur = cur.parentElement;
    }
    return segs.join(" > ");
  }

  function fingerprintOf(el: Element): string {
    const role = roleOf(el);
    return hash(
      [
        el.getAttribute("data-testid") || "",
        role,
        nameOf(el, role),
        el.id || "",
        clip(labelTextFor(el)),
        cssPath(el, 4),
      ].join("|"),
    );
  }

  function attrsOf(el: Element): Record<string, string> {
    const out: Record<string, string> = {};
    for (let i = 0; i < el.attributes.length; i += 1) {
      const a = el.attributes[i] as Attr;
      out[a.name] = a.value.length > 120 ? a.value.slice(0, 120) : a.value;
    }
    return out;
  }

  function toRect(r: DOMRect): PageRect {
    return { x: r.left, y: r.top, width: r.width, height: r.height };
  }

  function depthOf(el: Element, root: Element): number {
    let d = 0;
    let cur = el.parentElement;
    while (cur && cur !== root.parentElement) {
      if (LANDMARKS.indexOf(roleOf(cur)) >= 0) d += 1;
      cur = cur.parentElement;
    }
    return d;
  }

  installRev();
  installConsole();
  w.__tauriAgentFp = fingerprintOf;

  // Namespacing refs by generation is what makes a stale ref FAIL instead of
  // aliasing: `@s1e2` is simply absent from generation 2's map, and a scoped
  // snapshot (rootCss) can no longer hand out refs that collide with a full one.
  const generation = (w.__tauriAgentSnapGen || 0) + 1;
  w.__tauriAgentSnapGen = generation;

  const root = opts.rootCss ? document.querySelector(opts.rootCss) : document.body;
  const revision = w.__tauriAgentRev ? w.__tauriAgentRev.rev : 0;
  const empty: PageSnapshot = {
    rootFound: false,
    generation,
    revision,
    title: document.title,
    viewport: { width: window.innerWidth, height: window.innerHeight },
    nodes: [],
    total: 0,
    truncated: false,
  };
  if (!root) return empty;

  const refs = new Map<string, Element>();
  const nodes: PageNode[] = [];
  const candidates = new Set<Element>();
  let total = 0;

  function emit(el: Element, role: string, name: string, rect: PageRect, isText: boolean): void {
    total += 1;
    if (nodes.length >= opts.maxNodes) return;
    const ref = "@s" + generation + "e" + (nodes.length + 1);
    refs.set(ref, el);
    const testId = el.getAttribute("data-testid");
    const node: PageNode = {
      ref,
      fp: isText ? hash(cssPath(el, 4) + "|text|" + name) : fingerprintOf(el),
      role,
      name,
      rect,
      depth: depthOf(el, root as Element) + (isText ? 1 : 0),
      state: isText ? {} : stateOf(el),
      css: cssPath(el, 8),
      dragRegion: el.closest("[data-tauri-drag-region]") !== null,
      disabled: isText ? false : isDisabled(el),
    };
    if (testId !== null) node.testId = testId;
    if (opts.mode === "dom") node.attrs = attrsOf(el);
    nodes.push(node);
  }

  function considerElement(el: Element): void {
    if (el.closest('[aria-hidden="true"]')) return;
    const role = roleOf(el);
    const testId = el.getAttribute("data-testid");
    const wanted =
      INTERACTIVE.indexOf(role) >= 0 ||
      LANDMARKS.indexOf(role) >= 0 ||
      testId !== null ||
      (opts.mode === "accessibility" && role === "heading");
    if (!wanted) return;
    const rect = toRect(el.getBoundingClientRect());
    if (!visible(el, rect)) return;
    candidates.add(el);
    emit(el, role, nameOf(el, role), rect, false);
  }

  function considerText(t: Text): void {
    const parent = t.parentElement;
    if (!parent || candidates.has(parent)) return;
    const tag = parent.tagName.toLowerCase();
    if (tag === "script" || tag === "style" || tag === "title") return;
    const text = clip(t.data);
    if (!text) return;
    if (parent.closest('[aria-hidden="true"]')) return;
    const rect = toRect(parent.getBoundingClientRect());
    if (!visible(parent, rect)) return;
    emit(parent, "text", text, rect, true);
  }

  const showText = opts.mode === "accessibility";
  considerElement(root);
  const walker = document.createTreeWalker(
    root,
    showText ? NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT : NodeFilter.SHOW_ELEMENT,
  );
  let cur = walker.nextNode();
  while (cur) {
    if (cur.nodeType === 1) considerElement(cur as Element);
    else considerText(cur as Text);
    cur = walker.nextNode();
  }

  w.__tauriAgentRefs = refs;
  return {
    rootFound: true,
    generation,
    revision,
    title: document.title,
    viewport: { width: window.innerWidth, height: window.innerHeight },
    nodes,
    total,
    truncated: total > nodes.length,
  };
}
