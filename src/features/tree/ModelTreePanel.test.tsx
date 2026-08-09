import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelTreePanel } from "./ModelTreePanel";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { mockClient } from "@/ipc/mockClient";
import { resetStores } from "@/test/resetStores";
import { bootTestPlatform, renderWithPlatform } from "@/test/renderWithPlatform";
import { addonId, contributionId, type Platform, type TreeProviderId } from "@/platform";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { contributeModelingTree } from "@/modules/modeling/treeProvider";

describe("ModelTreePanel", () => {
  beforeEach(() => resetStores());

  it("renders the prototype tree (Body 1 + Sketch 2/4/5) with Sketch 2 selected", () => {
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    expect(screen.getByRole("option", { name: /Body 1/ })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /Sketch 4/ })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /Sketch 2/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("selects a row on click", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
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
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
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
    await apply.mock.results[0]?.value; // drain — see the rename test's note
    apply.mockRestore();
  });

  it("does not own Settings — that is application chrome, and lives in the StatusBar", () => {
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    expect(screen.queryByRole("button", { name: "Open settings" })).toBeNull();
  });
});

describe("ModelTreePanel — context menu", () => {
  beforeEach(() => resetStores());

  const openMenuOn = async (name: RegExp) => {
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name }) });
    return user;
  };

  it("opens on right-click with Rename + Hide, and no sketch delete on a BODY row", async () => {
    await openMenuOn(/Body 1/);
    expect(screen.getByTestId("tree-menu-rename")).toBeInTheDocument();
    expect(screen.getByTestId("tree-menu-visibility")).toHaveTextContent("Hide");
    expect(screen.queryByTestId("tree-action-onecad.modeling.command.deleteSketch.action")).not.toBeInTheDocument();
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
    await apply.mock.results[0]?.value; // drain — see the rename test's note
    apply.mockRestore();
  });

  it("sketch delete is a TWO-CLICK confirm (the HistoryList idiom)", async () => {
    const del = vi.spyOn(mockClient, "deleteSketch");
    const user = await openMenuOn(/Sketch 4/);
    await user.click(screen.getByTestId("tree-action-onecad.modeling.command.deleteSketch.action"));
    expect(del).not.toHaveBeenCalled(); // first click only arms
    await user.click(screen.getByTestId("tree-action-onecad.modeling.command.deleteSketch.action-confirm"));
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
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
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
    // Drain the in-flight command before leaving. The store write above is the
    // OPTIMISTIC one; `mockApplyEditCommand` waits out the simulated backend
    // latency and only then records the rename in the mock's metadata registry.
    // Left in flight, that write lands after the next test's `resetStores()` and
    // re-asserts "Housing" onto the freshly reseeded document.
    await apply.mock.results[0]?.value;
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
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
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
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
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
    const { rerender } = renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
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

// ── HISTORY-HARDEN H9: Reattach… on a tree sketch row ────────────────────────

describe("ModelTreePanel — Reattach (H9)", () => {
  beforeEach(() => resetStores());

  it("offers Reattach… on a sketch row and dispatches the picked target", async () => {
    const user = userEvent.setup();
    const reattach = vi.spyOn(mockClient, "reattachSketch");
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.pointer({
      keys: "[MouseRight]",
      target: screen.getByRole("option", { name: /Sketch 4/ }),
    });
    await user.click(screen.getByTestId("tree-menu-reattach"));
    await user.click(await screen.findByTestId("reattach-world-XZ"));
    await waitFor(() =>
      expect(reattach).toHaveBeenCalledWith("sketch4", { kind: "world", plane: "XZ" }),
    );
    reattach.mockRestore();
  });

  it("does NOT offer it on a BODY row (only a sketch has an attachment)", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.pointer({
      keys: "[MouseRight]",
      target: screen.getByRole("option", { name: /Body 1/ }),
    });
    expect(screen.queryByTestId("tree-menu-reattach")).toBeNull();
  });

  it("does NOT offer it on a FACE-HOSTED sketch — its geometry lives in the face's frame", async () => {
    documentStore.setState({
      sketches: {
        sketch4: {
          id: "sketch4",
          name: "Sketch 4",
          visible: true,
          dof: 0,
          status: "ok",
          geometryToken: "t",
          hostFace: { bodyId: "body1", elementId: "el_1" },
        },
      },
    });
    const user = userEvent.setup();
    renderWithPlatform(<ModelTreePanel />, { contribute: contributeModelingTree });
    await user.pointer({
      keys: "[MouseRight]",
      target: screen.getByRole("option", { name: /Sketch 4/ }),
    });
    expect(screen.queryByTestId("tree-menu-reattach")).toBeNull();
    // Delete is still there — the row is not otherwise crippled.
    expect(screen.getByTestId("tree-action-onecad.modeling.command.deleteSketch.action")).toBeInTheDocument();
  });
});

describe("ModelTreePanel — foreign tree providers", () => {
  beforeEach(() => resetStores());

  it("re-renders when a provider announces its rows changed", async () => {
    // A provider backed by data the panel does NOT watch (i.e. any non-modeling
    // one) is otherwise frozen at mount: the registry never changes and neither
    // do modeling's stores, so nothing re-runs sections().
    let label = "Before";
    let notify: (() => void) | null = null;
    renderWithPlatform(<ModelTreePanel />, {
      contribute: (scope) =>
        scope.registerTreeProvider({
          id: contributionId<TreeProviderId>(
            MODELING_MODULE_ID,
            "onecad.modeling.tree.foreign",
          ),
          priority: 500,
          sections: () => [
            {
              id: "foreign.section",
              title: "Foreign",
              nodes: [
                {
                  id: "n1",
                  label,
                  icon: "cube",
                  kind: "foreign",
                  selected: false,
                  select: () => {},
                },
              ],
            },
          ],
          subscribe: (onChange) => {
            notify = onChange;
            return { dispose: () => {} };
          },
        }),
    });

    expect(screen.getByText("Before")).toBeInTheDocument();

    label = "After";
    await act(async () => {
      notify?.();
    });
    expect(screen.getByText("After")).toBeInTheDocument();
  });
});

/*
 * The public-surface cases (P2.5 WP3). A second provider is the thing the tree
 * host existed for and had never actually been given.
 */
describe("ModelTreePanel — a second provider", () => {
  const VENDOR = addonId("com.example.foo");
  /** Deliberately the SAME node id modeling mints for its seeded body. */
  const COLLIDING_ID = "body1";

  beforeEach(() => resetStores());

  function foreignProvider(platform: Platform, onSelect = () => {}): void {
    platform.createScope(VENDOR).registerTreeProvider({
      id: contributionId<TreeProviderId>(VENDOR, "com.example.foo.tree.items"),
      priority: 200,
      sections: () => [
        {
          id: "com.example.foo.tree.section",
          title: "Items",
          nodes: [
            {
              id: COLLIDING_ID,
              label: "Vendor item",
              icon: "no-such-glyph",
              kind: "com.example.foo.item",
              selected: false,
              select: onSelect,
            },
          ],
        },
      ],
    });
  }

  it("renders its rows alongside modeling's, unknown icon and all", () => {
    const platform = bootTestPlatform(contributeModelingTree);
    foreignProvider(platform);
    renderWithPlatform(<ModelTreePanel />, { platform });

    expect(screen.getByRole("listbox", { name: "Items" })).toBeInTheDocument();
    // The old panel cast `node.icon as IconName`, which threw inside `Icon`.
    expect(screen.getByRole("option", { name: /Vendor item/ })).toBeInTheDocument();
  });

  it("a colliding node id does not cross-wire the context menu", async () => {
    const user = userEvent.setup();
    const platform = bootTestPlatform(contributeModelingTree);
    foreignProvider(platform);
    renderWithPlatform(<ModelTreePanel />, { platform });

    // Right-click the FOREIGN row whose id equals modeling's body id. Resolving
    // rows by `node.id` alone would hand back modeling's body here, and its menu
    // (Rename / Hide) would appear over someone else's row.
    await user.pointer({
      keys: "[MouseRight]",
      target: screen.getByRole("option", { name: /Vendor item/ }),
    });

    expect(screen.queryByTestId("tree-menu-rename")).toBeNull();
    expect(screen.queryByTestId("tree-menu-visibility")).toBeNull();
  });

  it("updates its rows from its OWN subscription, with no store the host watches", async () => {
    let label = "First";
    let notify: (() => void) | undefined;
    const platform = bootTestPlatform();
    platform.createScope(VENDOR).registerTreeProvider({
          id: contributionId<TreeProviderId>(VENDOR, "com.example.foo.tree.items"),
          sections: () => [
            {
              id: "com.example.foo.tree.section",
              title: "Items",
              nodes: [
                {
                  id: "item-1",
                  label,
                  icon: "cube",
                  kind: "com.example.foo.item",
                  selected: false,
                  select: () => {},
                },
              ],
            },
          ],
      subscribe: (onChange) => {
        notify = onChange;
        return { dispose: () => (notify = undefined) };
      },
    });
    renderWithPlatform(<ModelTreePanel />, { platform });

    expect(screen.getByRole("option", { name: /First/ })).toBeInTheDocument();

    label = "Second";
    act(() => notify?.());

    expect(screen.getByRole("option", { name: /Second/ })).toBeInTheDocument();
  });
});
