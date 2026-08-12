/*
 * Component Library WP-1.4 — the library panel's states (loading / empty /
 * error / populated + search filter) and its half of the shared sidebar-tab
 * mechanism.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LibraryPanel } from "./LibraryPanel";
import { sidebarTabStore } from "@/stores/sidebarTabStore";
import { tasksStore } from "@/stores/tasksStore";
import type { LibraryComponent } from "@/ipc/types";

const listLibraryComponents = vi.fn<() => Promise<LibraryComponent[]>>();
const reindexLibrary = vi.fn(() => Promise.resolve({ total: 0, indexed: 0, skipped: [] }));
vi.mock("@/ipc/client", () => ({
  createClient: () => ({ listLibraryComponents, detachComponent: vi.fn(), reindexLibrary }),
}));

// The real controller reaches `getViewportEngine()`/window listeners — none of
// which exist in jsdom. This panel's own contract is just "click arms/cancels
// the gesture", proven against a mock; the gesture itself is proven by
// `placementSolver.test.ts` (math) and manual browser verification (viewport).
const armPlacementMock = vi.fn();
const cancelPlacementMock = vi.fn();
let armedKeyForTest: string | null = null;
vi.mock("@/modules/library/placementController", () => ({
  armPlacement: (c: LibraryComponent) => armPlacementMock(c),
  cancelPlacement: () => cancelPlacementMock(),
  useArmedComponentKey: () => armedKeyForTest,
}));

const iso4762: LibraryComponent = {
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
    shankAxis: { on: "cylinder:shank", accepts: ["cylinder", "hole", "circularEdge"] },
  },
  parameters: {},
};

beforeEach(() => {
  listLibraryComponents.mockReset();
  reindexLibrary.mockClear();
  armPlacementMock.mockReset();
  cancelPlacementMock.mockReset();
  armedKeyForTest = null;
  sidebarTabStore.getState().setActiveTab("library");
  tasksStore.setState({ tasks: [], open: false });
});

describe("LibraryPanel", () => {
  it("renders nothing when the library tab isn't active", () => {
    sidebarTabStore.getState().setActiveTab("model");
    listLibraryComponents.mockResolvedValue([]);
    render(<LibraryPanel />);
    expect(screen.queryByLabelText("Search library components")).toBeNull();
  });

  it("shows an honest empty state when nothing is indexed", async () => {
    listLibraryComponents.mockResolvedValue([]);
    render(<LibraryPanel />);
    await waitFor(() =>
      expect(screen.getByText(/No components indexed yet/)).toBeInTheDocument(),
    );
  });

  it("surfaces a read failure distinctly from an empty library", async () => {
    listLibraryComponents.mockRejectedValue(new Error("io"));
    render(<LibraryPanel />);
    await waitFor(() =>
      expect(screen.getByText(/Could not read the component library/)).toBeInTheDocument(),
    );
  });

  it("lists indexed components as cards", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    render(<LibraryPanel />);
    await waitFor(() => expect(screen.getAllByTestId("library-card")).toHaveLength(1));
    expect(screen.getByText("Socket Head Cap Screw")).toBeInTheDocument();
  });

  it("filters cards by the search query (name, id, category, tags)", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    render(<LibraryPanel />);
    await waitFor(() => expect(screen.getAllByTestId("library-card")).toHaveLength(1));

    fireEvent.change(screen.getByLabelText("Search library components"), {
      target: { value: "shcs" },
    });
    expect(screen.getAllByTestId("library-card")).toHaveLength(1);

    fireEvent.change(screen.getByLabelText("Search library components"), {
      target: { value: "bearing" },
    });
    expect(screen.queryByTestId("library-card")).toBeNull();
    expect(screen.getByText(/No matches for/)).toBeInTheDocument();
  });

  it("clicking a card arms placement; clicking the armed card cancels it", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    const { rerender } = render(<LibraryPanel />);
    await waitFor(() => expect(screen.getAllByTestId("library-card")).toHaveLength(1));

    fireEvent.click(screen.getByTestId("library-card"));
    expect(armPlacementMock).toHaveBeenCalledWith(iso4762);
    expect(cancelPlacementMock).not.toHaveBeenCalled();

    armedKeyForTest = "onecad.std.iso4762@1.0.0";
    rerender(<LibraryPanel />);
    fireEvent.click(screen.getByTestId("library-card"));
    expect(cancelPlacementMock).toHaveBeenCalledTimes(1);
  });

  it("reindex reloads the list", async () => {
    listLibraryComponents.mockResolvedValueOnce([]).mockResolvedValueOnce([iso4762]);
    render(<LibraryPanel />);
    await waitFor(() => expect(screen.getByText(/No components indexed yet/)).toBeInTheDocument());

    fireEvent.click(screen.getByLabelText("Reindex library"));
    await waitFor(() => expect(reindexLibrary).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getAllByTestId("library-card")).toHaveLength(1));
    expect(listLibraryComponents).toHaveBeenCalledTimes(2);
  });

  it("WP-1.6: reindex drives the tasks chip begin→end (its first real producer)", async () => {
    listLibraryComponents.mockResolvedValue([]);
    let resolveReindex: (() => void) | undefined;
    reindexLibrary.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveReindex = () => resolve({ total: 0, indexed: 0, skipped: [] });
        }),
    );
    render(<LibraryPanel />);
    await waitFor(() => expect(screen.getByText(/No components indexed yet/)).toBeInTheDocument());

    fireEvent.click(screen.getByLabelText("Reindex library"));
    await waitFor(() =>
      expect(tasksStore.getState().tasks.some((t) => t.id === "library.reindex")).toBe(true),
    );

    resolveReindex?.();
    await waitFor(() =>
      expect(tasksStore.getState().tasks.some((t) => t.id === "library.reindex")).toBe(false),
    );
  });
});
