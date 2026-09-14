/**
 * The in-page half of `resolve(target, { forInput | forDrag })`.
 *
 * Both functions are serialized with `Function.prototype.toString` by webdriverio
 * and evaluated in the webview, so they MUST NOT close over anything in this
 * module: every helper lives inside its own body. Only the types below cross the
 * boundary, and types are erased before serialization.
 */

export interface CheckRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface InputCheck {
  found: boolean;
  fp: string | null;
  fpAvailable: boolean;
  rect0: CheckRect | null;
  rect1: CheckRect | null;
  dragRegion: boolean;
  hitIsSelf: boolean;
  occluder: { tag: string; testId: string | null; role: string | null } | null;
}

interface CheckWindow extends Window {
  __tauriAgentRefs?: Map<string, Element>;
  __tauriAgentFp?: (el: Element) => string;
}

/**
 * Identity, two rect samples 50 ms apart, drag-region, and a hit test at the exact
 * point that will be clicked (offset included).
 *
 * `cssSel` is a FALLBACK, and the caller decides whether one exists at all: a ref
 * is passed with `cssSel === null`, because re-finding a detached element by a
 * positional path silently addresses whichever sibling slid into its place.
 */
export async function checkInPage(
  ref: string | null,
  cssSel: string | null,
  offX: number,
  offY: number,
): Promise<InputCheck> {
  const w = window as unknown as CheckWindow;
  const refs = w.__tauriAgentRefs;
  let el: Element | null = ref && refs ? refs.get(ref) ?? null : null;
  if (el === null && cssSel) el = document.querySelector(cssSel);
  const fpFn = w.__tauriAgentFp;
  const fpAvailable = typeof fpFn === "function";
  const missing: InputCheck = {
    found: false, fp: null, fpAvailable, rect0: null, rect1: null,
    dragRegion: false, hitIsSelf: false, occluder: null,
  };
  if (el === null || !el.isConnected) return missing;
  const fp = fpAvailable && fpFn ? fpFn(el) : null;
  const a = el.getBoundingClientRect();
  const rect0 = { x: a.left, y: a.top, width: a.width, height: a.height };
  await new Promise((res) => setTimeout(res, 50));
  if (!el.isConnected) return { ...missing, fp };
  const b = el.getBoundingClientRect();
  const rect1 = { x: b.left, y: b.top, width: b.width, height: b.height };
  const hit = document.elementFromPoint(
    rect1.x + rect1.width / 2 + offX,
    rect1.y + rect1.height / 2 + offY,
  );
  // An icon span is routinely `pointer-events: none`: the hit is then the ANCESTOR
  // that receives the click on its behalf, not an occluder.
  const selfHit =
    hit !== null &&
    (hit === el ||
      el.contains(hit) ||
      (hit.contains(el) && getComputedStyle(el).pointerEvents === "none"));
  return {
    found: true,
    fp,
    fpAvailable,
    rect0,
    rect1,
    dragRegion: el.closest("[data-tauri-drag-region]") !== null,
    hitIsSelf: selfHit,
    occluder:
      hit && !selfHit
        ? { tag: hit.tagName.toLowerCase(), testId: hit.getAttribute("data-testid"), role: hit.getAttribute("role") }
        : null,
  };
}

/** A raw point has no element to check, so the drag-region gate is asked directly. */
export function dragRegionAtPoint(x: number, y: number): boolean {
  const el = document.elementFromPoint(x, y);
  return el !== null && el.closest("[data-tauri-drag-region]") !== null;
}
