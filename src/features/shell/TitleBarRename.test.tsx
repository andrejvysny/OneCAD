/*
 * File ▸ Rename… + the one-geometry rule for the title-bar buttons.
 *
 * The rename is DISPLAY-ONLY today (no `RenameDocument` in the protocol), which
 * is exactly why it needs a test: the store override must survive a projection
 * for the same document, must NOT follow the user to a different one, and must
 * never be mistaken for a write to `title`.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { documentStore, emptyDocument } from "@/stores/documentStore";
import { appStore } from "@/stores/appStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { TitleBar } from "./TitleBar";

const projection = (documentId: string, title: string) => ({
  ...emptyDocument(),
  status: "ready" as const,
  documentId,
  title,
});

/** Open File ▸ Rename… and hand back the focused field. */
async function openRename(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: /^File$/ }));
  await user.click(screen.getByTestId("file-menu-rename"));
  return screen.getByTestId("rename-document-input");
}

describe("File ▸ Rename…", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
    appStore.setState({
      screen: "editor",
      document: { documentId: "doc-1", title: "Untitled" },
      pendingCloseIntent: null,
    });
    documentStore.getState().applySnapshot(projection("doc-1", "Bracket v2"));
  });

  it("shows the backend title until the user renames", () => {
    renderWithPlatform(<TitleBar />);
    expect(screen.getByTestId("document-title")).toHaveTextContent("Bracket v2");
  });

  it("renders no app wordmark — the window chrome already says which app this is", () => {
    renderWithPlatform(<TitleBar />);
    expect(screen.queryByTestId("app-wordmark")).not.toBeInTheDocument();
    expect(screen.queryByText("OneCAD")).not.toBeInTheDocument();
  });

  it("the title itself is a label, not a rename affordance", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    // Double-clicking the title used to open an editor inside the window's own
    // drag region. It must not any more.
    await user.dblClick(screen.getByTestId("document-title"));
    expect(screen.queryByTestId("rename-document-dialog")).not.toBeInTheDocument();
  });

  it("opens from the File menu, seeded with the current name", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    expect(await openRename(user)).toHaveValue("Bracket v2");
  });

  it("commits the new name and says it did not touch the file", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    const input = await openRename(user);
    await user.clear(input);
    await user.type(input, "Bracket v3");
    await user.click(screen.getByTestId("rename-document-commit"));

    expect(screen.getByTestId("document-title")).toHaveTextContent("Bracket v3");
    expect(documentStore.getState().displayTitle).toBe("Bracket v3");
    // `title` is backend-authoritative and must not have been written.
    expect(documentStore.getState().title).toBe("Bracket v2");
    expect(viewportStore.getState().statusHint?.message).toContain("file on disk is unchanged");
  });

  it("Enter commits too", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    const input = await openRename(user);
    await user.clear(input);
    await user.type(input, "Bracket v3{Enter}");
    expect(documentStore.getState().displayTitle).toBe("Bracket v3");
  });

  it("Cancel abandons the edit with no write", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    const input = await openRename(user);
    await user.clear(input);
    await user.type(input, "Nope");
    await user.click(screen.getByTestId("rename-document-cancel"));
    expect(documentStore.getState().displayTitle).toBeNull();
    expect(screen.getByTestId("document-title")).toHaveTextContent("Bracket v2");
  });

  it("Escape abandons the edit too", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    const input = await openRename(user);
    await user.type(input, "{Escape}");
    expect(screen.queryByTestId("rename-document-dialog")).not.toBeInTheDocument();
    expect(documentStore.getState().displayTitle).toBeNull();
  });

  it("refuses an empty name rather than leaving the document unnamed", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    const input = await openRename(user);
    await user.clear(input);
    await user.type(input, "   ");
    expect(screen.getByTestId("rename-document-commit")).toBeDisabled();
  });

  it("refuses a no-op rename", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<TitleBar />);
    await openRename(user);
    expect(screen.getByTestId("rename-document-commit")).toBeDisabled();
  });

  it("survives a projection for the SAME document", () => {
    documentStore.getState().setDisplayTitle("Bracket v3");
    // A regen lands; the backend still calls it Bracket v2.
    documentStore.getState().applySnapshot(projection("doc-1", "Bracket v2"));
    expect(documentStore.getState().displayTitle).toBe("Bracket v3");
  });

  it("does NOT follow the user to a different document", () => {
    documentStore.getState().setDisplayTitle("Bracket v3");
    documentStore.getState().applySnapshot(projection("doc-2", "Enclosure rev C"));
    expect(documentStore.getState().displayTitle).toBeNull();
  });
});

describe("title-bar controls share one geometry", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
    documentStore.getState().applySnapshot(projection("doc-1", "Bracket v2"));
  });

  it("home, File, workspace and ⌘K are all the same 28px control", () => {
    renderWithPlatform(<TitleBar />);
    const labels = [/^Close project/, /^File$/, /^Workspace:/, /^Search commands/];
    for (const name of labels) {
      const cls = screen.getByRole("button", { name }).className;
      // One height and one radius across all four — what broke before was
      // three hand-rolled buttons drifting apart.
      expect(cls).toContain("h-[28px]");
      expect(cls).toContain("rounded-sm");
      expect(cls).toContain("border-none");
    }
  });
});
