/*
 * The pure half of model-tool gesture ownership: which pointer owns a gesture,
 * when an event from another pointer is foreign, when a mouse sample proves the
 * release was missed, and when a cancel may put the pre-grab value back.
 *
 * jsdom is why the null rules exist: a synthetic `MouseEvent` has no `pointerId`
 * at all and a default `PointerEvent` reports pointerId 0 and pointerType "", so
 * a gesture started from either must keep working with the other.
 */
import { describe, it, expect } from "vitest";
import {
  beginModelGesture,
  cancelsLiveGesture,
  gestureOwnerOf,
  isMissedRelease,
  normalizePointerId,
  pointerMatches,
  restoreAllowed,
  supersedesLostContact,
  type GestureOwner,
} from "./modelGesture";

const mouse = (pointerId: number | null = 1): GestureOwner => ({ pointerId, pointerType: "mouse" });

describe("normalizePointerId", () => {
  it("keeps a numeric id, including 0", () => {
    expect(normalizePointerId({ pointerId: 7 })).toBe(7);
    expect(normalizePointerId({ pointerId: 0 })).toBe(0);
  });

  it("maps a missing or non-finite id to null", () => {
    expect(normalizePointerId({})).toBeNull();
    expect(normalizePointerId({ pointerId: undefined })).toBeNull();
    expect(normalizePointerId({ pointerId: Number.NaN })).toBeNull();
  });

  it("reads a real jsdom MouseEvent (no pointerId) as null", () => {
    expect(normalizePointerId(new MouseEvent("pointermove") as unknown as PointerEvent)).toBeNull();
  });
});

describe("pointerMatches", () => {
  it("matches the same id and refuses a different one", () => {
    expect(pointerMatches(1, 1)).toBe(true);
    expect(pointerMatches(1, 2)).toBe(false);
  });

  it("treats null on either side as unknown, which matches", () => {
    expect(pointerMatches(null, 3)).toBe(true);
    expect(pointerMatches(3, null)).toBe(true);
    expect(pointerMatches(null, null)).toBe(true);
  });
});

describe("gestureOwnerOf", () => {
  it("reads id and type, treating an empty type as unknown", () => {
    expect(gestureOwnerOf({ pointerId: 3, pointerType: "touch" })).toEqual({ pointerId: 3, pointerType: "touch" });
    expect(gestureOwnerOf({ pointerId: 0, pointerType: "" })).toEqual({ pointerId: 0, pointerType: null });
    expect(gestureOwnerOf(null)).toEqual({ pointerId: null, pointerType: null });
  });
});

describe("isMissedRelease", () => {
  it("is true for a mouse or pen sample with the primary bit clear", () => {
    expect(isMissedRelease({ pointerType: "mouse", buttons: 0 })).toBe(true);
    // Chorded: primary released while the secondary is still held.
    expect(isMissedRelease({ pointerType: "mouse", buttons: 2 })).toBe(true);
    expect(isMissedRelease({ pointerType: "pen", buttons: 0 })).toBe(true); // lifted into hover
    expect(isMissedRelease({ pointerType: "mouse", buttons: 1 })).toBe(false);
    expect(isMissedRelease({ pointerType: "mouse", buttons: 3 })).toBe(false);
    expect(isMissedRelease({ pointerType: "pen", buttons: 3 })).toBe(false); // barrel button on contact
  });

  it("never fires for touch or a synthetic event with no pointer type", () => {
    expect(isMissedRelease({ pointerType: "touch", buttons: 0 })).toBe(false);
    expect(isMissedRelease({ pointerType: "", buttons: 0 })).toBe(false);
    expect(isMissedRelease({ buttons: 0 })).toBe(false);
  });

  it("does not guess when buttons is absent", () => {
    expect(isMissedRelease({ pointerType: "mouse" })).toBe(false);
  });
});

describe("supersedesLostContact", () => {
  const touchGesture = () => beginModelGesture("offsetFace", { pointerId: 21, pointerType: "touch" }, 1, {});

  it("a new primary contact of the same type under another id proves the owner lifted", () => {
    expect(supersedesLostContact(touchGesture(), { pointerId: 22, pointerType: "touch", isPrimary: true })).toBe(true);
  });

  it("is not evidence when non-primary, another type, the same id, or anything is unknown", () => {
    const g = touchGesture();
    expect(supersedesLostContact(g, { pointerId: 22, pointerType: "touch", isPrimary: false })).toBe(false);
    expect(supersedesLostContact(g, { pointerId: 22, pointerType: "pen", isPrimary: true })).toBe(false);
    expect(supersedesLostContact(g, { pointerId: 21, pointerType: "touch", isPrimary: true })).toBe(false);
    expect(supersedesLostContact(g, { pointerType: "touch", isPrimary: true })).toBe(false);
    expect(supersedesLostContact(null, { pointerId: 22, pointerType: "touch", isPrimary: true })).toBe(false);
    const synthetic = beginModelGesture("extrude", { pointerId: null, pointerType: null }, 1, {});
    expect(supersedesLostContact(synthetic, { pointerId: 22, pointerType: "touch", isPrimary: true })).toBe(false);
  });

  it("a released gesture is superseded by nothing", () => {
    const g = touchGesture();
    g.released = true;
    expect(supersedesLostContact(g, { pointerId: 22, pointerType: "touch", isPrimary: true })).toBe(false);
  });
});

describe("cancelsLiveGesture", () => {
  it("cancels a live gesture for its own pointer", () => {
    const g = beginModelGesture("extrude", mouse(1), 4, { depth: 10 });
    expect(cancelsLiveGesture(g, 1)).toBe(true);
    expect(cancelsLiveGesture(g, null)).toBe(true);
  });

  it("ignores a foreign pointer, no gesture, and a gesture already released", () => {
    const g = beginModelGesture("fillet", mouse(1), 4, { radius: 2 });
    expect(cancelsLiveGesture(g, 2)).toBe(false);
    expect(cancelsLiveGesture(null, 1)).toBe(false);
    g.released = true;
    expect(cancelsLiveGesture(g, 1)).toBe(false);
  });
});

describe("restoreAllowed", () => {
  it("restores only for the arm that grabbed and only while still dragging", () => {
    const g = beginModelGesture("shell", mouse(null), 9, { thickness: 2 });
    expect(restoreAllowed(g, 9, true)).toBe(true);
    expect(restoreAllowed(g, 10, true)).toBe(false);
    expect(restoreAllowed(g, 9, false)).toBe(false);
  });

  it("records the grab-time facts verbatim", () => {
    const g = beginModelGesture("transform", { pointerId: 5, pointerType: "pen" }, 2, { translate: [1, 2, 3] });
    expect(g).toEqual({
      kind: "transform",
      pointerId: 5,
      pointerType: "pen",
      armGen: 2,
      snapshot: { translate: [1, 2, 3] },
      released: false,
    });
  });
});
