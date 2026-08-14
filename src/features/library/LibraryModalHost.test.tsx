/*
 * `LibraryModalHost` — the `Slots.ShellOverlay` occupant that wires the
 * "Library" toolbar tool to `LibraryModal`. The contract under test: activating
 * the registered tool opens the modal, and EVERY close path (here: the modal's
 * own `onClose`) reports back through `platform.toolHost` so the tool's
 * `deactivate()` runs and the toolbar's active-highlight clears in the same
 * call — not a direct store close that could leave the two out of sync.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { act, screen, waitFor } from "@testing-library/react";
import { renderWithPlatform, bootTestPlatform } from "@/test/renderWithPlatform";
import { LIBRARY_MODULE_ID } from "@/modules/library/manifest";
import { contributeLibraryUi } from "@/modules/library/register";
import { LibraryTools } from "@/modules/library/panelIds";
import { libraryModalStore } from "@/modules/library/libraryModalStore";
import { LibraryModalHost } from "./LibraryModalHost";

// `vi.hoisted`, not a plain top-level `const`: this file imports
// `renderWithPlatform`/`contributeLibraryUi`, which transitively import
// `@/ipc/client` for real (`appStore.ts` → `fileActions.ts` calls
// `createClient()` at module scope) during the dependency-load phase — BEFORE
// this file's own statements run. A plain `const` referenced from the mock
// factory would hit its temporal-dead-zone at that point.
const { listLibraryComponents } = vi.hoisted(() => ({
  listLibraryComponents: vi.fn(() => Promise.resolve([])),
}));
vi.mock("@/ipc/client", () => ({
  createClient: () => ({ listLibraryComponents, reindexLibrary: vi.fn() }),
}));

beforeEach(() => {
  libraryModalStore.getState().close();
});

describe("LibraryModalHost", () => {
  it("stays closed until the Library tool activates", () => {
    const platform = bootTestPlatform();
    contributeLibraryUi(platform.createScope(LIBRARY_MODULE_ID));
    renderWithPlatform(<LibraryModalHost />, { platform });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("activating the Library tool opens the modal", async () => {
    const platform = bootTestPlatform();
    contributeLibraryUi(platform.createScope(LIBRARY_MODULE_ID));
    renderWithPlatform(<LibraryModalHost />, { platform });

    await act(() => platform.toolHost.activate(LibraryTools.OpenLibrary));
    await waitFor(() => expect(screen.getByRole("dialog")).toBeInTheDocument());
  });

  it("closing the modal deactivates the tool through toolHost, not a direct store write", async () => {
    const platform = bootTestPlatform();
    contributeLibraryUi(platform.createScope(LIBRARY_MODULE_ID));
    renderWithPlatform(<LibraryModalHost />, { platform });

    await act(() => platform.toolHost.activate(LibraryTools.OpenLibrary));
    await waitFor(() => expect(screen.getByRole("dialog")).toBeInTheDocument());
    expect(platform.toolHost.activeToolId()).toBe(LibraryTools.OpenLibrary);

    await act(async () => {
      screen.getByLabelText("Close library").click();
    });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(platform.toolHost.activeToolId()).toBeNull();
    expect(libraryModalStore.getState().isOpen).toBe(false);
  });
});
