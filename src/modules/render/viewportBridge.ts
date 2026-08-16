/*
 * `onecad.render` → viewport adapter.
 *
 * The viewport declares what it can consume (`BodyMaterialSource`, owned by
 * `src/viewport/engine/pbrParams.ts`) and never imports a module; this file
 * points the arrow the legal way round and is the ONLY place the two vocabularies
 * meet. Everything it does is translation:
 *
 *   MaterialQuery.poolKeyForBody   → the pool's cache key, verbatim
 *   MaterialQuery.resolvedForBody  → PbrMaterialParams (structural, not nominal)
 *   faceOverridesForBody + getMaterial + resolveMaterial → per-face base colors
 *   viewport's binding report      → renderStore's session-only bound/unbound
 *
 * No caching, no state: every method is a synchronous read of already-parsed
 * store state, which is exactly the contract `MaterialQuery` was written to
 * (manifest.ts). Adding a cache here would put a second, staler copy of the
 * material library on a repaint path.
 */
import type { ModuleScope } from "@/platform";
import type { BodyMaterialSource } from "@/viewport/engine/pbrParams";
import { setBodyMaterialSource } from "@/viewport/materialSourceBridge";

import type { MaterialQueryService } from "./manifest";
import { createMaterialQueryService } from "./materialQuery";
import { resolveMaterial } from "./model/material";
import { renderStore } from "./store/renderStore";

/** Adapt a `MaterialQuery` into the shape the viewport's mesh ingest consumes. */
export function createRenderMaterialSource(query: MaterialQueryService): BodyMaterialSource {
  return {
    poolKeyForBody: (bodyId) => query.poolKeyForBody(bodyId),

    // `ResolvedMaterial` satisfies `PbrMaterialParams` structurally — all 29
    // fields, filled and clamped — so there is nothing to map field by field.
    paramsForBody: (bodyId) => query.resolvedForBody(bodyId),

    faceOverrideBaseColors(bodyId, faceElementIds) {
      const overrides = query.faceOverridesForBody(bodyId, faceElementIds);
      const out: Record<string, [number, number, number]> = {};
      for (const [faceId, materialId] of Object.entries(overrides)) {
        const def = query.getMaterial(materialId);
        // A dangling id yields NO entry rather than a fabricated default colour:
        // the face then draws with the body's material, which is the honest
        // answer to "this override names a material that isn't there".
        if (def === null) continue;
        const [r, g, b] = resolveMaterial(def).base_color;
        out[faceId] = [r, g, b];
      }
      return out;
    },

    reportFaceOverrideBindings(bodyId, bound, unbound) {
      const store = renderStore.getState();
      store.reportBoundFaceOverrides(bodyId, bound);
      store.reportUnboundFaceOverrides(bodyId, unbound);
    },

    subscribe: (cb) => query.subscribe(cb),
  };
}

/**
 * Publish the source for the viewport's lifetime, and withdraw it when the
 * module goes away.
 *
 * `query` defaults to a fresh service rather than being threaded in from
 * `contributeRender`: the implementation is stateless (every method reads
 * `renderStore` at call time), so a second instance costs one object and keeps
 * this a pure one-line hookup at the call site — which matters, because that
 * file is edited by more than one concern. Tests pass an explicit query.
 */
export function attachRenderViewportBridge(
  scope: ModuleScope,
  query: MaterialQueryService = createMaterialQueryService(),
): void {
  setBodyMaterialSource(createRenderMaterialSource(query));
  scope.own({ dispose: () => setBodyMaterialSource(null) });
}
