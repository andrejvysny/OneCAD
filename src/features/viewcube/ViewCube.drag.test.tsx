/*
 * Drag-to-orbit on the ViewCube.
 *
 * A press that moves more than DRAG_THRESHOLD px orbits the camera (ungated
 * turntable, same math as RMB+Shift in the viewport) and suppresses the face
 * click so a drag never also snaps. A press that does not move stays a click
 * and snaps to that face as before.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import * as THREE from "three";
import { ViewCube } from "./ViewCube";
import { setViewportEngine } from "@/viewport/engineBridge";

type EngineLike = {
  getCameraQuaternion: ReturnType<typeof vi.fn>;
  onCameraChanged: ReturnType<typeof vi.fn>;
  snapToViewDirection: ReturnType<typeof vi.fn>;
  orbitBy: ReturnType<typeof vi.fn>;
  cancelViewAnimation: ReturnType<typeof vi.fn>;
};

const identity = new THREE.Quaternion();

function mockEngine(): EngineLike {
  return {
    getCameraQuaternion: vi.fn((out: THREE.Quaternion) => out.copy(identity)),
    onCameraChanged: vi.fn(() => () => {}),
    snapToViewDirection: vi.fn(),
    orbitBy: vi.fn(),
    cancelViewAnimation: vi.fn(),
  };
}

/** Dispatch a native pointer event (jsdom lacks PointerEvent; handlers only
 *  read clientX/clientY, so a MouseEvent of the right type stands in). */
function pointer(target: Element | Window, type: string, x: number, y: number): void {
  target.dispatchEvent(
    new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y }),
  );
}

describe("ViewCube drag-to-orbit", () => {
  let engine: EngineLike;

  beforeEach(() => {
    engine = mockEngine();
    setViewportEngine(engine as never);
  });

  afterEach(() => {
    setViewportEngine(null);
    vi.restoreAllMocks();
  });

  it("orbits while dragging a face past the threshold", () => {
    render(<ViewCube />);
    const front = screen.getByRole("button", { name: "FRONT view" });

    pointer(front, "pointerdown", 100, 100);
    pointer(window, "pointermove", 120, 105);
    pointer(window, "pointerup", 120, 105);

    expect(engine.cancelViewAnimation).toHaveBeenCalled();
    expect(engine.orbitBy).toHaveBeenCalledWith(20, 5);
  });

  it("accumulates deltas across moves and ignores sub-threshold presses", () => {
    render(<ViewCube />);
    const front = screen.getByRole("button", { name: "FRONT view" });

    pointer(front, "pointerdown", 100, 100);
    pointer(window, "pointermove", 102, 102); // 2.8px — under threshold
    pointer(window, "pointermove", 120, 106); // crosses threshold → starts
    pointer(window, "pointermove", 130, 108);
    pointer(window, "pointerup", 130, 108);

    expect(engine.orbitBy).toHaveBeenNthCalledWith(1, 18, 4); // first crossing delta
    expect(engine.orbitBy).toHaveBeenNthCalledWith(2, 10, 2);
  });

  it("suppresses the face click after a drag (no snap)", () => {
    render(<ViewCube />);
    const front = screen.getByRole("button", { name: "FRONT view" });

    pointer(front, "pointerdown", 100, 100);
    pointer(window, "pointermove", 130, 100);
    pointer(window, "pointerup", 130, 100);
    fireEvent.click(front);

    expect(engine.orbitBy).toHaveBeenCalled();
    expect(engine.snapToViewDirection).not.toHaveBeenCalled();
  });

  it("plain click still snaps to the face", () => {
    render(<ViewCube />);
    const front = screen.getByRole("button", { name: "FRONT view" });

    pointer(front, "pointerdown", 100, 100);
    pointer(window, "pointerup", 100, 100);
    fireEvent.click(front);

    expect(engine.orbitBy).not.toHaveBeenCalled();
    expect(engine.snapToViewDirection).toHaveBeenCalledTimes(1);
  });
});
