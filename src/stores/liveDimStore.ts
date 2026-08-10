/*
 * Live-dimension chip store (SP-1 Wave 3) — the React bridge between the
 * imperative SketchController and the chips it draws next to the cursor.
 *
 * SPLIT SUBSCRIPTION, deliberately: `fields` / `focus` / `text` / `placement`
 * ARE subscribed by React (they change what a chip READS), while `anchors` are
 * read through `getState()` ONLY. Anchors move at rAF frequency during a draw
 * gesture, and re-rendering a text input on every pointer move to reposition it
 * would fight the user's own typing; the engine's overlay driver moves the chip
 * hosts imperatively instead (`ViewportEngine.moveChip`), exactly like the
 * constraint badges.
 *
 * `update()` therefore has two jobs beyond writing state: skip a set whose
 * numbers did not actually move (float noise below 1e-9 is not a new value), and
 * NEVER overwrite the FOCUSED field — a value that keeps re-deriving under a
 * half-typed number is the bug this whole store exists to avoid.
 *
 * Handlers are INJECTED by the controller (`show`), never implemented here: the
 * chip input FSM (`liveDimStep`) is controller-owned, so this store stays a dumb
 * mailbox and cannot drift from it.
 */
import { createStore, useStore } from "zustand";
import type { DimDomain, DimFieldId } from "@/tools/sketch/liveDimension";
import type { Vec3 } from "@/tools/preview/depthProjection";

/** One chip's renderable facts — `ToolDimension` minus the plane anchor (which
 *  the controller has already converted to world coords for the overlay). */
export interface LiveDimChipField {
  field: DimFieldId;
  label: string;
  domain: DimDomain;
  /** mm · DEGREES · integer count, per `domain`. */
  value: number;
  locked: boolean;
  /** False ⇒ the number drives geometry but authors NO constraint (honest rule). */
  drives: boolean;
}

/** World-space chip anchors, keyed by field. Read via `getState()` only. */
export type LiveDimAnchors = Partial<Record<DimFieldId, Vec3>>;

/**
 * Overlay id for a field's chip host. Lives HERE rather than beside the
 * component so the controller — which calls `moveChip` with it on every pointer
 * move — never has to import a React module.
 */
export const liveDimChipId = (field: DimFieldId): string => `__live_dim_${field}`;

/** Overlay cluster id shared by every host of one chip set. The driver keeps
 *  the hosts a constant screen distance apart, so chips never overlap no matter
 *  how close their world anchors project (a short line's length + angle). */
export const LIVE_DIM_CLUSTER_ID = "__live_dim_cluster";

/** Which quadrant of the cursor the chips sit in (screen-edge flip). */
export type LiveDimPlacement = "tr" | "tl" | "br" | "bl";

/**
 * What a chip does with a key. All controller-owned — the store just relays.
 * `value` is already parsed + validated by the field (mm · degrees · count), and
 * `null` means "nothing lockable was typed", which the FSM turns into a focus
 * move without a lock.
 */
export interface LiveDimHandlers {
  onFocus(field: DimFieldId): void;
  onText(text: string): void;
  onTab(back: boolean, value: number | null): void;
  onEnter(value: number | null): void;
  onEscape(): void;
  /**
   * Focus lost to the viewport: lock the value, do NOT commit the gesture.
   * Carries the FIELD that fired it because a Tab between two chips makes the
   * departed field's DOM blur fire AFTER the FSM has already moved on — the
   * controller must drop that stale blur (field ≠ FSM focus) or it resets the
   * FSM's focus to null one tick after landing on the next field, swallowing
   * every subsequent keystroke (found by e2e, invisible to handler-level tests).
   */
  onBlur(field: DimFieldId, value: number | null): void;
}

/**
 * The fraction of the viewport past which the chips flip to the other side of
 * the cursor. One threshold both ways (the vertical test uses its complement),
 * so a chip cluster never runs off the right or the top edge.
 */
const EDGE_MARGIN = 0.7;

/**
 * Which quadrant of the cursor at (`x`, `y`) has room in a `w`×`h` viewport.
 *
 * Pure so the flip is testable without a camera: the default is up-and-right
 * (`"tr"`, the constraint-badge convention), horizontal flips near the RIGHT
 * edge and vertical near the TOP — those are the two directions the default
 * placement pushes the chip toward. A zero-sized viewport keeps the default
 * rather than guessing.
 */
export function placementFor(x: number, y: number, w: number, h: number): LiveDimPlacement {
  const left = w > 0 && x > EDGE_MARGIN * w;
  const below = h > 0 && y < (1 - EDGE_MARGIN) * h;
  return `${below ? "b" : "t"}${left ? "l" : "r"}` as LiveDimPlacement;
}

export interface LiveDimStoreState {
  open: boolean;
  fields: LiveDimChipField[];
  focus: DimFieldId | null;
  /** Raw text of the FOCUSED field — the single source while editing. */
  text: string;
  anchors: LiveDimAnchors;
  placement: LiveDimPlacement;
  handlers: LiveDimHandlers | null;

  /** Open the chip set for a fresh gesture phase (drops any prior focus). */
  show(fields: LiveDimChipField[], anchors: LiveDimAnchors, handlers: LiveDimHandlers): void;
  /** Per-pointer-move refresh. No-ops when nothing moved; keeps the focused field. */
  update(fields: LiveDimChipField[], anchors: LiveDimAnchors): void;
  setFocus(field: DimFieldId | null, text: string): void;
  setText(text: string): void;
  setPlacement(placement: LiveDimPlacement): void;
  clear(): void;
}

const CLEARED = {
  open: false,
  fields: [] as LiveDimChipField[],
  focus: null as DimFieldId | null,
  text: "",
  anchors: {} as LiveDimAnchors,
  placement: "tr" as LiveDimPlacement,
  handlers: null as LiveDimHandlers | null,
};

/** Below this, two numbers are the same number — pointer jitter, not an edit. */
const EPS = 1e-9;

/** Keep the FOCUSED field's descriptor verbatim: the user is typing into it, and
 *  a re-measured value underneath a half-typed number is a moving target. */
function mergeFocused(
  prev: LiveDimChipField[],
  next: LiveDimChipField[],
  focus: DimFieldId | null,
): LiveDimChipField[] {
  if (focus === null) return next;
  const held = prev.find((f) => f.field === focus);
  if (!held) return next;
  return next.map((f) => (f.field === focus ? held : f));
}

function sameFields(a: LiveDimChipField[], b: LiveDimChipField[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((f, i) => {
    const g = b[i];
    return (
      f.field === g.field &&
      f.label === g.label &&
      f.locked === g.locked &&
      f.drives === g.drives &&
      Math.abs(f.value - g.value) < EPS
    );
  });
}

function sameAnchors(a: LiveDimAnchors, b: LiveDimAnchors, fields: LiveDimChipField[]): boolean {
  return fields.every((f) => {
    const pa = a[f.field];
    const pb = b[f.field];
    if (!pa || !pb) return pa === pb;
    return pa.every((n, i) => Math.abs(n - pb[i]) < EPS);
  });
}

export const liveDimStore = createStore<LiveDimStoreState>()((set, get) => ({
  ...CLEARED,

  show(fields, anchors, handlers) {
    set({ ...CLEARED, open: true, fields, anchors, handlers, placement: get().placement });
  },
  update(fields, anchors) {
    const s = get();
    // A closed set is never resurrected here — only `show` opens one, so a late
    // move frame after `clear()` cannot bring the chips back.
    if (!s.open) return;
    const merged = mergeFocused(s.fields, fields, s.focus);
    if (sameFields(s.fields, merged) && sameAnchors(s.anchors, anchors, merged)) return;
    set({ fields: merged, anchors });
  },
  setFocus(focus, text) {
    set({ focus, text });
  },
  setText(text) {
    set({ text });
  },
  setPlacement(placement) {
    set({ placement });
  },
  clear() {
    set({ ...CLEARED });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useLiveDimStore<T>(selector: (s: LiveDimStoreState) => T): T {
  return useStore(liveDimStore, selector);
}
