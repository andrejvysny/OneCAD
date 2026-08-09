/*
 * Contribution contracts.
 *
 * These types are deliberately domain-neutral: nothing here knows Extrude,
 * Fillet, sketches or bodies, and nothing here may learn them. A modeling concept
 * that leaks into this file is the failure mode the whole refactor exists to
 * prevent (docs/ARCHITECTURE.md §3).
 *
 * The split that matters:
 *   Command = an action with identity, availability and metadata.
 *   Tool    = an interaction lifecycle (a mode / state machine).
 * A tool may invoke commands. One command can feed a toolbar, a context menu, a
 * palette, a shortcut and addon automation without knowing any of them exist.
 */
import type { ComponentType } from "react";
// Type-only: see ADR-0009. v1 has one host renderer, and an in-scene
// contribution is a Three.js object by construction (slots.ts).
import type { Camera, Object3D, Raycaster } from "three";
import type { Contribution, Disposable } from "./registry";
import type { SlotId } from "./slots";
import type {
  CommandId,
  EntityId,
  InspectorContributionId,
  ModuleId,
  PanelId,
  ToolId,
  TypeId,
  ViewportContributionId,
  TreeProviderId,
  WorkspaceId,
} from "./ids";

// ── Input ────────────────────────────────────────────────────────────────────

/**
 * A key chord. Deliberately not mouse- or platform-specific: tools receive
 * pointer-level events, not `onMouseDown`, so touch and pencil stay possible.
 */
export interface Shortcut {
  /** Single printable key (compared case-insensitively) or a named key. */
  readonly key: string;
  readonly shift?: boolean;
  readonly alt?: boolean;
  readonly ctrl?: boolean;
  readonly meta?: boolean;
}

// ── Selection ────────────────────────────────────────────────────────────────

/**
 * Platform-level selection identity. The owning module interprets `subElement`;
 * the platform only routes it.
 *
 * Note this is NOT modeling's `ElementId`: that identifies a persistent
 * topological element with guarantees the platform does not make (ADR-0002).
 * The two coexist rather than collapsing into one id space.
 */
export interface SelectionRef {
  readonly entityId: EntityId;
  readonly owner: ModuleId;
  readonly typeId: TypeId;
  readonly subElement?: string;
}

// ── Commands ─────────────────────────────────────────────────────────────────

export interface CommandAvailability {
  readonly enabled: boolean;
  /** Shown to the user when disabled. A disabled command should say why. */
  readonly reason?: string;
}

export type CommandResult =
  | { readonly status: "done" }
  | { readonly status: "cancelled" }
  | { readonly status: "failed"; readonly message: string };

export interface CommandContext {
  readonly selection: readonly SelectionRef[];
  /** Opaque scope tokens the host is currently in (e.g. an editor mode). */
  readonly scopes: readonly string[];
}

export interface CommandDefinition extends Contribution {
  readonly id: CommandId;
  readonly title: string;
  readonly description?: string;
  /** Extra search terms for a command palette. */
  readonly keywords?: readonly string[];
  readonly icon?: string;
  readonly defaultShortcut?: Shortcut;
  /**
   * Scopes in which the command is offered. Empty/absent ⇒ always. The strings
   * are opaque to the platform; the shell decides what they mean.
   */
  readonly scopes?: readonly string[];
  canExecute?(ctx: CommandContext): CommandAvailability;
  execute(ctx: CommandContext): CommandResult | Promise<CommandResult>;
}

// ── Tools ────────────────────────────────────────────────────────────────────

export interface ToolContext {
  readonly selection: readonly SelectionRef[];
  /** The same opaque scope tokens `CommandContext` carries. */
  readonly scopes: readonly string[];
}

/**
 * Same shape as `CommandAvailability`, and deliberately so: a toolbar that grays
 * a tool out must be able to say WHY, exactly as a disabled command does. A bare
 * boolean would have thrown that away the moment the toolbar stopped reading
 * modeling's `getToolApplicability` (which has carried a reason all along).
 */
export type ToolAvailability = CommandAvailability;

export interface ToolDefinition extends Contribution {
  readonly id: ToolId;
  readonly title: string;
  readonly icon?: string;
  /**
   * Opaque family key: tools sharing one render as a single split button whose
   * menu lists every member (e.g. rectangle-by-diagonal vs by-centre). Absent ⇒
   * standalone button. Like `group` and `CommandDefinition.scopes`, the strings
   * are opaque to the platform — it only routes them past the toolbar.
   */
  readonly flyout?: string;
  readonly defaultShortcut?: Shortcut;
  /**
   * How the chord is WRITTEN in the UI. Not derivable from `defaultShortcut`:
   * Measure binds ⇧/ and is shown as "?". Absent ⇒ the host formats the chord.
   */
  readonly shortcutLabel?: string;
  readonly scopes?: readonly string[];
  /**
   * Absent ⇒ always available. The host must not gate the ACTIVE tool on this:
   * a tool whose own arm handshake changes the selection would otherwise disable
   * itself mid-gesture.
   */
  canActivate?(ctx: ToolContext): ToolAvailability;
  /**
   * Announce that `canActivate`'s ANSWER may have changed.
   *
   * `canActivate` is a plain predicate read during the host's render, exactly
   * like `TreeProvider.sections()` — so a tool whose availability depends on
   * state the host does not otherwise watch MUST implement this, or its button
   * will keep showing the answer it had at mount. Absent ⇒ the answer never
   * changes on its own.
   */
  subscribe?(onChange: () => void): Disposable;
  activate(ctx: ToolContext): void | Promise<void>;
  deactivate(ctx: ToolContext): void | Promise<void>;
}

// ── UI contributions ─────────────────────────────────────────────────────────

/** A component the host mounts with no props of its own. */
export type ContributionComponent = ComponentType<object>;

export interface PanelContribution extends Contribution {
  readonly id: PanelId;
  readonly slot: SlotId;
  /** Shown in a panel frame that has a header; overlays usually omit it. */
  readonly title?: string;
  readonly component: ContributionComponent;
}

export interface InspectorContext {
  readonly selection: readonly SelectionRef[];
  /**
   * Opaque scope tokens active right now — the SAME currency as
   * `ToolDefinition.scopes`, and just as uninterpreted here. A section that is
   * only meaningful inside its module's mode gates on one of these; the
   * alternative was an `InspectorContext.mode`, which would teach the platform
   * a modeling concept it has no business knowing.
   */
  readonly scopes: readonly string[];
}

export interface InspectorContribution extends Contribution {
  readonly id: InspectorContributionId;
  readonly title?: string;
  canRender(ctx: InspectorContext): boolean;
  readonly component: ContributionComponent;
}

// ── Model tree ───────────────────────────────────────────────────────────────

/**
 * One row. The host renders it and routes the gestures back; it never learns
 * what the row IS.
 *
 * `select` and `activate` are SEPARATE because click and double-click mean
 * different things on every row shipped today — a sketch row selects on click
 * and re-opens its sketch on double-click. Collapsing them would either change
 * behavior or push the owner's selection semantics back into the generic panel.
 *
 * An ABSENT capability is the way to say a row does not have it: a datum carries
 * no visibility fact in the document, so it supplies no `toggleVisible` and the
 * host renders no eye. That is different from `visible: false`.
 */
/**
 * A row-scoped menu item.
 *
 * `commandId` rather than a callback, so the same action is reachable from a
 * palette, a keystroke and automation instead of existing only inside one
 * popover — the split docs/ARCHITECTURE.md §6 draws between a command and the UI
 * that offers it.
 *
 * WHICH ROW: opening a row's menu SELECTS that row first (the host calls
 * `select()`), so the command reads its target through the ordinary selection
 * path rather than through a private argument only this menu could supply.
 *
 * `confirm` makes the two-click destructive idiom declarative: the host renders
 * this label as a second step before running anything. Without it, "delete"
 * items would each re-implement their own confirmation and drift.
 */
export interface TreeNodeAction {
  readonly id: string;
  readonly title: string;
  readonly commandId: CommandId;
  readonly icon?: string;
  /** Rendered as destructive. */
  readonly danger?: boolean;
  /** Consumer-visible cluster — the host draws a separator between groups. */
  readonly group?: string;
  /** Present ⇒ the item first becomes this label, and only then runs. */
  readonly confirm?: string;
}

export interface TreeNode {
  readonly id: string;
  readonly label: string;
  readonly icon: string;
  /** Owner-defined row category. Opaque to the host. */
  readonly kind: string;
  readonly selected: boolean;
  /** Rendered dimmed — shown but not currently in play. */
  readonly dimmed?: boolean;
  /**
   * A short trailing value the row carries (a load magnitude, "not installed").
   * Right-aligned mono, and NOT a second label: the host truncates the label
   * before it truncates this, so a row whose meaning lives in the value keeps it.
   */
  readonly meta?: string;
  /**
   * The row names something wrong (missing owner, unresolved reference). The
   * host tints it with the caution treatment. Opaque to the host beyond that —
   * WHAT is wrong is the owner's to say, through `meta` or an action.
   */
  readonly problem?: boolean;
  /** Absent ⇒ this row has no visibility fact at all. */
  readonly visible?: boolean;
  select(): void;
  activate?(): void;
  toggleVisible?(next: boolean): void;
  rename?(next: string): void;
  /** Extra context-menu items beyond the capabilities above. */
  readonly actions?: readonly TreeNodeAction[];
}

export interface TreeSection {
  readonly id: string;
  readonly title: string;
  readonly nodes: readonly TreeNode[];
  /**
   * How the section OPENS the first time this workspace shows it. The user's
   * later expand/collapse wins and is remembered by the host — this is a
   * default, not a controlled value, which is why the provider is not asked to
   * hold the open/closed state it would otherwise have to thread through
   * `sections()` on every render.
   */
  readonly defaultCollapsed?: boolean;
  /**
   * Shown under the section when it has no rows — "Run Solve to see results."
   * An empty section with nothing to say renders as a bare header instead.
   */
  readonly emptyNote?: string;
}

/**
 * A source of tree rows. `sections()` is a PROJECTION read during the host's
 * render — it must be cheap and must not subscribe to anything itself.
 *
 * `subscribe` is how a provider says its rows changed. It is optional only
 * because the built-in modeling provider is backed by stores the host already
 * watches for its own reasons; ANY provider whose data the host does not
 * otherwise observe MUST implement it, or its rows will silently never update
 * after mount.
 */
export interface TreeProvider extends Contribution {
  readonly id: TreeProviderId;
  sections(): readonly TreeSection[];
  subscribe?(onChange: () => void): Disposable;
}

/**
 * A projected HTML label the host positions every frame. The host owns the
 * overlay element and the projection; a contribution owns the element's content.
 *
 * This exists so a layer never receives the overlay HOST — handing out the
 * engine's driver would put a renderer internal in the platform's public
 * surface, and the label would then be positioned by whoever grabbed it.
 */
export interface ViewportLabelHandle {
  /** Move the anchor. Coordinates are world-space, Z-up (docs/ARCHITECTURE.md §7). */
  setPosition(world: { x: number; y: number; z: number }): void;
  dispose(): void;
}

/**
 * What a viewport contribution is handed.
 *
 * `invalidate` is the point of the on-demand renderer: a contribution asks for a
 * frame instead of running its own animation loop (docs/ARCHITECTURE.md §7).
 * Everything else here exists because a real in-scene layer needs it — the datum
 * layer, ported onto this contract, needs every one.
 *
 * The Three.js types are a deliberate, type-only coupling: v1 has exactly one
 * host renderer and `slots.ts` already describes in-scene contributions as
 * Three.js objects (ADR-0009). The forbidden edge is `@/viewport`, not the
 * rendering library.
 */
export interface ViewportContext {
  invalidate(): void;
  /** Host-owned group. A contribution adds its own objects and removes them on dispose. */
  readonly root: Object3D;
  createLabel(element: HTMLElement): ViewportLabelHandle;
  /**
   * Runs inside the host's render loop, at the layer position in the painter's
   * ladder. NOT a place to start an animation — it fires only on frames the host
   * was already going to draw.
   */
  onFrame(cb: (camera: Camera, viewportHeight: number) => void): Disposable;
  /**
   * Fires after the host has switched palettes. A layer that reads theme colors
   * MUST subscribe: the host cannot tell that it did not, so a missed
   * subscription is a silent staleness bug (see viewport/engine/README.md).
   */
  onThemeChange(cb: () => void): Disposable;
  /**
   * A raycaster already set from a client point, or null when the point is
   * outside the canvas. Contributions do not get the camera's projection maths.
   */
  raycastFromClient(clientX: number, clientY: number): Raycaster | null;
  /**
   * Contribute a hover token for the host's pick pass, consulted only when
   * nothing closer was hit. Built-ins are asked first, then contributions in
   * registry order, and the first non-null answer wins.
   */
  registerSecondaryHover(fn: (clientX: number, clientY: number) => string | null): Disposable;
}

export interface ViewportContribution extends Contribution {
  readonly id: ViewportContributionId;
  attach(ctx: ViewportContext): Disposable;
}

// ── Workspaces ───────────────────────────────────────────────────────────────

export interface PanelPlacement {
  readonly panelId: PanelId;
  readonly slot: SlotId;
  /** Absent ⇒ visible. */
  readonly visible?: boolean;
}

export interface CommandGroup {
  readonly id: string;
  readonly title?: string;
  readonly commands: readonly CommandId[];
}

export interface WorkspaceDefinition extends Contribution {
  readonly id: WorkspaceId;
  readonly title: string;
  readonly icon?: string;
  readonly panels: readonly PanelPlacement[];
  readonly commandGroups?: readonly CommandGroup[];
}
