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
}

export interface ToolDefinition extends Contribution {
  readonly id: ToolId;
  readonly title: string;
  readonly icon?: string;
  readonly defaultShortcut?: Shortcut;
  /**
   * How the chord is WRITTEN in the UI. Not derivable from `defaultShortcut`:
   * Measure binds ⇧/ and is shown as "?". Absent ⇒ the host formats the chord.
   */
  readonly shortcutLabel?: string;
  readonly scopes?: readonly string[];
  canActivate?(ctx: ToolContext): boolean;
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

/**
 * What a viewport contribution is handed. `invalidate` is the whole point: the
 * renderer is on-demand, so a contribution asks for a frame instead of running
 * its own animation loop (docs/ARCHITECTURE.md §7).
 */
export interface ViewportContext {
  invalidate(): void;
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
