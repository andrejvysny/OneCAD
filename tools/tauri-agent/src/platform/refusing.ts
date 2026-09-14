/**
 * The background policy's teeth.
 *
 * "A background session must never move the cursor or steal focus" is worthless as a guard
 * scattered through call sites: the next verb anyone adds forgets it, and the failure mode is
 * the harness grabbing the pointer out of the user's hand halfway through what was advertised
 * as a silent test. So the rule is enforced at the SEAM instead. A background session is handed
 * a `PlatformAdapter` whose input verbs and whose `focus` do not work — not "are not called",
 * do not work — and no code path, however new or however buggy, can post a `CGEvent` or activate
 * the application through it.
 *
 * What still passes through, because none of it disturbs anything: `permissions` and `layout`
 * (capability reads), `cursor` (reads the pointer, never moves it), `dispose` (teardown must
 * still work), `windows.list` / `windows.isFrontmost` (CGWindowList and NSWorkspace reads), and
 * every accessibility READ. Screen capture is untouched — `screencapture -l <windowId>`
 * photographs a specific window whether or not it is in front.
 *
 * `releaseAll` is the one deliberate exception among the input verbs. It releases only what the
 * helper itself pressed (`osState:false`), it is the crash-recovery path, and refusing it would
 * mean a session that changed policy mid-life could never hand back a key the helper was holding.
 * It can only ever reduce held state, never create any.
 */
import { AgentError } from "../errors.ts";
import type { NativeInput, NativeWindows, PlatformAdapter } from "./adapter.ts";

/** Verbs the background lane can offer instead, named in every refusal. */
const SUPPORTED_IN_BACKGROUND = [
  "pointer_move",
  "pointer_hover",
  "pointer_click",
  "pointer_scroll",
  "keyboard_press",
  "keyboard_shortcut",
  "keyboard_down",
  "keyboard_up",
  "keyboard_type_text",
  "native_press",
  "native_set_value",
  "native_menu_invoke",
] as const;

export function backgroundRefusal(what: string, detail?: Record<string, unknown>): AgentError {
  return new AgentError(
    "BACKGROUND_CAPABILITY_UNAVAILABLE",
    `${what} needs native input, which an interaction:"background" session does not have`,
    { details: { what, supportedInBackground: [...SUPPORTED_IN_BACKGROUND], ...detail } },
  );
}

/**
 * Every verb that posts an OS event refuses. `permissions`, `layout`, `cursor`, `dispose` and
 * `releaseAll` pass through — see the module comment for why each is safe.
 */
export function refusingInput(real: NativeInput): NativeInput {
  const no =
    (verb: string) =>
    async (): Promise<never> => {
      throw backgroundRefusal(`native input (${verb})`);
    };
  return {
    move: no("move"),
    down: no("down"),
    up: no("up"),
    click: no("click"),
    path: no("path"),
    scroll: no("scroll"),
    keyDown: no("keydown"),
    keyUp: no("keyup"),
    press: no("press"),
    type: no("type"),
    releaseAll: () => real.releaseAll(),
    cursor: () => real.cursor(),
    permissions: (opts) => real.permissions(opts),
    ...(real.layout === undefined ? {} : { layout: (): ReturnType<NonNullable<NativeInput["layout"]>> => real.layout!() }),
    dispose: () => real.dispose(),
  };
}

/** `focus` refuses; the two read verbs pass through. */
export function refusingFocus(real: NativeWindows): NativeWindows {
  return {
    list: (pid, opts) => real.list(pid, opts),
    isFrontmost: (pid) => real.isFrontmost(pid),
    focus: async (): Promise<never> => {
      throw backgroundRefusal("activating the application (windows.focus)");
    },
  };
}

/**
 * Wraps a real adapter for background use. `capture` and `ax` are passed through by reference:
 * capture never needs the window in front, and accessibility is read-only except for the three
 * actuation verbs, which are the background lane's whole point.
 */
export function backgroundAdapter(real: PlatformAdapter): PlatformAdapter {
  return {
    input: refusingInput(real.input),
    windows: refusingFocus(real.windows),
    capture: real.capture,
    ax: real.ax,
    name: real.name,
  };
}
