/*
 * The Materials sidebar tab.
 *
 * Driven against the REAL store and the real mock lane, not a fake: every row
 * action here is a document write, and the thing worth proving is that clicking
 * a starter really mints a material into the document and that deleting one
 * really takes its assignments with it.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { mockClient, getMockLatency, setMockLatency } from "@/ipc/mockClient";
import { sidebarTabStore } from "@/stores/sidebarTabStore";
import { settleUntil } from "@/test/settle";

import { createMaterial, isMaterialId } from "../model/material";
import { renderStore, setRenderDocumentStateService } from "../store/renderStore";
import { renderDialogStore } from "./dialogStore";
import { MaterialLibraryPanel } from "./MaterialLibraryPanel";
import { STARTER_MATERIALS } from "./starterMaterials";

const STEEL = createMaterial("Steel", { base: { base_metalness: 1 } });
const realLatency = getMockLatency();

const settle = {
  turn: () => act(async () => void (await new Promise((r) => setTimeout(r, 0)))),
};

beforeEach(async () => {
  setMockLatency(0);
  setRenderDocumentStateService(null);
  renderStore.getState().reset();
  renderDialogStore.getState().reset();
  sidebarTabStore.getState().setActiveTab("materials");
  await mockClient.closeDocument();
});

afterEach(() => {
  setMockLatency(realLatency);
  renderStore.getState().reset();
  renderDialogStore.getState().reset();
  sidebarTabStore.getState().setActiveTab("model");
});

describe("MaterialLibraryPanel", () => {
  it("renders nothing when the Materials tab isn't active", () => {
    sidebarTabStore.getState().setActiveTab("model");
    render(<MaterialLibraryPanel />);
    expect(screen.queryByRole("tablist")).toBeNull();
  });

  it("lists the document's materials and every starter template", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    render(<MaterialLibraryPanel />);

    expect(screen.getByTestId(`material-row-${STEEL.id}`)).toBeInTheDocument();
    for (const t of STARTER_MATERIALS) {
      expect(screen.getByTestId(`starter-material-${t.name}`)).toBeInTheDocument();
    }
    expect(STARTER_MATERIALS).toHaveLength(8);
  });

  it("says so when the design has no materials yet", () => {
    render(<MaterialLibraryPanel />);
    expect(screen.getByTestId("material-library-empty")).toBeInTheDocument();
  });

  it("clicking a starter ADDS a copy with a freshly minted id", async () => {
    render(<MaterialLibraryPanel />);
    await userEvent.click(screen.getByTestId("starter-material-Brass"));
    await settleUntil(
      () => expect(Object.keys(renderStore.getState().state.library)).toHaveLength(1),
      settle,
    );

    const [added] = Object.values(renderStore.getState().state.library);
    expect(added.name).toBe("Brass");
    expect(isMaterialId(added.id)).toBe(true);
    // A COPY, not the template: the template carries no id at all, and editing
    // the added material must never reach the seed.
    expect(added.base.base_metalness).toBe(1);
    expect("id" in STARTER_MATERIALS[2]).toBe(false);

    // Adding twice gives two independent materials, not one shared entry.
    await userEvent.click(screen.getByTestId("starter-material-Brass"));
    await settleUntil(
      () => expect(Object.keys(renderStore.getState().state.library)).toHaveLength(2),
      settle,
    );
  });

  it("shows how many assignments each material carries", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().assignBody("body1", STEEL.id);
    await renderStore.getState().assignFace("el_top", STEEL.id);
    render(<MaterialLibraryPanel />);

    // Bodies AND faces — the count answers "is anything wearing this?", which is
    // what makes a delete safe or destructive.
    expect(screen.getByTestId(`material-usage-${STEEL.id}`)).toHaveTextContent("2");
  });

  it("deleting a material CASCADES to every assignment naming it", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().assignBody("body1", STEEL.id);
    await renderStore.getState().assignFace("el_top", STEEL.id);
    render(<MaterialLibraryPanel />);

    // Two-step, the repo's declarative destructive idiom (`TreeNodeAction.confirm`):
    // the button becomes its own confirmation before anything is written.
    await userEvent.click(screen.getByTestId(`material-delete-${STEEL.id}`));
    expect(renderStore.getState().state.library[STEEL.id]).toBeDefined();
    await userEvent.click(screen.getByTestId(`material-delete-confirm-${STEEL.id}`));

    await settleUntil(
      () => expect(renderStore.getState().state.library[STEEL.id]).toBeUndefined(),
      settle,
    );
    expect(renderStore.getState().state.assignments.bodies).toEqual({});
    expect(renderStore.getState().state.assignments.faces).toEqual({});
  });

  it("renames in place", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    render(<MaterialLibraryPanel />);

    // Scoped to the document row: "Steel" is also a starter template's label,
    // and a bare text query would be ambiguous.
    const row = screen.getByTestId(`material-row-${STEEL.id}`);
    await userEvent.click(within(row).getByText("Steel"));
    const input = screen.getByTestId(`material-rename-${STEEL.id}`);
    await userEvent.clear(input);
    await userEvent.type(input, "Stainless{Enter}");

    await settleUntil(
      () => expect(renderStore.getState().state.library[STEEL.id]?.name).toBe("Stainless"),
      settle,
    );
    // The id survives a rename, so every assignment naming it does too.
    expect(renderStore.getState().state.assignments.bodies).toEqual({});
    expect(Object.keys(renderStore.getState().state.library)).toEqual([STEEL.id]);
  });

  it("Edit opens the editor on that material", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    render(<MaterialLibraryPanel />);

    await userEvent.click(screen.getByTestId(`material-edit-${STEEL.id}`));
    expect(renderDialogStore.getState().editor).toBe(STEEL.id);
  });

  it("READ-ONLY says so and disables every mutation", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    act(() => renderStore.setState({ readOnly: true }));
    render(<MaterialLibraryPanel />);

    expect(screen.getByTestId("material-read-only")).toBeInTheDocument();
    expect(screen.getByTestId(`material-delete-${STEEL.id}`)).toBeDisabled();
    expect(screen.getByTestId(`material-edit-${STEEL.id}`)).toBeDisabled();
    expect(screen.getByTestId("starter-material-Steel")).toBeDisabled();
    // Not draggable either: a drop could not be written, and a gesture that
    // silently does nothing is worse than one that is not offered.
    expect(screen.getByTestId(`material-row-${STEEL.id}`)).toHaveAttribute("draggable", "false");
  });

  it("reports unresolved assignments without blocking anything", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    act(() => {
      renderStore.setState({ danglingBodies: ["body_gone"] });
      renderStore.getState().reportUnboundFaceOverrides("body1", ["el_a", "el_b"]);
    });
    render(<MaterialLibraryPanel />);

    expect(screen.getByTestId("material-unresolved")).toHaveTextContent("3 assignments");
    // Reported, never removed (H5-B): the rows are all still there.
    expect(screen.getByTestId(`material-row-${STEEL.id}`)).toBeInTheDocument();
    expect(screen.getByTestId("starter-material-Steel")).toBeEnabled();
  });
});
