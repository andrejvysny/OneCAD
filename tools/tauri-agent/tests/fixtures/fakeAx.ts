/**
 * Accessibility fakes. Nothing here reads a real AX tree: `unusedAx` satisfies the adapter shape
 * for tests that never touch accessibility, and `FakeAx` answers canned replies so the resolver's
 * refusal ladder can be driven without a window server or a TCC grant.
 */
import { AgentError } from "../../src/errors.ts";
import type { Rect } from "../../src/geometry/types.ts";
import type {
  AxFindOpts,
  AxMenuResult,
  AxModalResult,
  AxNode,
  AxPointInfo,
  AxSnapshotOpts,
  AxSnapshotResult,
  AxWalkResult,
  AxWindowInfo,
  AxWindowsResult,
  AxMenuItem,
  AxMenuPressResult,
  AxPressResult,
  AxSetValueResult,
  NativeAccessibility,
  NativeWindowInfo,
  NativeWindows,
} from "../../src/platform/adapter.ts";

/** For a test that constructs a PlatformAdapter but never reaches accessibility. */
export function unusedAx(): NativeAccessibility {
  const no = (verb: string) => async (): Promise<never> => {
    throw new Error(`accessibility verb '${verb}' is not used in this test`);
  };
  return {
    windows: no("ax_windows"),
    snapshot: no("ax_snapshot"),
    find: no("ax_find"),
    point: no("ax_point"),
    focusedWindow: no("ax_focused_window"),
    modal: no("ax_modal"),
    menu: no("ax_menu"),
    press: no("ax_press"),
    setValue: no("ax_set_value"),
    menuPress: no("ax_menu_press"),
  };
}

export function axNode(over: Partial<AxNode> & { ref: string }): AxNode {
  return {
    role: "AXButton",
    subrole: null,
    title: "Save",
    value: null,
    enabled: true,
    focused: null,
    bounds: { x: 100, y: 100, width: 80, height: 24 },
    depth: 3,
    actions: ["AXPress"],
    ...over,
  };
}

/** One AX window. `window` defaults to a KNOWN id, so a test opts in to the unknowable case. */
export function axWindow(over: Partial<AxWindowInfo> = {}): AxWindowInfo {
  return {
    role: "AXWindow",
    subrole: "AXStandardWindow",
    title: "OneCAD",
    bounds: { x: 100, y: 50, width: 1200, height: 800 },
    main: true,
    modal: false,
    focused: true,
    window: { known: true, id: 7, source: "axPrivate" },
    ...over,
  };
}

export function axPoint(over: Partial<AxPointInfo> & { ref: string }): AxPointInfo {
  return {
    generation: 1,
    pid: 4242,
    role: "AXButton",
    subrole: null,
    title: "Save",
    enabled: true,
    focused: null,
    bounds: { x: 100, y: 100, width: 80, height: 24 },
    center: { x: 140, y: 112 },
    display: { x: 0, y: 0, width: 1512, height: 982 },
    frontmost: true,
    window: { known: true, id: 7, source: "axPrivate" },
    windowTitle: "OneCAD",
    ...over,
  };
}

/** Canned accessibility. Every call is recorded, so a test can prove what was NOT called. */
export class FakeAx implements NativeAccessibility {
  readonly calls: Array<{ verb: string; arg: unknown }> = [];
  /** `find` answers these, in order of the queries it is given. */
  findNodes: AxNode[] = [];
  findGeneration = 2;
  pointReply: AxPointInfo | AgentError = axPoint({ ref: "@a1e1" });
  /** `snapshot` answers these. */
  snapshotNodes: AxNode[] = [];
  snapshotGeneration = 1;
  snapshotWindow: AxWindowInfo | null = null;
  snapshotTruncated = false;
  /** Mirrors the helper, where every walk mints a fresh generation. */
  bumpOnSnapshot = false;

  /** `windows` answers these; the window-identity table and the input gate both read it. */
  windowList: AxWindowInfo[] = [];
  privateWindowIdApi = true;
  /** Set to make `windows` reject, as a broken helper would. */
  windowsError?: AgentError;

  async windows(pid: number): Promise<AxWindowsResult> {
    this.calls.push({ verb: "ax_windows", arg: pid });
    if (this.windowsError) throw this.windowsError;
    return { pid, privateWindowIdApi: this.privateWindowIdApi, windows: this.windowList };
  }

  async snapshot(pid: number, opts: AxSnapshotOpts = {}): Promise<AxSnapshotResult> {
    this.calls.push({ verb: "ax_snapshot", arg: { pid, ...opts } });
    const generation = this.snapshotGeneration;
    if (this.bumpOnSnapshot) this.snapshotGeneration += 1;
    return {
      pid,
      generation,
      nodes: this.snapshotNodes,
      total: this.snapshotNodes.length,
      truncated: this.snapshotTruncated,
      stopReason: this.snapshotTruncated ? "maxNodes" : "complete",
      windowSource: "focused",
      window: this.snapshotWindow,
    };
  }

  async find(pid: number, opts: AxFindOpts): Promise<AxWalkResult> {
    this.calls.push({ verb: "ax_find", arg: { pid, ...opts } });
    return {
      pid,
      generation: this.findGeneration,
      nodes: this.findNodes,
      total: this.findNodes.length,
      truncated: false,
      stopReason: "complete",
    };
  }

  async point(ref: string): Promise<AxPointInfo> {
    this.calls.push({ verb: "ax_point", arg: ref });
    if (this.pointReply instanceof AgentError) throw this.pointReply;
    return { ...this.pointReply, ref };
  }

  async focusedWindow(pid: number): Promise<AxWindowInfo | null> {
    this.calls.push({ verb: "ax_focused_window", arg: pid });
    return null;
  }

  async modal(pid: number): Promise<AxModalResult> {
    this.calls.push({ verb: "ax_modal", arg: pid });
    return { pid, modal: false, blockers: [] };
  }

  /**
   * The menu bar `menu()` answers. It defaults to EMPTY-with-no-bar, which is what the earlier
   * fixture hardcoded — and that made every rendering assertion vacuous, because a renderer
   * cannot be wrong about zero items. A test that cares about rendering sets this.
   */
  menuItems: AxMenuItem[] = [];
  hasMenuBar = false;

  async menu(pid: number): Promise<AxMenuResult> {
    this.calls.push({ verb: "ax_menu", arg: pid });
    return {
      pid,
      hasMenuBar: this.hasMenuBar || this.menuItems.length > 0,
      items: this.menuItems,
      total: this.menuItems.length,
      truncated: false,
      stopReason: "complete",
    };
  }

  // --- actuation ------------------------------------------------------------
  // Set the `*Error` fields to make a verb refuse, exactly as the helper does for an
  // unadvertised action, a non-settable attribute or an unknown menu path.

  pressError?: AgentError;
  setValueError?: AgentError;
  menuPressError?: AgentError;
  /** What `setValue` reports the element holds afterwards; defaults to what was written. */
  setValueReadBack?: string | null;

  async press(ref: string, action?: string, acceptDisruption?: "yes"): Promise<AxPressResult> {
    this.calls.push({ verb: "ax_press", arg: { ref, action, acceptDisruption } });
    if (this.pressError) throw this.pressError;
    return { ref, action: action ?? "AXPress", role: "AXButton" };
  }

  async setValue(ref: string, value: string): Promise<AxSetValueResult> {
    this.calls.push({ verb: "ax_set_value", arg: { ref, value } });
    if (this.setValueError) throw this.setValueError;
    const back = this.setValueReadBack === undefined ? value : this.setValueReadBack;
    return { ref, role: "AXTextField", requested: value, value: back, matched: back === value };
  }

  async menuPress(pid: number, path: string[]): Promise<AxMenuPressResult> {
    this.calls.push({ verb: "ax_menu_press", arg: { pid, path } });
    if (this.menuPressError) throw this.menuPressError;
    return { pid, path };
  }
}

export function nativeWindow(windowId: number, bounds: Rect, over: Partial<NativeWindowInfo> = {}): NativeWindowInfo {
  return { windowId, layer: 0, bounds, onscreen: true, ...over };
}

/** A `NativeWindows` whose list is fixed; `focus`/`isFrontmost` are never the subject here. */
export function fakeWindows(list: NativeWindowInfo[]): NativeWindows & { listCalls: number } {
  const self = {
    listCalls: 0,
    list: async (): Promise<NativeWindowInfo[]> => {
      self.listCalls += 1;
      return list;
    },
    isFrontmost: async (): Promise<boolean> => true,
    focus: async (): Promise<boolean> => true,
  };
  return self;
}

/**
 * One menu item shaped exactly as the helper reports it — `path` ALREADY ENDS with this item's
 * own title, because the walk pushes onto its ancestor stack before it reads the path. Getting
 * that wrong in a fixture is how a renderer that duplicates the last segment passes its tests.
 */
export function axMenuItem(over: Partial<AxMenuItem> & { path: string[] }): AxMenuItem {
  const title = over.path[over.path.length - 1] ?? null;
  return {
    title,
    role: "AXMenuItem",
    enabled: true,
    bounds: null,
    depth: over.path.length + 1,
    key: null,
    ...over,
  };
}
