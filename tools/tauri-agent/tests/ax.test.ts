/**
 * `MacAccessibility` against `tests/fixtures/fake-helper.ts`, over the real JSON-line protocol.
 *
 * The machine is untouched: no AX tree is read, no window server is involved, and every reply is
 * canned. What IS real is the path a reply travels — `HelperClient.request` → `mapHelperError` →
 * `CODE_MAP` → the coercion in `ax.ts` — which is exactly where an unmapped refusal turns into a
 * misleading INTERNAL and where a null window id could turn into a number.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { AgentError, ErrorCode } from "../src/errors.ts";
import { isAgentError } from "../src/errors.ts";
import { MacAccessibility } from "../src/platform/macos/ax.ts";
import { HelperClient, mapHelperError } from "../src/platform/macos/input.ts";

const FIXTURE = join(import.meta.dir, "fixtures", "fake-helper.ts");
const clients: HelperClient[] = [];

/** A `MacAccessibility` wired to a fake helper that answers `replies` and optionally fails `fail`. */
function fakeAx(replies: Record<string, unknown> = {}, fail?: string): MacAccessibility {
  const dir = mkdtempSync(join(tmpdir(), "tauri-agent-ax-"));
  const cannedPath = join(dir, "canned.json");
  writeFileSync(cannedPath, JSON.stringify(replies));
  const argv = [process.execPath, FIXTURE, `--log=${join(dir, "requests.jsonl")}`, `--canned=${cannedPath}`];
  if (fail !== undefined) argv.push(`--fail=${fail}`);
  const client = new HelperClient(() => Promise.resolve(argv));
  clients.push(client);
  return new MacAccessibility(client);
}

async function reject(p: Promise<unknown>): Promise<AgentError> {
  const e = await p.then(
    () => null,
    (err: unknown) => err,
  );
  expect(isAgentError(e)).toBe(true);
  return e as AgentError;
}

const POINT_REPLY = {
  ref: "@a3e7",
  generation: 3,
  pid: 4242,
  role: "AXButton",
  subrole: "AXCloseButton",
  title: "Save",
  enabled: true,
  focused: null,
  bounds: { x: 100, y: 200, w: 80, h: 24 },
  center: { x: 140, y: 212 },
  display: { x: 0, y: 0, w: 1512, h: 982 },
  frontmost: true,
  windowId: 91,
  windowIdSource: "axPrivate",
  windowTitle: "Save As",
};

afterEach(async () => {
  while (clients.length > 0) await clients.pop()?.dispose();
});

describe("CODE_MAP", () => {
  test("the accessibility refusals map to themselves, not to INTERNAL", () => {
    expect(mapHelperError("ax_point", "ELEMENT_STALE", "older generation").code).toBe("ELEMENT_STALE");
    expect(mapHelperError("ax_point", "ELEMENT_NOT_FOUND", "gone").code).toBe("ELEMENT_NOT_FOUND");
    expect(mapHelperError("ax_point", "ELEMENT_MOVING", "animating").code).toBe("ELEMENT_MOVING");
    expect(mapHelperError("ax_point", "POINT_OUTSIDE_WINDOW", "no display").code).toBe("POINT_OUTSIDE_WINDOW");
    // The helper's own code still rides along, so a report can name the verb's exact refusal.
    expect(mapHelperError("ax_point", "ELEMENT_MOVING", "x").details?.helperCode).toBe("ELEMENT_MOVING");
    // Unchanged: an unknown code is still an agent bug.
    expect(mapHelperError("ax_point", "SOMETHING_NEW", "?").code).toBe("INTERNAL");
  });
});

describe("MacAccessibility.point", () => {
  test("a live ref comes back in global points, with no conversion", async () => {
    const ax = fakeAx({ ax_point: POINT_REPLY });
    const p = await ax.point("@a3e7");
    expect(p.center).toEqual({ x: 140, y: 212 });
    expect(p.bounds).toEqual({ x: 100, y: 200, width: 80, height: 24 });
    expect(p.display).toEqual({ x: 0, y: 0, width: 1512, height: 982 });
    expect(p.subrole).toBe("AXCloseButton");
    expect(p.frontmost).toBe(true);
    // `null` is "AX did not expose it", which must not collapse into `false`.
    expect(p.focused).toBeNull();
    expect(p.enabled).toBe(true);
  });

  // The five distinguishable refusals of `ax_point`. Each must arrive as itself: a caller that
  // cannot tell "re-snapshot" from "it is gone" from "it is still moving" cannot recover.
  const REFUSALS: Array<[string, ErrorCode]> = [
    ["ELEMENT_STALE", "ELEMENT_STALE"],
    ["ELEMENT_NOT_FOUND", "ELEMENT_NOT_FOUND"],
    ["ELEMENT_MOVING", "ELEMENT_MOVING"],
    ["POINT_OUTSIDE_WINDOW", "POINT_OUTSIDE_WINDOW"],
    ["INVALID_ARGS", "INVALID_TARGET"],
  ];
  for (const [helperCode, expected] of REFUSALS) {
    test(`${helperCode} reaches the caller as ${expected}`, async () => {
      const ax = fakeAx({}, `ax_point:${helperCode}`);
      const e = await reject(ax.point("@a3e7"));
      expect(e.code).toBe(expected);
      expect((e.details as { helperCode?: string }).helperCode).toBe(helperCode);
    });
  }

  test("a reply missing a geometry field refuses instead of reporting the origin", async () => {
    // (0, 0) is a real, clickable screen location. Defaulting to it would send a click to the
    // menu-bar corner and report success.
    const ax = fakeAx({ ax_point: { ...POINT_REPLY, center: null } });
    const e = await reject(ax.point("@a3e7"));
    expect(e.code).toBe("INTERNAL");
    expect((e.details as { field?: string }).field).toBe("center");
  });
});

describe("MacAccessibility window ids", () => {
  test("a correlated id is `known`, and carries the source that correlated it", async () => {
    const ax = fakeAx({
      ax_windows: {
        pid: 4242,
        privateWindowIdApi: true,
        windows: [
          {
            role: "AXWindow",
            subrole: "AXStandardWindow",
            title: "OneCAD",
            bounds: { x: 0, y: 33, w: 1512, h: 949 },
            main: true,
            modal: null,
            focused: true,
            windowId: 91,
            windowIdSource: "axPrivate",
          },
        ],
      },
    });
    const r = await ax.windows(4242);
    expect(r.privateWindowIdApi).toBe(true);
    const w = r.windows[0]!;
    expect(w.window.known).toBe(true);
    expect(w.window.known === true ? w.window.id : null).toBe(91);
    expect(w.window.source).toBe("axPrivate");
    expect(w.bounds).toEqual({ x: 0, y: 33, width: 1512, height: 949 });
    // `modal: null` from AX is not a modal window.
    expect(w.modal).toBe(false);
    expect(w.main).toBe(true);
  });

  test("an ambiguous id is `known: false` and carries no number a caller could gate on", async () => {
    const ax = fakeAx({
      ax_windows: {
        pid: 4242,
        privateWindowIdApi: false,
        windows: [
          {
            role: "AXWindow",
            subrole: null,
            title: "OneCAD",
            bounds: { x: 0, y: 33, w: 1512, h: 949 },
            main: false,
            modal: false,
            focused: false,
            windowId: null,
            windowIdSource: "ambiguous",
          },
        ],
      },
    });
    const w = (await ax.windows(4242)).windows[0]!;
    expect(w.window.known).toBe(false);
    expect(w.window.source).toBe("ambiguous");
    // The union has no `id` on the unknown arm, so there is nothing to mistake for one.
    expect(Object.hasOwn(w.window, "id")).toBe(false);
    // And `0` — the value a `Number(null)` coercion would have produced — is nowhere in it.
    expect(JSON.stringify(w.window)).not.toContain("\"id\"");
  });

  test("ax_point carries the same union for the window it climbed to", async () => {
    const ax = fakeAx({ ax_point: { ...POINT_REPLY, windowId: null, windowIdSource: "none" } });
    const p = await ax.point("@a3e7");
    expect(p.window.known).toBe(false);
    expect(p.window.source).toBe("none");
  });
});

describe("MacAccessibility walks", () => {
  test("find coerces nodes and reports the generation it minted", async () => {
    const ax = fakeAx({
      ax_find: {
        pid: 4242,
        generation: 5,
        total: 120,
        truncated: true,
        stopReason: "maxVisit",
        nodes: [
          {
            ref: "@a5e1",
            role: "AXButton",
            subrole: null,
            title: "Save",
            value: 3,
            enabled: true,
            focused: false,
            bounds: { x: 1, y: 2, w: 3, h: 4 },
            depth: 7,
            actions: ["AXPress", 42],
          },
        ],
      },
    });
    const walk = await ax.find(4242, { role: "AXButton", title: "Sav" });
    expect(walk.generation).toBe(5);
    expect(walk.truncated).toBe(true);
    expect(walk.stopReason).toBe("maxVisit");
    const n = walk.nodes[0]!;
    expect(n.ref).toBe("@a5e1");
    expect(n.value).toBe(3);
    expect(n.bounds).toEqual({ x: 1, y: 2, width: 3, height: 4 });
    // A non-string action name is dropped rather than stringified into a name nothing can press.
    expect(n.actions).toEqual(["AXPress"]);
  });

  test("snapshot reports which window it chose, and a null window stays null", async () => {
    const ax = fakeAx({
      ax_snapshot: {
        pid: 4242,
        generation: 2,
        nodes: [],
        total: 0,
        truncated: false,
        stopReason: "complete",
        windowSource: "requested",
        window: null,
      },
    });
    const snap = await ax.snapshot(4242, { window: 91, maxNodes: 20 });
    expect(snap.windowSource).toBe("requested");
    expect(snap.window).toBeNull();
  });

  test("menu modifiers are validated against the physical keys, not cast", async () => {
    const ax = fakeAx({
      ax_menu: {
        pid: 4242,
        hasMenuBar: true,
        total: 2,
        truncated: false,
        stopReason: "complete",
        items: [
          {
            title: "Save",
            role: "AXMenuItem",
            enabled: true,
            bounds: null,
            depth: 3,
            path: ["File", "Save"],
            key: { char: "s", virtualKey: null, glyph: null, modifiersRaw: 0, mods: ["Command", "Hyper"] },
          },
          { title: null, role: "AXMenu", enabled: null, bounds: null, depth: 2, path: ["File"], key: null },
        ],
      },
    });
    const menu = await ax.menu(4242);
    expect(menu.hasMenuBar).toBe(true);
    // "Hyper" is not a modifier the input verbs can send, so it never reaches a caller as one.
    expect(menu.items[0]!.key?.mods).toEqual(["Command"]);
    expect(menu.items[1]!.key).toBeNull();
  });

  test("modal blockers keep the window shape and add the reason", async () => {
    const ax = fakeAx({
      ax_modal: {
        pid: 4242,
        modal: true,
        blockers: [
          {
            role: "AXSheet",
            subrole: null,
            title: "Save changes?",
            bounds: { x: 10, y: 20, w: 400, h: 200 },
            main: false,
            modal: true,
            focused: true,
            windowId: 92,
            windowIdSource: "boundsMatch",
            reason: "sheet",
          },
        ],
      },
    });
    const m = await ax.modal(4242);
    expect(m.modal).toBe(true);
    expect(m.blockers[0]!.reason).toBe("sheet");
    expect(m.blockers[0]!.window.known).toBe(true);
  });
});
