/*
 * `onecad.modeling` — the first built-in module.
 *
 * Compiled in rather than loaded from a package, but it carries manifest-shaped
 * metadata anyway so built-ins and addons share one conceptual model (spec §107).
 * Nothing privileged is expressed here: what makes modeling privileged is that
 * the kernel is closed to everyone else (ADR-0002), not a flag in this file.
 */
import { moduleId, contributionId, type ModuleId, type ServiceId } from "@/platform";

export const MODELING_MODULE_ID: ModuleId = moduleId("onecad.modeling");

/**
 * The module's own state schema version, independent of the app version and of
 * the container format version (docs/ARCHITECTURE.md §8).
 */
export const MODELING_SCHEMA_VERSION = 1;

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
