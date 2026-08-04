/*
 * RepairMarkerOverlay (H9) — the world-anchored crosshair for the hovered repair
 * candidate.
 *
 * `repairStore.hoveredWorldPos` was a data seam with ZERO consumers until this
 * component; these tests pin that it is actually consumed, that the chip is
 * registered/unregistered through the ENGINE's overlay registry (not positioned
 * in React), and that it never eats a pick.
 */
import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";
import { render, act, screen } from "@testing-library/react";
import { RepairMarkerOverlay } from "./RepairMarkerOverlay";
import { repairStore } from "@/stores/repairStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { resetStores } from "@/test/resetStores";

/**
 * A stand-in for the engine's overlay registry that does what the real one does
 * with the chip host: PARENT it (the engine appends into its chip layer). Without
 * that the portal target stays detached and the chip is invisible to queries —
 * which is exactly the failure mode a no-op spy would hide.
 */
function fakeEngine() {
  const mountChip = vi.fn((_id: string, el: HTMLElement, _world: [number, number, number]) => {
    document.body.appendChild(el);
  });
  const unmountChip = vi.fn((_id: string, el: HTMLElement) => {
    el.remove();
  });
  return {
    engine: { mountChip, unmountChip } as unknown as ViewportEngine,
    mountChip,
    unmountChip,
  };
}

beforeEach(() => resetStores());
afterEach(() => act(() => setViewportEngine(null)));

describe("RepairMarkerOverlay", () => {
  it("renders nothing until a candidate is hovered", () => {
    const { engine } = fakeEngine();
    act(() => setViewportEngine(engine));
    render(<RepairMarkerOverlay />);
    expect(screen.queryByTestId("repair-marker")).not.toBeInTheDocument();
  });

  it("mounts a chip at the hovered world position through the engine registry", () => {
    const { engine, mountChip } = fakeEngine();
    act(() => setViewportEngine(engine));
    render(<RepairMarkerOverlay />);

    act(() => repairStore.getState().setHoveredWorldPos([12, 3.5, 0]));
    expect(mountChip).toHaveBeenCalledTimes(1);
    const [id, el, world] = mountChip.mock.calls[0];
    expect(id).toBe("repair-marker");
    expect(world).toEqual([12, 3.5, 0]);
    // Positioning is the ENGINE's job — React only owns the chip's content.
    expect(el).toBeInstanceOf(HTMLElement);
    expect(screen.getByTestId("repair-marker")).toBeInTheDocument();
  });

  it("re-registers when the anchor MOVES (the engine caches the world point at mount)", () => {
    const { engine, mountChip, unmountChip } = fakeEngine();
    act(() => setViewportEngine(engine));
    render(<RepairMarkerOverlay />);
    act(() => repairStore.getState().setHoveredWorldPos([1, 2, 3]));
    act(() => repairStore.getState().setHoveredWorldPos([4, 5, 6]));
    expect(unmountChip).toHaveBeenCalled();
    expect(mountChip).toHaveBeenCalledTimes(2);
    expect(mountChip.mock.calls[1][2]).toEqual([4, 5, 6]);
  });

  it("unmounts the chip when the hover clears (leaving the panel / choosing)", () => {
    const { engine, unmountChip } = fakeEngine();
    act(() => setViewportEngine(engine));
    render(<RepairMarkerOverlay />);
    act(() => repairStore.getState().setHoveredWorldPos([1, 2, 3]));
    act(() => repairStore.getState().setHoveredWorldPos(null));
    expect(unmountChip).toHaveBeenCalledWith("repair-marker", expect.any(HTMLElement));
    expect(screen.queryByTestId("repair-marker")).not.toBeInTheDocument();
  });

  it("closing the repair panel clears the marker (the store drops the hover)", () => {
    const { engine, unmountChip } = fakeEngine();
    act(() => setViewportEngine(engine));
    render(<RepairMarkerOverlay />);
    act(() => repairStore.getState().setHoveredWorldPos([1, 2, 3]));
    act(() => repairStore.getState().closePanel());
    expect(unmountChip).toHaveBeenCalled();
  });

  it("is pointer-events-none — a marker must never steal a pick from the model", () => {
    const { engine } = fakeEngine();
    act(() => setViewportEngine(engine));
    render(<RepairMarkerOverlay />);
    act(() => repairStore.getState().setHoveredWorldPos([1, 2, 3]));
    const inner = screen.getByTestId("repair-marker").firstElementChild;
    expect(inner?.className).toContain("pointer-events-none");
  });

  it("survives having no engine yet (chrome mounts before the viewport initializes)", () => {
    render(<RepairMarkerOverlay />);
    expect(() =>
      act(() => repairStore.getState().setHoveredWorldPos([1, 2, 3])),
    ).not.toThrow();
  });
});
