/*
 * rebindPick — resolving a selection ref against a mesh, and deciding what
 * happens to one whose label did not survive a regen.
 *
 * A TopoKey (`"f:22"`) is a SNAPSHOT-SCOPED shape-map ordinal (SCHEMA §10): a
 * rebuild may renumber every one of them, and an edge a fillet consumed is gone
 * outright. So a ref made before a parametric edit routinely names NOTHING after
 * it — `TopoIndex.ordinalForId` returns -1 and `HighlightLayer` draws no overlay.
 *
 * This module USED to close that gap by searching the new mesh for the nearest
 * element that matched the old one's position and direction. That search is what
 * put the highlight on the chamfer's neighbouring edge after the picked one was
 * consumed: a plausible answer the backend never agreed to.
 *
 * **D-5 (PLAN, 2026-09-11): selection never guesses.** A ref survives a REGEN
 * only when something AUTHORITATIVE still names it:
 *
 *  - the new mesh's id table names its promoted `ElementId` (MESH1
 *    `IDS_HAVE_ELEMENTIDS`: `Tessellate.cpp` substitutes the minted id for any
 *    element that already has a live binding), or
 *  - it carries a promoted `ElementId` and the BACKEND resolves that id at the
 *    current head, in which case the ref adopts the TopoKey the backend
 *    returned — provided that key is drawable in the mesh actually on screen.
 *
 * Anything else is dropped, silently — the highlight goes with the geometry, and
 * the user re-picks. This is the identity ladder's "deterministic NeedsRepair
 * beats a silent wrong bind" law applied to the viewport.
 *
 * **A SNAPSHOT ORDINAL IS NOT EVIDENCE ACROSS A REGEN** (Astra break F1). Element
 * counts only grow over the measured regens — a 3 mm chamfer takes the box from
 * 12 to 15 edges, each hole adds two faces and three edges — so an old `f:N` /
 * `e:N` almost always still names SOMETHING afterwards, just not the element the
 * user picked. Keeping a ref because "the new mesh still resolves its label" is
 * therefore the silent wrong bind wearing a different hat, and an UNPROMOTED ref
 * is dropped across a regen even when its old ordinal resolves.
 *
 * That rule applies to a REGEN only. A colour edit, a visibility flip and the
 * self-healing `reconcile()` pass all re-publish the SAME topology, so `meshSync`
 * does not reconcile for them, and a body's first mesh has no previous
 * publication for a pick to have been taken against.
 *
 * `id` is IDENTITY (`${bodyId}#${topoKey}` — what `sameRef` and the selection
 * toggle compare on) and is NEVER rewritten; only the `topoKey` field the
 * viewport resolves through. Same policy as `promotePick`'s elementId write-back.
 * It is also REUSABLE: a deselect plus a fresh pick of the same label mints the
 * same string, so an in-flight reply is applied to the ref OBJECT it was asked
 * about, never to whatever currently carries its id.
 */
import type { CadClient } from "@/ipc/client";
import type { ElementInfo } from "@/ipc/types";
import { bareBodyId } from "@/ipc/tauriCommandMap";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import type { TopoIndex } from "./faceRangeIndex";
import { getEntry, type MeshEntry } from "./meshRegistry";
import { clearPickProof } from "./pickProof";
import type { BodyMeshView } from "./parseMeshPayload";

/**
 * Ordinal of the element a ref names, or -1.
 *
 * The mesh's id tables hold TopoKeys today, but MESH1 `IDS_HAVE_ELEMENTIDS`
 * lets a producer emit MINTED ElementIds instead (mesh_format.md §2). A promoted
 * ref carries both, and the PERSISTENT one is tried first: a renumber can hand
 * the ref's old transient key to a different element, so reading it first draws
 * that other element (Astra break F1). The transient key stays the fallback for
 * an unpromoted pick, whose label is the only handle it has.
 */
export function ordinalForRef(index: TopoIndex, view: BodyMeshView, ref: EntityRef): number {
  if (ref.elementId && view.idsHaveElementIds) {
    const ord = index.ordinalForId(ref.elementId);
    if (ord >= 0) return ord;
  }
  if (ref.topoKey) {
    const ord = index.ordinalForId(ref.topoKey);
    if (ord >= 0) return ord;
  }
  return -1;
}

/** A face/edge ref belonging to `bodyId` — the only kinds a mesh swap may touch. */
function isBodyElementRef(ref: EntityRef, bodyId: string): boolean {
  return (ref.kind === "face" || ref.kind === "edge") && ref.bodyId === bodyId;
}

/** The id table that could name this ref, or null when the blob carries none. */
function indexFor(ref: EntityRef, entry: MeshEntry): TopoIndex | null {
  return ref.kind === "face" ? entry.faceIndex : entry.edgeIndex;
}

/**
 * What the NEW publication says about one face/edge ref.
 *
 *  - `keep` — its id table names the ref's persistent ElementId. `label` is that
 *    id: the ref adopts it so the highlight resolves through the handle the
 *    backend agrees on rather than a reusable ordinal.
 *  - `confirm` — the ref has a persistent id the mesh does not name. Only the
 *    backend can say whether the element survived, and under what label.
 *  - `drop` — no persistent handle at all. There is nothing to ask about, and
 *    the transient key carries no authority across a regen.
 *
 * A MISSING index is a `drop`/`confirm`, never a keep: a blob with no EDGE
 * sections cannot draw an edge highlight, and a selection that draws nothing but
 * still authors operations is the invisible-selection defect (Astra break F4).
 */
function verdictFor(
  ref: EntityRef,
  next: MeshEntry,
): { kind: "keep"; label: string } | { kind: "confirm" } | { kind: "drop" } {
  const elementId = ref.elementId;
  if (elementId && next.view.idsHaveElementIds) {
    const index = indexFor(ref, next);
    if (index && index.ordinalForId(elementId) >= 0) return { kind: "keep", label: elementId };
  }
  return elementId ? { kind: "confirm" } : { kind: "drop" };
}

/**
 * The TopoKey an `elementInfo` answer entitles this ref to adopt, or null.
 *
 * `QueryElement` answers about the HEAD, which may already be ahead of the mesh
 * on screen (Astra break F2). An answer is only usable when it is about this
 * body, about this kind of element, and names something the DISPLAYED mesh can
 * actually draw — otherwise adopting it either points the highlight at another
 * element or leaves the ref selected and invisible.
 */
function acceptedLabel(
  bodyId: string,
  ref: EntityRef,
  next: MeshEntry,
  info: ElementInfo | null,
): string | null {
  if (!info?.topoKey) return null;
  if (bareBodyId(info.bodyId) !== bareBodyId(bodyId)) return null;
  if (info.kind !== ref.kind) return null;
  const index = indexFor(ref, next);
  return index && index.ordinalForId(info.topoKey) >= 0 ? info.topoKey : null;
}

/**
 * Ask the backend whether `ref`'s promoted ElementId still resolves at the head,
 * and adopt the TopoKey it answers with — or drop the ref when it does not.
 *
 * `elementInfo` with an empty `topoKey` walks the ELEMENTID rung only
 * (`api::element_info`), which is exactly the question: "does this persistent id
 * still name an element of this body, and what is it called now?". An element
 * that no operation references is not re-bound across a regen, so `null` here is
 * the ordinary answer for a plain pick that a rebuild renumbered — and dropping
 * it is the point.
 */
async function confirmRef(
  bodyId: string,
  ref: EntityRef,
  next: MeshEntry,
  client: CadClient,
): Promise<void> {
  let info: ElementInfo | null = null;
  try {
    info = await client.elementInfo(bodyId, ref.elementId ?? "", "");
  } catch {
    // A refused / unreachable query is not evidence that the element survived.
    info = null;
  }
  // A later swap of the SAME body supersedes this answer outright: it was asked
  // about a mesh that is no longer on screen.
  if (getEntry(bodyId) !== next) return;
  const sel = selectionStore.getState();
  // The ref OBJECT, never its id: `id` is reusable, so a deselect plus a fresh
  // pick of the same label would otherwise take this answer's verdict.
  const at = sel.selected.indexOf(ref);
  if (at < 0) return; // deselected / superseded while in flight — never re-add
  const topoKey = acceptedLabel(bodyId, ref, next, info);
  if (!topoKey) {
    sel.set(sel.selected.filter((_, i) => i !== at));
    return;
  }
  if (sel.selected[at].topoKey === topoKey) return; // confirmed, nothing to rewrite
  const out = sel.selected.slice();
  // `id` is identity and stays put; only the transient TopoKey is refreshed.
  out[at] = { ...out[at], topoKey };
  sel.set(out);
}

/**
 * Store glue: reconcile the selection + hover against `bodyId`'s NEW mesh. Call
 * between the registry `swap` and the highlight refresh, so the rebuild reads the
 * surviving refs.
 *
 * `prev` is the entry that was on screen. Absent (a body's FIRST mesh) or
 * identical to `next`, there is no regen to survive and nothing is touched —
 * the pick that armed the current tool was taken against this very publication.
 * `meshSync` calls this only for a regen reload for the same reason.
 *
 * Synchronous for everything that can be decided locally (kept against the new
 * id table, or dropped for want of a persistent id); a ref that carries an
 * ElementId the mesh does not name stays in place while {@link confirmRef} asks
 * the backend, so the selection never flickers out and back. Its highlight
 * simply does not draw until the answer lands.
 *
 * Hover is never resolved against the backend — it is a pointer artefact that the
 * next mouse move re-establishes, and a hover path must not issue IO.
 */
export function reconcileSelectionForBody(
  bodyId: string,
  prev: MeshEntry | undefined,
  next: MeshEntry,
  client: CadClient | null,
): void {
  if (!prev || prev === next) return;
  const sel = selectionStore.getState();

  // Hover is pointer-derived, not a persisted identity claim. A topology swap
  // invalidates it even if the new mesh names the same ElementId; the next
  // pointer move re-establishes hover against the rendered publication.
  const hover = sel.hover;
  if (hover && isBodyElementRef(hover, bodyId)) sel.setHover(null);

  const kept: EntityRef[] = [];
  const pending: EntityRef[] = [];
  let rewritten = false;
  for (const ref of sel.selected) {
    if (!isBodyElementRef(ref, bodyId)) {
      kept.push(ref);
      continue;
    }
    const verdict = verdictFor(ref, next);
    // Whatever happens below, this ref stops being a viewport pick: the
    // publication it was taken against is gone, so its pick-time proof is no
    // longer evidence about anything drawn and must never be reused to promote.
    // A ref that survives here does so on its persistent `elementId` alone —
    // which every acquisition path reads before it reaches for promotion.
    clearPickProof(ref);
    if (verdict.kind === "keep") {
      if (ref.topoKey === verdict.label) kept.push(ref);
      else {
        kept.push({ ...ref, topoKey: verdict.label });
        rewritten = true;
      }
    } else if (verdict.kind === "confirm" && client) {
      kept.push(ref);
      pending.push(ref);
    }
    // else: no persistent handle (or no client to ask) and no evidence — dropped.
  }
  if (rewritten || kept.length !== sel.selected.length) sel.set(kept);
  if (!client) return;
  for (const ref of pending) void confirmRef(bodyId, ref, next, client);
}

/**
 * The body left the document: drop every ref that named it.
 *
 * Removing the mesh alone leaves refs pointing at geometry that no longer exists
 * anywhere — undrawable, and still authorable by the next tool. A body-row ref
 * is equally stale once its body is absent: retaining it is how the inspector
 * acquired a nameless, actionable Solid state after Pattern undo. Sketches and
 * features are reconciled by the authoritative document projection instead.
 */
export function dropSelectionForBody(bodyId: string): void {
  const sel = selectionStore.getState();
  const ownsBody = (ref: EntityRef): boolean =>
    ref.kind === "body"
      ? ref.id === bodyId
      : (ref.kind === "face" || ref.kind === "edge" || ref.kind === "vertex") && ref.bodyId === bodyId;
  if (sel.hover && ownsBody(sel.hover)) sel.setHover(null);
  const kept = sel.selected.filter((ref) => !ownsBody(ref));
  if (kept.length !== sel.selected.length) sel.set(kept);
}
