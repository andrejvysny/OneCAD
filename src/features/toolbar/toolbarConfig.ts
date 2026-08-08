/*
 * Floating-toolbar tool sets.
 *
 * The arrangement itself now lives with the module that owns the tools
 * (`@/modules/modeling/tools`); this file is the toolbar's view of it. The
 * exported shapes are unchanged, so every call site and every test keeps working
 * — what moved is ownership, not behavior.
 *
 * Separators are derived, not authored: a group is a contiguous run of
 * priorities, and a separator is drawn wherever the group changes (ADR-0007).
 */
import type { IconName } from "@/icons/paths";
import type { EditorMode, Tool } from "@/stores/toolStore";
import {
  descriptorsForScope,
  type ModelingToolDescriptor,
  type ToolScope,
} from "@/modules/modeling/tools";

export interface ToolItem {
  id: Tool;
  icon: IconName;
  label: string;
  shortcut: string;
}

export interface ToolSeparator {
  sep: true;
}

export type ToolEntry = ToolItem | ToolSeparator;

export function isSeparator(e: ToolEntry): e is ToolSeparator {
  return "sep" in e;
}

function itemOf(d: ModelingToolDescriptor): ToolItem {
  return { id: d.tool, icon: d.icon, label: d.label, shortcut: d.shortcut };
}

/** Descriptors in registry order, with a separator at each group boundary. */
export function toolEntriesFor(descriptors: readonly ModelingToolDescriptor[]): ToolEntry[] {
  const entries: ToolEntry[] = [];
  let group: string | null = null;
  for (const d of descriptors) {
    if (group !== null && d.group !== group) entries.push({ sep: true });
    group = d.group;
    entries.push(itemOf(d));
  }
  return entries;
}

function entriesForScope(scope: ToolScope): ToolEntry[] {
  return toolEntriesFor(descriptorsForScope(scope));
}

export const MODEL_TOOLS: ToolEntry[] = entriesForScope("model");
export const SKETCH_TOOLS: ToolEntry[] = entriesForScope("sketch");

export function toolsForMode(mode: EditorMode): ToolEntry[] {
  return mode === "sketch" ? SKETCH_TOOLS : MODEL_TOOLS;
}
