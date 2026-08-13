/*
 * `onecad.render` — stub module for the future Render workspace.
 *
 * Registers with zero contributions (`module.ts`). No panels, tools, commands,
 * workspace, or document state exist yet — this file only reserves the module's
 * identity and schema-version namespace so future work has a stable home from
 * day one. See `docs/RENDER_MODULE.md` for the design draft and
 * `docs/adr/0014-render-module-openpbr.md` for the governing decisions.
 */
import { moduleId, contributionId, type ModuleId, type ServiceId } from "@/platform";

export const RENDER_MODULE_ID: ModuleId = moduleId("onecad.render");

/**
 * The module's own state schema version, independent of the app version and of
 * the container format version (docs/ARCHITECTURE.md §8). Unused until the
 * module actually reads/writes document state.
 */
export const RENDER_SCHEMA_VERSION = 1;

/**
 * Services this module intends to publish once it has a material/render data
 * model. Documentation + diagnostics only — nothing is registered yet.
 */
export const RenderServices = {
  /** Read-only material/appearance queries, once a material store exists. */
  MaterialQuery: contributionId<ServiceId>(RENDER_MODULE_ID, "onecad.render.material-query"),
} as const;
