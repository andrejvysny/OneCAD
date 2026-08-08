/*
 * The toolbar ENTRY SHAPE — and nothing else.
 *
 * This file no longer configures anything: the arrangement is built from the
 * platform tool registry at render time (`@/modules/modeling/registryToolbar`).
 * What survives here is the type the frozen contract is written in
 * (`src/test/contracts/toolbarContract.ts` imports `ToolEntry` from this path),
 * so the file stays put — moving it would mean editing a contract file, which
 * `src/test/contracts/README.md` forbids.
 *
 * Separators are still derived rather than authored: a group is a contiguous run
 * of priorities, and a separator is drawn wherever the group changes (ADR-0007).
 * That derivation now lives with the producer.
 */
import type { IconName } from "@/icons/paths";
import type { Tool } from "@/stores/toolStore";

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
