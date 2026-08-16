/*
 * Which render dialog is open, and what it was opened on.
 *
 * A vanilla zustand store rather than React state for the same reason
 * `libraryModalStore` is one: the things that OPEN these dialogs are not
 * components. A `Slots.TreeContext` menu contribution, a viewport drop handler
 * and an inspector row all have to raise the same dialog from outside React,
 * and `RenderDialogHost` (a `Slots.ShellOverlay` panel) is the single mount
 * point that renders whichever one is open.
 *
 * THE OVERRIDE PROMPT IS A QUESTION, not a notification: its opener needs the
 * ANSWER before it can finish the assignment. So the request carries the
 * promise's `resolve`, and `requestOverrideChoice` awaits it — a dialog that
 * only set a flag would force every caller to re-implement "what do I do once
 * they answer". A second request while one is pending resolves the first as
 * `"cancel"`, so no caller is ever left awaiting a promise nothing will settle.
 */
import { createStore, useStore } from "zustand";

import type { MaterialId } from "../model/material";

/** What the keep/replace prompt needs to say. */
export interface OverridePromptRequest {
  readonly bodyId: string;
  /** Display name for the body, when the opener knows one (a tree row does). */
  readonly bodyLabel?: string;
  /** The material being assigned, or `null` for "None". */
  readonly materialId: MaterialId | null;
  /** Its display name — resolved by the opener, so the dialog reads no state. */
  readonly materialName: string;
  /** Face overrides on this body the viewport actually bound. */
  readonly overrideElementIds: readonly string[];
}

export type OverrideChoice = "keep" | "replace" | "cancel";

/** What "Assign material…" was opened on. */
export interface AssignTarget {
  readonly bodyId: string;
  readonly bodyLabel?: string;
}

interface PendingOverride {
  readonly request: OverridePromptRequest;
  readonly resolve: (choice: OverrideChoice) => void;
}

interface RenderDialogState {
  override: PendingOverride | null;
  assign: AssignTarget | null;
  /** The material being edited in the sheet, by id. */
  editor: MaterialId | null;

  requestOverrideChoice(request: OverridePromptRequest): Promise<OverrideChoice>;
  answerOverride(choice: OverrideChoice): void;
  openAssign(target: AssignTarget): void;
  closeAssign(): void;
  openEditor(materialId: MaterialId): void;
  closeEditor(): void;
  /** Close everything, answering any pending question as `"cancel"`. */
  reset(): void;
}

export const renderDialogStore = createStore<RenderDialogState>()((set, get) => ({
  override: null,
  assign: null,
  editor: null,

  requestOverrideChoice(request) {
    // Supersede rather than queue: the pending question belongs to a gesture the
    // user has already moved on from, and leaving its promise unsettled would
    // strand whatever awaited it.
    get().override?.resolve("cancel");
    return new Promise<OverrideChoice>((resolve) => {
      set({ override: { request, resolve } });
    });
  },

  answerOverride(choice) {
    const pending = get().override;
    if (pending === null) return;
    set({ override: null });
    pending.resolve(choice);
  },

  openAssign(target) {
    set({ assign: target });
  },

  closeAssign() {
    set({ assign: null });
  },

  openEditor(materialId) {
    set({ editor: materialId });
  },

  closeEditor() {
    set({ editor: null });
  },

  reset() {
    get().override?.resolve("cancel");
    set({ override: null, assign: null, editor: null });
  },
}));

export function useRenderDialogStore<T>(selector: (s: RenderDialogState) => T): T {
  return useStore(renderDialogStore, selector);
}
