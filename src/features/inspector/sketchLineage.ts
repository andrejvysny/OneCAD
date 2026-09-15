/*
 * Sketch → consuming-feature lookup for the inspector (UX review 2026-09-14, N10).
 *
 * The projection carries NO link from a sketch to the features built on it: a
 * `FeatureDto` has no sketch id, and every sketch timeline record is labelled
 * literally "Sketch" (`upsert_sketch_record`), so neither the id nor the label
 * pairs a registry sketch with its row. Pairing them by ORDINAL would be wrong
 * on the real lane — `DocumentProjection.sketches` is a `BTreeMap`, sorted by
 * uuid, not timeline order — and a wrong feature name is exactly the kind of
 * confident-but-false claim this codebase refuses.
 *
 * So the one sound link is the stored params: a Sketch record serializes its
 * sketch as `sketchId` (core `SketchOpParams`), which `getOperationParams`
 * already serves on both lanes. This resolves the sketch's OWN row through it —
 * the same read `SketchDimensionsSection` makes from the other direction — and
 * hands the row to the pure `consumingFeatureLabel` scan.
 */
import { useEffect, useState } from "react";
import { useDocumentStore } from "@/stores/documentStore";
import { createClient } from "@/ipc/client";
import { consumingFeatureLabel } from "@/features/sketch/constraintStatus";

/**
 * The label of the applied feature standing on `sketchId`, or `null`.
 *
 * Pass `null` to ask nothing (the lookup only earns its round-trips when the
 * panel would otherwise have to claim the sketch was never evaluated).
 */
export function useSketchConsumer(sketchId: string | null): string | null {
  const features = useDocumentStore((s) => s.features);
  const appliedOps = useDocumentStore((s) => s.appliedOps);
  const [sketchFeatureId, setSketchFeatureId] = useState<string | null>(null);

  useEffect(() => {
    setSketchFeatureId(null);
    if (!sketchId) return;
    let alive = true;
    const rows = features.filter((f) => f.kind === "sketch");
    void (async () => {
      const client = createClient();
      for (const row of rows) {
        let params: Record<string, unknown>;
        try {
          params = await client.getOperationParams(row.id);
        } catch {
          // A record the backend cannot serve params for tells us nothing about
          // this sketch either way — keep scanning the rest of the timeline.
          continue;
        }
        if (!alive) return;
        if (params.sketchId === sketchId) {
          setSketchFeatureId(row.id);
          return;
        }
      }
    })();
    return () => {
      alive = false;
    };
  }, [sketchId, features]);

  return consumingFeatureLabel(features, sketchFeatureId, appliedOps);
}
