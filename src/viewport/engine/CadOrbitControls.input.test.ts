/*
 * Behavioural tests for the CadOrbitControls EVENT layer.
 *
 * The routing logic itself is covered by navInput.test.ts; what is verified here
 * is the adapter — that DOM events reach the reducer, that the returned ops move
 * the camera in the right direction, and that the gating seams are honoured.
 *
 * jsdom has no PointerEvent, but the handlers only read plain properties, so
 * MouseEvent/WheelEvent stand in (the same trick SketchController.select.test.ts
 * uses).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";
import { CadOrbitControls, isMacPlatform } from "./CadOrbitControls";
import { CameraRig } from "./CameraRig";
import type { DevicePref } from "./navInput";

interface Harness {
  controls: CadOrbitControls;
  el: HTMLElement;
  pref: { value: DevicePref };
  dragActive: { value: boolean };
  devices: string[];
}

function setup(
  pref: DevicePref = "auto",
  getBounds: () => THREE.Box3 | null = () => null,
): Harness {
  const el = document.createElement("div");
  Object.defineProperty(el, "clientHeight", { value: 800, configurable: true });
  Object.defineProperty(el, "clientWidth", { value: 1000, configurable: true });
  el.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: 1000, height: 800, right: 1000, bottom: 800 }) as DOMRect;
  document.body.appendChild(el);

  const prefRef = { value: pref };
  const dragActive = { value: false };
  const devices: string[] = [];

  const controls = new CadOrbitControls({
    rig: new CameraRig(76),
    element: el,
    onChange: () => {},
    getBounds,
    getDevicePref: () => prefRef.value,
    isDragActive: () => dragActive.value,
    onDeviceChange: (d) => devices.push(d),
  });
  controls.applyToRig();
  return { controls, el, pref: prefRef, dragActive, devices };
}

function wheel(
  el: HTMLElement,
  init: Partial<WheelEventInit> & { deltaY?: number; deltaX?: number } = {},
): void {
  el.dispatchEvent(
    new WheelEvent("wheel", {
      deltaX: 0,
      deltaY: 0,
      deltaMode: 0,
      clientX: 500,
      clientY: 400,
      bubbles: true,
      cancelable: true,
      ...init,
    }),
  );
}

/** A trackpad-shaped burst: fractional, fast, slight horizontal drift. */
function padScroll(el: HTMLElement, n: number, init: Partial<WheelEventInit> = {}): void {
  for (let i = 0; i < n; i++) wheel(el, { deltaX: 0.5, deltaY: 2.5 + i * 0.25, ...init });
}

describe("CadOrbitControls — wheel routing", () => {
  let h: Harness;
  beforeEach(() => {
    h = setup();
  });

  it("dollies on a mouse wheel", () => {
    const before = h.controls.getDistance();
    wheel(h.el, { deltaY: 120 });
    expect(h.controls.getDistance()).toBeGreaterThan(before);
  });

  it("scrolling up moves the camera closer", () => {
    const before = h.controls.getDistance();
    wheel(h.el, { deltaY: -120 });
    expect(h.controls.getDistance()).toBeLessThan(before);
  });

  it("normalizes DOM_DELTA_LINE instead of barely moving", () => {
    // The pre-existing bug: deltaY=3 in line mode gave exp(0.0045) ≈ 1.004.
    const before = h.controls.getDistance();
    wheel(h.el, { deltaY: 3, deltaMode: 1 });
    const ratio = h.controls.getDistance() / before;
    expect(ratio).toBeGreaterThan(1.05);
  });

  it("pans on a trackpad two-finger scroll", () => {
    h.pref.value = "trackpad";
    const before = h.controls.getTarget().clone();
    padScroll(h.el, 3);
    expect(h.controls.getTarget().distanceTo(before)).toBeGreaterThan(0);
  });

  it("does not change distance while panning", () => {
    h.pref.value = "trackpad";
    const before = h.controls.getDistance();
    padScroll(h.el, 3);
    expect(h.controls.getDistance()).toBeCloseTo(before, 6);
  });

  it("orbits on shift + two-finger scroll", () => {
    h.pref.value = "trackpad";
    const yaw = h.controls.yaw;
    const pitch = h.controls.pitch;
    padScroll(h.el, 3, { shiftKey: true });
    expect(h.controls.yaw !== yaw || h.controls.pitch !== pitch).toBe(true);
  });

  it("does not move the target while orbiting", () => {
    h.pref.value = "trackpad";
    const before = h.controls.getTarget().clone();
    padScroll(h.el, 3, { shiftKey: true });
    expect(h.controls.getTarget().distanceTo(before)).toBeCloseTo(0, 9);
  });

  it("keeps pitch clamped inside the poles under sustained orbit", () => {
    h.pref.value = "trackpad";
    for (let i = 0; i < 200; i++) wheel(h.el, { deltaX: 0.5, deltaY: -40, shiftKey: true });
    expect(Math.abs(h.controls.pitch)).toBeLessThan(Math.PI / 2);
  });

  it("calls preventDefault so the page never scrolls", () => {
    const ev = new WheelEvent("wheel", { deltaY: 120, cancelable: true, bubbles: true });
    const spy = vi.spyOn(ev, "preventDefault");
    h.el.dispatchEvent(ev);
    expect(spy).toHaveBeenCalled();
  });
});

describe("CadOrbitControls — drag gating", () => {
  it("suppresses wheel-orbit while a tool drag is in flight", () => {
    const h = setup("trackpad");
    h.dragActive.value = true;
    const yaw = h.controls.yaw;
    const pitch = h.controls.pitch;
    padScroll(h.el, 3, { shiftKey: true });
    expect(h.controls.yaw).toBe(yaw);
    expect(h.controls.pitch).toBe(pitch);
  });

  it("still pans while a tool drag is in flight", () => {
    // Only orbit is hazardous: it changes the projection the drag is measured against.
    const h = setup("trackpad");
    h.dragActive.value = true;
    const before = h.controls.getTarget().clone();
    padScroll(h.el, 3);
    expect(h.controls.getTarget().distanceTo(before)).toBeGreaterThan(0);
  });

  it("orbits again once the drag ends", () => {
    const h = setup("trackpad");
    h.dragActive.value = true;
    padScroll(h.el, 2, { shiftKey: true });
    h.dragActive.value = false;
    const yaw = h.controls.yaw;
    padScroll(h.el, 2, { shiftKey: true });
    expect(h.controls.yaw).not.toBe(yaw);
  });

  it("does not let the ARMED-tool orbit suppression block a wheel orbit", () => {
    // lmbOrbitSuppressed stops an LMB DRAG from orbiting; looking around with
    // the trackpad while a tool is armed must keep working.
    const h = setup("trackpad");
    h.controls.setLmbOrbitSuppressed(true);
    const yaw = h.controls.yaw;
    padScroll(h.el, 3, { shiftKey: true });
    expect(h.controls.yaw).not.toBe(yaw);
  });
});

describe("CadOrbitControls — device reporting", () => {
  it("reports a device change exactly once per transition", () => {
    const h = setup();
    padScroll(h.el, 5);
    padScroll(h.el, 5);
    expect(h.devices).toEqual(["trackpad"]);
  });

  it("reports the flip back to mouse", () => {
    const h = setup();
    padScroll(h.el, 5);
    for (let i = 0; i < 4; i++) wheel(h.el, { deltaY: 120, deltaMode: 1 });
    expect(h.devices).toEqual(["trackpad", "mouse"]);
  });
});

describe("CadOrbitControls — macOS trackpad orbit direction", () => {
  // jsdom's navigator.platform is "" and its UA carries "(darwin)", so the
  // default environment reads as non-mac; mac cases pin the platform explicitly.
  const setPlatform = (value: string): void => {
    Object.defineProperty(window.navigator, "platform", { value, configurable: true });
  };
  afterEach(() => setPlatform(""));

  it("isMacPlatform matches WKWebView/Safari platform strings", () => {
    expect(isMacPlatform({ platform: "MacIntel", userAgent: "" })).toBe(true);
    expect(isMacPlatform({ platform: "Win32", userAgent: "" })).toBe(false);
    expect(
      isMacPlatform({ platform: "", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X)" }),
    ).toBe(true);
  });

  it("reverses shift+scroll orbit on macOS (drag-direction)", () => {
    setPlatform("MacIntel");
    const h = setup("trackpad");
    const pitch = h.controls.pitch;
    padScroll(h.el, 3, { shiftKey: true }); // deltaY > 0
    expect(h.controls.pitch).toBeLessThan(pitch);
  });

  it("keeps scroll-direction orbit off macOS", () => {
    const h = setup("trackpad");
    const pitch = h.controls.pitch;
    padScroll(h.el, 3, { shiftKey: true });
    expect(h.controls.pitch).toBeGreaterThan(pitch);
  });

  it("leaves mouse wheel zoom untouched on macOS", () => {
    setPlatform("MacIntel");
    const h = setup("mouse");
    const before = h.controls.getDistance();
    wheel(h.el, { deltaY: 120 });
    expect(h.controls.getDistance()).toBeGreaterThan(before);
  });

  it("leaves LMB-drag orbit direction untouched on macOS", () => {
    setPlatform("MacIntel");
    Object.assign(HTMLElement.prototype, {
      setPointerCapture: () => {},
      releasePointerCapture: () => {},
      hasPointerCapture: () => false,
    });
    const h = setup();
    const yaw = h.controls.yaw;
    h.el.dispatchEvent(
      new MouseEvent("pointerdown", { button: 0, clientX: 500, clientY: 400, bubbles: true }),
    );
    h.el.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 560, clientY: 400, bubbles: true }),
    );
    // dx > 0 ⇒ yaw decreases — same as off-mac; the flip is wheel-orbit only.
    expect(h.controls.yaw).toBeLessThan(yaw);
  });
});

describe("CadOrbitControls — pointer buttons", () => {
  function down(el: HTMLElement, button: number, x = 500, y = 400): void {
    const ev = new MouseEvent("pointerdown", { button, clientX: x, clientY: y, bubbles: true });
    el.dispatchEvent(ev);
  }
  function move(el: HTMLElement, x: number, y: number): void {
    el.dispatchEvent(new MouseEvent("pointermove", { clientX: x, clientY: y, bubbles: true }));
  }

  beforeEach(() => {
    // setPointerCapture is not in jsdom.
    Object.assign(HTMLElement.prototype, {
      setPointerCapture: () => {},
      releasePointerCapture: () => {},
      hasPointerCapture: () => false,
    });
  });

  it("orbits on a left-button drag", () => {
    const h = setup();
    const yaw = h.controls.yaw;
    down(h.el, 0);
    move(h.el, 560, 400);
    expect(h.controls.yaw).not.toBe(yaw);
  });

  it("pans on a middle-button drag", () => {
    const h = setup();
    const before = h.controls.getTarget().clone();
    const yaw = h.controls.yaw;
    down(h.el, 1);
    move(h.el, 560, 400);
    expect(h.controls.getTarget().distanceTo(before)).toBeGreaterThan(0);
    expect(h.controls.yaw).toBe(yaw);
  });

  it("pans on a right-button drag rather than orbiting", () => {
    const h = setup();
    const before = h.controls.getTarget().clone();
    const yaw = h.controls.yaw;
    down(h.el, 2);
    move(h.el, 560, 400);
    expect(h.controls.getTarget().distanceTo(before)).toBeGreaterThan(0);
    expect(h.controls.yaw).toBe(yaw);
  });

  it("suppresses the context menu", () => {
    const h = setup();
    const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    h.el.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
  });
});

/*
 * fitView(bounds?) — zoom-to-fit vs zoom-to-SELECTION (W3). The optional
 * argument is the only difference: same sphere-fit math, a different box.
 */
describe("CadOrbitControls — fitView bounds argument", () => {
  const box = (min: [number, number, number], max: [number, number, number]) =>
    new THREE.Box3(new THREE.Vector3(...min), new THREE.Vector3(...max));

  /** Run the fit tween to completion so target/distance are the final values. */
  const settle = (h: Harness) => h.controls.update(performance.now() + 10_000);

  it("frames the passed bounds, NOT the whole scene", () => {
    const scene = box([-100, -100, -100], [100, 100, 100]);
    const h = setup("auto", () => scene);

    h.controls.fitView(box([40, 40, 0], [60, 60, 20]));
    settle(h);

    expect(h.controls.getTarget().toArray()).toEqual([50, 50, 10]);
    const selectionDistance = h.controls.getDistance();

    h.controls.fitView(); // no argument → whole scene
    settle(h);
    expect(h.controls.getTarget().toArray()).toEqual([0, 0, 0]);
    expect(h.controls.getDistance()).toBeGreaterThan(selectionDistance);
  });

  it("empty bounds fall back to the default framed view at the origin", () => {
    const h = setup("auto", () => box([-10, -10, -10], [10, 10, 10]));
    h.controls.fitView(new THREE.Box3()); // makeEmpty()
    settle(h);
    expect(h.controls.getTarget().toArray()).toEqual([0, 0, 0]);
    expect(h.controls.getDistance()).toBeGreaterThan(0);
  });
});

describe("CadOrbitControls — teardown", () => {
  it("stops responding to wheel events after dispose", () => {
    const h = setup();
    h.controls.dispose();
    const before = h.controls.getDistance();
    wheel(h.el, { deltaY: 120 });
    expect(h.controls.getDistance()).toBe(before);
  });
});
