import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelTreePanel } from "./ModelTreePanel";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { mockClient } from "@/ipc/mockClient";
import { resetStores } from "@/test/resetStores";

describe("ModelTreePanel", () => {
  beforeEach(() => resetStores());

  it("renders the prototype tree (Body 1 + Sketch 2/4/5) with Sketch 2 selected", () => {
    render(<ModelTreePanel />);
    expect(screen.getByRole("option", { name: /Body 1/ })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /Sketch 4/ })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /Sketch 2/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("selects a row on click", async () => {
    const user = userEvent.setup();
    render(<ModelTreePanel />);
    await user.click(screen.getByRole("option", { name: /Body 1/ }));
    expect(selectionStore.getState().selected).toEqual([
      { kind: "body", id: "body1" },
    ]);
    expect(screen.getByRole("option", { name: /Body 1/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("option", { name: /Sketch 2/ })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("toggles visibility through a SetVisibility command without changing selection", async () => {
    const user = userEvent.setup();
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    render(<ModelTreePanel />);
    await user.click(screen.getByRole("option", { name: /Body 1/ }));

    expect(documentStore.getState().sketches.sketch2.visible).toBe(true);
    await user.click(
      screen.getByRole("switch", { name: "Toggle Sketch 2 visibility" }),
    );
    expect(documentStore.getState().sketches.sketch2.visible).toBe(false);
    // Backend-backed (TRUST wave): the flip is a real command, not a local lie.
    await waitFor(() =>
      expect(apply).toHaveBeenCalledWith({
        cmd: "setVisibility",
        target: { sketch: "sketch2" },
        visible: false,
      }),
    );
    // Eye click must not steal selection from Body 1.
    expect(selectionStore.getState().selected).toEqual([
      { kind: "body", id: "body1" },
    ]);
    apply.mockRestore();
  });
});

describe("ModelTreePanel — context menu", () => {
  beforeEach(() => resetStores());

  const openMenuOn = async (name: RegExp) => {
    const user = userEvent.setup();
    render(<ModelTreePanel />);
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name }) });
    return user;
  };

  it("opens on right-click with Rename + Hide, and no sketch delete on a BODY row", async () => {
    await openMenuOn(/Body 1/);
    expect(screen.getByTestId("tree-menu-rename")).toBeInTheDocument();
    expect(screen.getByTestId("tree-menu-visibility")).toHaveTextContent("Hide");
    expect(screen.queryByTestId("tree-menu-delete")).not.toBeInTheDocument();
  });

  it("shows Show (not Hide) for an already-hidden row", async () => {
    await openMenuOn(/Sketch 5/); // seeded hidden
    expect(screen.getByTestId("tree-menu-visibility")).toHaveTextContent("Show");
  });

  it("the visibility item dispatches the same SetVisibility command", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    const user = await openMenuOn(/Sketch 4/);
    await user.click(screen.getByTestId("tree-menu-visibility"));
    await waitFor(() =>
      expect(apply).toHaveBeenCalledWith({
        cmd: "setVisibility",
        target: { sketch: "sketch4" },
        visible: false,
      }),
    );
    apply.mockRestore();
  });

  it("sketch delete is a TWO-CLICK confirm (the HistoryList idiom)", async () => {
    const del = vi.spyOn(mockClient, "deleteSketch");
    const user = await openMenuOn(/Sketch 4/);
    await user.click(screen.getByTestId("tree-menu-delete"));
    expect(del).not.toHaveBeenCalled(); // first click only arms
    await user.click(screen.getByTestId("tree-menu-delete-confirm"));
    await waitFor(() => expect(del).toHaveBeenCalledWith("sketch4"));
    await waitFor(() => expect(documentStore.getState().sketches.sketch4).toBeUndefined());
    del.mockRestore();
  });
});

describe("ModelTreePanel — inline rename", () => {
  beforeEach(() => resetStores());

  /** Right-click a row → Rename → the focused inline field. */
  async function startRename(name: RegExp) {
    const user = userEvent.setup();
    render(<ModelTreePanel />);
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name }) });
    await user.click(screen.getByTestId("tree-menu-rename"));
    return { user, field: screen.getByRole("textbox") };
  }

  it("commits on Enter through a RenameBody command", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    const { user, field } = await startRename(/Body 1/);
    await user.clear(field);
    await user.type(field, "Housing{Enter}");
    await waitFor(() =>
      expect(apply).toHaveBeenCalledWith({ cmd: "renameBody", body: "body1", name: "Housing" }),
    );
    expect(documentStore.getState().bodies.body1.name).toBe("Housing");
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    apply.mockRestore();
  });

  it("reverts on Escape with no command sent", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    const { user, field } = await startRename(/Sketch 4/);
    await user.clear(field);
    await user.type(field, "Profile{Escape}");
    expect(apply).not.toHaveBeenCalled();
    expect(documentStore.getState().sketches.sketch4.name).toBe("Sketch 4");
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    apply.mockRestore();
  });

  it("refuses an empty name: reverts + sticky error hint, nothing sent", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    const { user, field } = await startRename(/Body 1/);
    await user.clear(field);
    await user.type(field, "   {Enter}");
    expect(apply).not.toHaveBeenCalled();
    expect(documentStore.getState().bodies.body1.name).toBe("Body 1");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("empty");
    apply.mockRestore();
  });

  it("F2 renames the single selected row", async () => {
    const user = userEvent.setup();
    render(<ModelTreePanel />);
    await user.click(screen.getByRole("option", { name: /Body 1/ }));
    await user.keyboard("{F2}");
    const field = screen.getByRole("textbox");
    expect(field).toHaveAttribute("aria-label", "Rename Body 1");
  });

  it("F2 does nothing when the selection is not exactly one tree row", async () => {
    const user = userEvent.setup();
    selectionStore.getState().set([
      { kind: "body", id: "body1" },
      { kind: "sketch", id: "sketch4" },
    ]);
    render(<ModelTreePanel />);
    await user.keyboard("{F2}");
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  /*
   * W3 isolation dims the rows of bodies isolated AWAY. It must NOT touch the
   * eye: the eye reports the document's persisted `visible` fact, isolation is a
   * transient viewport mask, and collapsing the two would make "hidden" ambiguous.
   */
  it("dims the rows of bodies isolated away, leaving their eyes untouched", () => {
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        body2: { id: "body2", name: "Body 2", visible: true },
      },
    });
    const { rerender } = render(<ModelTreePanel />);
    const dimmed = (name: RegExp) =>
      screen.getByRole("option", { name }).className.includes("opacity-50");

    expect(dimmed(/Body 1/)).toBe(false);
    expect(dimmed(/Body 2/)).toBe(false);

    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    rerender(<ModelTreePanel />);
    expect(dimmed(/Body 1/)).toBe(false);
    expect(dimmed(/Body 2/)).toBe(true);
    expect(
      screen.getByRole("option", { name: /Body 2/ }).querySelector('[role="switch"]'),
    ).toHaveAttribute("aria-checked", "true");

    viewportStore.setState({ isolatedBodyIds: null });
    rerender(<ModelTreePanel />);
    expect(dimmed(/Body 2/)).toBe(false);
  });
});
