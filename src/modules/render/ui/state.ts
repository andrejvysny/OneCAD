/*
 * How render's UI reads render's state.
 *
 * TWO SEAMS, and the split matters.
 *
 * `useRenderStore` is a plain zustand subscription to the module's own vanilla
 * store — the same shape `useSidebarTabStore` uses. It is here rather than in
 * `store/renderStore.ts` because that file is deliberately React-free (the
 * viewport, which is not React at all, drives the identical API).
 *
 * `materialQuery` is the module's OWN published read contract
 * (`RenderServices.MaterialQuery`), instantiated directly rather than resolved
 * off the platform. The implementation is a stateless view over the same
 * singleton store, so this is the same code path a consumer gets through the
 * service registry — not a second, drifting derivation of "how many bodies wear
 * this material". Going through `usePlatform()` for it would make every one of
 * these components untestable without booting a platform, and would buy nothing:
 * a module never needs the registry to reach its own service.
 */
import { useStore } from "zustand";

import { createMaterialQueryService } from "../materialQuery";
import { renderStore, type RenderStoreState } from "../store/renderStore";

export function useRenderStore<T>(selector: (s: RenderStoreState) => T): T {
  return useStore(renderStore, selector);
}

/** Read-only material queries — see the header note on why this is not the
 *  registry's copy. */
export const materialQuery = createMaterialQueryService();
