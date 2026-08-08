/*
 * View navigation moved from `onecad.modeling` to `onecad.shell`. The failure to
 * guard against is not "the command is missing" — it is a QUIET behavior change:
 * implementing the service over the engine's `fitView()` (the obvious reading of
 * "zoom to fit") turns ⇧F from *frame the selection* into *frame everything*,
 * and nothing in the type system or the keymap contract notices.
 *
 * So every case below asserts the SELECTION-AWARE behavior, and asserts it
 * through BOTH entry points — the registered command and the keystroke — because
 * the point of the wave is that there is one implementation, not two.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPlatform, type Platform } from "@/platform";
import { resetStores } from "@/test/resetStores";
import { selectionStore } from "@/stores/selectionStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import { runAction } from "@/shortcuts/useShortcuts";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { registerShellModule } from "./module";
import { ShellCommands, ShellServices } from "./ids";
import type { ViewportNavigation } from "./viewportNavigation";

function fakeEngine() {
  return {
    fitToBodies: vi.fn(),
    fitView: vi.fn(),
    homeView: vi.fn(),
  };
}

function bootShell(): Platform {
  const platform = createPlatform();
  registerShellModule(platform);
  platform.initializeSync();
  return platform;
}

/** Run the registered command, i.e. the palette/automation entry point. */
function runCommand(platform: Platform, id: (typeof ShellCommands)[keyof typeof ShellCommands]) {
  const cmd = platform.commands.get(id);
  expect(cmd, `command ${id} is registered`).toBeDefined();
  cmd?.execute({ selection: [], scopes: [] });
}

describe("shell view navigation", () => {
  let engine: ReturnType<typeof fakeEngine>;

  beforeEach(() => {
    resetStores();
    engine = fakeEngine();
    setViewportEngine(engine as unknown as ViewportEngine);
  });

  it("frames the SELECTED body, not everything", () => {
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    runAction({ type: "zoomFit" });
    expect(engine.fitToBodies).toHaveBeenCalledWith(["body1"]);
    expect(engine.fitView).not.toHaveBeenCalled();
  });

  it("frames every selected body when several are picked", () => {
    selectionStore.getState().set([
      { kind: "body", id: "body1" },
      { kind: "body", id: "body2" },
    ]);
    runAction({ type: "zoomFit" });
    expect(engine.fitToBodies).toHaveBeenCalledWith(["body1", "body2"]);
  });

  it("falls back to fit-all for a selection that names no body", () => {
    // A sketch/datum selection yields no body ids — that is a fit-ALL, not a
    // fit-nothing (viewportStore.zoomFit's own comment).
    selectionStore.getState().set([{ kind: "sketch", id: "sketch2" }]);
    runAction({ type: "zoomFit" });
    expect(engine.fitView).toHaveBeenCalled();
    expect(engine.fitToBodies).not.toHaveBeenCalled();
  });

  it("falls back to fit-all with an empty selection", () => {
    runAction({ type: "zoomFit" });
    expect(engine.fitView).toHaveBeenCalled();
  });

  it("is a no-op before the engine mounts, rather than a throw", () => {
    setViewportEngine(null);
    expect(() => runAction({ type: "zoomFit" })).not.toThrow();
    expect(() => runAction({ type: "home" })).not.toThrow();
  });

  it("the COMMAND and the KEY are the same call", () => {
    const platform = bootShell();
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);

    runCommand(platform, ShellCommands.zoomFit);
    runAction({ type: "zoomFit" });
    expect(engine.fitToBodies).toHaveBeenNthCalledWith(1, ["body1"]);
    expect(engine.fitToBodies).toHaveBeenNthCalledWith(2, ["body1"]);

    runCommand(platform, ShellCommands.home);
    runAction({ type: "home" });
    expect(engine.homeView).toHaveBeenCalledTimes(2);
  });

  it("publishes the navigation service, and it is the same implementation again", () => {
    const platform = bootShell();
    const nav = platform.services.require<ViewportNavigation>(ShellServices.ViewportNavigation);
    selectionStore.getState().set([{ kind: "body", id: "body7" }]);
    nav.zoomFit();
    expect(engine.fitToBodies).toHaveBeenCalledWith(["body7"]);
  });

  it("carries the shipped chords, so the keymap contract keeps its owner", () => {
    const platform = bootShell();
    expect(platform.commands.get(ShellCommands.home)?.defaultShortcut).toEqual({ key: "h" });
    expect(platform.commands.get(ShellCommands.zoomFit)?.defaultShortcut).toEqual({
      key: "f",
      shift: true,
    });
  });
});
