/*
 * `onecad.modeling` — the first built-in module.
 *
 * Compiled in rather than loaded from a package, but it carries manifest-shaped
 * metadata anyway so built-ins and addons share one conceptual model (spec §107).
 * Nothing privileged is expressed here: what makes modeling privileged is that
 * the kernel is closed to everyone else (ADR-0002), not a flag in this file.
 */
import { moduleId, contributionId, type ModuleId, type ServiceId } from "@/platform";
import type {
  ApplyOperationResult,
  ClassifyResult,
  ComponentParamValue,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  PlaceComponentMate,
  PreviewSession,
  TransformRotationParams,
  Unsubscribe,
} from "@/ipc/types";

export const MODELING_MODULE_ID: ModuleId = moduleId("onecad.modeling");

/**
 * The module's own state schema version, independent of the app version and of
 * the container format version (docs/ARCHITECTURE.md §8).
 */
export const MODELING_SCHEMA_VERSION = 1;

/**
 * `ModelingServices.GeometryQuery`'s contract (Component Library WP-0.1, the
 * first real registration — see `register.ts::contributeModeling`).
 *
 * Deliberately narrow: just the classification the placement/mate-snap
 * solver needs. Widen it as further read-only geometry consumers arrive
 * rather than pre-building a general query surface no one calls yet.
 */
export interface GeometryQueryService {
  /** `CadClient.classifyElement`, addressed by ElementId or `{bodyId, topoKey}`. */
  classifyElement(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<ClassifyResult | null>;
}

/**
 * `ModelingServices.CommandApi`'s contract (Component Library WP-1.3, the
 * first real registration — see `register.ts::contributeModeling`).
 *
 * Deliberately narrow: ADR-0002 requires anything that touches the kernel to
 * route through modeling's published services, and these two are the only
 * KERNEL-TOUCHING library operations today (both mint/edit a
 * `KnownOperation` record). Pure library metadata reads
 * (`listLibraryComponents`/`reindexLibrary`) never reach the kernel, so
 * library UI calls `CadClient` directly for those rather than routing
 * through here — widen this surface only as further kernel-touching
 * component operations arrive (`setComponentParams`/`replaceComponent`).
 */
export interface CommandApiService {
  /**
   * `CadClient.placeComponent` — spec §3.1 `PlaceComponent`. `params` carries
   * the gesture's own free-parameter overrides (auto-size on a hole rim,
   * WP-A3); the backend validates them against the component's signature.
   */
  placeComponent(
    componentId: string,
    componentVersion: string,
    translate: [number, number, number],
    rotate?: TransformRotationParams,
    params?: Record<string, ComponentParamValue>,
    mate?: PlaceComponentMate,
  ): Promise<ApplyOperationResult>;
  /** `CadClient.detachComponent` — spec §3.4 `DetachComponent`. */
  detachComponent(recordId: string): Promise<ApplyOperationResult>;
  /**
   * The generic kernel-preview lane (`CadClient.beginPreview`/`updatePreview`/
   * `endPreview`/`onPreviewResult`), routed through here per ADR-0002 —
   * library's placement drag ghost (WP-1.5) is a further kernel-touching
   * component operation, same reasoning as `placeComponent` above. Never
   * commits through `endPreview`: the ghost session is always cancelled, the
   * real commit goes through `placeComponent`.
   */
  beginPreview(draft: PreviewDraft): Promise<PreviewSession>;
  updatePreview(sessionId: string, params: PreviewParams, epoch: number): void;
  endPreview(sessionId: string, commit: boolean): Promise<ApplyOperationResult | null>;
  onPreviewResult(cb: (r: PreviewResult) => void): Unsubscribe;
}

/** Services this module intends to publish. Documentation + diagnostics. */
export const ModelingServices = {
  /** Read-only geometry questions: bodies, meshes, bounds, shape metadata. */
  GeometryQuery: contributionId<ServiceId>(
    MODELING_MODULE_ID,
    "onecad.modeling.geometry-query",
  ),
  /** Invoking supported modeling operations through the transaction layer. */
  CommandApi: contributionId<ServiceId>(MODELING_MODULE_ID, "onecad.modeling.command-api"),
} as const;

/**
 * Opaque scope tokens. The platform never interprets these — it only routes them
 * — which is what keeps "model" and "sketch" out of platform code.
 */
export const ModelingScopes = {
  Model: "onecad.modeling.mode.model",
  Sketch: "onecad.modeling.mode.sketch",
} as const;

export type ModelingScope = (typeof ModelingScopes)[keyof typeof ModelingScopes];
