/*
 * UX review 2026-09-14 — N7/N8: the completed-operation recap outlived its
 * moment.
 *
 * After a commit the read-only "Extrude / Completed" block stayed pinned to the
 * top of the inspector, so selecting a face afterwards showed the PREVIOUS
 * operation's targets above the new selection (A-076, A-117) — and after a
 * fillet the lower half read "Nothing selected" while the top still reported the
 * finished operation (A-120). The first selection write after a `completed`
 * attempt is the moment the recap stops being about anything on screen.
 *
 * `applying` and `failed` keep their priority: an application in flight and a
 * failure the user has not read yet are both still true. In particular a
 * history-row RE-EDIT never passes through `activateTool` (the only caller of
 * `operationAttemptStore.clear()`), so its `applying` phase must survive a
 * selection write that lands while it is in flight.
 */
import { act, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { InspectorPanel } from "./InspectorPanel";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { operationAttemptStore } from "@/stores/operationAttemptStore";
import { activeToolPresentation } from "@/tools/modelTools/activeToolPresentation";
import { resetStores } from "@/test/resetStores";
import { setMockLatency } from "@/ipc/mockClient";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { contributeInspectorSections } from "@/modules/modeling/inspectorSections";

const FACE = {
  kind: "face" as const,
  id: "body1#f:0",
  bodyId: "body1",
  topoKey: "f:0",
  elementId: "el_top",
};

/** Arm an edge op, then settle the attempt it started at `phase`. */
function commitEdgeOp(phase: "completed" | "applying"): void {
  act(() => {
    toolChipStore
      .getState()
      .showFillet(2, [0, 0, 0], vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }, { edgeOp: "Fillet" });
    const token = operationAttemptStore
      .getState()
      .begin(documentStore.getState().documentId, activeToolPresentation(toolChipStore.getState()));
    if (token === null) throw new Error("an attempt was already applying");
    if (phase === "completed") operationAttemptStore.getState().settle(token, "completed");
    toolChipStore.getState().clear();
  });
}

describe("completed recap vs the next selection", () => {
  beforeEach(() => {
    resetStores();
    setMockLatency(0);
  });

  it("drops the completed recap on the first selection write after it", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, {
      contribute: contributeInspectorSections,
    });
    commitEdgeOp("completed");
    expect(screen.getByText(/^Completed/)).toBeInTheDocument();

    act(() => selectionStore.getState().set([FACE]));

    expect(screen.queryByText(/^Completed/)).toBeNull();
    expect(operationAttemptStore.getState().attempt).toBeNull();
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    const sections = [...container.querySelectorAll("div")]
      .filter((el) => el.className.includes("tracking-[0.07em]"))
      .map((el) => el.textContent?.trim() ?? "");
    expect(sections).toContain("History");
  });

  it("keeps an in-flight re-edit's recap across a selection write", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    commitEdgeOp("applying");

    act(() => selectionStore.getState().set([FACE]));

    expect(operationAttemptStore.getState().attempt?.phase).toBe("applying");
    expect(screen.getByText("Applying…")).toBeInTheDocument();
  });
});
