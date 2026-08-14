/*
 * `VariablesPanel` — the left-sidebar "Variables" tab that took over
 * `LibraryPanel`'s old `Slots.ShellLeft` slot. Its own contract is just the
 * tab-gating mechanism; `VariablesSection.test.tsx` already proves the table
 * itself against the mock lane.
 */
import { beforeEach, describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { VariablesPanel } from "./VariablesPanel";
import { sidebarTabStore } from "@/stores/sidebarTabStore";

beforeEach(() => {
  sidebarTabStore.getState().setActiveTab("model");
});

describe("VariablesPanel", () => {
  it("renders nothing when the variables tab isn't active", () => {
    render(<VariablesPanel />);
    expect(screen.queryByRole("tablist")).toBeNull();
  });

  it("renders the tab strip + variables section when active", () => {
    sidebarTabStore.getState().setActiveTab("variables");
    render(<VariablesPanel />);
    expect(screen.getByRole("tablist")).toBeInTheDocument();
    expect(screen.getByTestId("sidebar-tab-variables")).toHaveAttribute("aria-selected", "true");
  });
});
