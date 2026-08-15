/*
 * `LibraryModal` — the merged grid/search/detail/preview surface (replaces
 * the old sidebar `LibraryPanel` and the start-screen `ComponentDetails`
 * card). Coverage ported from `LibraryPanel.test.tsx` (states, search,
 * reindex, tasks-chip) plus the detail/place flow `ComponentDetails` used to
 * own.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LibraryModal } from "./LibraryModal";
import { tasksStore } from "@/stores/tasksStore";
import type { LibraryComponent } from "@/ipc/types";

const listLibraryComponents = vi.fn<() => Promise<LibraryComponent[]>>();
const reindexLibrary = vi.fn(() => Promise.resolve({ total: 0, indexed: 0, skipped: [] }));
vi.mock("@/ipc/client", () => ({
  createClient: () => ({ listLibraryComponents, detachComponent: vi.fn(), reindexLibrary }),
}));

// The real controller reaches `getViewportEngine()`/window listeners — none of
// which exist in jsdom. This modal's own contract is just "Place arms the
// gesture and closes"; the gesture itself is proven by
// `placementSolver.test.ts` (math) and manual browser verification (viewport).
const armPlacementMock = vi.fn();
vi.mock("@/modules/library/placementController", () => ({
  armPlacement: (c: LibraryComponent) => armPlacementMock(c),
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
  },
  parameters: {
    thread: { role: "free", key: "M6", domain: ["M3", "M4", "M5", "M6"] },
  },
};

beforeEach(() => {
  listLibraryComponents.mockReset();
  reindexLibrary.mockClear();
  armPlacementMock.mockReset();
  tasksStore.setState({ tasks: [], open: false });
});

describe("LibraryModal", () => {
  it("renders nothing when closed", () => {
    listLibraryComponents.mockResolvedValue([]);
    render(<LibraryModal open={false} onClose={vi.fn()} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows an honest empty state when nothing is indexed", async () => {
    listLibraryComponents.mockResolvedValue([]);
    render(<LibraryModal open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/No components indexed yet/)).toBeInTheDocument());
  });

  it("surfaces a read failure distinctly from an empty library", async () => {
    listLibraryComponents.mockRejectedValue(new Error("io"));
    render(<LibraryModal open onClose={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByText(/Could not read the component library/)).toBeInTheDocument(),
    );
  });

  it("lists indexed components as cards", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    render(<LibraryModal open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getAllByTestId("library-modal-card")).toHaveLength(1));
    expect(screen.getByText("Socket Head Cap Screw")).toBeInTheDocument();
  });

  it("filters cards by the search query (name, id, category, tags)", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    render(<LibraryModal open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getAllByTestId("library-modal-card")).toHaveLength(1));

    fireEvent.change(screen.getByLabelText("Search library components"), {
      target: { value: "bearing" },
    });
    expect(screen.queryByTestId("library-modal-card")).toBeNull();
    expect(screen.getByText(/No matches for/)).toBeInTheDocument();
  });

  it("selecting a card shows its detail + preview", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    render(<LibraryModal open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByTestId("library-modal-card")).toBeInTheDocument());

    fireEvent.click(screen.getByTestId("library-modal-card"));
    expect(screen.getByText("onecad.std.iso4762")).toBeInTheDocument();
  });

  /*
   * AMENDED (LGU-1 WP-A, canvas D2-A): the "needs a project" sentence became a
   * DISABLED PLACE BUTTON reading "Place — open a project first". Same refusal,
   * said where the user is looking for the action rather than as a note beside
   * its absence.
   */
  it("canPlace unset: the Place action is present but disabled, and says why", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    render(<LibraryModal open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByTestId("library-modal-card")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("library-modal-card"));

    const place = screen.getByTestId("detail-place-disabled");
    expect(place).toHaveTextContent("Place — open a project first");
    expect(place).toBeDisabled();
    expect(screen.queryByText("Place in scene")).toBeNull();
  });

  it("canPlace set: Place arms the gesture and closes the modal", async () => {
    listLibraryComponents.mockResolvedValue([iso4762]);
    const onClose = vi.fn();
    render(<LibraryModal open onClose={onClose} canPlace />);
    await waitFor(() => expect(screen.getByTestId("library-modal-card")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("library-modal-card"));

    fireEvent.click(screen.getByText("Place in scene"));
    expect(armPlacementMock).toHaveBeenCalledWith(iso4762);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("close button calls onClose", async () => {
    listLibraryComponents.mockResolvedValue([]);
    const onClose = vi.fn();
    render(<LibraryModal open onClose={onClose} />);
    await waitFor(() => expect(screen.getByLabelText("Close library")).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText("Close library"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Escape calls onClose", async () => {
    listLibraryComponents.mockResolvedValue([]);
    const onClose = vi.fn();
    render(<LibraryModal open onClose={onClose} />);
    await waitFor(() => expect(screen.getByRole("dialog")).toBeInTheDocument());
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  /*
   * AMENDED (LGU-1 WP-A, defect F9): "Reindex" was a developer verb holding the
   * most prominent slot beside Search. The action survives unchanged — same
   * call, same tasks-chip begin/end — behind the header overflow as
   * "Rebuild index".
   */
  it("rebuilding the index reloads the list and drives the tasks chip begin→end", async () => {
    listLibraryComponents.mockResolvedValueOnce([]).mockResolvedValueOnce([iso4762]);
    render(<LibraryModal open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/No components indexed yet/)).toBeInTheDocument());

    expect(screen.queryByLabelText("Reindex library")).toBeNull();
    fireEvent.click(screen.getByLabelText("Library options"));
    fireEvent.click(screen.getByTestId("library-rebuild-index"));
    await waitFor(() =>
      expect(tasksStore.getState().tasks.some((t) => t.id === "library.reindex")).toBe(true),
    );
    await waitFor(() => expect(screen.getAllByTestId("library-modal-card")).toHaveLength(1));
    await waitFor(() =>
      expect(tasksStore.getState().tasks.some((t) => t.id === "library.reindex")).toBe(false),
    );
    expect(listLibraryComponents).toHaveBeenCalledTimes(2);
  });
});
