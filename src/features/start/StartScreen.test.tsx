import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { StartScreen } from "./StartScreen";
import { appStore } from "@/stores/appStore";
import { mockClient, resetMockRecents } from "@/ipc/mockClient";

beforeEach(() => {
  resetMockRecents();
  appStore.setState({
    screen: "start",
    recents: [],
    recentsStatus: "idle",
    document: null,
    recovery: null,
    recoveryStatus: "ready",
    importError: null,
  });
});

afterEach(() => vi.restoreAllMocks());

/** The name shown on the first (top-left) project card in DOM order. */
function firstCard(): HTMLElement {
  return screen.getAllByTitle(/\.onecad$/)[0];
}

/** The header action button, not the sidebar utility duplicate. */
function headerImportButton(): HTMLElement {
  return within(screen.getByRole("main")).getByRole("button", { name: /Import/ });
}

describe("StartScreen", () => {
  it("loads and renders recent projects", async () => {
    render(<StartScreen />);
    expect(await screen.findByText("Bracket v2")).toBeInTheDocument();
    expect(screen.getByText("Gearbox mount")).toBeInTheDocument();
  });

  it("opens settings from the bottom sidebar item", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close settings" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument(),
    );
  });

  it("filters recents by case-insensitive name substring", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await screen.findByText("Bracket v2");

    await user.type(screen.getByLabelText("Search projects"), "brack");

    expect(screen.getByText("Bracket v2")).toBeInTheDocument();
    expect(screen.queryByText("Enclosure rev C")).not.toBeInTheDocument();
    expect(screen.queryByText("Gearbox mount")).not.toBeInTheDocument();
  });

  it("toggles sort order between date (default) and name", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await screen.findByText("Bracket v2");

    // Default sort = date desc → newest first.
    expect(within(firstCard()).getByText("Bracket v2")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Date modified/ }));
    await user.click(screen.getByRole("menuitem", { name: "Name" }));

    // Sort = name asc → "Adapter flange" first.
    expect(within(firstCard()).getByText("Adapter flange")).toBeInTheDocument();
  });

  it("shows the search empty-state when nothing matches", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await screen.findByText("Bracket v2");

    await user.type(screen.getByLabelText("Search projects"), "zzz-no-match");

    expect(
      screen.getByText("No projects match your search."),
    ).toBeInTheDocument();
  });

  it("New project transitions to the editor screen", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: /New project/ }));

    // Editor shell (F-WP3) mounts — its titlebar File menu is a stable marker
    // (AUTO-MODE deleted the Model⇄Sketch toggle; the mode is tool-derived now).
    // Longer timeout: EditorScreen is now a lazy chunk (App.tsx), and a cold
    // vite-node transform of its ~130-module tree can exceed the 1000ms default.
    expect(
      await screen.findByRole("button", { name: "File" }, { timeout: 5000 }),
    ).toBeInTheDocument();
  });

  // ── Unified import (start-screen lane) ────────────────────────────────────

  it("imports a STEP file into a new document", async () => {
    const user = userEvent.setup();
    vi.spyOn(mockClient, "importFileDialog").mockResolvedValue(
      "/Users/andrej/CAD/Projects/Imported.step",
    );
    const importStep = vi.spyOn(mockClient, "importStep");
    const openDocument = vi.spyOn(mockClient, "openDocument");
    render(<App />);

    await user.click(headerImportButton());

    // Longer timeout: EditorScreen is a lazy chunk now (see the New-project test).
    expect(
      await screen.findByRole("button", { name: "File" }, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(importStep).toHaveBeenCalledWith("/Users/andrej/CAD/Projects/Imported.step");
    expect(openDocument).not.toHaveBeenCalled();
    expect(appStore.getState().importError).toBeNull();
  });

  it("opens a project file as a project", async () => {
    const user = userEvent.setup();
    vi.spyOn(mockClient, "importFileDialog").mockResolvedValue(
      "/Users/andrej/CAD/Projects/Imported.onecad",
    );
    const openDocument = vi.spyOn(mockClient, "openDocument");
    const importStep = vi.spyOn(mockClient, "importStep");
    render(<App />);

    await user.click(headerImportButton());

    expect(
      await screen.findByRole("button", { name: "File" }, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(openDocument).toHaveBeenCalledWith("/Users/andrej/CAD/Projects/Imported.onecad");
    expect(importStep).not.toHaveBeenCalled();
    expect(appStore.getState().importError).toBeNull();
  });

  it("a cancelled import dialog is a silent no-op (no error, no screen change)", async () => {
    const user = userEvent.setup();
    vi.spyOn(mockClient, "importFileDialog").mockResolvedValue(null);
    const importStep = vi.spyOn(mockClient, "importStep");
    const openDocument = vi.spyOn(mockClient, "openDocument");
    render(<StartScreen />);

    await user.click(headerImportButton());

    await waitFor(() => expect(importStep).not.toHaveBeenCalled());
    expect(openDocument).not.toHaveBeenCalled();
    expect(appStore.getState().screen).toBe("start");
    expect(appStore.getState().importError).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /**
   * A failed import must not reject into the caller's `void importStep()` NOR be
   * routed to the editor StatusBar — the user is still on the start screen, where
   * that bar is not mounted, so the reason has to render right here.
   */
  it("shows an inline reason when the import fails, staying on the start screen", async () => {
    const user = userEvent.setup();
    vi.spyOn(mockClient, "importFileDialog").mockResolvedValue(
      "/Users/andrej/CAD/Projects/Imported.step",
    );
    vi.spyOn(mockClient, "importStep").mockRejectedValueOnce(
      new Error("io error: not a STEP file"),
    );
    render(<StartScreen />);

    await user.click(headerImportButton());

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Import failed: io error: not a STEP file",
    );
    expect(appStore.getState().screen).toBe("start");
  });

  it("shows the recovery card and Restore enters the editor", async () => {
    const user = userEvent.setup();
    appStore.setState({
      screen: "start",
      recents: [],
      recentsStatus: "ready",
      document: null,
      recovery: {
        autosavePath: "/x/autosave/foo.onecad",
        originalPath: "/docs/Bracket.onecad",
        modifiedMs: 1_700_000_000_000,
      },
      recoveryStatus: "ready",
    });
    render(<StartScreen />);

    expect(screen.getByText("Unsaved changes recovered")).toBeInTheDocument();
    expect(screen.getByText("Bracket")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Restore/ }));

    await waitFor(() => expect(appStore.getState().screen).toBe("editor"));
    expect(appStore.getState().recovery).toBeNull();
  });

  it("Discard clears the recovery card without leaving the start screen", async () => {
    const user = userEvent.setup();
    appStore.setState({
      screen: "start",
      recents: [],
      recentsStatus: "ready",
      document: null,
      recovery: {
        autosavePath: "/x/autosave/foo.onecad",
        originalPath: "/docs/Bracket.onecad",
        modifiedMs: 1_700_000_000_000,
      },
      recoveryStatus: "ready",
    });
    render(<StartScreen />);
    expect(screen.getByText("Unsaved changes recovered")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Discard/ }));

    await waitFor(() => expect(appStore.getState().recovery).toBeNull());
    expect(appStore.getState().screen).toBe("start");
    expect(
      screen.queryByText("Unsaved changes recovered"),
    ).not.toBeInTheDocument();
  });
});

describe("StartScreen — boot failure", () => {
  it("shows a retry affordance instead of an endless skeleton when recents fail", async () => {
    const user = userEvent.setup();
    const spy = vi
      .spyOn(mockClient, "listRecents")
      .mockRejectedValueOnce(new Error("backend down"));
    render(<StartScreen />);

    expect(
      await screen.findByText("Could not load the start screen."),
    ).toBeInTheDocument();
    // The skeleton is what a stuck "loading" would leave on screen forever;
    // the error branch must REPLACE it, not sit next to it.
    expect(document.querySelector(".animate-pulse")).toBeNull();

    spy.mockResolvedValueOnce([]);
    await user.click(screen.getByRole("button", { name: "Retry recents" }));

    await waitFor(() =>
      expect(appStore.getState().recentsStatus).toBe("ready"),
    );
    expect(
      screen.queryByText("Could not load the start screen."),
    ).not.toBeInTheDocument();
  });

  it("offers a recovery retry when only the recovery probe failed", async () => {
    appStore.setState({ recentsStatus: "ready", recoveryStatus: "idle" });
    vi.spyOn(mockClient, "checkRecovery").mockRejectedValueOnce(new Error("nope"));
    render(<StartScreen />);

    expect(
      await screen.findByRole("button", { name: "Retry recovery" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Retry recents" }),
    ).not.toBeInTheDocument();
  });
});

describe("StartScreen — project card menu", () => {
  it("opens the overflow menu with rename / reveal / delete", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await screen.findByText("Bracket v2");

    await user.click(screen.getByRole("button", { name: "Bracket v2 options" }));

    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: "Reveal in Finder" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeInTheDocument();
  });

  it("renames a project via the card menu", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await screen.findByText("Bracket v2");

    await user.click(screen.getByRole("button", { name: "Bracket v2 options" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByLabelText("Rename Bracket v2"));
    await user.type(screen.getByLabelText("Rename Bracket v2"), "Bracket v3{Enter}");

    expect(await screen.findByText("Bracket v3")).toBeInTheDocument();
    expect(screen.queryByText("Bracket v2")).not.toBeInTheDocument();
  });

  it("reverts a rename that collides with an existing project name", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await screen.findByText("Bracket v2");
    await screen.findByText("Enclosure rev C");

    await user.click(screen.getByRole("button", { name: "Bracket v2 options" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByLabelText("Rename Bracket v2"));
    await user.type(screen.getByLabelText("Rename Bracket v2"), "Enclosure rev C{Enter}");

    // The collision rejects (mockClient throws); nothing was refetched, so the
    // original name is still what's on screen — no toast, just a revert.
    expect(await screen.findByText("Bracket v2")).toBeInTheDocument();
  });

  it("deletes a project via the two-click confirm", async () => {
    const user = userEvent.setup();
    render(<StartScreen />);
    await screen.findByText("Bracket v2");

    await user.click(screen.getByRole("button", { name: "Bracket v2 options" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete" }));
    await user.click(screen.getByRole("menuitem", { name: "Confirm delete" }));

    await waitFor(() =>
      expect(screen.queryByText("Bracket v2")).not.toBeInTheDocument(),
    );
  });

  it("reveals a project in the file manager", async () => {
    const user = userEvent.setup();
    const spy = vi.spyOn(mockClient, "revealInFileManager");
    render(<StartScreen />);
    await screen.findByText("Bracket v2");

    await user.click(screen.getByRole("button", { name: "Bracket v2 options" }));
    await user.click(screen.getByRole("menuitem", { name: "Reveal in Finder" }));

    await waitFor(() =>
      expect(spy).toHaveBeenCalledWith("/Users/andrej/CAD/Projects/Bracket v2.onecad"),
    );
  });
});
