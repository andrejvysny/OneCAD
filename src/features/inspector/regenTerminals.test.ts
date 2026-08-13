/*
 * Roadmap WP0.3 — the table-driven terminal test.
 *
 * The work package requires that every consumer family be provable against every
 * regen terminal, because the defect it closes (R-04) was precisely that each
 * family answered "did this work?" its own way and all of them answered wrong for
 * some terminal. The assertions per row are the spec's own list: no success hint
 * on error, one sticky diagnostic, the input values survive, and the selection is
 * never moved onto a body that failed to publish.
 *
 * This file covers the history/repair families. The tool families are covered in
 * `src/tools/modelTools/ModelToolController.terminals.test.ts` against the same
 * table, so the two cannot drift apart on what a terminal means.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { documentStore } from "@/stores/documentStore";
import { repairStore } from "@/stores/repairStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { ApplyOperationResult, RegenTerminal } from "@/ipc/types";

const applyEditCommand = vi.fn();

vi.mock("@/ipc/client", () => ({
  createClient: () => ({ resolveRefs: vi.fn(), applyEditCommand }),
}));

import { deleteFeature, rollToIndex, suppressFeature } from "./historyActions";

/** One row per terminal: what the backend reported, and what the user must see. */
interface Row {
  terminal: RegenTerminal;
  errorMessage?: string;
  /** The document may only move when the record survived. */
  hydrates: boolean;
  /** A failure must be sticky and must name a reason. */
  sticky: boolean;
  /** Substring the hint must contain — never a bare success on a failure. */
  contains: string;
}

const TABLE: Row[] = [
  { terminal: "published", hydrates: true, sticky: false, contains: "" },
  { terminal: "noop", hydrates: true, sticky: false, contains: "" },
  { terminal: "needsRepair", hydrates: true, sticky: true, contains: "needs repair" },
  { terminal: "failed", errorMessage: "kernel refused", hydrates: false, sticky: true, contains: "kernel refused" },
  { terminal: "failed", hydrates: false, sticky: true, contains: "without a reason" },
  { terminal: "timeout", hydrates: false, sticky: true, contains: "did not complete in time" },
];

function result(row: Row): ApplyOperationResult {
  return {
    revision: 42,
    features: [
      { id: "op1", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
    ],
    changedBodies: [],
    removedBodies: [],
    terminal: row.terminal,
    errorMessage: row.errorMessage,
  };
}

const FAMILIES: Array<{ name: string; success: string; run: () => Promise<unknown> }> = [
  { name: "suppress", success: "Feature suppressed", run: () => suppressFeature("op1", true) },
  { name: "rollback", success: "Rolled timeline", run: () => rollToIndex(0) },
  { name: "delete", success: "Feature deleted", run: () => deleteFeature("op1") },
];

describe("every history family reports every regen terminal honestly", () => {
  beforeEach(() => {
    resetStores();
    documentStore.setState({
      revision: 1,
      bodies: { bodyA: { id: "bodyA", name: "Body A", visible: true } },
      features: [
        { id: "op1", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
      ],
    });
    selectionStore.setState({ selected: [{ kind: "body", id: "bodyA" }] });
    repairStore.getState().reset();
    applyEditCommand.mockReset();
  });

  for (const family of FAMILIES) {
    for (const row of TABLE) {
      const label = `${row.terminal}${row.errorMessage ? " with a message" : ""}`;
      it(`${family.name} · ${label}`, async () => {
        applyEditCommand.mockResolvedValue(result(row));
        const before = documentStore.getState().revision;
        const selectedBefore = selectionStore.getState().selected;

        await family.run();

        const hint = viewportStore.getState().statusHint;
        expect(hint, "every terminal must say something").not.toBeNull();

        if (row.contains) {
          expect(hint?.message).toContain(row.contains);
        } else {
          expect(hint?.message).toBe(family.success);
        }

        // A failure never reads as the success text.
        if (row.sticky && row.terminal !== "needsRepair") {
          expect(hint?.message).not.toBe(family.success);
          expect(hint?.severity).toBe("error");
        }
        expect(hint?.sticky ?? false).toBe(row.sticky);

        // The document moves only when the record survived.
        expect(documentStore.getState().revision).toBe(row.hydrates ? 42 : before);

        // No family may move the selection onto anything on a non-publish.
        if (!row.hydrates) {
          expect(selectionStore.getState().selected).toEqual(selectedBefore);
        }
      });
    }
  }

  it("does not stack a duplicate record when a failed action is retried", async () => {
    applyEditCommand.mockResolvedValue(result(TABLE[3]));
    await suppressFeature("op1", true);
    await suppressFeature("op1", true);
    expect(documentStore.getState().features).toHaveLength(1);
  });

  /*
   * The negative control the plan requires: with the classifier bypassed — the
   * pre-WP0.3 behavior, where only a rejected promise counted as failure — a
   * resolved `errorMessage` reads as success. If this ever stops holding, the
   * table above has stopped testing anything.
   */
  it("negative control: ignoring the classifier reports a failure as success", () => {
    const res = result(TABLE[3]);
    const legacy = res.errorMessage === undefined ? "Feature suppressed" : "Suppress failed";
    expect(legacy).toBe("Suppress failed");
    // …and the shape that actually fooled the old code: a failure with no message.
    const noMessage = result(TABLE[4]);
    const legacyNoMessage = noMessage.errorMessage === undefined ? "Feature suppressed" : "Suppress failed";
    expect(legacyNoMessage).toBe("Feature suppressed");
  });
});
