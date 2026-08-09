/*
 * Title-bar home button: left-most control, next to FileMenu. Routes through
 * closeProject() -> appStore.requestClose("close") — the ONE shared entry
 * point every close/quit path uses — so a dirty document arms the
 * UnsavedChangesDialog instead of discarding work, and a clean one returns
 * straight to the start screen.
 *
 * Rendered through `renderWithPlatform` since the bar gained the workspace
 * switcher, which reads the workspace registry — a bare `render` would only
 * prove the component throws without a provider.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TitleBar } from "./TitleBar";
import { appStore } from "@/stores/appStore";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform } from "@/test/renderWithPlatform";

function openDocument(dirty: boolean): void {
  appStore.setState({
    screen: "editor",
    document: { documentId: "doc-1", title: "Untitled" },
    pendingCloseIntent: null,
  });
  documentStore.setState({ dirty });
}

describe("TitleBar home button", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
    openDocument(false);
  });

  it("is the left-most control, before FileMenu", () => {
    renderWithPlatform(<TitleBar />);
    const buttons = screen.getAllByRole("button");
    expect(buttons[0]).toHaveAccessibleName("Close project and return to start screen");
  });

  it("does not carry the Tauri drag region — that would swallow the click", () => {
    renderWithPlatform(<TitleBar />);
    const btn = screen.getByRole("button", { name: /^Close project/ });
    expect(btn).not.toHaveAttribute("data-tauri-drag-region");
  });

  it("clean document: click returns straight to the start screen, no prompt", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);

    await user.click(screen.getByRole("button", { name: /^Close project/ }));

    await waitFor(() => expect(appStore.getState().screen).toBe("start"));
    expect(appStore.getState().document).toBeNull();
    expect(appStore.getState().pendingCloseIntent).toBeNull();
  });

  it("dirty document: click arms the save/discard/cancel prompt and stays open", async () => {
    const user = userEvent.setup();
    openDocument(true);
    renderWithPlatform(<TitleBar />);

    await user.click(screen.getByRole("button", { name: /^Close project/ }));

    expect(appStore.getState().pendingCloseIntent).toBe("close");
    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().document).not.toBeNull();
  });

  it("no appearance toggle in the title bar", () => {
    renderWithPlatform(<TitleBar />);
    expect(screen.queryByRole("button", { name: /^Appearance:/ })).not.toBeInTheDocument();
  });
});
