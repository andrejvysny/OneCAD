/**
 * `mode:"webview"` — the low-fidelity lane, kept only for the three verbs the spec allows
 * (move, click, type) and always labelled `backend:"webdriver"` so nobody can mistake it
 * for user-grade evidence.
 *
 * It reaches the page through the WebDriver bridge and dispatches the event sequence at the
 * resolved point. What it CANNOT do is drag, wheel, hold a chord, or reach anything the
 * webview does not own (native menus, the traffic lights, another window) — which is exactly
 * why every other verb refuses rather than faking a result.
 */
import { AgentError } from "../../errors.ts";
import type { Pt } from "../../geometry/types.ts";
import type { SessionBridge } from "../../session/types.ts";
import type { ModInput, TargetInput } from "../schemas.ts";

export function webviewUnsupported(tool: string): AgentError {
  return new AgentError(
    "INVALID_TARGET",
    `${tool} is not supported in webview mode`,
    {
      remediation:
        'Only pointer_move, pointer_click and keyboard_type_text have a webview lane. Use mode:"real_user" (the default) for everything else — it posts real OS events, which is also the only acceptable evidence.',
      details: { tool, supportedInWebviewMode: ["pointer_move", "pointer_click", "keyboard_type_text"] },
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
  const flags = {
    metaKey: a.mods.includes("Primary") || a.mods.includes("Command"),
    ctrlKey: a.mods.includes("Control"),
    altKey: a.mods.includes("Option"),
    shiftKey: a.mods.includes("Shift"),
  };
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

export function selectorOf(target: TargetInput, nodeCss: string | undefined): string | null {
  if (nodeCss !== undefined) return nodeCss;
  if ("css" in target) return target.css;
  if ("testId" in target) return `[data-testid="${target.testId}"]`;
  return null;
}
