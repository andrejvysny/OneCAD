/*
 * `onecad.render` — the Render module's identity, schema version and published
 * service contract.
 *
 * No UI yet: this module owns a material library, its body/face assignments, and
 * the document-state slice they persist to (`store/renderStore.ts`), and it
 * publishes `MaterialQuery` for the viewport and the future Render workspace to
 * read. See `docs/RENDER_MODULE.md` for the design and
 * `docs/adr/0014-render-module-openpbr.md` for the governing decisions.
 */
import { moduleId, contributionId, type ModuleId, type ServiceId } from "@/platform";
import type { MaterialDef, MaterialId, ResolvedMaterial } from "./model/material";

export const RENDER_MODULE_ID: ModuleId = moduleId("onecad.render");

/**
 * The module's own state schema version, independent of the app version and of
 * the container format version (docs/ARCHITECTURE.md §8). The value the store
 * actually writes is `model/state.ts`'s `CURRENT_RENDER_SCHEMA_VERSION`, which
 * the parser also checks against — this constant is the manifest-level
 * declaration of the same number.
 */
export const RENDER_SCHEMA_VERSION = 1;

/**
 * `RenderServices.MaterialQuery`'s contract — READ-ONLY, by design.
 *
 * Everything here answers a question about appearance; nothing here changes it.
 * Mutation goes through `renderStore`, which owns the undoable write path, so a
 * consumer cannot acquire a second way to edit the document by holding this
 * service (docs/ARCHITECTURE.md §9). The viewport is the first consumer: it
 * needs a stable POOL KEY per body and an override-aware per-face answer, and
 * both are cheap synchronous reads of already-parsed state.
 */
export interface MaterialQueryService {
  /** Every material in the document's library, in insertion order. */
  listMaterials(): readonly MaterialDef[];
  /** One material by id, or `null` when the library has none. */
  getMaterial(id: MaterialId): MaterialDef | null;
  /** The BODY-level assignment only — a face override never shows up here. */
  materialIdForBody(bodyId: string): MaterialId | null;
  /**
   * Override-aware: the face's own material if it has one, else the body's,
   * else `null`. A material id that names nothing in the library resolves to
   * `null` rather than to a fabricated default.
   */
  materialForFace(bodyId: string, faceElementId: string): MaterialDef | null;
  /** The body's material with every OpenPBR parameter filled and clamped. */
  resolvedForBody(bodyId: string): ResolvedMaterial | null;
  /**
   * `${materialId}:${resolvedMaterialHash(def)}` — the key a viewport material
   * pool caches on. Carries the id as well as the hash so two materials that
   * happen to resolve identically stay separately addressable in the UI while
   * still being recognisable as interchangeable shading state.
   */
  poolKeyForBody(bodyId: string): string | null;
  /** `elementId → materialId` for those of `faceElementIds` that are overridden. */
  faceOverridesForBody(
    bodyId: string,
    faceElementIds: readonly string[],
  ): Record<string, MaterialId>;
  /** `materialId → assignment count` (bodies + faces) — the "In This Design" list. */
  usageCounts(): Record<MaterialId, number>;
  /** Fires on any material or assignment change. Returns the unsubscribe. */
  subscribe(cb: () => void): () => void;
}

/** Services this module publishes (see `module.ts::contributeRender`). */
export const RenderServices = {
  /** Read-only material/appearance queries ({@link MaterialQueryService}). */
  MaterialQuery: contributionId<ServiceId>(RENDER_MODULE_ID, "onecad.render.material-query"),
} as const;
