/*
 * The gate, from the user's side.
 *
 * Two independent conditions have to hold before anything about the assistant
 * is on screen: the preference is on, and the sidebar's active tab is the
 * assistant's. Either one missing renders `null` — the same rule
 * `ModelTreePanel` and `VariablesPanel` follow for the region they share.
 */
import { describe, it, expect, afterEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { SidebarTabHeader } from "@/features/shell/SidebarTabHeader";
import { settingsStore } from "@/stores/settingsStore";
import { sidebarTabStore } from "@/stores/sidebarTabStore";
import { AssistantPanel } from "./AssistantPanel";

afterEach(() => {
  act(() => {
    settingsStore.getState().setAssistantEnabled(false);
    sidebarTabStore.getState().setActiveTab("model");
  });
});

describe("AssistantPanel gating", () => {
  it("renders nothing while the preference is off", () => {
    act(() => sidebarTabStore.getState().setActiveTab("assistant"));
    const { container } = render(<AssistantPanel />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing when another sidebar tab is active", () => {
    act(() => settingsStore.getState().setAssistantEnabled(true));
    const { container } = render(<AssistantPanel />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders the panel, its tab strip and its composer when both hold", () => {
    act(() => {
      settingsStore.getState().setAssistantEnabled(true);
      sidebarTabStore.getState().setActiveTab("assistant");
    });
    render(<AssistantPanel />);

    expect(screen.getByTestId("assistant-panel")).toBeInTheDocument();
    expect(screen.getByTestId("sidebar-tab-assistant")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByTestId("assistant-composer-input")).toBeInTheDocument();
  });

  it("offers no Assistant tab in the strip while the preference is off", () => {
    // The strip itself, not through a panel: with the preference off the panel
    // renders nothing at all, so asserting on it would pass vacuously.
    const { rerender } = render(<SidebarTabHeader />);
    expect(screen.queryByTestId("sidebar-tab-assistant")).not.toBeInTheDocument();

    act(() => settingsStore.getState().setAssistantEnabled(true));
    rerender(<SidebarTabHeader />);
    expect(screen.getByTestId("sidebar-tab-assistant")).toBeInTheDocument();
  });
});
