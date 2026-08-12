/*
 * Library inspector-section ids, split out for the same reason `panelIds.ts`
 * is — anything that only needs an id must not drag the editor component in.
 */
import { contributionId, type InspectorContributionId } from "@/platform";
import { LIBRARY_MODULE_ID } from "./manifest";

const sectionId = (name: string) =>
  contributionId<InspectorContributionId>(LIBRARY_MODULE_ID, `onecad.library.inspector.${name}`);

export const LibraryInspectorSections = {
  ComponentParameters: sectionId("componentParameters"),
} as const;

/**
 * Sits after modeling's History (200) and before Constraints (300) — a
 * PlaceComponent feature never carries sketch constraints, so the two never
 * actually compete, but the number keeps this section in the same visual
 * neighborhood as History for a selected feature.
 */
export const LibraryInspectorPriorities = {
  ComponentParameters: 250,
} as const;
