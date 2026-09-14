/*
 * pickProof — the installed-entry PROOF a viewport pick carries from the frame
 * it was taken on to the moment it is promoted (VP-HARDENING PR-01, spec §9).
 *
 * A `TopoKey` (`"f:22"`) is a SNAPSHOT-SCOPED shape-map ordinal, and a MESH1 id
 * table is the only thing that turns one into geometry. So the question
 * promotion has to answer is not "does this label resolve?" — after a regen it
 * almost always does, against a different element — but "is the exact mesh the
 * user clicked still the one installed and drawn?".
 *
 * Only the hit itself can answer that, and only at hit time: the entry the
 * ordinal was read from is a concrete object, and re-deriving it later with
 * `getEntry(bodyId)` pairs an OLD ordinal with NEW geometry, which is the
 * silent-wrong-bind class this repository exists to remove. Hence a proof is
 * CAPTURED, never recomputed.
 *
 * It travels with an `EntityRef` through a module `WeakMap` keyed by the ref
 * OBJECT rather than as a field, for two reasons:
 *  - `EntityRef` is store state: a plain, serialisable record with no live
 *    references to GPU resources (a `MeshEntry` holds `BufferGeometry`), and
 *  - selection already compares refs by object identity (`selected.indexOf(ref)`
 *    in `promotePick` / `confirmRef`, Astra break F3), so a ref that is REPLACED
 *    — rewritten by a reconcile, or re-picked with the same reusable
 *    `${bodyId}#${topoKey}` id — correctly carries no proof, and a dropped ref
 *    costs nothing to forget.
 */
import type { EntityRef } from "@/stores/selectionStore";
import type { MeshEntry } from "./meshRegistry";

/**
 * What a viewport pick proves about the publication it was taken against.
 *
 * `entry` is the EXACT installed object `Picker.resolvePick` read the ordinal
 * from — identity-compared against the registry at promotion time, never
 * re-looked-up. `kind` is `face | edge` only: a BODY pick names a body id, not a
 * topological element, and has nothing to promote.
 */
export interface InstalledPickProof {
  readonly entry: MeshEntry;
  readonly kind: "face" | "edge";
  readonly topoKey: string;
}

const proofs = new WeakMap<EntityRef, InstalledPickProof>();

/** Record the pick-time proof for a selection/hover ref. */
export function attachPickProof(ref: EntityRef, proof: InstalledPickProof): void {
  proofs.set(ref, proof);
}

/** The proof this exact ref OBJECT was created with, or `undefined`. */
export function pickProofFor(ref: EntityRef): InstalledPickProof | undefined {
  return proofs.get(ref);
}

/**
 * Forget a ref's proof.
 *
 * Called for every ref a REGEN reconcile keeps: the publication it was picked
 * against is gone, so the proof is no longer evidence about anything on screen.
 * Such a ref survives only by carrying a persistent `elementId` (rebindPick's
 * `keep` / `confirm` verdicts), and an `elementId`-bearing ref needs no
 * promotion at all — every acquisition path reads it first.
 */
export function clearPickProof(ref: EntityRef): void {
  proofs.delete(ref);
}

/**
 * The proof a Picker hit carries, or `null` when it has none.
 *
 * Typed structurally so a controller that must not import Three.js (the
 * `PickHit.worldPos` is a `THREE.Vector3`) can still build one.
 */
export function proofFromHit(
  hit: { entry?: MeshEntry; kind: string; topoKey: string } | null | undefined,
): InstalledPickProof | null {
  if (!hit?.entry) return null;
  if (hit.kind !== "face" && hit.kind !== "edge") return null;
  return { entry: hit.entry, kind: hit.kind, topoKey: hit.topoKey };
}
