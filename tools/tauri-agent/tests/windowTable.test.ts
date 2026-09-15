/**
 * Window identity: which native window a Tauri label actually names.
 *
 * The cost of being wrong here is a real CGEvent posted into a window the caller did not name,
 * so the rule under test throughout is that an unresolved label **refuses** rather than falling
 * back to the largest-window heuristic. `pickLargestWindow` is bootstrap; the table is identity.
 */
import { describe, expect, test } from "bun:test";
import { isAgentError } from "../src/errors.ts";
import {
  buildWindowTable,
  pickLargestWindow,
  requireIdentity,
  selectTargetWindow,
} from "../src/session/calibrate.ts";
import type { TauriWindowReading } from "../src/session/types.ts";
import type { AxWindowInfo, AxWindowsResult, NativeWindowInfo } from "../src/platform/adapter.ts";
import { axWindow } from "./fixtures/fakeAx.ts";

const PID = 4242;

function reading(over: Partial<TauriWindowReading> & { label: string }): TauriWindowReading {
  return {
    title: "OneCAD",
    outerPositionPx: { x: 200, y: 100 },
    innerSizePx: { width: 2400, height: 1600 },
    scaleFactor: 2,
    ...over,
  };
}

function native(over: Partial<NativeWindowInfo> & { windowId: number }): NativeWindowInfo {
  return {
    layer: 0,
    bounds: { x: 100, y: 50, width: 1200, height: 800 },
    onscreen: true,
    ...over,
  };
}

function axResult(windows: AxWindowInfo[], privateWindowIdApi = true): AxWindowsResult {
  return { pid: PID, privateWindowIdApi, windows };
}

function code(fn: () => unknown): string {
  try {
    fn();
    return "(did not throw)";
  } catch (e) {
    return isAgentError(e) ? e.code : `(${String(e)})`;
  }
}

describe("buildWindowTable", () => {
  test("correlates a single window without needing to disambiguate anything", () => {
    const table = buildWindowTable(
      PID,
      [reading({ label: "main" })],
      axResult([axWindow({ window: { known: true, id: 7, source: "axPrivate" } })]),
      [native({ windowId: 7 })],
    );

    expect(table.entries).toHaveLength(1);
    expect(table.entries[0]?.label).toBe("main");
    expect(table.entries[0]?.windowId).toBe(7);
    expect(table.entries[0]?.unresolved).toBeUndefined();
    expect(table.privateWindowIdApi).toBe(true);
  });

  test("two windows of different size and title each resolve to their own id", () => {
    const table = buildWindowTable(
      PID,
      [
        reading({ label: "main", title: "OneCAD — Untitled" }),
        reading({
          label: "settings",
          title: "Settings",
          outerPositionPx: { x: 900, y: 400 },
          innerSizePx: { width: 800, height: 600 },
        }),
      ],
      axResult([
        axWindow({ title: "OneCAD — Untitled", window: { known: true, id: 7, source: "axPrivate" } }),
        axWindow({
          title: "Settings",
          bounds: { x: 450, y: 200, width: 400, height: 300 },
          main: false,
          focused: false,
          window: { known: true, id: 9, source: "axPrivate" },
        }),
      ]),
      [native({ windowId: 7 }), native({ windowId: 9, bounds: { x: 450, y: 200, width: 400, height: 300 } })],
    );

    const byLabel = Object.fromEntries(table.entries.map((e) => [e.label, e.windowId]));
    expect(byLabel).toEqual({ main: 7, settings: 9 });
  });

  /**
   * The deliberate exception: ONE Tauri window and ONE layer-0 native window. AX could not name
   * an id, but counting can — with a single window on each side there is nothing to confuse it
   * with. This is the common single-window app, and refusing it would be needless.
   */
  test("one window each side resolves by counting even when AX cannot name the id", () => {
    const table = buildWindowTable(
      PID,
      [reading({ label: "main" })],
      axResult([axWindow({ window: { known: false, source: "ambiguous" } })], false),
      [native({ windowId: 7 })],
    );

    expect(table.entries[0]?.windowId).toBe(7);
    expect(table.entries[0]?.source).toBe("sole");
    // Still reported, so a caller can see every id in this table came from counting, not AX.
    expect(table.privateWindowIdApi).toBe(false);
  });

  /**
   * With more than one window the counting shortcut does not apply, and an unnameable id must
   * refuse: guessing would post clicks into whichever window happened to sort first.
   */
  test("an unknowable window id among SEVERAL leaves the label unresolved rather than guessing", () => {
    const table = buildWindowTable(
      PID,
      [
        reading({ label: "main", title: "OneCAD" }),
        reading({ label: "settings", title: "Settings", outerPositionPx: { x: 900, y: 400 } }),
      ],
      axResult(
        [
          axWindow({ title: "OneCAD", window: { known: false, source: "ambiguous" } }),
          axWindow({
            title: "Settings",
            bounds: { x: 450, y: 200, width: 400, height: 300 },
            window: { known: true, id: 9, source: "boundsMatch" },
          }),
        ],
        false,
      ),
      [native({ windowId: 7 }), native({ windowId: 9, bounds: { x: 450, y: 200, width: 400, height: 300 } })],
    );

    const main = table.entries.find((e) => e.label === "main");
    expect(main?.windowId).toBeUndefined();
    expect(main?.unresolved).toContain("could not name a CGWindowID");
    // The window that COULD be named is unaffected: one bad correlation does not poison the table.
    expect(table.entries.find((e) => e.label === "settings")?.windowId).toBe(9);
  });

  test("one CGWindowID claimed by two labels unresolves both, never picks a winner", () => {
    const shared = { known: true as const, id: 7, source: "boundsMatch" };
    const table = buildWindowTable(
      PID,
      [reading({ label: "main", title: null }), reading({ label: "second", title: null })],
      axResult([axWindow({ title: null, window: shared }), axWindow({ title: null, window: shared })], false),
      [native({ windowId: 7 })],
    );

    for (const entry of table.entries) {
      expect(entry.windowId, entry.label).toBeUndefined();
      expect(entry.unresolved, entry.label).toBeTruthy();
    }
  });
});

describe("requireIdentity", () => {
  const table = buildWindowTable(
    PID,
    [reading({ label: "main" })],
    axResult([axWindow({ window: { known: true, id: 7, source: "axPrivate" } })]),
    [native({ windowId: 7 })],
  );

  test("returns the entry for a correlated label", () => {
    expect(requireIdentity(table, "main").windowId).toBe(7);
  });

  test("a label that does not exist is WINDOW_NOT_FOUND, and says which labels do", () => {
    expect(code(() => requireIdentity(table, "settings"))).toBe("WINDOW_NOT_FOUND");
    try {
      requireIdentity(table, "settings");
    } catch (e) {
      expect(isAgentError(e) && (e.details as { known?: string[] }).known).toEqual(["main"]);
    }
  });

  test("an unresolved label refuses and carries the reason, not a fallback id", () => {
    // Two AX windows at the same origin with no title to separate them: genuinely ambiguous.
    const unresolved = buildWindowTable(
      PID,
      [reading({ label: "main", title: null }), reading({ label: "second", title: null })],
      axResult([axWindow({ title: null }), axWindow({ title: null, window: { known: true, id: 9, source: "axPrivate" } })]),
      [native({ windowId: 7 }), native({ windowId: 9 })],
    );
    expect(code(() => requireIdentity(unresolved, "main"))).toBe("WINDOW_NOT_FOUND");
  });
});

describe("selectTargetWindow", () => {
  const windows = [
    native({ windowId: 7, bounds: { x: 100, y: 50, width: 1200, height: 800 } }),
    native({ windowId: 9, bounds: { x: 450, y: 200, width: 400, height: 300 } }),
  ];

  /**
   * The ordering is the whole point: before AX has produced a table there is nothing better than
   * the heuristic, and after it there is nothing worse than falling back to it.
   */
  test("falls back to the largest window ONLY while no table exists", () => {
    expect(selectTargetWindow(windows, undefined, "main", PID).windowId).toBe(
      pickLargestWindow(windows, PID).windowId,
    );
  });

  test("once a table exists, identity comes from it — even when it names the smaller window", () => {
    const table = buildWindowTable(
      PID,
      [reading({ label: "settings", title: "Settings", innerSizePx: { width: 800, height: 600 } })],
      axResult([
        axWindow({
          title: "Settings",
          bounds: { x: 450, y: 200, width: 400, height: 300 },
          window: { known: true, id: 9, source: "axPrivate" },
        }),
      ]),
      windows,
    );

    const chosen = selectTargetWindow(windows, table, "settings", PID);
    expect(chosen.windowId).toBe(9);
    // The heuristic would have answered 7, which is exactly the mis-target this replaces.
    expect(pickLargestWindow(windows, PID).windowId).toBe(7);
  });

  test("a correlated window that is no longer on screen refuses instead of substituting another", () => {
    const table = buildWindowTable(
      PID,
      [reading({ label: "main" })],
      axResult([axWindow({ window: { known: true, id: 7, source: "axPrivate" } })]),
      [native({ windowId: 7 })],
    );
    expect(code(() => selectTargetWindow([native({ windowId: 9 })], table, "main", PID))).toBe("WINDOW_NOT_FOUND");
  });
});
