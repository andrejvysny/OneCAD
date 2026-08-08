/*
 * View navigation — the shell's, not modeling's.
 *
 * `zoomFit` and `home` were registered as `onecad.modeling.command.*` because
 * that is where their key bindings happened to live. They are host concerns:
 * docs/ARCHITECTURE.md §7 puts camera and navigation with the host, and a future
 * non-modeling workspace still needs to frame its viewport.
 *
 * THERE IS EXACTLY ONE IMPLEMENTATION. Both the command and the keystroke call
 * the functions below, and the functions delegate to `viewportStore`, which is
 * where the real behavior lives — notably that `zoomFit` frames the SELECTED
 * bodies and only falls back to fit-all when the selection names none. Calling
 * the engine's `fitView()` from here instead would have quietly turned ⇧F into
 * fit-all, which is the kind of regression that survives a green test run.
 *
 * Both are no-ops before the engine mounts (the store guards on the bridge), so
 * a command palette invocation on the start screen does nothing rather than
 * throwing.
 */
import { viewportStore } from "@/stores/viewportStore";

export interface ViewportNavigation {
  /** Frame the selected bodies, or everything when the selection names none. */
  zoomFit(): void;
  /** Restore the framed isometric home view. */
  home(): void;
}

export const viewportNavigation: ViewportNavigation = {
  zoomFit: () => viewportStore.getState().zoomFit(),
  home: () => viewportStore.getState().homeView(),
};
