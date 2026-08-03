/*
 * The armed HOLE cluster (WP-C T3) — render + dispatch.
 *
 * The rule under test is CONDITIONAL RENDERING: only the active profile's
 * dimension pair may be on screen. This is not cosmetic — a field that is visible
 * implies a field that is in the record, and SCHEMA §7.3 makes carrying the OTHER
 * profile's block a rejected edit. A cluster that rendered both would invite the
 * user to author something the backend refuses.
 *
 * Same portal harness as `ModelToolChips.test.tsx`: a fake engine hosts the chip
 * in the document so the portaled controls are queryable.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import { ModelToolChips } from "./ModelToolChips";
import { toolChipStore, type HoleChipHandlers } from "@/stores/toolChipStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";

const WORLD: [number, number, number] = [0, 0, 0];

function fakeEngine(): ViewportEngine {
  return {
    mountChip: (_id: string, el: HTMLElement) => document.body.appendChild(el),
    unmountChip: (_id: string, el: HTMLElement) => el.remove(),
  } as unknown as ViewportEngine;
}

function handlers(): HoleChipHandlers {
  return {
    onValue: vi.fn(),
    onHoleType: vi.fn(),
    onDepth: vi.fn(),
    onCbDiameter: vi.fn(),
    onCbDepth: vi.fn(),
    onCsDiameter: vi.fn(),
    onCsAngle: vi.fn(),
    onStandard: vi.fn(),
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };
}

describe("HoleChipCluster", () => {
  let h: HoleChipHandlers;

  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
    h = handlers();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  it("a simple hole shows the profile segments, Ø, depth and the standards button — and NO conditional pair", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showHole(6.6, WORLD, h, { holeType: "simple", depth: null }));

    expect(screen.getByTestId("chip-hole-simple")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByLabelText("Dimension value")).toHaveValue("6.6");
    // Through-all reads as a word, never as a blank field.
    expect(screen.getByTestId("chip-hole-depth")).toHaveValue("Thru");
    expect(screen.getByTestId("chip-hole-std")).toBeInTheDocument();
    expect(screen.getByTestId("chip-confirm")).toBeInTheDocument();

    expect(screen.queryByTestId("chip-hole-cb-diameter")).toBeNull();
    expect(screen.queryByTestId("chip-hole-cs-diameter")).toBeNull();
    expect(screen.queryByTestId("chip-hole-cs-90")).toBeNull();
  });

  it("a counterbore shows ONLY the cb pair", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showHole(6.6, WORLD, h, {
        holeType: "counterbore",
        depth: 20,
        cbDiameter: 11,
        cbDepth: 6.8,
      }),
    );
    expect(screen.getByTestId("chip-hole-counterbore")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-hole-cb-diameter")).toHaveValue("11");
    expect(screen.getByTestId("chip-hole-cb-depth")).toHaveValue("6.8");
    expect(screen.getByTestId("chip-hole-depth")).toHaveValue("20");
    expect(screen.queryByTestId("chip-hole-cs-diameter")).toBeNull();
    expect(screen.queryByTestId("chip-hole-cs-90")).toBeNull();
  });

  it("a countersink shows ONLY the cs pair, with the four SCHEMA angles as segments", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showHole(6.6, WORLD, h, {
        holeType: "countersink",
        depth: 20,
        csDiameter: 12.4,
        csAngleDeg: 90,
      }),
    );
    expect(screen.getByTestId("chip-hole-cs-diameter")).toHaveValue("12.4");
    for (const a of [82, 90, 100, 120]) {
      expect(screen.getByTestId(`chip-hole-cs-${a}`)).toBeInTheDocument();
    }
    // A free-text angle would let the user author a value the backend refuses,
    // so the control is a segmented pick and 90° is the seeded one.
    expect(screen.getByTestId("chip-hole-cs-90")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-hole-cs-82")).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByTestId("chip-hole-cb-diameter")).toBeNull();
  });

  it("dispatches the profile flip, the angle pick and ✓/✕", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore
        .getState()
        .showHole(6.6, WORLD, h, { holeType: "countersink", csDiameter: 12.4, csAngleDeg: 90 }),
    );
    fireEvent.click(screen.getByTestId("chip-hole-counterbore"));
    expect(h.onHoleType).toHaveBeenCalledWith("counterbore");
    fireEvent.click(screen.getByTestId("chip-hole-cs-82"));
    expect(h.onCsAngle).toHaveBeenCalledWith(82);
    fireEvent.click(screen.getByTestId("chip-confirm"));
    expect(h.onConfirm).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("chip-cancel"));
    expect(h.onCancel).toHaveBeenCalled();
  });

  it("the depth field authors a blind depth, and 'Thru' takes it back to through-all", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showHole(6.6, WORLD, h, { depth: null }));
    const depth = screen.getByTestId("chip-hole-depth");

    fireEvent.change(depth, { target: { value: "14" } });
    fireEvent.blur(depth);
    expect(h.onDepth).toHaveBeenCalledWith(14);

    act(() => toolChipStore.getState().showHole(6.6, WORLD, h, { depth: 14 }));
    const again = screen.getByTestId("chip-hole-depth");
    fireEvent.change(again, { target: { value: "thru" } });
    fireEvent.blur(again);
    expect(h.onDepth).toHaveBeenCalledWith(null);

    // A value the length parser refuses REVERTS rather than authoring a partial.
    (h.onDepth as ReturnType<typeof vi.fn>).mockClear();
    fireEvent.change(again, { target: { value: "abc" } });
    fireEvent.blur(again);
    expect(h.onDepth).not.toHaveBeenCalled();
    expect(again).toHaveValue("14");
  });

  it("the standards popover is closed until asked, and a pick reports thread + fit", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showHole(6.6, WORLD, h, { holeType: "counterbore" }));
    expect(screen.queryByTestId("chip-hole-std-panel")).toBeNull();

    fireEvent.click(screen.getByTestId("chip-hole-std"));
    expect(screen.getByTestId("chip-hole-std-panel")).toBeInTheDocument();
    // The whole M3…M12 range is offered in both ISO 273 series.
    for (const t of ["M3", "M4", "M5", "M6", "M8", "M10", "M12"]) {
      expect(screen.getByTestId(`chip-hole-std-${t}-close`)).toBeInTheDocument();
      expect(screen.getByTestId(`chip-hole-std-${t}-normal`)).toBeInTheDocument();
    }

    fireEvent.click(screen.getByTestId("chip-hole-std-M6-normal"));
    expect(h.onStandard).toHaveBeenCalledWith("M6", "normal");
    // The pick closes the popover — it is a one-shot filler, not a mode.
    expect(screen.queryByTestId("chip-hole-std-panel")).toBeNull();
  });

  it("a cb dimension edit dispatches the raw millimetres", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore
        .getState()
        .showHole(6.6, WORLD, h, { holeType: "counterbore", cbDiameter: 11, cbDepth: 6.8 }),
    );
    const cbd = screen.getByTestId("chip-hole-cb-diameter");
    fireEvent.change(cbd, { target: { value: "15" } });
    fireEvent.blur(cbd);
    expect(h.onCbDiameter).toHaveBeenCalledWith(15);
  });
});
