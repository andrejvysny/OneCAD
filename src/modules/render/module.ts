/*
 * `onecad.render` module registration.
 *
 * What activation wires, and why each piece is here rather than somewhere else:
 *
 *  - The material store is HYDRATED from the document's own slice, through
 *    `platform/documentState.ts`. The module never touches another module's
 *    state and never reaches around the seam for its own.
 *  - A `document-changed` subscription RE-HYDRATES. This is what makes the
 *    backend's undo/redo of a material write visible: the write went through the
 *    transaction path, so undo reverts it in the document, and the store has to
 *    hear about that rather than keep its own optimistic copy forever. (Real-lane
 *    seam: Rust emits `document-changed` only when a regen publishes, so an undo
 *    of a module-state-only transaction currently arrives as `projection-updated`
 *    alone — see the gate record in `TODO.md`.)
 *  - Modeling's `GeometryQuery` is consumed as a SERVICE, fetched lazily on each
 *    use. Render must never import `documentStore` — modeling owns it — and a
 *    service handle captured at activation would outlive a modeling reload.
 *  - `MaterialQuery` is published for the viewport and the future Render
 *    workspace.
 *
 * Its UI home is still the `onecad.shell` `Visualization` workspace placeholder
 * (`src/modules/shell/workspaces.ts`); no workspace is registered here, and the
 * question of whether this module eventually registers its own is still open —
 * see `docs/RENDER_MODULE.md`.
 */
import type { ModuleScope, Platform } from "@/platform";
import { createClient } from "@/ipc/client";
import {
  MODELING_MODULE_ID,
  ModelingServices,
  type GeometryQueryService,
} from "@/modules/modeling/manifest";
import { RENDER_MODULE_ID, RenderServices } from "./manifest";
import { createMaterialQueryService } from "./materialQuery";
import { renderStore, setRenderBodyLister } from "./store/renderStore";

/** Registers every render contribution into `scope`. Exported for tests. */
export function contributeRender(scope: ModuleScope): void {
  const geometry = (): GeometryQueryService | undefined =>
    scope.platform.services.get<GeometryQueryService>(ModelingServices.GeometryQuery);

  // Undefined (modeling gone) reads as "the body set is unknown", not "there are
  // no bodies" — see `RenderBodyLister`.
  setRenderBodyLister(() => geometry()?.listBodies().map((b) => b.id) ?? null);
  scope.own({ dispose: () => setRenderBodyLister(null) });

  const unsubscribeBodies = geometry()?.subscribeBodies(() =>
    renderStore.getState().recomputeDangling(),
  );
  if (unsubscribeBodies) scope.own({ dispose: unsubscribeBodies });

  const unsubscribeDocument = createClient().onDocumentChanged(() => {
    void renderStore.getState().hydrate();
  });
  scope.own({ dispose: unsubscribeDocument });

  // Real lane only (the mock's subscription is a no-op): a module-state-only
  // transaction has RegenHint::None, so its undo/redo publishes no regen and
  // therefore no `document-changed` — `projection-updated` is the event that DOES
  // fire there, and on open of a document whose render slice arrives with no
  // regen. Stale reads are discarded by the store's generation guard, so
  // re-hydrating on both events is safe over-notification, not a revert risk.
  const unsubscribeProjection = createClient().onProjectionUpdated(() => {
    void renderStore.getState().hydrate();
  });
  scope.own({ dispose: unsubscribeProjection });

  scope.registerService(RenderServices.MaterialQuery, createMaterialQueryService());

  // Fire-and-forget: `activate` is synchronous (the React root needs a Platform
  // on its first render, see `Platform.initializeSync`), and nothing registered
  // above depends on the read having landed — every reader of the store handles
  // `hydrated: false` as "empty so far".
  void renderStore.getState().hydrate();
}

export function registerRenderModule(platform: Platform): void {
  platform.registerModule({
    id: RENDER_MODULE_ID,
    version: "0.2.0",
    // Modeling must be READY first: `contributeRender` reads its GeometryQuery
    // service at activation, and the platform's topological order is the only
    // thing that guarantees it is registered by then (registration order in
    // `app/bootstrap.ts` deliberately guarantees nothing).
    dependsOn: [MODELING_MODULE_ID],
    provides: [RenderServices.MaterialQuery],
    activate: (scope) => contributeRender(scope),
  });
}
