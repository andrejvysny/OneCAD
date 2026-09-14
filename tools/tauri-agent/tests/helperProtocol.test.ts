/**
 * Builds the real Swift helper and drives it over the real JSON-line protocol.
 *
 * Deliberately read-only with respect to the machine: it reads the cursor, reads
 * permissions WITHOUT prompting, lists windows, and moves the cursor to where it
 * already is. It never clicks, presses a key, or scrolls — those would land in whatever
 * application happens to be frontmost.
 */
import { afterAll, describe, expect, test } from "bun:test";

import { AgentError, isAgentError } from "../src/errors.ts";
import { ensureHelper, helperCacheDir } from "../src/platform/macos/build.ts";
import { getHelperClient, MacInput } from "../src/platform/macos/input.ts";
import { MacWindows } from "../src/platform/macos/windows.ts";

const swiftc = Bun.which("swiftc");
const runnable = process.platform === "darwin" && swiftc !== null;
const why = `needs macOS with swiftc on PATH (platform=${process.platform}, swiftc=${swiftc ?? "missing"})`;
// Mirror of ONECAD_REQUIRE_WORKER: a gate that silently skips is a gate that proves nothing.
if (!runnable && process.env.TAURI_AGENT_REQUIRE_HELPER === "1") {
  throw new Error(`[helperProtocol] TAURI_AGENT_REQUIRE_HELPER=1 but the real helper cannot run: ${why}`);
}
if (!runnable) console.error(`[helperProtocol] skipped: ${why}.`);

const input = new MacInput();
const windows = new MacWindows();
const client = getHelperClient();

afterAll(async () => {
  if (runnable) await input.dispose();
});

/** Permission-independent: the verb either works or reports the missing grant. */
async function tolerate(fn: () => Promise<void>): Promise<string | null> {
  try {
    await fn();
    return null;
  } catch (e) {
    if (isAgentError(e) && e.code === "NATIVE_INPUT_PERMISSION_DENIED") return e.code;
    throw e;
  }
}

describe.skipIf(!runnable)("macOS helper build + protocol", () => {
  test("ensureHelper compiles a cached binary named by source hash", async () => {
    const bin = await ensureHelper();
    expect(bin.startsWith(`${helperCacheDir()}/macos-helper-`)).toBe(true);
    expect(await Bun.file(bin).exists()).toBe(true);
    expect(await ensureHelper()).toBe(bin);
  }, 180_000);

  test("--selftest reports permissions, its own windows and a cursor round-trip", async () => {
    const bin = await ensureHelper();
    const proc = Bun.spawn([bin, "--selftest"], { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
    const [code, out] = await Promise.all([proc.exited, new Response(proc.stdout).text()]);
    console.error(`[helperProtocol] --selftest exit=${code} ${out.trim()}`);

    const report = JSON.parse(out) as Record<string, unknown>;
    expect(typeof report.ok).toBe("boolean");
    expect(report.version).toBe("1.3.0");
    expect(typeof report.pid).toBe("number");
    const perms = report.permissions as Record<string, unknown>;
    expect(typeof perms.accessibility).toBe("boolean");
    expect(typeof perms.screenRecording).toBe("boolean");
    // The helper is a faceless CLI, so an empty window list is the expected shape.
    expect(Array.isArray(report.windows)).toBe(true);
    const cursor = report.cursor as Record<string, { x: number; y: number }>;
    expect(typeof cursor.before.x).toBe("number");
    expect(typeof cursor.after.y).toBe("number");
    expect(typeof report.moveOk).toBe("boolean");
    expect(code).toBe(report.ok === true ? 0 : 1);
  }, 180_000);

  test("--version prints the protocol banner", async () => {
    const bin = await ensureHelper();
    const proc = Bun.spawn([bin, "--version"], { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
    const [code, out] = await Promise.all([proc.exited, new Response(proc.stdout).text()]);
    expect(code).toBe(0);
    expect(JSON.parse(out)).toEqual({ version: "1.3.0", protocol: 4 });
  }, 180_000);

  test("cursor() returns finite global points", async () => {
    const p = await input.cursor();
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  }, 180_000);

  test("permissions() does not prompt and returns two booleans", async () => {
    const perms = await input.permissions({ prompt: false });
    console.error(`[helperProtocol] permissions ${JSON.stringify(perms)}`);
    expect(typeof perms.accessibility).toBe("boolean");
    expect(typeof perms.screenRecording).toBe("boolean");
  });

  test("layout() reports a non-empty input source id and a boolean, without erroring", async () => {
    const layout = await input.layout?.();
    console.error(`[helperProtocol] layout ${JSON.stringify(layout)}`);
    expect(layout).toBeDefined();
    expect(typeof layout?.inputSourceId).toBe("string");
    expect((layout?.inputSourceId ?? "").length).toBeGreaterThan(0);
    expect(typeof layout?.isAnsiUs).toBe("boolean");
  });

  test("windows.list() for this pid returns well-formed rows (usually none)", async () => {
    const rows = await windows.list(process.pid);
    expect(Array.isArray(rows)).toBe(true);
    for (const row of rows) {
      expect(typeof row.windowId).toBe("number");
      expect(typeof row.bounds.width).toBe("number");
      expect(typeof row.bounds.height).toBe("number");
      expect(typeof row.onscreen).toBe("boolean");
    }
  });

  test("move() to the current cursor position is a no-op that exercises the input path", async () => {
    const before = await input.cursor();
    const denied = await tolerate(() => input.move(before, { durationMs: 0, steps: 1 }));
    if (denied) {
      console.error("[helperProtocol] Accessibility not granted; move path reported the grant instead.");
      return;
    }
    const after = await input.cursor();
    expect(Math.abs(after.x - before.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(1);
  });

  test("releaseAll() never throws and the helper actually answered it", async () => {
    await expect(input.releaseAll()).resolves.toBeUndefined();
    // A swallowed failure also resolves, so assert the reply shape the helper alone produces.
    const reply = await client.releaseAll();
    expect(Array.isArray(reply.releasedButtons)).toBe(true);
    expect(Array.isArray(reply.releasedMods)).toBe(true);
  });

  test("a forced release of something not held reports nothing and does not error", async () => {
    // Still read-only with respect to the machine: `force` reaches an input only when the OS
    // reports it held, and a test run is not holding the middle mouse button. The unknown name
    // must be dropped rather than refused — `release_all` is the recovery verb.
    const reply = await client.request("release_all", { osState: false, force: ["middle", "no-such-button"] });
    expect(reply.releasedButtons).toEqual([]);
    expect(reply.releasedMods).toEqual([]);
    // The hostile name did not wedge or kill the helper: it still answers.
    expect(Number.isFinite((await input.cursor()).x)).toBe(true);
  });

  test("a bad argument maps to an INVALID_TARGET AgentError", async () => {
    const err = await input.keyDown("no-such-key").then(
      () => null,
      (e: unknown) => e,
    );
    expect(isAgentError(err)).toBe(true);
    expect((err as AgentError).code).toBe("INVALID_TARGET");
  });

  test("an out-of-range pid is rejected instead of trapping the helper", async () => {
    const err = await client.request("frontmost", { pid: 1e10 }).then(
      () => null,
      (e: unknown) => e,
    );
    expect(isAgentError(err)).toBe(true);
    expect((err as AgentError).code).toBe("INVALID_TARGET");
    // The helper survived the hostile argument: it still answers.
    expect(Number.isFinite((await input.cursor()).x)).toBe(true);
  });

  test("an absurd durationMs is clamped by the helper rather than trapping it", async () => {
    const before = await input.cursor();
    // steps:1 keeps this a single no-op move; only the numeric conversion is under test.
    const denied = await tolerate(async () => {
      await client.request("move", { x: before.x, y: before.y, durationMs: 1e20, steps: 1 });
    });
    if (denied) return;
    const after = await input.cursor();
    expect(Math.abs(after.x - before.x)).toBeLessThanOrEqual(1);
  });

  test("isFrontmost() reports without activating anything", async () => {
    expect(await windows.isFrontmost(process.pid)).toBe(false);
  });

  test("MacInput refuses an out-of-range duration before the helper sees it", async () => {
    const before = await input.cursor();
    const err = await input.move(before, { durationMs: 1e20 }).then(
      () => null,
      (e: unknown) => e,
    );
    expect((err as AgentError).code).toBe("INVALID_TARGET");
    expect(Number.isFinite((await input.cursor()).y)).toBe(true);
  });
});

/**
 * Accessibility reads.
 *
 * Still read-only with respect to the machine, and deliberately so: these verbs only READ the AX
 * tree of processes that are ALREADY running, and none of them performs an AXAction — no AXPress,
 * no AXSetValue. Nothing is launched and nothing is clicked, so a run cannot land in whatever
 * application happens to be frontmost.
 *
 * The primary target is this test process, which — like any faceless CLI — normally has no AX
 * windows at all, so every assertion below that does not name `guiTarget` is meaningful with an
 * empty window list. The richer assertions additionally need a GUI application already on screen;
 * when the sweep finds none they log that they were vacuous rather than inventing a target.
 */
interface AxRect {
  x: number;
  y: number;
  w: number;
  h: number;
}
interface AxNode {
  ref: string;
  role: string | null;
  subrole: string | null;
  title: string | null;
  value: string | number | null;
  enabled: boolean | null;
  focused: boolean | null;
  bounds: AxRect | null;
  depth: number;
  actions: string[];
}
interface AxWindowRow {
  role: string | null;
  subrole: string | null;
  title: string | null;
  bounds: AxRect | null;
  main: boolean | null;
  modal: boolean | null;
  focused: boolean;
  windowId: number | null;
  windowIdSource: string;
}
interface AxWalkReply {
  pid: number;
  generation: number;
  nodes: AxNode[];
  total: number;
  truncated: boolean;
  stopReason: string;
}

async function ax<T>(verb: string, args: Record<string, unknown>): Promise<T> {
  return (await client.request(verb, args)) as unknown as T;
}

/** The mapped AgentError code plus the helper's own code, which rides in `details.helperCode`. */
async function axFailure(verb: string, args: Record<string, unknown>): Promise<[string, string | undefined]> {
  const e = await client.request(verb, args).then(
    () => null,
    (err: unknown) => err,
  );
  expect(isAgentError(e)).toBe(true);
  const err = e as AgentError;
  return [err.code, (err.details as { helperCode?: string } | undefined)?.helperCode];
}

interface GuiTarget {
  pid: number;
  /** An on-screen window of that process whose AX element resolves to a CGWindowID. */
  windowId: number;
  /** Whether `ax_snapshot` can find a window on its own: focused, main, or the process's only one. */
  rooted: boolean;
}

let guiTargetCache: GuiTarget | null | undefined;

/**
 * A GUI application that is already on screen, or null.
 *
 * Nothing is launched: the sweep reads `ps` and then asks `ax_windows` about processes that are
 * already running, which is the same read-only traffic as every other assertion here.
 *
 * A candidate only counts when it has an AX window that resolves to a CGWindowID AND that
 * CGWindowList reports on screen. Both conditions are load-bearing: an unresolvable id would make
 * the coordinate cross-check vacuous, and an off-screen window would make `ax_point` refuse for a
 * reason that has nothing to do with what is under test.
 */
async function guiTarget(): Promise<GuiTarget | null> {
  if (guiTargetCache !== undefined) return guiTargetCache;
  const ps = Bun.spawn(["ps", "-axo", "pid=,args="], { stdin: "ignore", stdout: "pipe", stderr: "ignore" });
  const listing = await new Response(ps.stdout).text();
  const candidates: number[] = [];
  for (const line of listing.split("\n")) {
    const match = /^\s*(\d+)\s+(.*)$/.exec(line);
    if (!match || !match[2].includes(".app/Contents/MacOS/")) continue;
    const pid = Number(match[1]);
    if (pid !== process.pid && !candidates.includes(pid)) candidates.push(pid);
  }
  let fallback: GuiTarget | null = null;
  // The cap used to be 25, which on a developer machine ran out before reaching any application
  // with a VISIBLE window — this machine has 70 candidates and the visible ones sort late — so the
  // whole AX group quietly went vacuous while still reporting green. Each probe is one helper
  // round-trip and the loop stops at the first usable target, so a wider sweep costs nothing on a
  // machine that has one and little on a headless runner, which has almost no candidates at all.
  for (const pid of candidates.slice(0, 120)) {
    try {
      const reply = await ax<{ windows: AxWindowRow[] }>("ax_windows", { pid });
      const onscreen = await windows.list(pid);
      const usable = reply.windows.filter(
        (w) =>
          w.windowId !== null &&
          w.bounds !== null &&
          w.bounds.w > 0 &&
          w.bounds.h > 0 &&
          onscreen.some((c) => c.windowId === w.windowId),
      );
      if (usable.length === 0) continue;
      const rootable = usable.find((w) => w.focused || w.main === true) ?? (reply.windows.length === 1 ? usable[0] : undefined);
      const found: GuiTarget = {
        pid,
        windowId: (rootable ?? usable[0]).windowId as number,
        rooted: rootable !== undefined,
      };
      // Prefer a process whose window `ax_snapshot` can root itself at, so the focused/main ladder
      // is exercised rather than always short-circuited by an explicit window id.
      if (found.rooted) {
        guiTargetCache = found;
        return found;
      }
      fallback ??= found;
    } catch {
      // The process exited between `ps` and the probe, or does not answer AX. Try the next one.
      continue;
    }
  }
  guiTargetCache = fallback;
  if (fallback === null) {
    console.error(
      `[helperProtocol] no GUI application with an on-screen AX window among ${candidates.length} candidates;` +
        " AX tree assertions are VACUOUS. On a developer machine, open any application window and re-run" +
        " to make them real; on a headless runner this is expected.",
    );
  }
  return fallback;
}

describe.skipIf(!runnable)("macOS helper accessibility reads", () => {
  test("ax_windows says where every window id came from (usually no windows at all)", async () => {
    const reply = await ax<{ pid: number; privateWindowIdApi: boolean; windows: AxWindowRow[] }>("ax_windows", {
      pid: process.pid,
    });
    console.error(
      `[helperProtocol] ax privateWindowIdApi=${reply.privateWindowIdApi} ownWindows=${reply.windows.length}`,
    );
    expect(reply.pid).toBe(process.pid);
    // Whether `_AXUIElementGetWindow` resolved is reported, never assumed: it is a private symbol.
    expect(typeof reply.privateWindowIdApi).toBe("boolean");
    expect(Array.isArray(reply.windows)).toBe(true);
    for (const row of reply.windows) {
      expect(["axPrivate", "boundsMatch", "ambiguous", "none"]).toContain(row.windowIdSource);
      // A guessed id is never invented: an unresolved one is null and says so in the source.
      expect(row.windowId === null || typeof row.windowId === "number").toBe(true);
      expect(typeof row.focused).toBe("boolean");
    }
  }, 30_000);

  test("an AX window's bounds are the same global points CGWindowList reports", async () => {
    const target = await guiTarget();
    if (target === null) return;
    const pid = target.pid;
    const reply = await ax<{ windows: AxWindowRow[] }>("ax_windows", { pid });
    const cg = await windows.list(pid, { all: true });
    let compared = 0;
    for (const row of reply.windows) {
      if (row.windowId === null || row.bounds === null) continue;
      const match = cg.find((w) => w.windowId === row.windowId);
      if (match === undefined) continue;
      compared += 1;
      // This is the whole coordinate-space claim: AX position/size are already global display
      // points, top-left origin, so an AX rect can go straight to `click` with no conversion.
      expect(Math.abs(row.bounds.x - match.bounds.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(row.bounds.y - match.bounds.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(row.bounds.w - match.bounds.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(row.bounds.h - match.bounds.height)).toBeLessThanOrEqual(1);
    }
    console.error(`[helperProtocol] ax/CGWindowList bounds agreed for ${compared} window(s) of pid ${pid}`);
    expect(compared).toBeGreaterThan(0);
  }, 30_000);

  test("ax_focused_window and ax_modal answer for a process with no windows", async () => {
    const focused = await ax<{ pid: number; window: AxWindowRow | null }>("ax_focused_window", { pid: process.pid });
    expect(focused.pid).toBe(process.pid);
    expect(focused.window).toBeNull();

    const modal = await ax<{ modal: boolean; blockers: AxWindowRow[] }>("ax_modal", { pid: process.pid });
    expect(modal.modal).toBe(false);
    expect(modal.blockers).toEqual([]);
  }, 30_000);

  test("ax_menu reports no menu bar for a faceless process, and key equivalents for a real one", async () => {
    const own = await ax<{ hasMenuBar: boolean; items: unknown[]; total: number }>("ax_menu", { pid: process.pid });
    expect(own.hasMenuBar).toBe(false);
    expect(own.items).toEqual([]);

    const target = await guiTarget();
    if (target === null) return;
    const pid = target.pid;
    interface MenuItem {
      title: string | null;
      role: string | null;
      enabled: boolean | null;
      depth: number;
      path: string[];
      bounds: AxRect | null;
      key: { char: string | null; virtualKey: number | null; mods: string[]; modifiersRaw: number | null } | null;
    }
    const menu = await ax<{ hasMenuBar: boolean; items: MenuItem[]; total: number; truncated: boolean }>("ax_menu", {
      pid,
    });
    expect(menu.hasMenuBar).toBe(true);
    expect(menu.items.length).toBeGreaterThan(0);
    expect(menu.items[0].role).toBe("AXMenuBar");
    const withKeys = menu.items.filter((i) => i.key !== null);
    console.error(
      `[helperProtocol] ax_menu pid=${pid} items=${menu.items.length} withKeyEquivalent=${withKeys.length} truncated=${menu.truncated}`,
    );
    expect(withKeys.length).toBeGreaterThan(0);
    for (const item of withKeys.slice(0, 20)) {
      expect(item.key?.char !== null || item.key?.virtualKey !== null).toBe(true);
      for (const mod of item.key?.mods ?? []) expect(["Command", "Control", "Option", "Shift"]).toContain(mod);
    }
    // `path` is built from titled ancestors, so an untitled `AXMenu` wrapper never appears in it.
    for (const item of menu.items) expect(item.path.every((p) => p.length > 0)).toBe(true);
    // `path` ALREADY ends with the item's own title. The renderer in `native.ts` relies on this;
    // appending the title again produced "File > Save > Save", which `ax_menu_press` then could
    // not resolve. Pinned here against a REAL menu bar, where the fixture cannot be wrong.
    for (const item of menu.items.filter((i) => i.title !== null && i.title !== "")) {
      expect(item.path[item.path.length - 1]).toBe(item.title as string);
    }
  }, 30_000);

  /**
   * The gate that stops an actuation reporting success for nothing.
   *
   * This is a REFUSAL test, which is why it is safe to run against a live application: it asks the
   * helper to press a control it must decline, so nothing on the machine is clicked. A disabled
   * menu item is the exact shape that defeats the action-advertised check — it still offers
   * `AXPress`, and `AXUIElementPerformAction` still answers `.success` while doing nothing.
   */
  test("ax_press refuses a DISABLED control, which still advertises AXPress", async () => {
    const target = await guiTarget();
    if (target === null) return;
    const { pid } = target;
    interface Node { ref: string; title: string | null; enabled: boolean | null; actions: string[] }
    const walk = await ax<{ nodes: Node[] }>("ax_find", { pid, role: "AXMenuItem" });
    const disabled = walk.nodes.find((n) => n.enabled === false && n.actions.includes("AXPress"));
    if (disabled === undefined) {
      console.error("[helperProtocol] no disabled menu item offering AXPress; ax_press gate not exercised.");
      return;
    }
    console.error(
      `[helperProtocol] ax_press gate: "${disabled.title}" enabled=${disabled.enabled} actions=${disabled.actions.join(",")}`,
    );
    const err = await client.request("ax_press", { ref: disabled.ref }).then(
      (ok) => ({ pressed: ok }),
      (e: unknown) => e,
    );
    // It must REFUSE. A resolved promise here means the helper reported a successful press of a
    // control that cannot be pressed — the exact false success this gate exists to prevent.
    expect(isAgentError(err), `ax_press resolved instead of refusing: ${JSON.stringify(err)}`).toBe(true);
    const agentError = err as AgentError;
    console.error(`[helperProtocol] ax_press refused: ${agentError.code} — ${agentError.message}`);
    expect(agentError.message).toContain("disabled");
  }, 30_000);

  /**
   * Two refusals that keep an actuation from visibly disrupting the person at the machine. Both are
   * safe to run live precisely because they refuse: nothing is pressed.
   */
  test("ax_menu_press refuses a bare menu-bar title, which would open the menu on screen", async () => {
    const target = await guiTarget();
    if (target === null) return;
    const err = await client.request("ax_menu_press", { pid: target.pid, path: ["File"] }).then(
      (ok) => ({ pressed: ok }),
      (e: unknown) => e,
    );
    expect(isAgentError(err), `ax_menu_press resolved: ${JSON.stringify(err)}`).toBe(true);
    expect((err as AgentError).message).toContain("menu bar title");
  }, 30_000);

  test("ax_press refuses window chrome unless the caller accepts the disruption", async () => {
    const target = await guiTarget();
    if (target === null) return;
    interface Node { ref: string; subrole: string | null; actions: string[] }
    const walk = await ax<{ nodes: Node[] }>("ax_find", { pid: target.pid, role: "AXButton" });
    const chrome = walk.nodes.find(
      (n) => n.subrole !== null && ["AXFullScreenButton", "AXZoomButton", "AXMinimizeButton", "AXCloseButton"].includes(n.subrole),
    );
    if (chrome === undefined) {
      console.error("[helperProtocol] no window-chrome button found; the disruption gate is not exercised.");
      return;
    }
    console.error(`[helperProtocol] chrome gate: ${chrome.subrole} actions=${chrome.actions.join(",")}`);
    const err = await client.request("ax_press", { ref: chrome.ref }).then(
      (ok) => ({ pressed: ok }),
      (e: unknown) => e,
    );
    // Pressing it would move the user's Space or take the window away. It must decline.
    expect(isAgentError(err), `ax_press resolved on ${chrome.subrole}: ${JSON.stringify(err)}`).toBe(true);
    expect((err as AgentError).message).toContain("desktop");
  }, 30_000);

  test("ax_snapshot refuses a process that has no window to walk", async () => {
    const [code, helperCode] = await axFailure("ax_snapshot", { pid: process.pid });
    expect(code).toBe("WINDOW_NOT_FOUND");
    expect(helperCode).toBe("WINDOW_NOT_FOUND");
  }, 30_000);

  test("snapshot refs are generation-scoped, so an older one is stale and not merely absent", async () => {
    // `ax_find` walks the application element, so it works — and bumps the generation — even for a
    // process with no windows at all. A role that matches nothing keeps the map empty on purpose.
    const first = await ax<AxWalkReply>("ax_find", { pid: process.pid, role: "AXNoSuchRole" });
    const second = await ax<AxWalkReply>("ax_find", { pid: process.pid, role: "AXNoSuchRole" });
    expect(second.generation).toBe(first.generation + 1);
    expect(second.nodes).toEqual([]);

    // A ref from the previous generation is STALE: it cannot alias onto whatever now occupies that
    // slot, which is the entire reason the generation is in the ref.
    const [staleCode, staleHelper] = await axFailure("ax_point", { ref: `@a${first.generation}e1` });
    expect(staleCode).toBe("ELEMENT_STALE");
    expect(staleHelper).toBe("ELEMENT_STALE");

    // An unknown index in the CURRENT generation is a different failure, and says so.
    const [unknownCode, unknownHelper] = await axFailure("ax_point", { ref: `@a${second.generation}e1` });
    expect(unknownCode).toBe("ELEMENT_NOT_FOUND");
    expect(unknownHelper).toBe("ELEMENT_NOT_FOUND");
  }, 30_000);

  test("ax_point returns a clickable point for a live ref, and refuses it after a re-snapshot", async () => {
    const target = await guiTarget();
    if (target === null) return;
    const { pid, windowId } = target;
    // Addressed by CGWindowID, the id `windows` and `screencapture -l` already speak, so the walk
    // is rooted at one named window instead of whichever one happens to be key during the run.
    const snap = await ax<AxWalkReply & { windowSource: string; window: AxWindowRow | null }>("ax_snapshot", {
      pid,
      window: windowId,
      maxNodes: 20,
    });
    expect(snap.windowSource).toBe("requested");
    expect(snap.window?.windowId).toBe(windowId);
    expect(snap.nodes.length).toBeGreaterThan(0);
    expect(snap.nodes[0].ref).toBe(`@a${snap.generation}e1`);
    expect(Array.isArray(snap.nodes[0].actions)).toBe(true);

    interface AxPoint {
      ref: string;
      generation: number;
      bounds: AxRect;
      center: { x: number; y: number };
      display: AxRect;
      frontmost: boolean;
      windowId: number | null;
    }
    // Take the first node that yields a point. A node whose rect straddles a display edge is
    // REFUSED by design, and a refusal is exactly what must not be read as this test failing.
    let node: AxNode | null = null;
    let point: AxPoint | null = null;
    for (const candidate of snap.nodes.slice(0, 5)) {
      try {
        point = await ax<AxPoint>("ax_point", { ref: candidate.ref });
        node = candidate;
        break;
      } catch {
        continue;
      }
    }
    expect(node).not.toBeNull();
    expect(point).not.toBeNull();
    if (node === null || point === null) return;
    expect(point.generation).toBe(snap.generation);
    expect(Number.isFinite(point.center.x)).toBe(true);
    expect(Number.isFinite(point.center.y)).toBe(true);
    // The refusals are the point of the verb: a point that came back is on a real display.
    expect(point.center.x).toBeGreaterThanOrEqual(point.display.x);
    expect(point.center.x).toBeLessThanOrEqual(point.display.x + point.display.w);
    expect(point.center.y).toBeGreaterThanOrEqual(point.display.y);
    expect(point.center.y).toBeLessThanOrEqual(point.display.y + point.display.h);
    expect(typeof point.frontmost).toBe("boolean");
    console.error(
      `[helperProtocol] ax_point ${node.role ?? "?"} center=${point.center.x},${point.center.y} windowId=${point.windowId}`,
    );

    const again = await ax<AxWalkReply>("ax_snapshot", { pid, window: windowId, maxNodes: 20 });
    expect(again.generation).toBe(snap.generation + 1);
    const [code, helperCode] = await axFailure("ax_point", { ref: node.ref });
    expect(code).toBe("ELEMENT_STALE");
    expect(helperCode).toBe("ELEMENT_STALE");
  }, 30_000);

  test("ax_snapshot finds its own window, or refuses rather than picking one of several", async () => {
    const target = await guiTarget();
    if (target === null) return;
    if (target.rooted) {
      const snap = await ax<AxWalkReply & { windowSource: string }>("ax_snapshot", {
        pid: target.pid,
        maxNodes: 1,
      });
      expect(["focused", "main", "only"]).toContain(snap.windowSource);
    } else {
      // Several windows, none of them focused or main: there is no unambiguous answer, so the verb
      // names the ids it can see instead of guessing one. NotificationCenter is a real example.
      const [code, helperCode] = await axFailure("ax_snapshot", { pid: target.pid });
      expect(code).toBe("WINDOW_NOT_FOUND");
      expect(helperCode).toBe("WINDOW_NOT_FOUND");
    }
    console.error(`[helperProtocol] ax_snapshot ladder pid=${target.pid} rooted=${target.rooted}`);
  }, 30_000);

  test("an unknown window id is refused with the ids that do exist", async () => {
    const target = await guiTarget();
    if (target === null) return;
    const [code, helperCode] = await axFailure("ax_snapshot", { pid: target.pid, window: 1 });
    expect(code).toBe("WINDOW_NOT_FOUND");
    expect(helperCode).toBe("WINDOW_NOT_FOUND");
  }, 30_000);

  test("maxNodes caps the walk and the reply says it was truncated", async () => {
    const target = await guiTarget();
    if (target === null) return;
    const { pid, windowId } = target;
    const full = await ax<AxWalkReply>("ax_snapshot", { pid, window: windowId });
    const capped = await ax<AxWalkReply>("ax_snapshot", { pid, window: windowId, maxNodes: 2 });
    expect(capped.nodes.length).toBeLessThanOrEqual(2);
    console.error(`[helperProtocol] ax_snapshot pid=${pid} full=${full.nodes.length} capped=${capped.nodes.length}`);
    if (full.nodes.length > 2) {
      expect(capped.nodes.length).toBe(2);
      expect(capped.truncated).toBe(true);
      expect(capped.stopReason).toBe("maxNodes");
    }
  }, 30_000);

  test("an out-of-range maxNodes is clamped rather than trapping the helper", async () => {
    // `maxNodes` is read BEFORE the window is resolved, so this exercises the conversion even on a
    // process with no window: what matters is that the helper answers at all. A trapping conversion
    // would take the process down instead (the exit-133 failure mode this helper is built around).
    const [code] = await axFailure("ax_snapshot", { pid: process.pid, maxNodes: 1e20 });
    expect(code).toBe("WINDOW_NOT_FOUND");
    expect(Number.isFinite((await input.cursor()).x)).toBe(true);

    const target = await guiTarget();
    if (target === null) return;
    const reply = await ax<AxWalkReply>("ax_snapshot", {
      pid: target.pid,
      window: target.windowId,
      maxNodes: 1e20,
    });
    expect(reply.nodes.length).toBeLessThanOrEqual(5_000);
  }, 30_000);

  test("a malformed AX argument maps to INVALID_TARGET and leaves the helper answering", async () => {
    // A ref that is not a ref at all.
    expect((await axFailure("ax_point", { ref: "not-a-ref" }))[0]).toBe("INVALID_TARGET");
    // A filter of the wrong type is REFUSED, never dropped: silently widening `ax_find` to every
    // element and still answering `ok` is the failure this check exists to prevent.
    expect((await axFailure("ax_find", { pid: process.pid, role: 7 }))[0]).toBe("INVALID_TARGET");
    expect((await axFailure("ax_snapshot", { pid: process.pid, root: 7 }))[0]).toBe("INVALID_TARGET");
    expect(Number.isFinite((await input.cursor()).x)).toBe(true);
  }, 30_000);

  test("an out-of-range pid is rejected and a dead one is reported as WINDOW_NOT_FOUND", async () => {
    // Same refusal `windows` already makes: the conversion is range-checked, never trapped.
    expect((await axFailure("ax_windows", { pid: 1e10 }))[0]).toBe("INVALID_TARGET");
    const [code, helperCode] = await axFailure("ax_windows", { pid: 999_999 });
    expect(code).toBe("WINDOW_NOT_FOUND");
    expect(helperCode).toBe("WINDOW_NOT_FOUND");
    expect(Number.isFinite((await input.cursor()).x)).toBe(true);
  }, 30_000);
});
