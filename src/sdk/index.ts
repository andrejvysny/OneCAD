/*
 * `@onecad/sdk` — the public extension surface.
 *
 * This barrel is a CONTRACT, not a convenience re-export. Everything named here
 * is something OneCAD promises to keep working for an installed addon; anything
 * not named here is implementation that may move in any release.
 *
 * `@/platform` is the INTERNAL host API. It exposes `createPlatform`, the
 * registry objects, `disposeOwner`, `scopeFor`, `SlotHost`, `PlatformProvider`
 * and the document-state factory that takes a `CadClient`. Built-in modules,
 * compiled into the app and versioned with it, may use all of that. An addon may
 * not: whatever an addon can reach is what we must keep working forever, and the
 * host's own composition root is not on that list.
 *
 * WHAT IS DELIBERATELY ABSENT — the list is the point of the file:
 *   createPlatform, Platform, ModuleScope   the host itself
 *   Registry / createRegistry              mutate your own registrations only
 *   disposeOwner, scopeFor, createScope    an addon's lifetime is the host's
 *   PlatformProvider, SlotHost             the shell composes, addons contribute
 *   createDocumentStateService             leaks `CadClient` — use context.document
 *   CadClient, invoke, any store, engine   application implementation
 *   KnownOperation, worker protocol        closed in v1 (ADR-0002)
 *
 * `three` and `react` are HOST-PROVIDED. A viewport contribution hands the host
 * `Object3D`s, so a second Three.js runtime inside an addon bundle is a
 * correctness bug, not a size problem (ADR-0009).
 */

// ── Identity ─────────────────────────────────────────────────────────────────
// Branded ids plus the checked constructors. `addonId` and `contributionId`
// enforce the namespace rule (an addon cannot mint `onecad.*`) at the only place
// an addon can create one.
export {
  addonId,
  contributionId,
  isOwnerIdShape,
  IdError,
  type AddonId,
  type CommandId,
  type EntityId,
  type EventId,
  type InspectorContributionId,
  type OwnerId,
  type PanelId,
  type ServiceId,
  type ToolId,
  type TreeProviderId,
  type TypeId,
  type ViewportContributionId,
  type WorkspaceId,
} from "@/platform/ids";

// ── Lifetimes ────────────────────────────────────────────────────────────────
export { DEFAULT_PRIORITY, type Contribution, type Disposable } from "@/platform/registry";

// ── Contribution contracts ───────────────────────────────────────────────────
export type {
  CommandAvailability,
  CommandContext,
  CommandDefinition,
  CommandGroup,
  CommandResult,
  ContributionComponent,
  InspectorContext,
  InspectorContribution,
  PanelContribution,
  PanelPlacement,
  SelectionRef,
  Shortcut,
  ToolAvailability,
  ToolContext,
  ToolDefinition,
  TreeNode,
  TreeNodeAction,
  TreeProvider,
  TreeSection,
  ViewportContext,
  ViewportContribution,
  ViewportLabelHandle,
  WorkspaceDefinition,
} from "@/platform/contributions";

// ── Where a panel may go ─────────────────────────────────────────────────────
// The slot list is CLOSED (ADR-0003): a contribution picks a host-owned region,
// it does not describe a layout of its own.
export { Slots, isSlotId, type SlotId } from "@/platform/slots";

// ── The extension surface ────────────────────────────────────────────────────
// `createExtensionContext` and `registerExtension` are host-side and NOT here:
// an addon that could build its own context could hand itself the scope the
// context exists to hide.
export type {
  ExtensionContext,
  ExtensionDefinition,
  ExtensionDocumentApi,
  ExtensionEventApi,
  ExtensionServiceApi,
  ExtensionToolHostApi,
} from "@/platform/extension";

export type {
  DocumentStateService,
  ModuleDocumentState,
} from "@/platform/documentState";
