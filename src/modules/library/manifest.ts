/*
 * `onecad.library` — the Component Library built-in module (WP-1.4).
 *
 * Built in, not an addon: placing/detaching a component mints/edits a
 * `KnownOperation` record, which ADR-0002 reserves to built-in modules.
 * Everything this module puts on screen is a slot contribution (ADR-0003).
 */
import { moduleId, type ModuleId } from "@/platform";

export const LIBRARY_MODULE_ID: ModuleId = moduleId("onecad.library");

/**
 * The module's own state schema version, independent of the app version and
 * of the container format version (docs/ARCHITECTURE.md §8). No document
 * state of its own yet (P1 has nothing to persist per-document — the
 * library root and its index live outside any document); bumped when P3's
 * authoring flow gives it one.
 */
export const LIBRARY_SCHEMA_VERSION = 1;
