/*
 * Component Library WP-2.4 follow-up — the start-screen library browser:
 * list/search/reindex with no open document, and a details pane that says
 * placing needs a project rather than faking a drag gesture with nowhere to
 * land it.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { StartLibraryPanel } from "./StartLibraryPanel";
import type { LibraryComponent } from "@/ipc/types";

const listLibraryComponents = vi.fn<() => Promise<LibraryComponent[]>>();
const reindexLibrary = vi.fn(() => Promise.resolve({ total: 0, indexed: 0, skipped: [] }));
vi.mock("@/ipc/client", () => ({
  createClient: () => ({ listLibraryComponents, reindexLibrary }),
}));

const SHCS: LibraryComponent = {
  id: "onecad.std.iso4762",
  version: "1.0.0",
  name: "Socket Head Cap Screw",
  category: ["fasteners"],
  tags: ["metric", "shcs"],
  sourceKind: "generator",
  revision: `sha256:${"0".repeat(64)}`,
  generatorId: "iso4762",
  generatorVersion: 1,
  attachments: {
    headSeat: { on: "face:head_underside", accepts: ["plane"] },
  },
  parameters: {
    thread: { role: "free", key: "M6", domain: ["M3", "M4", "M5", "M6", "M8"] },
  },
  designation: "ISO 4762 {thread}",
};

beforeEach(() => {
  listLibraryComponents.mockReset();
  reindexLibrary.mockClear();
});

describe("StartLibraryPanel", () => {
  it("renders indexed components with no document open", async () => {
    listLibraryComponents.mockResolvedValue([SHCS]);
    render(<StartLibraryPanel />);
    await waitFor(() => expect(screen.getByText("Socket Head Cap Screw")).toBeInTheDocument());
    expect(listLibraryComponents).toHaveBeenCalledTimes(1);
  });

  it("shows the empty-library hint, not a fabricated catalog", async () => {
    listLibraryComponents.mockResolvedValue([]);
    render(<StartLibraryPanel />);
    await waitFor(() =>
      expect(screen.getByText("No components indexed yet.")).toBeInTheDocument(),
    );
  });

  it("filters by search query", async () => {
    listLibraryComponents.mockResolvedValue([SHCS]);
    render(<StartLibraryPanel />);
    await waitFor(() => expect(screen.getByText("Socket Head Cap Screw")).toBeInTheDocument());

    fireEvent.change(screen.getByLabelText("Search library components"), {
      target: { value: "bearing" },
    });
    expect(screen.getByText(/No matches for/)).toBeInTheDocument();
  });

  /*
   * AMENDED (LGU-1 WP-A, defect F9). This grid no longer offers a Reindex
   * button at all: it reads the catalog on mount, and the manual rebuild has
   * ONE home now — the library modal's overflow menu — rather than a copy on
   * every surface that happens to list components.
   */
  it("offers no reindex verb of its own", async () => {
    listLibraryComponents.mockResolvedValue([SHCS]);
    render(<StartLibraryPanel />);
    await waitFor(() => expect(listLibraryComponents).toHaveBeenCalledTimes(1));

    expect(screen.queryByLabelText("Reindex library")).toBeNull();
    expect(reindexLibrary).not.toHaveBeenCalled();
  });

  it("selecting a card shows details and says placing needs a project — no arm gesture", async () => {
    listLibraryComponents.mockResolvedValue([SHCS]);
    render(<StartLibraryPanel />);
    await waitFor(() => expect(screen.getByTestId("start-library-card")).toBeInTheDocument());

    fireEvent.click(screen.getByTestId("start-library-card"));

    expect(screen.getByText("onecad.std.iso4762")).toBeInTheDocument();
    // AMENDED (LGU-1 WP-A, canvas D2-A): the refusal moved onto the Place
    // action itself as a disabled button, rather than sitting beside its
    // absence as a sentence.
    expect(screen.getByTestId("detail-place-disabled")).toHaveTextContent(
      "Place — open a project first",
    );

    fireEvent.click(screen.getByLabelText("Close library"));
    expect(screen.queryByTestId("detail-place-disabled")).toBeNull();
  });
});
