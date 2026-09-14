/*
 * The shared promote lane (SCHEMA §7.5 `AcquireElementIds`) — HISTORY-HARDEN H4,
 * hardened by VP-HARDENING PR-01.
 *
 * Promotion is SNAPSHOT-SCOPED on both sides of the wire: the backend refuses
 * a pick taken against a snapshot that is no longer the head (VF-M3/VF-M4), rather
 * than resolving its `TopoKey` — a 1-based shape-map ordinal — against different
 * geometry and minting a persistent id for a face the user never picked.
 *
 * That turns "promote failed" from a theoretical worker error into an ORDINARY
 * outcome the user can hit by picking, editing, and then acting on the old pick.
 * It reports that outcome uniformly; each caller decides whether its gesture can
 * keep a non-persistent ref. A target that would be authored into a record must
 * fail closed rather than degrade to an anchor-only reference.
 *
 * There are exactly TWO ways in, and they are separately typed so neither can be
 * reached by the other's callers:
 *
 *  - {@link promoteViewportPick} — everything the user CLICKED. The pick's label
 *    came out of a MESH1 id table, so it means nothing without the installed
 *    entry it was read from: the proof is mandatory, captured at hit time
 *    (`viewport/mesh/pickProof.ts`), and checked BEFORE anything else — including
 *    the already-an-ElementId short-circuit, which is otherwise a way to lift a
 *    persistent-looking handle off geometry that is no longer current.
 *  - {@link promoteAuthoritativeRef} — labels the BACKEND itself named (a repair
 *    candidate, an edge-op or offset closure). There is no mesh entry to prove
 *    anything about such a label, so the fence is an explicit `snapshotId`
 *    instead, and the `origin` tag makes it impossible to pass a Picker hit here
 *    by accident.
 *
 * {@link promoteRef} is the viewport lane's convenience form for a caller that
 * holds a selection `EntityRef`: it looks the ref's captured proof up and FAILS
 * CLOSED when there is none. A proof-free viewport promotion has no path at all.
 */
import type { CadClient } from "@/ipc/client";
import type { PromotePick, PromotedElement } from "@/ipc/types";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import type { EntityRef } from "@/stores/selectionStore";
import {
  getCurrentMeshPublication,
  getEntry,
  isEntryPromotable,
  type MeshEntry,
} from "@/viewport/mesh/meshRegistry";
import { pickProofFor, type InstalledPickProof } from "@/viewport/mesh/pickProof";

export type { InstalledPickProof };

/**
 * Is `entry` still the current, actionable publication of `bodyId`?
 *
 * Everything that is true of a body regardless of WHICH element was picked:
 * live geometry, this document and runtime, visible (and not isolated away),
 * the exact object the registry has installed, not demoted to
 * `stale-inspection-only`, and stamped with the current publication's
 * provenance. {@link installedProofIsCurrent} adds the per-element half.
 */
export function installedEntryIsCurrent(bodyId: string, entry: MeshEntry): boolean {
  const document = documentStore.getState();
  const publication = getCurrentMeshPublication();
  const installed = getEntry(bodyId);
  const isolated = viewportStore.getState().isolatedBodyIds;
  const effectivelyVisible = document.bodies[bodyId]?.visible === true
    && (isolated === null || isolated.includes(bodyId));
  return document.geometrySource === "live"
    && document.documentId !== undefined
    && effectivelyVisible
    && publication !== null
    && publication.documentId === document.documentId
    && publication.runtimeSession === document.runtimeSession
    && installed === entry
    // A body whose last replacement failed keeps its old geometry on screen as
    // stale inspection-only state (spec §9): orbit, fit and tagged measurement
    // are fine, but a face picked off it must never become an operation target.
    && isEntryPromotable(entry)
    && entry.bodyId === bodyId
    && entry.provenance?.documentId === publication.documentId
    && entry.provenance.runtimeSession === publication.runtimeSession
    && entry.provenance.snapshotId === publication.snapshotId
    && entry.provenance.generation === publication.generation
    && entry.meshRev > 0;
}

/**
 * Is the exact installed entry this pick was taken from still current, and does
 * it still name the picked element?
 */
export function installedProofIsCurrent(bodyId: string, proof: InstalledPickProof): boolean {
  if (!installedEntryIsCurrent(bodyId, proof.entry)) return false;
  const index = proof.kind === "face" ? proof.entry.faceIndex : proof.entry.edgeIndex;
  return index != null && index.ordinalForId(proof.topoKey) >= 0;
}

/** The single user-facing wording for a refused / unresolvable promotion. */
export const STALE_PICK_HINT = "Selection is out of date — pick again";

/** Sticky hint (mirrors `historyActions.errorHint`) so it survives a repaint. */
export function stalePickHint(): void {
  viewportStore.getState().setStatusHint(STALE_PICK_HINT, { severity: "error", sticky: true });
}

/**
 * The MESH1 id table is a TWO-NAMESPACE table (mesh_format.md §2
 * `IDS_HAVE_ELEMENTIDS`): the worker's `Tessellate.cpp` substitutes the minted
 * ElementId for any element that already has a live binding — a Hole's host face
 * right after the hole regen, say — and leaves every other element named by its
 * snapshot-scoped TopoKey. `Picker.ts` hands that label back verbatim as the
 * pick's `topoKey`, so a pick on such an element used to be sent to
 * `AcquireElementIds`, whose `resolve_pick` parses only `f:N`/`e:N`/`v:N`,
 * dropped it, and surfaced as "Selection is out of date" on a perfectly fresh
 * pick (UX review 2026-09-11).
 *
 * There is nothing to promote: an ElementId IS the persistent handle promotion
 * exists to mint. Answer from the pick itself — no wire call, no hint.
 *
 * This is NOT a way past a currentness check. An `el_` label read off a mesh the
 * user is no longer looking at is a persistent handle for an element of a
 * publication that has been replaced; both entry points below check their own
 * fence FIRST and only then short-circuit.
 */
function alreadyPromoted(
  bodyId: string,
  pick: PromotePick,
  kind: PromotedElement["kind"],
): PromotedElement {
  return {
    topoKey: pick.topoKey,
    elementId: pick.topoKey,
    // The mesh label carries no kind, so the only honest answer is the one the
    // caller proved. `""` would be a value outside the field's documented
    // `face|edge|vertex` domain.
    ...(kind ? { kind } : {}),
    bodyId,
  };
}

/**
 * Promote ONE pick the user made IN THE VIEWPORT, or return `null` — emitting
 * the status hint — when its proof is no longer current, the backend refused it
 * (stale snapshot), errored, or resolved nothing.
 *
 * `proof` is mandatory and is checked BEFORE the wire call, before the `el_`
 * short-circuit, and AGAIN after the reply lands: a publication that arrives
 * while the promotion is in flight invalidates the answer, because the ordinal
 * the backend resolved was read off geometry that is no longer installed.
 *
 * `null` covers the pre-first-publish case too: nothing is installed, so no pick
 * can hold a current proof and none reaches the wire.
 */
export async function promoteViewportPick(
  client: CadClient,
  proof: InstalledPickProof,
  pick: PromotePick,
): Promise<PromotedElement | null> {
  // The proof's own body, never a caller-supplied one: two sources for the same
  // fact is a channel for them to disagree.
  const bodyId = proof.entry.bodyId;
  if (
    proof.topoKey !== pick.topoKey ||
    (pick.kind !== undefined && pick.kind !== proof.kind) ||
    !installedProofIsCurrent(bodyId, proof)
  ) {
    stalePickHint();
    return null;
  }
  if (pick.topoKey.startsWith("el_")) return alreadyPromoted(bodyId, pick, proof.kind);

  // `installedProofIsCurrent` has already matched this provenance field-by-field
  // against the current publication, so these are the published values.
  const provenance = proof.entry.provenance;
  let promoted: PromotedElement[];
  try {
    promoted = await client.promoteSelection(
      bodyId,
      [pick],
      provenance?.snapshotId,
      provenance?.runtimeSession,
    );
  } catch {
    stalePickHint();
    return null;
  }
  // The backend OMITS a pick it could not resolve, so one pick in means an array
  // of 0 or 1: an empty reply IS the "unresolvable" signal, not a partial result.
  const hit = promoted[0];
  if (
    !hit ||
    !installedProofIsCurrent(bodyId, proof) ||
    hit.bodyId !== bodyId ||
    hit.topoKey !== proof.topoKey ||
    hit.kind !== proof.kind
  ) {
    stalePickHint();
    return null;
  }
  return hit;
}

/**
 * Which non-viewport lane authored the label being promoted.
 *
 * A string-literal union rather than a free-form string so this entry point
 * cannot be reached from a Picker hit by accident: every value names a source
 * that is AUTHORITATIVE — the label came back from the backend, fenced by the
 * snapshot it answered against — rather than read off a mesh on screen.
 */
export type AuthoritativePromotionOrigin =
  /** A `NeedsRepair` candidate the user chose in the repair panel. */
  | "history-repair"
  /** `prepareEdgeOp`'s `adjacentFaces` for a typed chamfer's reference face. */
  | "edge-op-closure"
  /** `prepareOffsetFace`'s resolved face closure (picked and chained alike). */
  | "offset-closure";

/**
 * Promote ONE label the BACKEND named, fenced by the snapshot it named it at.
 *
 * There is no installed-entry proof for such a label — it never came off a mesh
 * — so `snapshotId` is required and is the whole fence: the backend refuses the
 * promotion outright if the head has moved past it. Never call this for anything
 * the user clicked; {@link promoteViewportPick} is that lane.
 */
export async function promoteAuthoritativeRef(
  client: CadClient,
  bodyId: string,
  pick: PromotePick,
  snapshotId: number,
  // Type-level tag only: it exists to make the call site state, and the compiler
  // check, that this label is not viewport-derived.
  _origin: AuthoritativePromotionOrigin,
): Promise<PromotedElement | null> {
  if (pick.topoKey.startsWith("el_")) return alreadyPromoted(bodyId, pick, pick.kind);
  let promoted: PromotedElement[];
  try {
    promoted = await client.promoteSelection(bodyId, [pick], snapshotId);
  } catch {
    stalePickHint();
    return null;
  }
  const hit = promoted[0];
  if (!hit) {
    stalePickHint();
    return null;
  }
  return hit;
}

/**
 * Promote the element a selection `EntityRef` names, using the proof captured
 * when it was picked.
 *
 * FAILS CLOSED. A ref with no proof is one the viewport did not just produce —
 * restored from persistence or undo, rewritten by a regen reconcile, or
 * synthesised by a tool — and there is nothing to prove it still names what the
 * user is looking at. Such a ref survives only by carrying a persistent
 * `elementId`, which every acquisition path reads before reaching for
 * promotion; falling back to a proof-free wire call here is precisely the hole
 * this lane closes.
 */
export async function promoteRef(
  client: CadClient,
  ref: EntityRef,
): Promise<PromotedElement | null> {
  const topoKey = ref.topoKey;
  if ((ref.kind !== "face" && ref.kind !== "edge") || topoKey === undefined) {
    stalePickHint();
    return null;
  }
  const proof = pickProofFor(ref);
  if (!proof) {
    stalePickHint();
    return null;
  }
  return promoteViewportPick(client, proof, {
    topoKey,
    kind: ref.kind,
    ...(ref.anchor
      ? {
          anchor: {
            worldPoint: ref.anchor.worldPoint,
            ...(ref.anchor.surfaceUv ? { surfaceUv: ref.anchor.surfaceUv } : {}),
          },
        }
      : {}),
  });
}
