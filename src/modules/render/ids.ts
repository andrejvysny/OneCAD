/*
 * Render contribution id namespace, split from `module.ts` for the same reason
 * `panelIds.ts`/`ids.ts` are split in the other modules: something that only
 * needs an id must not drag in whatever the module eventually imports for its
 * real registrations.
 *
 * `ui/register.ts` imports every UI component, so anything that only needs an
 * ID (a test, a future workspace definition) must import from HERE.
 */
import {
  contributionId,
  type InspectorContributionId,
  type MenuContributionId,
  type PanelId,
} from "@/platform";
import { RENDER_MODULE_ID } from "./manifest";

export const renderPanelId = (name: string): PanelId =>
  contributionId<PanelId>(RENDER_MODULE_ID, `onecad.render.panel.${name}`);

const sectionId = (name: string): InspectorContributionId =>
  contributionId<InspectorContributionId>(RENDER_MODULE_ID, `onecad.render.inspector.${name}`);

const menuItemId = (name: string): MenuContributionId =>
  contributionId<MenuContributionId>(RENDER_MODULE_ID, `onecad.render.menu.${name}`);

export const RenderPanels = {
  /** The left-sidebar "Materials" tab (`Slots.ShellLeft`). */
  MaterialLibrary: renderPanelId("materialLibrary"),
  /**
   * The `Slots.ShellOverlay` occupant that owns every render dialog (the
   * override keep/replace prompt, the tree-menu material picker, the editor).
   * ONE overlay rather than three: they are mutually exclusive by construction
   * — each is opened by the user's last gesture — and a single mount point
   * keeps the overlay's stacking order a single decision.
   */
  DialogHost: renderPanelId("dialogHost"),
} as const;

export const RenderInspectorSections = {
  /** Body and face Material are mutually exclusive — one selection, one kind. */
  MaterialBody: sectionId("material.body"),
  MaterialFace: sectionId("material.face"),
} as const;

/**
 * PRIORITIES ARE THE CONTRACT. `src/test/contracts/inspectorContract.ts` freezes
 * the section order per selection state, and registry order is
 * `(priority, insertion index)`.
 *
 * 120 sits between modeling's Appearance (100) and its Dimensions (150) /
 * History (200): "what colour is this?" and "what is this made of?" are the same
 * question asked twice, so they belong next to each other, and neither may push
 * History off the top of a feature selection.
 */
export const RenderInspectorPriorities = {
  Material: 120,
} as const;

export const RenderMenuItems = {
  /** "Assign material…" on a body row (`Slots.TreeContext`). */
  AssignMaterial: menuItemId("assignMaterial"),
} as const;
