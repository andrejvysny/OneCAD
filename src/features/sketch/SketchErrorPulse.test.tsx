import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { SketchErrorPulse, PULSE_DISMISS_MS } from "./SketchErrorPulse";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

/** jsdom has no PointerEvent constructor in every version we run on; a
 *  MouseEvent dispatched under the pointer type carries the same clientX/Y the
 *  component reads. */
function movePointer(x: number, y: number): void {
  act(() => {
    window.dispatchEvent(
      new MouseEvent("pointermove", { clientX: x, clientY: y, bubbles: true }),
    );
  });
}

const pulse = () => screen.queryByTestId("sketch-error-pulse");

describe("SketchErrorPulse", () => {
  beforeEach(() => {
    resetStores();
    act(() => toolStore.getState().setMode("sketch"));
  });
  afterEach(() => vi.useRealTimers());

  it("renders an error hint near the last pointer position", () => {
    render(<SketchErrorPulse />);
    movePointer(400, 300);
    act(() =>
      viewportStore.getState().setStatusHint("Dimension edit reverted", { severity: "error" }),
    );

    const el = pulse();
    expect(el).toBeInTheDocument();
    expect(el).toHaveTextContent("Dimension edit reverted");
    // Anchored down-right of the action point, not parked in a corner.
    expect(Number.parseFloat(el!.style.left)).toBeGreaterThan(400);
    expect(Number.parseFloat(el!.style.top)).toBeGreaterThan(300);
  });

  it("never intercepts a click aimed at the canvas underneath", () => {
    render(<SketchErrorPulse />);
    act(() => viewportStore.getState().setStatusHint("Sketch edit failed", { severity: "error" }));
    expect(pulse()).toHaveClass("pointer-events-none");
  });

  it("stays away for info hints (prompts and success confirmations)", () => {
    render(<SketchErrorPulse />);
    act(() => viewportStore.getState().setStatusHint("Finished Sketch 2 — 4 entities"));
    expect(pulse()).toBeNull();

    act(() => viewportStore.getState().setStatusHint("Click the opposite corner", { sticky: true }));
    expect(pulse()).toBeNull();
  });

  it("stays away in model mode — this is sketch-scope feedback", () => {
    act(() => toolStore.getState().setMode("model"));
    render(<SketchErrorPulse />);
    act(() => viewportStore.getState().setStatusHint("Extrude failed: boom", { severity: "error" }));
    expect(pulse()).toBeNull();
  });

  it("auto-dismisses, leaving the status-bar copy as the only survivor", () => {
    vi.useFakeTimers();
    render(<SketchErrorPulse />);
    act(() =>
      viewportStore.getState().setStatusHint("Reference geometry is locked", { severity: "error", sticky: true }),
    );
    expect(pulse()).toBeInTheDocument();

    act(() => void vi.advanceTimersByTime(PULSE_DISMISS_MS + 1));
    expect(pulse()).toBeNull();
    // Sticky hint: the corner still says it. Only the near-action copy expired.
    expect(viewportStore.getState().statusHint?.message).toBe("Reference geometry is locked");
  });

  it("re-pulses on a REPEATED identical error (the hint sequence, not the text)", () => {
    render(<SketchErrorPulse />);
    const emit = () =>
      act(() =>
        viewportStore.getState().setStatusHint("Constraint removed — it would over-constrain the sketch", {
          severity: "error",
        }),
      );
    emit();
    const first = pulse();
    emit();
    const second = pulse();
    expect(second).toBeInTheDocument();
    // A remount (new node), which is what replays the one-shot CSS animation.
    expect(second).not.toBe(first);
  });

  it("clears when the refusal is superseded by an ordinary prompt", () => {
    render(<SketchErrorPulse />);
    act(() => viewportStore.getState().setStatusHint("Sketch edit failed: boom", { severity: "error" }));
    expect(pulse()).toBeInTheDocument();

    act(() => viewportStore.getState().setStatusHint("Click a line, circle, arc, or two points"));
    expect(pulse()).toBeNull();
  });

  it("clears on leaving sketch mode", () => {
    render(<SketchErrorPulse />);
    act(() => viewportStore.getState().setStatusHint("Sketch edit failed: boom", { severity: "error" }));
    expect(pulse()).toBeInTheDocument();

    act(() => toolStore.getState().setMode("model"));
    expect(pulse()).toBeNull();
  });
});
