import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { StartScreen } from "./StartScreen";
import { appStore } from "@/stores/appStore";
import { mockClient } from "@/ipc/mockClient";

beforeEach(() => {
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

describe("StartScreen", () => {
  it("loads and renders recent projects", async () => {
    render(<StartScreen />);
    expect(await screen.findByText("Bracket v2")).toBeInTheDocument();
    expect(screen.getByText("Gearbox mount")).toBeInTheDocument();
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

  // ── Import STEP (start-screen lane) ───────────────────────────────────────

  it("Import STEP opens the imported document in the editor", async () => {
    const user = userEvent.setup();
    // The dialog must be the STEP-filtered one: `openFileDialog` filters `.onecad`,
    // so routing this button through it made a STEP file unpickable.
    const stepDialog = vi.spyOn(mockClient, "stepFileDialog");
    const openDialog = vi.spyOn(mockClient, "openFileDialog");
    render(<App />);

    await user.click(screen.getByRole("button", { name: /Import STEP/ }));

    // Longer timeout: EditorScreen is a lazy chunk now (see the New-project test).
    expect(
      await screen.findByRole("button", { name: "File" }, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(stepDialog).toHaveBeenCalledTimes(1);
    expect(openDialog).not.toHaveBeenCalled();
    expect(appStore.getState().importError).toBeNull();
  });

  it("a cancelled STEP dialog is a silent no-op (no error, no screen change)", async () => {
    const user = userEvent.setup();
    vi.spyOn(mockClient, "stepFileDialog").mockResolvedValue(null);
    const importStep = vi.spyOn(mockClient, "importStep");
    render(<StartScreen />);

    await user.click(screen.getByRole("button", { name: /Import STEP/ }));

    await waitFor(() => expect(importStep).not.toHaveBeenCalled());
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
    vi.spyOn(mockClient, "importStep").mockRejectedValueOnce(
      new Error("io error: not a STEP file"),
    );
    render(<StartScreen />);

    await user.click(screen.getByRole("button", { name: /Import STEP/ }));

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
