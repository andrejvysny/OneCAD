/*
 * The Document Explorer's sections: collapsible, with a provider-declared
 * default, plus the row affordances the modular shell added (`meta`, `problem`,
 * `emptyNote`).
 *
 * Driven through a REAL registered provider rather than by rendering rows
 * directly — the claim under test is that a provider can say these things and
 * the panel honours them without knowing what the rows are.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { contributionId, moduleId, type TreeProviderId, type TreeSection } from "@/platform";
import { renderWithPlatform, bootTestPlatform } from "@/test/renderWithPlatform";
import { resetStores } from "@/test/resetStores";
import { ModelTreePanel } from "./ModelTreePanel";

const OWNER = moduleId("onecad.demo");
const PROVIDER = contributionId<TreeProviderId>(OWNER, "onecad.demo.tree.demo");

const SECTIONS: readonly TreeSection[] = [
  {
    id: "loads",
    title: "Loads",
    nodes: [
      {
        id: "force1",
        label: "Force 1",
        icon: "load",
        kind: "load",
        selected: false,
        meta: "500 N",
        select: () => {},
      },
    ],
  },
  {
    id: "results",
    title: "Results",
    nodes: [],
    defaultCollapsed: true,
    emptyNote: "Run Solve to see results.",
  },
  {
    id: "unavailable",
    title: "Unavailable data",
    nodes: [
      {
        id: "com.vendor.robot",
        label: "com.vendor.robot",
        icon: "alert",
        kind: "missing-module",
        selected: false,
        problem: true,
        meta: "not installed",
        select: () => {},
      },
    ],
  },
];

function renderTree() {
  const platform = bootTestPlatform();
  platform.createScope(OWNER).registerTreeProvider({ id: PROVIDER, sections: () => SECTIONS });
  return renderWithPlatform(<ModelTreePanel />, { platform });
}

describe("Document Explorer sections", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
  });

  it("renders a provider's sections as collapsible headers", () => {
    renderTree();
    expect(screen.getByTestId("tree-section-loads")).toHaveAttribute("aria-expanded", "true");
  });

  it("honours a provider's defaultCollapsed", () => {
    renderTree();
    expect(screen.getByTestId("tree-section-results")).toHaveAttribute("aria-expanded", "false");
  });

  it("the user's expand/collapse overrides the default", async () => {
    const user = userEvent.setup();
    renderTree();
    await user.click(screen.getByTestId("tree-section-results"));
    expect(screen.getByTestId("tree-section-results")).toHaveAttribute("aria-expanded", "true");
    // Now the empty note is reachable — a bare header would have said nothing.
    expect(screen.getByText("Run Solve to see results.")).toBeInTheDocument();
  });

  it("collapsing a section hides its rows", async () => {
    const user = userEvent.setup();
    renderTree();
    expect(screen.getByText("Force 1")).toBeInTheDocument();
    await user.click(screen.getByTestId("tree-section-loads"));
    expect(screen.queryByText("Force 1")).not.toBeInTheDocument();
  });

  it("shows a row's trailing meta value", () => {
    renderTree();
    const row = screen.getByText("Force 1").closest("[role='option']");
    expect(within(row as HTMLElement).getByText("500 N")).toBeInTheDocument();
  });

  it("tints a problem row with the caution treatment, not the dimmed one", () => {
    renderTree();
    const row = screen.getByText("com.vendor.robot").closest("[role='option']");
    expect(row?.className).toContain("text-warn");
    expect(row?.className).not.toContain("opacity-50");
  });
});
