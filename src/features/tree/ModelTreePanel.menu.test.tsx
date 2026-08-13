/*
 * The Document Explorer's context menu: the frozen order of a row's own items,
 * and where a CONTRIBUTED item (WP-B1) is allowed to appear relative to them.
 *
 * Driven through a real registered provider and a real registered menu
 * contribution — the claim under test is that a module which owns NEITHER the
 * panel nor the row can add an item to it, and that doing so cannot displace
 * what the row's own owner already offers.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  contributionId,
  moduleId,
  Slots,
  type MenuContributionId,
  type MenuTarget,
  type TreeProviderId,
  type TreeSection,
} from "@/platform";
import { renderWithPlatform, bootTestPlatform } from "@/test/renderWithPlatform";
import { resetStores } from "@/test/resetStores";
import { TREE_MENU_CONTRACT } from "@/test/contracts/treeMenuContract";
import { ModelTreePanel } from "./ModelTreePanel";

const OWNER = moduleId("onecad.demo");
const FOREIGN = moduleId("onecad.other");
const PROVIDER = contributionId<TreeProviderId>(OWNER, "onecad.demo.tree.demo");

function sections(): readonly TreeSection[] {
  return [
    {
      id: "bodies",
      title: "Bodies",
      nodes: [
        {
          id: "body_1",
          label: "Body 1",
          icon: "cube",
          kind: "body",
          selected: false,
          visible: true,
          select: () => {},
          toggleVisible: () => {},
          rename: () => {},
        },
      ],
    },
  ];
}

interface Options {
  /** Registered by a DIFFERENT module than the one providing the rows. */
  contribute?: {
    title: string;
    appliesTo?: (t: MenuTarget) => boolean;
    onRun?: (t: MenuTarget) => void;
    slot?: (typeof Slots)[keyof typeof Slots];
  };
}

function renderTree(options: Options = {}) {
  const platform = bootTestPlatform();
  platform.createScope(OWNER).registerTreeProvider({ id: PROVIDER, sections });
  if (options.contribute) {
    platform.createScope(FOREIGN).registerMenuItem({
      id: contributionId<MenuContributionId>(FOREIGN, "onecad.other.menu.item"),
      slot: options.contribute.slot ?? Slots.TreeContext,
      title: options.contribute.title,
      appliesTo: options.contribute.appliesTo,
      run: options.contribute.onRun ?? (() => {}),
    });
  }
  return renderWithPlatform(<ModelTreePanel />, { platform });
}

/**
 * Every menu item's LABEL, in DOM order — the item's first span, not its
 * `textContent` (which also carries the right-aligned shortcut hint).
 */
function menuLabels(): string[] {
  return screen
    .getAllByRole("menuitem", { hidden: true })
    .map((el) => el.querySelector("span")?.textContent?.trim() ?? "");
}

async function openRowMenu(): Promise<void> {
  const user = userEvent.setup();
  await user.pointer({ keys: "[MouseRight]", target: screen.getByText("Body 1") });
}

describe("Document Explorer context menu", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
  });

  it("renders the row's own items in the frozen order", async () => {
    renderTree();
    await openRowMenu();
    expect(menuLabels()).toEqual([...TREE_MENU_CONTRACT.body]);
  });

  it("appends a contributed item AFTER the row's own items", async () => {
    renderTree({ contribute: { title: "Save as Component…" } });
    await openRowMenu();
    expect(menuLabels()).toEqual([...TREE_MENU_CONTRACT.body, "Save as Component…"]);
  });

  it("runs the contribution with the row as its target", async () => {
    const seen: MenuTarget[] = [];
    renderTree({ contribute: { title: "Save as Component…", onRun: (t) => seen.push(t) } });
    await openRowMenu();
    await userEvent.setup().click(screen.getByText("Save as Component…"));
    expect(seen).toEqual([{ kind: "body", id: "body_1", label: "Body 1" }]);
  });

  it("hides an item whose appliesTo rejects the row", async () => {
    renderTree({
      contribute: { title: "Sketch only", appliesTo: (t) => t.kind === "sketch" },
    });
    await openRowMenu();
    expect(menuLabels()).toEqual([...TREE_MENU_CONTRACT.body]);
  });

  it("hides an item registered for a different slot", async () => {
    // A viewport-context item must not leak into the tree's menu; the slot is
    // the whole reason the surface is addressable at all.
    renderTree({ contribute: { title: "Viewport only", slot: Slots.ViewportContext } });
    await openRowMenu();
    expect(menuLabels()).toEqual([...TREE_MENU_CONTRACT.body]);
  });

  it("treats a throwing appliesTo as not applying rather than failing the menu", async () => {
    renderTree({
      contribute: {
        title: "Broken",
        appliesTo: () => {
          throw new Error("contribution bug");
        },
      },
    });
    await openRowMenu();
    expect(menuLabels()).toEqual([...TREE_MENU_CONTRACT.body]);
  });
});
