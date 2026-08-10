/*
 * LiveDimChips + LiveDimField (SP-1 Wave 3) — the chip overlay and the one
 * editable field it portals into each engine-owned host.
 *
 * A fake engine attaches every host to `document.body` so the portaled inputs are
 * queryable (the ModelToolChips.test.tsx pattern). The controller seam is faked
 * too: tests drive `liveDimStore.show(...)` with vi.fn handlers and assert the
 * field parses, validates and dispatches exactly what the controller's FSM
 * expects — mm for a length, degrees for an angle, a clamped integer for a count.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import { LiveDimChips } from "./LiveDimChips";
import {
  liveDimStore,
  placementFor,
  type LiveDimAnchors,
  type LiveDimChipField,
  type LiveDimHandlers,
} from "@/stores/liveDimStore";
import { toolStore } from "@/stores/toolStore";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";

const mounted = new Map<string, HTMLElement>();

function fakeEngine(): ViewportEngine {
  return {
    mountChip: vi.fn((id: string, el: HTMLElement) => {
      mounted.set(id, el);
      document.body.appendChild(el);
    }),
    moveChip: vi.fn(),
    unmountChip: (id: string, el: HTMLElement) => {
      mounted.delete(id);
      el.remove();
    },
  } as unknown as ViewportEngine;
}

function chip(over: Partial<LiveDimChipField> = {}): LiveDimChipField {
  return {
    field: "length",
    label: "L",
    domain: "length",
    value: 30,
    locked: false,
    drives: true,
    ...over,
  };
}

const ANCHORS: LiveDimAnchors = {
  length: [0, 0, 0],
  angle: [1, 0, 0],
  radius: [0, 1, 0],
  sides: [0, 0, 1],
  major: [1, 1, 0],
  width: [2, 0, 0],
  height: [0, 2, 0],
};

function makeHandlers(): LiveDimHandlers & Record<string, ReturnType<typeof vi.fn>> {
  return {
    onFocus: vi.fn(),
    onText: vi.fn(),
    onTab: vi.fn(),
    onEnter: vi.fn(),
    onEscape: vi.fn(),
    onBlur: vi.fn(),
  } as unknown as LiveDimHandlers & Record<string, ReturnType<typeof vi.fn>>;
}

describe("placementFor", () => {
  it("keeps the default up-and-right quadrant in open space", () => {
    expect(placementFor(400, 400, 1000, 800)).toBe("tr");
  });

  it("flips LEFT past the right-edge margin", () => {
    expect(placementFor(800, 400, 1000, 800)).toBe("tl");
  });

  it("flips DOWN near the top edge", () => {
    expect(placementFor(400, 100, 1000, 800)).toBe("br");
  });

  it("flips both in the top-right corner", () => {
    expect(placementFor(900, 100, 1000, 800)).toBe("bl");
  });

  it("keeps the default rather than guessing in a zero-sized viewport", () => {
    expect(placementFor(0, 0, 0, 0)).toBe("tr");
  });
});

describe("LiveDimChips", () => {
  let handlers: ReturnType<typeof makeHandlers>;
  let engine: ViewportEngine;

  beforeEach(() => {
    resetStores();
    mounted.clear();
    engine = fakeEngine();
    setViewportEngine(engine);
    toolStore.getState().setMode("sketch", "sketch1");
    handlers = makeHandlers();
  });

  afterEach(() => {
    setViewportEngine(null);
    liveDimStore.getState().clear();
    mounted.clear();
  });

  const show = (fields: LiveDimChipField[]): void => {
    act(() => liveDimStore.getState().show(fields, ANCHORS, handlers));
  };

  it("renders nothing while the chip set is closed", () => {
    render(<LiveDimChips />);
    expect(screen.queryByTestId("live-dim-length")).toBeNull();
  });

  it("renders nothing outside sketch mode, even with an open set", () => {
    toolStore.getState().setMode("model");
    render(<LiveDimChips />);
    show([chip()]);
    expect(screen.queryByTestId("live-dim-length")).toBeNull();
  });

  it("mounts ONE engine-hosted chip per field, and unmounts them on clear", () => {
    render(<LiveDimChips />);
    // drives: false here (an arc sweep, say) — mount/unmount is what this
    // test is about; the drives+angle corner-angle display transform has its
    // own dedicated tests below.
    show([chip(), chip({ field: "angle", label: "∠", domain: "angle", value: 30, drives: false })]);

    expect([...mounted.keys()]).toEqual(["__live_dim_length", "__live_dim_angle"]);
    expect(screen.getByTestId("live-dim-length")).toHaveValue("30");
    expect(screen.getByTestId("live-dim-angle")).toHaveValue("30");

    act(() => liveDimStore.getState().clear());
    expect([...mounted.keys()]).toEqual([]);
    expect(screen.queryByTestId("live-dim-length")).toBeNull();
  });

  it("gives each field its own overlay cluster unless the frame opted two in together", () => {
    render(<LiveDimChips />);
    show([
      chip({ field: "length", clusterId: "segment-len-ang" }),
      chip({ field: "angle", label: "∠", domain: "angle", value: 30, clusterId: "segment-len-ang" }),
    ]);

    const calls = (engine.mountChip as ReturnType<typeof vi.fn>).mock.calls;
    const clusterOf = (id: string) => calls.find((c) => c[0] === id)?.[3]?.clusterId;
    // Opted-in siblings (length+angle on a segment) share the frame's cluster id.
    expect(clusterOf("__live_dim_length")).toBe("segment-len-ang");
    expect(clusterOf("__live_dim_angle")).toBe("segment-len-ang");
  });

  it("a field with no clusterId is its own cluster of one, not lumped with siblings", () => {
    render(<LiveDimChips />);
    // A box's width/height carry no clusterId (they sit at different edges).
    show([chip({ field: "width", label: "W" }), chip({ field: "height", label: "H" })]);

    const calls = (engine.mountChip as ReturnType<typeof vi.fn>).mock.calls;
    const clusterOf = (id: string) => calls.find((c) => c[0] === id)?.[3]?.clusterId;
    const widthCluster = clusterOf("__live_dim_width");
    const heightCluster = clusterOf("__live_dim_height");
    expect(widthCluster).not.toBe(heightCluster);
  });

  it("names each field for assistive tech without colliding with DimensionInput", () => {
    render(<LiveDimChips />);
    show([chip()]);
    expect(screen.getByLabelText("Live length")).toBe(screen.getByTestId("live-dim-length"));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
  });

  it("an UNFOCUSED field reads the live value; a FOCUSED one reads the typed text", () => {
    render(<LiveDimChips />);
    show([chip()]);
    expect(screen.getByTestId("live-dim-length")).toHaveValue("30");

    act(() => liveDimStore.getState().setFocus("length", "5"));
    expect(screen.getByTestId("live-dim-length")).toHaveValue("5");

    // The value keeps moving underneath — the typed text must survive it.
    act(() => liveDimStore.getState().update([chip({ value: 41.7 })], ANCHORS));
    expect(screen.getByTestId("live-dim-length")).toHaveValue("5");
  });

  it("a keystroke relays through onText", () => {
    render(<LiveDimChips />);
    show([chip()]);
    act(() => liveDimStore.getState().setFocus("length", "5"));
    fireEvent.change(screen.getByTestId("live-dim-length"), { target: { value: "50" } });
    expect(handlers.onText).toHaveBeenCalledWith("50");
  });

  it("Enter commits MILLIMETRES, whatever the display unit", () => {
    render(<LiveDimChips />);
    show([chip()]);
    act(() => liveDimStore.getState().setFocus("length", "2 in"));
    fireEvent.keyDown(screen.getByTestId("live-dim-length"), { key: "Enter" });
    expect(handlers.onEnter).toHaveBeenCalledWith(50.8);
  });

  it("Enter on a value the field refuses flashes instead of committing", () => {
    render(<LiveDimChips />);
    show([chip()]);
    act(() => liveDimStore.getState().setFocus("length", "0"));
    const input = screen.getByTestId("live-dim-length");
    fireEvent.keyDown(input, { key: "Enter" });

    expect(handlers.onEnter).not.toHaveBeenCalled();
    expect(input).toHaveAttribute("aria-invalid", "true");
  });

  it("Tab cycles the chip set (never the DOM tab order) and carries Shift", () => {
    render(<LiveDimChips />);
    show([chip()]);
    act(() => liveDimStore.getState().setFocus("length", "50"));
    const evt = fireEvent.keyDown(screen.getByTestId("live-dim-length"), {
      key: "Tab",
      shiftKey: true,
    });

    expect(handlers.onTab).toHaveBeenCalledWith(true, 50);
    expect(evt).toBe(false); // preventDefault ran
  });

  it("an EMPTY field Tabs on without pinning anything", () => {
    render(<LiveDimChips />);
    show([chip()]);
    act(() => liveDimStore.getState().setFocus("length", ""));
    fireEvent.keyDown(screen.getByTestId("live-dim-length"), { key: "Tab" });
    expect(handlers.onTab).toHaveBeenCalledWith(false, null);
  });

  it("Escape relays to the controller's field ladder", () => {
    render(<LiveDimChips />);
    show([chip()]);
    act(() => liveDimStore.getState().setFocus("length", "50"));
    fireEvent.keyDown(screen.getByTestId("live-dim-length"), { key: "Escape" });
    expect(handlers.onEscape).toHaveBeenCalled();
  });

  it("blur locks the parsed value without committing the gesture", () => {
    render(<LiveDimChips />);
    show([chip()]);
    act(() => liveDimStore.getState().setFocus("length", "50"));
    fireEvent.blur(screen.getByTestId("live-dim-length"));
    expect(handlers.onBlur).toHaveBeenCalledWith("length", 50);
    expect(handlers.onEnter).not.toHaveBeenCalled();
  });

  it("an arc SWEEP refuses 0 and 360", () => {
    const sweep = chip({ field: "angle", label: "∠", domain: "angle", value: 90, drives: false });
    render(<LiveDimChips />);
    show([sweep]);

    act(() => liveDimStore.getState().setFocus("angle", "360"));
    fireEvent.keyDown(screen.getByTestId("live-dim-angle"), { key: "Enter" });
    expect(handlers.onEnter).not.toHaveBeenCalled();
  });

  it("a chained leg's angle chip DISPLAYS the visual corner angle, not the stored turn", () => {
    render(<LiveDimChips />);
    // Stored turn = 71° (matches the dashed arc's own 180−71=109° sweep —
    // see `angleArcPreview.ts`'s `arcPreviewSweep`).
    show([chip({ field: "angle", label: "∠", domain: "angle", value: 71, drives: true })]);
    expect(screen.getByTestId("live-dim-angle")).toHaveValue("109");
  });

  it("the corner-angle field renders as a plain annotation, not a pill chip", () => {
    render(<LiveDimChips />);
    show([
      chip(), // length — keeps the pill
      chip({ field: "angle", label: "∠", domain: "angle", value: 71, drives: true }),
    ]);

    const lengthWrapper = screen.getByTestId("live-dim-length").parentElement!;
    expect(lengthWrapper.className).toMatch(/rounded-full/);
    expect(screen.getByText("L")).toBeInTheDocument(); // length keeps its glyph label

    const angleWrapper = screen.getByTestId("live-dim-angle").parentElement!;
    expect(angleWrapper.className).not.toMatch(/rounded-full/);
    expect(angleWrapper.className).not.toMatch(/border/);
    expect(angleWrapper.className).not.toMatch(/bg-surface/);
    expect(screen.queryByText("∠")).not.toBeInTheDocument(); // no glyph label — it sits inside the arc itself
  });

  it("typing a new corner angle keeps the CURRENT turn's sign", () => {
    render(<LiveDimChips />);
    show([chip({ field: "angle", label: "∠", domain: "angle", value: 71, drives: true })]);
    // Positive stored turn ⇒ typing "90" (a square corner) resolves to +90.
    act(() => liveDimStore.getState().setFocus("angle", "90"));
    fireEvent.keyDown(screen.getByTestId("live-dim-angle"), { key: "Enter" });
    expect(handlers.onEnter).toHaveBeenCalledWith(90);
  });

  it("a NEGATIVE stored turn's sign carries into a freshly-typed corner angle", () => {
    render(<LiveDimChips />);
    show([chip({ field: "angle", label: "∠", domain: "angle", value: -71, drives: true })]);
    expect(screen.getByTestId("live-dim-angle")).toHaveValue("109"); // |−71| ⇒ same corner
    act(() => liveDimStore.getState().setFocus("angle", "150"));
    fireEvent.keyDown(screen.getByTestId("live-dim-angle"), { key: "Enter" });
    expect(handlers.onEnter).toHaveBeenCalledWith(-30); // sign −1 × (180 − 150)
  });

  it("a side count is CLAMPED, not refused", () => {
    render(<LiveDimChips />);
    show([chip({ field: "sides", label: "n", domain: "count", value: 6, drives: false })]);
    act(() => liveDimStore.getState().setFocus("sides", "40"));
    fireEvent.keyDown(screen.getByTestId("live-dim-sides"), { key: "Enter" });
    expect(handlers.onEnter).toHaveBeenCalledWith(12);
  });

  it("a field that authors NO constraint says so rather than pretending", () => {
    render(<LiveDimChips />);
    show([chip({ field: "major", label: "a", drives: false })]);
    expect(screen.getByTitle("Sets the geometry — no constraint is created")).toBeInTheDocument();
  });

  it("a unit switch re-displays the same mm — it never re-commits", () => {
    render(<LiveDimChips />);
    show([chip({ value: 25.4 })]);
    expect(screen.getByTestId("live-dim-length")).toHaveValue("25.4");

    act(() => settingsStore.getState().setDisplayUnit("in"));
    expect(screen.getByTestId("live-dim-length")).toHaveValue("1");
    expect(handlers.onEnter).not.toHaveBeenCalled();
    expect(handlers.onText).not.toHaveBeenCalled();
  });
});

describe("liveDimStore", () => {
  beforeEach(() => resetStores());

  it("update() no-ops a move below the float-noise floor", () => {
    const handlers = makeHandlers();
    liveDimStore.getState().show([chip()], ANCHORS, handlers);
    const before = liveDimStore.getState().fields;

    liveDimStore.getState().update([chip({ value: 30 + 1e-12 })], ANCHORS);
    expect(liveDimStore.getState().fields).toBe(before); // same reference ⇒ no render

    liveDimStore.getState().update([chip({ value: 31 })], ANCHORS);
    expect(liveDimStore.getState().fields).not.toBe(before);
  });

  it("update() never overwrites the FOCUSED field's value", () => {
    liveDimStore.getState().show([chip(), chip({ field: "angle", value: 10 })], ANCHORS, makeHandlers());
    liveDimStore.getState().setFocus("length", "50");

    liveDimStore.getState().update([chip({ value: 99 }), chip({ field: "angle", value: 20 })], ANCHORS);
    const s = liveDimStore.getState();
    expect(s.fields.find((f) => f.field === "length")!.value).toBe(30); // held
    expect(s.fields.find((f) => f.field === "angle")!.value).toBe(20); // live
  });

  it("update() cannot resurrect a closed set", () => {
    liveDimStore.getState().show([chip()], ANCHORS, makeHandlers());
    liveDimStore.getState().clear();
    liveDimStore.getState().update([chip()], ANCHORS);
    expect(liveDimStore.getState().open).toBe(false);
    expect(liveDimStore.getState().fields).toEqual([]);
  });
});
