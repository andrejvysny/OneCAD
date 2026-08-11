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
 */
export async function promoteOne(
  client: CadClient,
  bodyId: string,
  pick: PromotePick,
  snapshotId?: number,
): Promise<PromotedElement | null> {
  let promoted: PromotedElement[];
  try {
    promoted = snapshotId === undefined
      ? await client.promoteSelection(bodyId, [pick])
      : await client.promoteSelection(bodyId, [pick], snapshotId);
  } catch {
    stalePickHint();
    return null;
  }
  // The backend OMITS a pick it could not resolve, so one pick in means an array
  // of 0 or 1: an empty reply IS the "unresolvable" signal, not a partial result.
  const hit = promoted[0];
  if (!hit) {
    stalePickHint();
    return null;
  }
  return hit;
}
