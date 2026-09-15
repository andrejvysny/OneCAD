/*
 * `onecad.assistant` — the AI assistant built-in module.
 *
 * Built in, not an addon: the assistant talks to a supervised sidecar over the
 * Rust bridge (`docs/assistant/wire-protocol.md`), and ADR-0002 keeps a
 * privileged host transport inside a built-in module. Everything it puts on
 * screen is a slot contribution (ADR-0003).
 *
 * The module owns NO geometry surface: it never speaks OCW1, and the modeling
 * services stay behind their own module boundary.
 */
import { moduleId, type ModuleId } from "@/platform";

export const ASSISTANT_MODULE_ID: ModuleId = moduleId("onecad.assistant");

/**
 * The module's own state schema version, independent of the app version and of
 * the container format version (docs/ARCHITECTURE.md §8). Nothing per-document
 * is persisted yet — conversations live in the assistant host's own store, not
 * in the OneCAD document — so this only reserves the namespace.
 */
export const ASSISTANT_SCHEMA_VERSION = 1;
