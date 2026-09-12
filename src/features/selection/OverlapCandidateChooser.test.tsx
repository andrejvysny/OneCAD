import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useRef } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { resetStores } from "@/test/resetStores";
import type { MeshEntry } from "@/viewport/mesh/meshRegistry";
import type { ProbeCandidate } from "@/viewport/engine/Picker";
import { OverlapCandidateChooser } from "./OverlapCandidateChooser";

const mocks = vi.hoisted(() => ({
  installedProofIsCurrent: vi.fn(() => true),
  promoteOne: vi.fn(),
  stalePickHint: vi.fn(),
}));

vi.mock("@/ipc/client", () => ({ createClient: vi.fn(() => ({})) }));
vi.mock("@/ipc/promote", () => mocks);

const entry = {} as MeshEntry;

function candidate(kind: ProbeCandidate["kind"], topoKey: string): ProbeCandidate {
  return {
    bodyId: "body1",
    kind,
    topoKey,
    distance: 1,
    worldPos: { x: 1, y: 2, z: 3 } as ProbeCandidate["worldPos"],
    entry,
  };
}

function Harness({
  candidates,
  meshEpoch = 0,
  chrome = false,
}: {
  candidates: readonly ProbeCandidate[];
  meshEpoch?: number;
  chrome?: boolean;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const engine = { probeCandidates: vi.fn(() => candidates) } as never;
  return (
    <div ref={containerRef} data-testid="viewport" tabIndex={-1}>
      {chrome && <button type="button" data-viewport-interactive>Viewport chrome</button>}
      <OverlapCandidateChooser containerRef={containerRef} engine={engine} meshEpoch={meshEpoch} />
    </div>
  );
}

function openChooser(): void {
  const viewport = screen.getByTestId("viewport");
  fireEvent.pointerDown(viewport, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
  fireEvent.pointerUp(viewport, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
}

describe("OverlapCandidateChooser", () => {
  beforeEach(() => {
    resetStores();
    selectionStore.getState().clear();
    toolStore.getState().setMode("model");
    toolStore.setState({ modelTool: "select" });
    mocks.installedProofIsCurrent.mockReset();
    mocks.installedProofIsCurrent.mockReturnValue(true);
    mocks.promoteOne.mockReset();
    mocks.stalePickHint.mockReset();
  });

  it("opens only from an unmodified stationary right-button release", () => {
    render(<Harness candidates={[candidate("body", "body1")]} />);
    const viewport = screen.getByTestId("viewport");

    fireEvent.pointerDown(viewport, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();
    fireEvent.pointerMove(viewport, { button: 2, pointerId: 3, clientX: 80, clientY: 50 });
    fireEvent.pointerMove(viewport, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
    fireEvent.pointerUp(viewport, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();

    fireEvent.pointerDown(viewport, { button: 2, pointerId: 4, altKey: true, clientX: 40, clientY: 50 });
    fireEvent.pointerUp(viewport, { button: 2, pointerId: 4, altKey: true, clientX: 40, clientY: 50 });
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();

    openChooser();
    expect(screen.getByRole("dialog", { name: "Select overlapping geometry" })).toBeInTheDocument();
  });

  it("does not open on pointer cancellation or outside model select", () => {
    render(<Harness candidates={[candidate("body", "body1")]} />);
    const viewport = screen.getByTestId("viewport");
    fireEvent.pointerDown(viewport, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
    fireEvent.pointerCancel(viewport, { pointerId: 3 });
    fireEvent.pointerUp(viewport, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();

    act(() => toolStore.setState({ modelTool: "measure" }));
    openChooser();
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();
  });

  it("ignores pointer streams that begin in viewport-interactive chrome", () => {
    render(<Harness candidates={[candidate("body", "body1")]} chrome />);
    const chrome = screen.getByRole("button", { name: "Viewport chrome" });
    fireEvent.pointerDown(chrome, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });
    fireEvent.pointerUp(chrome, { button: 2, pointerId: 3, clientX: 40, clientY: 50 });

    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();
  });

  it("filters, previews, cycles, and dismisses without changing selection", async () => {
    render(<Harness candidates={[candidate("body", "body1"), candidate("face", "f:0")]} />);
    openChooser();
    screen.getByRole("dialog", { name: "Select overlapping geometry" });

    await waitFor(() => expect(selectionStore.getState().hover?.kind).toBe("body"));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    await waitFor(() => expect(document.activeElement).toHaveTextContent("Face · Body 1"));
    await waitFor(() => expect(selectionStore.getState().hover?.kind).toBe("face"));
    fireEvent.click(screen.getByRole("button", { name: "Body" }));
    expect(screen.queryByRole("button", { name: "Body · Body 1" })).toBeNull();
    expect(selectionStore.getState().selected).toEqual([]);

    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();
    expect(selectionStore.getState().hover).toBeNull();
    await waitFor(() => expect(screen.getByTestId("viewport")).toHaveFocus());
  });

  it("focuses hovered candidates and retains filter focus when removing the active candidate", () => {
    render(<Harness candidates={[candidate("body", "body1"), candidate("face", "f:0")]} />);
    openChooser();
    const dialog = screen.getByRole("dialog", { name: "Select overlapping geometry" });
    const bodyFilter = within(dialog).getByRole("button", { name: /^Body$/ });
    const faceFilter = within(dialog).getByRole("button", { name: /^Face$/ });
    const faceCandidate = within(dialog).getByRole("button", { name: "Face · Body 1" });

    fireEvent.pointerEnter(faceCandidate);
    expect(document.activeElement).toBe(faceCandidate);
    fireEvent.click(bodyFilter);
    expect(document.activeElement).toBe(bodyFilter);
    fireEvent.click(faceFilter);
    expect(document.activeElement).toBe(faceFilter);
  });

  it("commits a body without IPC only after proof validation", () => {
    render(<Harness candidates={[candidate("body", "body1")]} />);
    openChooser();
    fireEvent.keyDown(document.activeElement!, { key: "Enter" });

    expect(mocks.promoteOne).not.toHaveBeenCalled();
    expect(selectionStore.getState().selected).toEqual([{ kind: "body", id: "body1" }]);
  });

  it("single-flights face promotion and waits to change selection", async () => {
    let resolvePromotion: ((value: { elementId: string }) => void) | undefined;
    mocks.promoteOne.mockReturnValue(new Promise((resolve) => { resolvePromotion = resolve; }));
    render(<Harness candidates={[candidate("face", "f:0")]} />);
    openChooser();
    fireEvent.keyDown(document.activeElement!, { key: "Enter" });
    fireEvent.keyDown(document.activeElement!, { key: "Enter" });

    expect(mocks.promoteOne).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Face · Body 1" })).toBeDisabled();
    expect(selectionStore.getState().selected).toEqual([]);
    act(() => documentStore.setState((state) => ({ revision: state.revision + 1, title: "Projection metadata" })));
    expect(screen.getByRole("dialog", { name: "Select overlapping geometry" })).toBeInTheDocument();
    resolvePromotion?.({ elementId: "el_face" });

    await waitFor(() => expect(selectionStore.getState().selected).toMatchObject([
      { kind: "face", bodyId: "body1", topoKey: "f:0", elementId: "el_face" },
    ]));
  });

  it("does not let an old promotion dismiss a newer chooser session", async () => {
    let resolvePromotion: ((value: { elementId: string }) => void) | undefined;
    mocks.promoteOne.mockReturnValue(new Promise((resolve) => { resolvePromotion = resolve; }));
    const view = render(<Harness candidates={[candidate("face", "f:0")]} />);
    openChooser();
    fireEvent.keyDown(document.activeElement!, { key: "Enter" });
    mocks.installedProofIsCurrent.mockReturnValue(false);
    act(() => documentStore.setState((state) => ({ revision: state.revision + 1 })));
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();

    mocks.installedProofIsCurrent.mockReturnValue(true);
    view.rerender(<Harness candidates={[candidate("body", "body1")]} />);
    openChooser();
    await waitFor(() => expect(selectionStore.getState().hover?.kind).toBe("body"));
    resolvePromotion?.({ elementId: "el_face" });

    await waitFor(() => expect(screen.getByRole("dialog", { name: "Select overlapping geometry" })).toBeInTheDocument());
    expect(selectionStore.getState().hover?.kind).toBe("body");
  });

  it("closes on proof invalidation and unmount clears only its preview", async () => {
    const view = render(<Harness candidates={[candidate("body", "body1")]} />);
    openChooser();
    await waitFor(() => expect(selectionStore.getState().hover?.kind).toBe("body"));
    mocks.installedProofIsCurrent.mockReturnValue(false);
    act(() => documentStore.setState((state) => ({ revision: state.revision + 1 })));
    expect(screen.queryByRole("dialog", { name: "Select overlapping geometry" })).toBeNull();

    mocks.installedProofIsCurrent.mockReturnValue(true);
    openChooser();
    await waitFor(() => expect(selectionStore.getState().hover?.kind).toBe("body"));
    view.unmount();
    expect(selectionStore.getState().hover).toBeNull();
  });
});
