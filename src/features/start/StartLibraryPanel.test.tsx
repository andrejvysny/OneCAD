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

  it("reindexes and reloads the list", async () => {
    listLibraryComponents.mockResolvedValue([]);
    render(<StartLibraryPanel />);
    await waitFor(() => expect(listLibraryComponents).toHaveBeenCalledTimes(1));

    listLibraryComponents.mockResolvedValue([SHCS]);
    fireEvent.click(screen.getByLabelText("Reindex library"));

    await waitFor(() => expect(reindexLibrary).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByText("Socket Head Cap Screw")).toBeInTheDocument());
  });

  it("selecting a card shows details and says placing needs a project — no arm gesture", async () => {
    listLibraryComponents.mockResolvedValue([SHCS]);
    render(<StartLibraryPanel />);
    await waitFor(() => expect(screen.getByTestId("start-library-card")).toBeInTheDocument());

    fireEvent.click(screen.getByTestId("start-library-card"));

    expect(screen.getByText("onecad.std.iso4762")).toBeInTheDocument();
    expect(screen.getByText("Open a project to place this component.")).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("Close details"));
    expect(screen.queryByText("Open a project to place this component.")).toBeNull();
  });
});
