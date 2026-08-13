/*
 * attachmentPicker — clicking a face/edge in the viewport to author an
 * attachment (WP-F1.2, spec §7/§9). The authoring counterpart of
 * `placementController`: that one CONSUMES attachments, this one produces them.
 *
 * SAME GESTURE PATTERN, deliberately copied rather than shared: capture-phase
 * `pointerdown`/`keydown` on `window` while armed, plus
 * `engine.setOrbitSuppressed(true)`. Capture-phase + `stopPropagation` shuts out
 * the canvas's own bubble-phase orbit/selection listeners without touching them
 * or `ViewportRoot.tsx` (ADR-0003: viewport affordances only through existing
 * engine surfaces — nothing in the shell or the viewport knows this exists).
 * Sharing one controller between placing and authoring would couple two
 * gestures whose only real commonality is "reads a pick".
 *
 * Events landing INSIDE the dialog are let through untouched — the dialog stays
 * open during a pick (the author watches the rows appear), so swallowing its
 * own clicks would trap them in pick mode.
 *
 * FRAMES ARE WORLD COORDINATES, and that is correct: `save_as_component` bakes
 * the body through `ExportGeometry`, which writes the session's own
 * coordinates verbatim — no re-origining — so a placed component's local space
 * IS the authoring document's world space. A frame captured here therefore
 * means the same thing after the bake.
 */
import { getViewportEngine } from "@/viewport/engineBridge";
import type { PickHit } from "@/viewport/engine/Picker";
import type { ClassifyResult, LibraryAttachmentFrame } from "@/ipc/types";
import { authoringServices } from "./authoringController";

/** The geometry an authored attachment sits on. Drives `accepts` + the default name. */
export type AttachmentKind = "plane" | "cylinder" | "circularEdge";

/**
 * What one pick resolves to. `accepts` mirrors `placementSolver`'s snap table
 * (`ACCEPTS_FOR_SNAP_KIND`) from the far side: an attachment authored on a
 * plane must be admitted by the `coincident` row, one on a cylinder by
 * `concentric`, one on a circular edge by `concentricAndCoincident`.
 */
export interface PickedAttachment {
  kind: AttachmentKind;
  accepts: string[];
  frame: LibraryAttachmentFrame;
}

/** `on` is `"<prefix>:<name>"`, matching the seeded packages' own convention. */
export const ON_PREFIX: Record<AttachmentKind, string> = {
  plane: "face",
  cylinder: "cylinder",
  circularEdge: "edge",
};

/** The name a fresh row of each kind starts with (de-duplicated by the dialog). */
export const DEFAULT_NAME: Record<AttachmentKind, string> = {
  plane: "seat",
  cylinder: "axis",
  circularEdge: "rim",
};

export const PICK_HINT =
  "Click a planar face, a cylindrical face, or a circular edge — Esc stops picking.";
export const PICK_REJECTED =
  "Pick a planar face, a cylindrical face, or a circular edge.";

type Vec3 = [number, number, number];

const EPS = 1e-9;

function sub(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function scale(v: Vec3, k: number): Vec3 {
  return [v[0] * k, v[1] * k, v[2] * k];
}

function add(a: Vec3, b: Vec3): Vec3 {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
}

function normalize(v: Vec3): Vec3 | null {
  const len = Math.sqrt(dot(v, v));
  if (!Number.isFinite(len) || len < EPS) return null;
  return [v[0] / len, v[1] / len, v[2] / len];
}

/** Foot of the perpendicular from `p` to the plane through `origin` with unit `n`. */
function projectOntoPlane(p: Vec3, origin: Vec3, n: Vec3): Vec3 {
  return sub(p, scale(n, dot(sub(p, origin), n)));
}

/** Nearest point to `p` on the infinite line through `origin` along unit `axis`. */
function projectOntoAxis(p: Vec3, origin: Vec3, axis: Vec3): Vec3 {
  return add(origin, scale(axis, dot(sub(p, origin), axis)));
}

/**
 * A stable roll reference perpendicular to `z`: world X projected into the
 * plane, falling back to world Y when `z` is (near) X itself. Any perpendicular
 * is geometrically valid — what matters is that the SAME z yields the same x
 * every time, so re-picking the same face authors the same frame.
 */
export function perpendicularTo(z: Vec3): Vec3 | null {
  const worldX: Vec3 = [1, 0, 0];
  const fromX = normalize(projectOntoPlane(worldX, [0, 0, 0], z));
  if (fromX) return fromX;
  return normalize(projectOntoPlane([0, 1, 0], [0, 0, 0], z));
}

/**
 * The attachment a classified pick authors, or `null` when the picked geometry
 * has no seatable frame (a spline face, a straight edge, a vertex). Pure —
 * `pickWorldPos` is the point the author actually clicked.
 *
 * The clicked point is what sets the frame ORIGIN for a face: projected onto
 * the plane for a planar seat, onto the axis for a cylinder. OCCT's own
 * surface origin is a parametrization artifact (usually the base circle or a
 * corner of the underlying plane) and would put the seat somewhere the author
 * never pointed at. A circle has one meaningful centre, so an edge pick uses it.
 */
export function deriveAttachment(
  classify: ClassifyResult,
  pickWorldPos: Vec3,
): PickedAttachment | null {
  const frame = classify.frame;
  if (!frame) return null;
  const origin: Vec3 = frame.origin;

  if (classify.kind === "face" && classify.surfaceType === "plane" && frame.normal) {
    const z = normalize(frame.normal);
    const x = z ? perpendicularTo(z) : null;
    if (!z || !x) return null;
    return {
      kind: "plane",
      accepts: ["plane"],
      frame: { origin: projectOntoPlane(pickWorldPos, origin, z), z, x },
    };
  }
  if (classify.kind === "face" && classify.surfaceType === "cylinder" && frame.axis) {
    const z = normalize(frame.axis);
    const x = z ? perpendicularTo(z) : null;
    if (!z || !x) return null;
    return {
      kind: "cylinder",
      accepts: ["cylinder", "hole"],
      frame: { origin: projectOntoAxis(pickWorldPos, origin, z), z, x },
    };
  }
  if (classify.kind === "edge" && classify.curveType === "circle" && frame.axis) {
    const z = normalize(frame.axis);
    const x = z ? perpendicularTo(z) : null;
    if (!z || !x) return null;
    return {
      kind: "circularEdge",
      accepts: ["cylinder", "hole", "circularEdge"],
      frame: { origin, z, x },
    };
  }
  return null;
}

export interface AttachmentPickHandlers {
  /** A usable pick. Pick mode STAYS ARMED — authors place several attachments in a row. */
  onPick: (picked: PickedAttachment) => void;
  /** An unusable pick (or a click on nothing). Pick mode stays armed. */
  onReject: (message: string) => void;
  /** Esc, or an unmountable engine — pick mode is over, the dialog stays open. */
  onExit: () => void;
}

let handlers: AttachmentPickHandlers | null = null;
/** Bumped per pick so a slow `classifyElement` cannot land after Esc. */
let pickSeq = 0;

export function isAttachmentPickActive(): boolean {
  return handlers !== null;
}

/**
 * Arms pick mode. Returns `false` (and arms nothing) when there is no viewport
 * or no geometry service — the caller shows why rather than leaving a dead
 * "picking…" state on screen.
 */
export function beginAttachmentPick(next: AttachmentPickHandlers): boolean {
  cancelAttachmentPick();
  const engine = getViewportEngine();
  if (!engine || !authoringServices()) return false;
  handlers = next;
  engine.setOrbitSuppressed(true);
  window.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("keydown", onKeyDown, true);
  return true;
}

/** Disarms. Safe to call when not armed (the dialog calls it on unmount). */
export function cancelAttachmentPick(): void {
  window.removeEventListener("pointerdown", onPointerDown, true);
  window.removeEventListener("keydown", onKeyDown, true);
  pickSeq += 1;
  if (!handlers) return;
  handlers = null;
  getViewportEngine()?.setOrbitSuppressed(false);
}

/** Whether the event happened inside the authoring dialog rather than the model. */
function insideDialog(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("[data-authoring-dialog]") !== null;
}

function onPointerDown(e: PointerEvent): void {
  const active = handlers;
  if (!active || insideDialog(e.target)) return;
  e.preventDefault();
  e.stopPropagation();
  const engine = getViewportEngine();
  const hit: PickHit | null = engine?.probePick(e.clientX, e.clientY) ?? null;
  if (!hit) {
    active.onReject(PICK_REJECTED);
    return;
  }
  const seq = ++pickSeq;
  const world: Vec3 = [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z];
  void authoringServices()
    ?.geometryQuery.classifyElement(hit.bodyId, hit.elementId ?? "", hit.topoKey)
    .then((classify) => {
      if (seq !== pickSeq || handlers !== active) return; // disarmed mid-flight
      const picked = classify ? deriveAttachment(classify, world) : null;
      if (!picked) {
        active.onReject(PICK_REJECTED);
        return;
      }
      active.onPick(picked);
    })
    .catch(() => {
      if (seq !== pickSeq || handlers !== active) return;
      active.onReject(PICK_REJECTED);
    });
}

function onKeyDown(e: KeyboardEvent): void {
  const active = handlers;
  if (!active) return;
  if (e.key !== "Escape") return;
  // Esc leaves PICK MODE, not the dialog — everything typed so far survives.
  e.preventDefault();
  e.stopPropagation();
  cancelAttachmentPick();
  active.onExit();
}
