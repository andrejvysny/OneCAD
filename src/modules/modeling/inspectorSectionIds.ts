/*
 * Modeling inspector-section ids, split out from the registration file for the
 * same reason `panelIds.ts` is: anything that only needs an ID (a test, a
 * future command palette) must not drag the editor components in.
 *
 * PRIORITIES ARE THE CONTRACT. `src/test/contracts/inspectorContract.ts` freezes
 * the section order per selection state, and registry order is
 * `(priority, insertion index)` — so a pair that can never render together may
 * share a priority, but two sections that CAN both appear must not.
 */
import { contributionId, type InspectorContributionId } from "@/platform";
import { MODELING_MODULE_ID } from "./manifest";

const sectionId = (name: string) =>
  contributionId<InspectorContributionId>(
    MODELING_MODULE_ID,
    `onecad.modeling.inspector.${name}`,
  );

export const ModelingInspectorSections = {
  /** Body and face Appearance are mutually exclusive — one selection, one kind. */
  AppearanceBody: sectionId("appearance.body"),
  AppearanceFace: sectionId("appearance.face"),
  Dimensions: sectionId("dimensions"),
  /** Lineage slice for a selected body/sketch vs the whole timeline for a feature. */
  HistorySelection: sectionId("history.selection"),
  HistoryFeature: sectionId("history.feature"),
  /** The model-mode prompt vs the live sketch-mode list. */
  ConstraintsHint: sectionId("constraints.hint"),
  ConstraintsList: sectionId("constraints.list"),
  Dependencies: sectionId("dependencies"),
} as const;

export const ModelingInspectorPriorities = {
  Appearance: 100,
  Dimensions: 150,
  History: 200,
  Constraints: 300,
  Dependencies: 400,
} as const;
