/**
 * `mode:"webview"` — the lane that touches nothing outside the page, and therefore the only
 * pointer/keyboard lane an `interaction:"background"` session has.
 *
 * It reaches the page through the WebDriver bridge and dispatches the event sequence itself.
 * It deliberately does NOT use the embedded driver's W3C `/actions` endpoint: that endpoint is
 * implemented by injecting `new MouseEvent(...)` (tauri-plugin-wdio-webdriver
 * `executor.rs:1391`), and OneCAD's viewport, picker and sketch controller all listen for
 * POINTER events, so a MouseEvent reaches none of them. What is dispatched here is a
 * `PointerEvent` sequence with the right `buttons` mask, which does.
 *
 * What it still cannot do, and refuses rather than fakes:
 *
 * - **Press-and-hold anything.** `CadOrbitControls.onPointerDown` calls
 *   `el.setPointerCapture(e.pointerId)` with no guard, and a synthetic pointer id has no active
 *   pointer to capture, so that call throws and the handler aborts before it records the drag.
 *   Orbit, pan and every sketch drag would look delivered and do nothing. `pointer_down`,
 *   `pointer_up`, `pointer_drag` and `pointer_drag_path` therefore refuse.
 * - **Reach anything the WebView does not own** — a native menu, a Save panel, the traffic
 *   lights, another window. Use the accessibility tools for those.
 *
 * Every result is labelled `mode:"webview"`, `backend:"webdriver"`, so nobody can mistake it for
 * user-grade evidence. Wheel is the one viewport gesture that survives, because zooming needs no
 * pointer capture: `CadOrbitControls` binds `wheel` on the canvas with `passive:false` and a
 * dispatched WheelEvent bubbles to it.
 */
import { AgentError } from "../../errors.ts";
import { backgroundRefusal } from "../../platform/refusing.ts";
import type { Pt } from "../../geometry/types.ts";
import type { SessionBridge } from "../../session/types.ts";
import type { ModInput, TargetInput } from "../schemas.ts";

/** Every verb the in-page lane implements; named in both refusals below. */
export const WEBVIEW_LANE_VERBS = [
  "pointer_move",
  "pointer_hover",
  "pointer_click",
  "pointer_scroll",
  "keyboard_press",
  "keyboard_shortcut",
  "keyboard_down",
  "keyboard_up",
  "keyboard_type_text",
] as const;

export function webviewUnsupported(tool: string): AgentError {
  return new AgentError(
    "INVALID_TARGET",
    `${tool} is not supported in webview mode`,
    {
      remediation:
        'The press-and-hold verbs cannot be faked in the page: OneCAD captures the pointer on pointerdown, and a synthetic pointer id cannot be captured, so the gesture would report delivered and do nothing. Use mode:"real_user" (the default) — it posts real OS events, which is also the only acceptable evidence.',
      details: { tool, supportedInWebviewMode: [...WEBVIEW_LANE_VERBS] },
    },
  );
}

interface DispatchArgs {
  ref: string | null;
  selector: string | null;
  point: Pt;
  kind: "move" | "click";
  button: number;
  mods: ModInput[];
}

/** Dispatches in the page and reports which element actually received the events. */
export function dispatchPointer(bridge: SessionBridge, a: DispatchArgs): Promise<{ ok: boolean; tag: string }> {
  const flags = modifierFlags(a.mods);
  return bridge.execute<{ ok: boolean; tag: string }>(
    "webview.pointer",
    ((
      ref: string | null,
      sel: string | null,
      x: number,
      y: number,
      kind: "move" | "click",
      button: number,
      f: { metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean },
    ) => {
      const refs = (window as unknown as { __tauriAgentRefs?: Map<string, Element> }).__tauriAgentRefs;
      let el: Element | null = ref && refs ? refs.get(ref) ?? null : null;
      if ((el === null || !el.isConnected) && sel) el = document.querySelector(sel);
      const target = el ?? document.elementFromPoint(x, y);
      if (target === null) return { ok: false, tag: "" };
      const init: PointerEventInit = {
        bubbles: true,
        cancelable: true,
        composed: true,
        clientX: x,
        clientY: y,
        button,
        buttons: button === 0 ? 1 : button === 2 ? 2 : 4,
        pointerId: 1,
        pointerType: "mouse",
        isPrimary: true,
        ...f,
      };
      const types =
        kind === "move"
          ? ["pointerover", "pointerenter", "pointermove", "mouseover", "mousemove"]
          : ["pointerdown", "mousedown", "pointerup", "mouseup", "click"];
      for (const type of types) target.dispatchEvent(new PointerEvent(type, init));
      return { ok: true, tag: target.tagName.toLowerCase() };
    }) as never,
    [a.ref, a.selector, a.point.x, a.point.y, a.kind, a.button, flags],
    { readOnly: false },
  );
}

/** `insertText` so the app sees real `beforeinput`/`input` events, not a silent value assignment. */
export function dispatchText(bridge: SessionBridge, text: string): Promise<{ ok: boolean; tag: string }> {
  return bridge.execute<{ ok: boolean; tag: string }>(
    "webview.type",
    ((value: string) => {
      const el = document.activeElement as HTMLElement | null;
      if (el === null) return { ok: false, tag: "" };
      const ok = document.execCommand("insertText", false, value);
      return { ok, tag: el.tagName.toLowerCase() };
    }) as never,
    [text],
    { readOnly: false },
  );
}

/** The verbs whose input cannot be dispatched in the page at all; see the module comment. */
const HOLD_VERBS = ["pointer_down", "pointer_up", "pointer_drag", "pointer_drag_path"] as const;

/**
 * Which lane a verb runs in, given the session's policy and what the caller asked for.
 *
 * Foreground is unchanged: the caller's `mode` decides, defaulting to `real_user`. Background
 * flips the default to the in-page lane and refuses an explicit `real_user` outright — refusing
 * is the whole point, since silently downgrading it would report page-dispatched events as
 * user-grade evidence.
 */
export function laneFor(
  policy: "foreground" | "background",
  requested: "real_user" | "webview" | undefined,
  tool: string,
): "real_user" | "webview" {
  if (policy === "foreground") return requested ?? "real_user";
  if (requested === "real_user") {
    throw backgroundRefusal(`${tool} with mode:"real_user"`, { tool, requestedMode: "real_user" });
  }
  if ((HOLD_VERBS as readonly string[]).includes(tool)) {
    throw backgroundRefusal(tool, {
      tool,
      why: "OneCAD captures the pointer on pointerdown and a synthetic pointer id cannot be captured, so this gesture would report delivered and do nothing",
    });
  }
  return "webview";
}

export function selectorOf(target: TargetInput, nodeCss: string | undefined): string | null {
  if (nodeCss !== undefined) return nodeCss;
  if ("css" in target) return target.css;
  if ("testId" in target) return `[data-testid="${target.testId}"]`;
  return null;
}

/**
 * Dispatches a wheel notch in the page.
 *
 * `deltaMode` is stated explicitly (0 = pixels, 1 = lines) rather than derived from a calibration
 * probe, because there is nothing to calibrate: no notch is posted to the OS, so the app's
 * device classifier is reading a value this lane chose. `wheelLinesPerNotch` is deliberately not
 * consulted, and the envelope says `mode:"webview"` so the difference is visible.
 */
export function dispatchWheel(
  bridge: SessionBridge,
  a: { point: Pt; deltaX: number; deltaY: number; deltaMode: 0 | 1; mods: ModInput[] },
): Promise<{ ok: boolean; tag: string; defaultPrevented: boolean }> {
  const flags = modifierFlags(a.mods);
  return bridge.execute<{ ok: boolean; tag: string; defaultPrevented: boolean }>(
    "webview.wheel",
    ((
      x: number,
      y: number,
      deltaX: number,
      deltaY: number,
      deltaMode: number,
      f: { metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean },
    ) => {
      const target = document.elementFromPoint(x, y);
      if (target === null) return { ok: false, tag: "", defaultPrevented: false };
      const ev = new WheelEvent("wheel", {
        bubbles: true,
        cancelable: true,
        composed: true,
        clientX: x,
        clientY: y,
        deltaX,
        deltaY,
        deltaMode,
        ...f,
      });
      // The return value of dispatchEvent is false exactly when a listener called
      // preventDefault, which is how a caller can tell the app CONSUMED the notch from the
      // case where it bubbled to the document and did nothing.
      const notCancelled = target.dispatchEvent(ev);
      return { ok: true, tag: target.tagName.toLowerCase(), defaultPrevented: !notCancelled };
    }) as never,
    [a.point.x, a.point.y, a.deltaX, a.deltaY, a.deltaMode, flags],
    { readOnly: false },
  );
}

/**
 * Dispatches a keydown (and optionally the matching keyup) in the page.
 *
 * It goes to `document.activeElement` so the event bubbles the way a real keystroke would,
 * which is what puts it in front of OneCAD's own listeners — `useShortcuts` binds `keydown` on
 * `window`, and both controllers bind it on `window` in the CAPTURE phase.
 *
 * What it does not do is reach the native menu bar. A real ⌘S on macOS is claimed by the
 * `NSMenu` accelerator before the WebView ever sees it; this dispatch exercises the app's
 * JavaScript keydown lane instead. For Save the two converge on the same action, but they are
 * different code and a report must not present one as the other.
 */
export function dispatchKey(
  bridge: SessionBridge,
  a: { key: string; mods: ModInput[]; phase: "down" | "up" | "press"; repeat: number },
): Promise<{ ok: boolean; tag: string; defaultPrevented: boolean }> {
  const flags = modifierFlags(a.mods);
  return bridge.execute<{ ok: boolean; tag: string; defaultPrevented: boolean }>(
    "webview.key",
    ((
      key: string,
      phase: "down" | "up" | "press",
      repeat: number,
      f: { metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean },
    ) => {
      // `code` is the PHYSICAL key name a US/ANSI keyboard would report for this character.
      // A single printable character maps to KeyA..KeyZ / Digit0..Digit9; everything else is
      // its own name ("Enter", "Escape", "ArrowUp", "F1"), which is already the code.
      const codeOf = (k: string): string => {
        if (k.length === 1) {
          const c = k.toUpperCase();
          if (c >= "A" && c <= "Z") return `Key${c}`;
          if (c >= "0" && c <= "9") return `Digit${c}`;
          if (c === " ") return "Space";
        }
        return k === "Space" ? "Space" : k;
      };
      const named = key === "Space" ? " " : key;
      const el: Element = document.activeElement ?? document.body;
      const init: KeyboardEventInit = {
        bubbles: true,
        cancelable: true,
        composed: true,
        key: named,
        code: codeOf(key),
        ...f,
      };
      let prevented = false;
      const fire = (type: "keydown" | "keyup"): void => {
        const ev = new KeyboardEvent(type, init);
        if (!el.dispatchEvent(ev)) prevented = true;
      };
      for (let i = 0; i < repeat; i += 1) {
        if (phase === "down" || phase === "press") fire("keydown");
        if (phase === "up" || phase === "press") fire("keyup");
      }
      return { ok: true, tag: el.tagName.toLowerCase(), defaultPrevented: prevented };
    }) as never,
    [a.key, a.phase, a.repeat, flags],
    { readOnly: false },
  );
}

function modifierFlags(mods: ModInput[]): {
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
} {
  return {
    metaKey: mods.includes("Primary") || mods.includes("Command"),
    ctrlKey: mods.includes("Control"),
    altKey: mods.includes("Option"),
    shiftKey: mods.includes("Shift"),
  };
}
