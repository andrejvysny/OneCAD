/*
 * ModelTreePanel — the DATUMS section (DATUM W1): render, select, the
 * sketch-on-datum double-click, and the two-click delete (including the
 * referenced-guard rejection surfacing as a sticky hint with nothing removed).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelTreePanel } from "./ModelTreePanel";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore, type DatumMeta } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { mockClient, mockAttachSketchToDatum } from "@/ipc/mockClient";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { contributeModelingTree } from "@/modules/modeling/treeProvider";

function datum(id: string, name: string): DatumMeta {
  return {
    id,
    name,
    basePlaneId: "XY",
    offset: 10,
    plane: { kind: "custom", origin: [0, 0, 10], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
    resolvedValid: true,
  };
}

const datumOptions = () => screen.getByRole("listbox", { name: "Datums" });

describe("ModelTreePanel — Datums section", () => {
  beforeEach(() => {
    resetStores();
    documentStore.getState().addDatum(datum("d1", "Datum 1"));
  });
  afterEach(() => {
    documentStore.getState().applyChange({ datums: {} });
    mockAttachSketchToDatum("sk-hosted", null);
  });

  it("renders a row per projection datum, with NO eye (a datum has no visibility)", () => {
    documentStore.getState().addDatum(datum("d2", "Datum 2"));
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    const rows = screen.getAllByRole("option", { name: /Datum \d/ });
    expect(rows).toHaveLength(2);
    expect(datumOptions()).toContainElement(rows[0]);
    expect(screen.queryByRole("switch", { name: /Datum 1/ })).not.toBeInTheDocument();
  });

  it("the section is empty (but present) when the document has no datums", () => {
    documentStore.getState().applyChange({ datums: {} });
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    expect(datumOptions()).toBeEmptyDOMElement();
  });

  it("a click selects the datum ref", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.click(screen.getByRole("option", { name: /Datum 1/ }));
    expect(selectionStore.getState().selected).toEqual([{ kind: "datum", id: "d1" }]);
    expect(screen.getByRole("option", { name: /Datum 1/ })).toHaveAttribute("aria-selected", "true");
  });

  it("a DOUBLE-click selects the datum AND enters sketch mode with no target (the on-datum entry)", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.dblClick(screen.getByRole("option", { name: /Datum 1/ }));
    // The controller's tryEnterOnSelectedDatum reads the selection; activeSketchId
    // must stay null so it is a NEW sketch, not a re-open of an existing one.
    expect(selectionStore.getState().selected).toEqual([{ kind: "datum", id: "d1" }]);
    expect(toolStore.getState().mode).toBe("sketch");
    expect(viewportStore.getState().activeSketchId).toBeNull();
  });

  it("the context menu offers ONLY delete — no rename, no visibility", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name: /Datum 1/ }) });
    expect(screen.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action")).toBeInTheDocument();
    expect(screen.queryByTestId("tree-menu-rename")).not.toBeInTheDocument();
    expect(screen.queryByTestId("tree-menu-visibility")).not.toBeInTheDocument();
  });

  it("delete is a TWO-CLICK confirm and removes the row", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name: /Datum 1/ }) });

    await user.click(screen.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action"));
    expect(apply).not.toHaveBeenCalled(); // first click only arms

    await user.click(screen.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action-confirm"));
    await waitFor(() =>
      expect(apply).toHaveBeenCalledWith({ cmd: "deleteDatum", datum: "d1" }),
    );
    await waitFor(() => expect(documentStore.getState().datums.d1).toBeUndefined());
    expect(viewportStore.getState().statusHint?.message).toBe("Datum deleted");
    apply.mockRestore();
  });

  it("a REFERENCED datum is refused: the row survives and the backend message is surfaced", async () => {
    // Non-vacuous guard: register a real sketch→datum attachment in the mock lane
    // (the same seam `enterSketch({newOnDatum})` uses).
    mockAttachSketchToDatum("sk-hosted", "d1");
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name: /Datum 1/ }) });
    await user.click(screen.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action"));
    await user.click(screen.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action-confirm"));

    await waitFor(() => {
      const hint = viewportStore.getState().statusHint;
      expect(hint?.severity).toBe("error");
      expect(hint?.message).toContain("sk-hosted");
    });
    // Rejected ⇒ the datum is still there (nothing was optimistically dropped).
    expect(documentStore.getState().datums.d1).toBeDefined();
    expect(screen.getByRole("option", { name: /Datum 1/ })).toBeInTheDocument();
  });

  it("F2 ignores a datum selection (there is no RenameDatum command)", async () => {
    const user = userEvent.setup();
    selectionStore.getState().set([{ kind: "datum", id: "d1" }]);
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.keyboard("{F2}");
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });
});
