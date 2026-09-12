/*
 * The shared promote lane (SCHEMA §7.5 `AcquireElementIds`) — HISTORY-HARDEN H4.
 *
 * Promotion is now SNAPSHOT-SCOPED on both sides of the wire: the backend refuses
 * a pick taken against a snapshot that is no longer the head (VF-M3/VF-M4), rather
 * than resolving its `TopoKey` — a 1-based shape-map ordinal — against different
 * geometry and minting a persistent id for a face the user never picked.
 *
 * That turns "promote failed" from a theoretical worker error into an ORDINARY
 * outcome the user can hit by picking, editing, and then acting on the old pick.
 * It reports that outcome uniformly; each caller decides whether its gesture can
 * keep a non-persistent ref. A target that would be authored into a record must
 * fail closed rather than degrade to an anchor-only reference.
 */
import type { CadClient } from "@/ipc/client";
import type { PromotePick, PromotedElement } from "@/ipc/types";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import { getCurrentMeshPublication, getEntry, type MeshEntry } from "@/viewport/mesh/meshRegistry";

export interface InstalledPickProof {
  entry: MeshEntry;
  kind: "body" | "face" | "edge";
  topoKey: string;
}

export function installedProofIsCurrent(bodyId: string, proof: InstalledPickProof): boolean {
  const document = documentStore.getState();
  const publication = getCurrentMeshPublication();
  const installed = getEntry(bodyId);
  const isolated = viewportStore.getState().isolatedBodyIds;
  const effectivelyVisible = document.bodies[bodyId]?.visible === true
    && (isolated === null || isolated.includes(bodyId));
  const index = proof.kind === "face"
    ? proof.entry.faceIndex
    : proof.kind === "edge"
      ? proof.entry.edgeIndex
      : null;
  return document.geometrySource === "live"
    && document.documentId !== undefined
    && effectivelyVisible
    && publication !== null
    && publication.documentId === document.documentId
    && publication.runtimeSession === document.runtimeSession
    && installed === proof.entry
    && proof.entry.bodyId === bodyId
    && proof.entry.provenance?.documentId === publication.documentId
    && proof.entry.provenance.runtimeSession === publication.runtimeSession
    && proof.entry.provenance.snapshotId === publication.snapshotId
    && proof.entry.provenance.generation === publication.generation
    && proof.entry.meshRev > 0
    && (proof.kind === "body" ? proof.topoKey === bodyId : index !== null && index.ordinalForId(proof.topoKey) >= 0);
}

/** The single user-facing wording for a refused / unresolvable promotion. */
export const STALE_PICK_HINT = "Selection is out of date — pick again";

/** Sticky hint (mirrors `historyActions.errorHint`) so it survives a repaint. */
export function stalePickHint(): void {
  viewportStore.getState().setStatusHint(STALE_PICK_HINT, { severity: "error", sticky: true });
}

/**
 * Promote ONE pick and return its element, or `null` when the backend refused it
 * (stale snapshot), errored, or resolved nothing — emitting the status hint in
 * every one of those cases.
 *
 * `null` covers the pre-first-publish case too: `tauriClient` starts at
 * `currentSnapshotId = 0` and only adopts a POSITIVE published id, so a pick made
 * before any `document-changed` lands addresses snapshot `0` against a positive
 * head and is refused here rather than silently binding.
 *
 * A pick whose label is ALREADY an ElementId never reaches the wire — see the
 * two-namespace note below.
 */
export async function promoteOne(
  client: CadClient,
  bodyId: string,
  pick: PromotePick,
  snapshotId?: number,
  proof?: InstalledPickProof,
): Promise<PromotedElement | null> {
  const proofSnapshot = proof?.entry.provenance?.snapshotId;
  if (proof && (
    proof.kind === "body" ||
    proof.topoKey !== pick.topoKey ||
    (pick.kind !== undefined && proof.kind !== pick.kind) ||
    !installedProofIsCurrent(bodyId, proof) ||
    (snapshotId !== undefined && snapshotId !== proofSnapshot)
  )) {
    stalePickHint();
    return null;
  }
  // The MESH1 id table is a TWO-NAMESPACE table (mesh_format.md §2
  // `IDS_HAVE_ELEMENTIDS`): the worker's `Tessellate.cpp` substitutes the minted
  // ElementId for any element that already has a live binding — a Hole's host
  // face right after the hole regen, say — and leaves every other element named
  // by its snapshot-scoped TopoKey. `Picker.ts` hands that label back verbatim as
  // the pick's `topoKey`, so a pick on such an element used to be sent to
  // `AcquireElementIds`, whose `resolve_pick` parses only `f:N`/`e:N`/`v:N`,
  // dropped it, and surfaced as "Selection is out of date" on a perfectly fresh
  // pick (UX review 2026-09-11).
  //
  // There is nothing to promote: an ElementId IS the persistent handle promotion
  // exists to mint. Answer from the pick itself — no wire call, no hint.
  if (pick.topoKey.startsWith("el_")) {
    return {
      topoKey: pick.topoKey,
      elementId: pick.topoKey,
      // The mesh label carries no kind, so the only honest answer is the
      // caller's own — and, absent that, none at all. `""` would be a value
      // outside the field's documented `face|edge|vertex` domain.
      ...(pick.kind ? { kind: pick.kind } : {}),
      bodyId,
    };
  }
  let promoted: PromotedElement[];
  try {
    const effectiveSnapshot = snapshotId ?? proofSnapshot;
    if (effectiveSnapshot === undefined) {
      promoted = await client.promoteSelection(bodyId, [pick]);
    } else if (proof === undefined) {
      // Keep the legacy call shape: a number of tool adapters provide a
      // three-argument promotion seam and do not participate in candidate
      // provenance yet.
      promoted = await client.promoteSelection(bodyId, [pick], effectiveSnapshot);
    } else {
      promoted = await client.promoteSelection(
        bodyId,
        [pick],
        effectiveSnapshot,
        proof.entry.provenance?.runtimeSession,
      );
    }
  } catch {
    stalePickHint();
    return null;
  }
  // The backend OMITS a pick it could not resolve, so one pick in means an array
  // of 0 or 1: an empty reply IS the "unresolvable" signal, not a partial result.
  const hit = promoted[0];
  if (
    !hit ||
    (proof && (
      !installedProofIsCurrent(bodyId, proof) ||
      hit.bodyId !== bodyId ||
      hit.topoKey !== proof.topoKey ||
      hit.kind !== proof.kind
    ))
  ) {
    stalePickHint();
    return null;
  }
  return hit;
}
